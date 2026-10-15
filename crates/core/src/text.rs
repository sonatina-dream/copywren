//! Small text helpers shared by classification, cards and search.

/// Maximum characters kept in `Item::preview_text`.
pub const PREVIEW_LIMIT: usize = 2_000;

/// First `PREVIEW_LIMIT` characters of `text`.
pub fn preview(text: &str) -> String {
    match text.char_indices().nth(PREVIEW_LIMIT) {
        Some((byte_index, _)) => text[..byte_index].to_string(),
        None => text.to_string(),
    }
}

/// "1 file", "3 files", "1,204 characters".
pub fn count_label(count: i64, noun: &str) -> String {
    let number = group_thousands(count);
    if count == 1 {
        format!("{number} {noun}")
    } else {
        format!("{number} {noun}s")
    }
}

pub fn group_thousands(value: i64) -> String {
    let digits = value.unsigned_abs().to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (i, ch) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(ch);
    }
    if value < 0 {
        out.insert(0, '-');
    }
    out
}

/// The start of `text` that fits in `rows` display rows of about
/// `chars_per_row` characters, counting wrapped paragraphs. Tabs become four
/// spaces. Used to keep card previews from showing a half-cut last line.
pub fn fit_rows(text: &str, rows: usize, chars_per_row: usize) -> String {
    let chars_per_row = chars_per_row.max(1);
    let mut out = String::new();
    let mut used = 0;
    for paragraph in text.split('\n') {
        let paragraph = paragraph.replace('\t', "    ");
        let len = paragraph.chars().count();
        let needed = len.div_ceil(chars_per_row).max(1);
        if used + needed <= rows {
            if used > 0 {
                out.push('\n');
            }
            out.push_str(&paragraph);
            used += needed;
            if used == rows {
                break;
            }
        } else {
            let remaining = rows - used;
            if remaining > 0 {
                if used > 0 {
                    out.push('\n');
                }
                out.extend(paragraph.chars().take(remaining * chars_per_row));
            }
            break;
        }
    }
    out
}

const URL_SCHEMES: &[&str] = &["http://", "https://", "ftp://", "mailto:"];

/// True when `text` is exactly one URL with a supported scheme.
pub fn is_single_url(text: &str) -> bool {
    if text.is_empty() || text.chars().any(char::is_whitespace) {
        return false;
    }
    let lower = text.to_ascii_lowercase();
    let Some(scheme) = URL_SCHEMES.iter().find(|s| lower.starts_with(**s)) else {
        return false;
    };
    let rest = &text[scheme.len()..];
    if *scheme == "mailto:" {
        return rest.contains('@');
    }
    let host = rest.split(['/', '?', '#']).next().unwrap_or_default();
    !host.is_empty() && !host.starts_with(':')
}

/// Host part of an http(s)/ftp URL, without a `www.` prefix.
pub fn link_host(url: &str) -> Option<&str> {
    let lower = url.to_ascii_lowercase();
    let scheme = URL_SCHEMES
        .iter()
        .filter(|s| **s != "mailto:")
        .find(|s| lower.starts_with(**s))?;
    let rest = &url[scheme.len()..];
    let authority = rest.split(['/', '?', '#']).next()?;
    let host = authority.rsplit('@').next()?;
    let host = host.split(':').next()?;
    let host = host.strip_prefix("www.").unwrap_or(host);
    (!host.is_empty()).then_some(host)
}

/// Decodes `%XX` escapes; invalid escapes are kept verbatim.
pub fn percent_decode(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            let hex = std::str::from_utf8(&bytes[i + 1..i + 3]).ok();
            if let Some(value) = hex.and_then(|h| u8::from_str_radix(h, 16).ok()) {
                out.push(value);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Converts a `file://` URL to a POSIX path.
pub fn file_url_to_path(url: &str) -> Option<String> {
    let rest = url.strip_prefix("file://")?;
    // `file://localhost/path` and `file:///path` are both valid.
    let rest = rest.strip_prefix("localhost").unwrap_or(rest);
    if !rest.starts_with('/') {
        return None;
    }
    let path = percent_decode(rest);
    let trimmed = path.trim_end_matches('/');
    Some(if trimmed.is_empty() {
        "/".into()
    } else {
        trimmed.into()
    })
}

/// Last path component, for file cards.
pub fn file_name(path: &str) -> &str {
    path.rsplit('/').find(|c| !c.is_empty()).unwrap_or(path)
}

/// Best-effort plain text from RTF, used only when a clip has no plain string.
pub fn rtf_to_text(rtf: &str) -> String {
    let mut out = String::new();
    let mut chars = rtf.chars().peekable();
    // Depth of groups whose text must be skipped ({\*\...}, fonttbl, colortbl...).
    let mut depth = 0usize;
    let mut skip_from: Option<usize> = None;
    let mut unicode_skip = 0usize;
    while let Some(ch) = chars.next() {
        match ch {
            '{' => depth += 1,
            '}' => {
                if skip_from == Some(depth) {
                    skip_from = None;
                }
                depth = depth.saturating_sub(1);
            }
            '\\' => {
                let Some(&next) = chars.peek() else { break };
                if next == '\'' {
                    chars.next();
                    let hex: String = chars.by_ref().take(2).collect();
                    if skip_from.is_none()
                        && unicode_skip == 0
                        && let Ok(byte) = u8::from_str_radix(&hex, 16)
                    {
                        out.push(byte as char);
                    }
                    unicode_skip = unicode_skip.saturating_sub(1);
                    continue;
                }
                if !next.is_ascii_alphabetic() {
                    chars.next();
                    if skip_from.is_none() {
                        match next {
                            '*' => skip_from = Some(depth),
                            '\\' | '{' | '}' => out.push(next),
                            '\n' | '\r' => out.push('\n'),
                            '~' => out.push('\u{a0}'),
                            _ => {}
                        }
                    }
                    continue;
                }
                let mut word = String::new();
                while let Some(&c) = chars.peek() {
                    if c.is_ascii_alphabetic() {
                        word.push(c);
                        chars.next();
                    } else {
                        break;
                    }
                }
                let mut param = String::new();
                while let Some(&c) = chars.peek() {
                    if c.is_ascii_digit() || (c == '-' && param.is_empty()) {
                        param.push(c);
                        chars.next();
                    } else {
                        break;
                    }
                }
                if chars.peek() == Some(&' ') {
                    chars.next();
                }
                if skip_from.is_some() {
                    continue;
                }
                match word.as_str() {
                    "fonttbl" | "colortbl" | "stylesheet" | "info" | "pict"
                    | "expandedcolortbl" => skip_from = Some(depth),
                    "par" | "line" => out.push('\n'),
                    "tab" => out.push('\t'),
                    "u" => {
                        if let Ok(code) = param.parse::<i32>() {
                            let code = if code < 0 { code + 65_536 } else { code };
                            if let Some(c) = char::from_u32(code as u32) {
                                out.push(c);
                            }
                            unicode_skip = 1;
                        }
                    }
                    _ => {}
                }
            }
            '\r' | '\n' => {}
            _ => {
                if skip_from.is_none() {
                    if unicode_skip > 0 {
                        unicode_skip -= 1;
                    } else {
                        out.push(ch);
                    }
                }
            }
        }
    }
    out.trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preview_truncates_on_char_boundary() {
        let text = "é".repeat(PREVIEW_LIMIT + 10);
        assert_eq!(preview(&text).chars().count(), PREVIEW_LIMIT);
        assert_eq!(preview("short"), "short");
    }

    #[test]
    fn count_labels() {
        assert_eq!(count_label(1, "file"), "1 file");
        assert_eq!(count_label(3, "file"), "3 files");
        assert_eq!(count_label(1204, "character"), "1,204 characters");
        assert_eq!(group_thousands(1_234_567), "1,234,567");
        assert_eq!(group_thousands(-1000), "-1,000");
        assert_eq!(group_thousands(999), "999");
    }

    #[test]
    fn fitting_rows() {
        assert_eq!(fit_rows("a\nb\nc\nd", 2, 10), "a\nb");
        assert_eq!(fit_rows("short", 3, 10), "short");
        // A 25-char paragraph needs 3 rows of 10.
        let long = "x".repeat(25);
        assert_eq!(fit_rows(&format!("{long}\nnext"), 3, 10), long);
        assert_eq!(fit_rows(&format!("{long}\nnext"), 2, 10), "x".repeat(20));
        assert_eq!(fit_rows("a\n\n\nb", 3, 10), "a\n\n");
        assert_eq!(fit_rows("\tx", 1, 10), "    x");
        assert_eq!(fit_rows("anything", 0, 10), "");
    }

    #[test]
    fn single_url_detection() {
        assert!(is_single_url("https://example.com/download"));
        assert!(is_single_url("http://localhost:8080/x?y=1"));
        assert!(is_single_url("mailto:someone@example.com"));
        assert!(is_single_url("ftp://files.example.com/a.zip"));
        assert!(!is_single_url("https://a.com b"));
        assert!(!is_single_url("see https://a.com"));
        assert!(!is_single_url("https://"));
        assert!(!is_single_url("mailto:nobody"));
        assert!(!is_single_url("example.com"));
        assert!(!is_single_url(""));
    }

    #[test]
    fn hosts() {
        assert_eq!(link_host("https://www.apple.com/mac"), Some("apple.com"));
        assert_eq!(link_host("http://user@host.io:80/x"), Some("host.io"));
        assert_eq!(link_host("mailto:a@b.c"), None);
        assert_eq!(link_host("HTTPS://Example.com"), Some("Example.com"));
    }

    #[test]
    fn file_urls() {
        assert_eq!(
            file_url_to_path("file:///Users/me/My%20Doc.txt").as_deref(),
            Some("/Users/me/My Doc.txt")
        );
        assert_eq!(
            file_url_to_path("file://localhost/tmp/dir/").as_deref(),
            Some("/tmp/dir")
        );
        assert_eq!(file_url_to_path("https://x").as_deref(), None);
        assert_eq!(file_name("/Users/me/My Doc.txt"), "My Doc.txt");
        assert_eq!(file_name("/tmp/dir/"), "dir");
        assert_eq!(percent_decode("a%2"), "a%2");
        assert_eq!(percent_decode("%E2%9C%93"), "✓");
    }

    #[test]
    fn rtf_stripping() {
        let rtf = [r"{\rtf1\ansi{\fonttbl\f0\fswiss Helvetica;}{\colortbl;\red255\green0\blue0;}\f0\pard Hello \b World\b0\par Caf\'e9 ", "\\", "u8364?}"].concat();
        assert_eq!(rtf_to_text(&rtf), "Hello World\nCafé \u{20ac}");
        let nested = r"{\rtf1{\*\expandedcolortbl;;}Plain}";
        assert_eq!(rtf_to_text(nested), "Plain");
    }
}
