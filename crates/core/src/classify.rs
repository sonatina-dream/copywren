//! Turns raw pasteboard contents into an item kind, preview, checksum and
//! thumbnail (SPEC §3.6–5.7).

use sha2::{Digest, Sha256};

use crate::model::{ClipContents, ItemKind};
use crate::{color, text, uti};

/// Longest edge of stored image thumbnails, in pixels.
pub const THUMBNAIL_MAX: u32 = 480;

#[derive(Debug, Clone, PartialEq)]
pub struct Classified {
    pub kind: ItemKind,
    pub preview_text: String,
    /// SHA-256 over the kind and primary content; equal checksums dedupe.
    pub checksum: String,
    pub char_count: i64,
    pub image_size: Option<(u32, u32)>,
    pub file_count: i64,
    /// PNG thumbnail for image clips whose format the decoder understands.
    pub thumbnail_png: Option<Vec<u8>>,
}

pub fn classify(contents: &ClipContents) -> Option<Classified> {
    let paths = file_paths(contents);
    if !paths.is_empty() {
        let joined = paths.join("\n");
        return Some(Classified {
            kind: ItemKind::File,
            checksum: checksum(ItemKind::File, joined.as_bytes()),
            preview_text: text::preview(&joined),
            char_count: 0,
            image_size: None,
            file_count: paths.len() as i64,
            thumbnail_png: None,
        });
    }

    let image = primary_image(contents);
    let string = primary_string(contents);
    let has_rich_text =
        contents.has_type(uti::RTF) || contents.has_type(uti::RTFD) || contents.has_type(uti::HTML);
    let has_meaningful_string = string.as_deref().is_some_and(|s| !s.trim().is_empty());

    // Documents (Word, Pages, Excel) put a rendered image next to the rich
    // text; those clips are text. A bare image (screenshot, "Copy Image") is
    // an image even when the browser adds its URL as a string.
    if let Some((image_uti, bytes)) = image
        && !(has_meaningful_string && has_rich_text)
    {
        return Some(classify_image(image_uti, bytes));
    }

    if let Some(string) = string.filter(|s| !s.trim().is_empty()) {
        return Some(classify_string(&string));
    }

    if let Some(rtf) = contents.first_string(uti::RTF) {
        let plain = text::rtf_to_text(&rtf);
        if !plain.is_empty() {
            return Some(classify_string(&plain));
        }
    }
    None
}

fn primary_image(contents: &ClipContents) -> Option<(&'static str, &[u8])> {
    uti::IMAGE_TYPES.iter().find_map(|t| {
        contents
            .entries
            .iter()
            .find_map(|e| e.get(t))
            .filter(|data| !data.is_empty())
            .map(|data| (*t, data))
    })
}

fn primary_string(contents: &ClipContents) -> Option<String> {
    if let Some(s) = contents.first_string(uti::PLAIN_TEXT) {
        return Some(s);
    }
    if let Some(s) = contents.first_string(uti::LEGACY_STRING) {
        return Some(s);
    }
    if let Some(bytes) = contents.entries.iter().find_map(|e| e.get(uti::UTF16_TEXT)) {
        return Some(decode_utf16(bytes));
    }
    contents.first_string(uti::URL)
}

fn decode_utf16(bytes: &[u8]) -> String {
    let (little_endian, body) = match bytes {
        [0xFF, 0xFE, rest @ ..] => (true, rest),
        [0xFE, 0xFF, rest @ ..] => (false, rest),
        _ => (true, bytes),
    };
    let units: Vec<u16> = body
        .as_chunks::<2>()
        .0
        .iter()
        .map(|c| {
            if little_endian {
                u16::from_le_bytes([c[0], c[1]])
            } else {
                u16::from_be_bytes([c[0], c[1]])
            }
        })
        .collect();
    String::from_utf16_lossy(&units)
}

fn classify_string(string: &str) -> Classified {
    let trimmed = string.trim();
    if color::parse(trimmed).is_some() {
        return Classified {
            kind: ItemKind::Color,
            checksum: checksum(ItemKind::Color, trimmed.to_ascii_lowercase().as_bytes()),
            preview_text: trimmed.to_string(),
            char_count: trimmed.chars().count() as i64,
            image_size: None,
            file_count: 0,
            thumbnail_png: None,
        };
    }
    if text::is_single_url(trimmed) {
        return Classified {
            kind: ItemKind::Link,
            checksum: checksum(ItemKind::Link, trimmed.as_bytes()),
            preview_text: text::preview(trimmed),
            char_count: trimmed.chars().count() as i64,
            image_size: None,
            file_count: 0,
            thumbnail_png: None,
        };
    }
    Classified {
        kind: ItemKind::Text,
        checksum: checksum(ItemKind::Text, string.as_bytes()),
        preview_text: text::preview(string),
        char_count: string.chars().count() as i64,
        image_size: None,
        file_count: 0,
        thumbnail_png: None,
    }
}

fn classify_image(image_uti: &str, bytes: &[u8]) -> Classified {
    let (thumbnail_png, image_size) = match make_thumbnail(image_uti, bytes) {
        Some((png, size)) => (Some(png), Some(size)),
        None => (None, None),
    };
    Classified {
        kind: ItemKind::Image,
        checksum: checksum(ItemKind::Image, bytes),
        preview_text: String::new(),
        char_count: 0,
        image_size,
        file_count: 0,
        thumbnail_png,
    }
}

/// Decodes an image and returns a PNG thumbnail plus the original size.
pub fn make_thumbnail(image_uti: &str, bytes: &[u8]) -> Option<(Vec<u8>, (u32, u32))> {
    use image::ImageFormat;
    let format = match image_uti {
        uti::PNG => ImageFormat::Png,
        uti::TIFF => ImageFormat::Tiff,
        uti::JPEG => ImageFormat::Jpeg,
        uti::GIF => ImageFormat::Gif,
        uti::BMP => ImageFormat::Bmp,
        uti::WEBP => ImageFormat::WebP,
        _ => return None,
    };
    let decoded = image::load_from_memory_with_format(bytes, format).ok()?;
    let size = (decoded.width(), decoded.height());
    let thumb = if size.0 > THUMBNAIL_MAX || size.1 > THUMBNAIL_MAX {
        decoded.thumbnail(THUMBNAIL_MAX, THUMBNAIL_MAX)
    } else {
        decoded
    };
    let mut png = Vec::new();
    thumb
        .write_to(&mut std::io::Cursor::new(&mut png), ImageFormat::Png)
        .ok()?;
    Some((png, size))
}

pub fn checksum(kind: ItemKind, primary: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(kind.label().as_bytes());
    hasher.update([0]);
    hasher.update(primary);
    hasher
        .finalize()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// POSIX paths of every file URL in the clip.
pub fn file_paths(contents: &ClipContents) -> Vec<String> {
    contents
        .entries
        .iter()
        .filter_map(|e| e.string(uti::FILE_URL))
        .filter_map(|url| text::file_url_to_path(url.trim()))
        .collect()
}

/// The text written to the pasteboard for "Paste as Plain Text".
pub fn plain_text_of(contents: &ClipContents) -> Option<String> {
    let paths = file_paths(contents);
    if !paths.is_empty() {
        return Some(paths.join("\n"));
    }
    if let Some(s) = primary_string(contents).filter(|s| !s.is_empty()) {
        return Some(s);
    }
    contents
        .first_string(uti::RTF)
        .map(|rtf| text::rtf_to_text(&rtf))
        .filter(|s| !s.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{PasteboardEntry, Representation};

    fn clip(reps: &[(&str, &[u8])]) -> ClipContents {
        ClipContents::new(vec![PasteboardEntry::new(
            reps.iter()
                .map(|(t, d)| Representation::new(*t, d.to_vec()))
                .collect(),
        )])
    }

    fn tiny_png(w: u32, h: u32) -> Vec<u8> {
        let img = image::RgbaImage::from_pixel(w, h, image::Rgba([10, 20, 30, 255]));
        let mut out = Vec::new();
        img.write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png)
            .unwrap();
        out
    }

    #[test]
    fn plain_text() {
        let c = classify(&clip(&[(uti::PLAIN_TEXT, b"Hello world")])).unwrap();
        assert_eq!(c.kind, ItemKind::Text);
        assert_eq!(c.preview_text, "Hello world");
        assert_eq!(c.char_count, 11);
    }

    #[test]
    fn whitespace_only_is_ignored() {
        assert!(classify(&clip(&[(uti::PLAIN_TEXT, b"  \n\t ")])).is_none());
        assert!(classify(&ClipContents::default()).is_none());
        assert!(classify(&clip(&[("com.example.private", b"x")])).is_none());
    }

    #[test]
    fn links_and_colors() {
        let link = classify(&clip(&[(uti::PLAIN_TEXT, b" https://example.com/a \n")])).unwrap();
        assert_eq!(link.kind, ItemKind::Link);
        assert_eq!(link.preview_text, "https://example.com/a");
        let url_only = classify(&clip(&[(uti::URL, b"https://x.io")])).unwrap();
        assert_eq!(url_only.kind, ItemKind::Link);
        let color = classify(&clip(&[(uti::PLAIN_TEXT, b"#FF8800")])).unwrap();
        assert_eq!(color.kind, ItemKind::Color);
        let sentence = classify(&clip(&[(uti::PLAIN_TEXT, b"go to https://x.io now")])).unwrap();
        assert_eq!(sentence.kind, ItemKind::Text);
    }

    #[test]
    fn color_checksum_ignores_case() {
        let a = classify(&clip(&[(uti::PLAIN_TEXT, b"#ff8800")])).unwrap();
        let b = classify(&clip(&[(uti::PLAIN_TEXT, b"#FF8800")])).unwrap();
        assert_eq!(a.checksum, b.checksum);
    }

    #[test]
    fn files_win_over_everything() {
        let contents = ClipContents::new(vec![
            PasteboardEntry::new(vec![
                Representation::new(uti::FILE_URL, "file:///tmp/a%20b.txt"),
                Representation::new(uti::PLAIN_TEXT, "a b.txt"),
            ]),
            PasteboardEntry::new(vec![Representation::new(
                uti::FILE_URL,
                "file:///tmp/c.png",
            )]),
        ]);
        let c = classify(&contents).unwrap();
        assert_eq!(c.kind, ItemKind::File);
        assert_eq!(c.file_count, 2);
        assert_eq!(c.preview_text, "/tmp/a b.txt\n/tmp/c.png");
        assert_eq!(
            plain_text_of(&contents).unwrap(),
            "/tmp/a b.txt\n/tmp/c.png"
        );
    }

    #[test]
    fn images_get_size_and_thumbnail() {
        let png = tiny_png(1000, 500);
        let c = classify(&clip(&[
            (uti::PNG, &png),
            (uti::URL, b"https://x.io/i.png"),
        ]))
        .unwrap();
        assert_eq!(c.kind, ItemKind::Image);
        assert_eq!(c.image_size, Some((1000, 500)));
        let thumb = image::load_from_memory(c.thumbnail_png.as_ref().unwrap()).unwrap();
        assert_eq!(
            (thumb.width(), thumb.height()),
            (THUMBNAIL_MAX, THUMBNAIL_MAX / 2)
        );
    }

    #[test]
    fn undecodable_images_still_classify() {
        let c = classify(&clip(&[(uti::HEIC, b"not really heic")])).unwrap();
        assert_eq!(c.kind, ItemKind::Image);
        assert_eq!(c.image_size, None);
        assert!(c.thumbnail_png.is_none());
    }

    #[test]
    fn rich_documents_with_images_are_text() {
        let png = tiny_png(4, 4);
        let c = classify(&clip(&[
            (uti::PLAIN_TEXT, b"Quarterly numbers"),
            (uti::RTF, b"{\\rtf1 Quarterly numbers}"),
            (uti::PNG, &png),
        ]))
        .unwrap();
        assert_eq!(c.kind, ItemKind::Text);
    }

    #[test]
    fn rtf_only_and_utf16() {
        let c = classify(&clip(&[(uti::RTF, b"{\\rtf1\\ansi Bold \\b text}")])).unwrap();
        assert_eq!(c.preview_text, "Bold text");
        let utf16: Vec<u8> = [0xFF, 0xFE]
            .into_iter()
            .chain("سلام".encode_utf16().flat_map(|u| u.to_le_bytes()))
            .collect();
        let c = classify(&clip(&[(uti::UTF16_TEXT, &utf16)])).unwrap();
        assert_eq!(c.preview_text, "سلام");
    }

    #[test]
    fn same_text_same_checksum_different_kind_different_checksum() {
        let a = classify(&clip(&[(uti::PLAIN_TEXT, b"abc")])).unwrap();
        let b = classify(&clip(&[
            (uti::PLAIN_TEXT, b"abc"),
            (uti::RTF, b"{\\rtf1 abc}"),
        ]))
        .unwrap();
        assert_eq!(a.checksum, b.checksum);
        assert_ne!(
            checksum(ItemKind::Text, b"x"),
            checksum(ItemKind::Link, b"x")
        );
    }

    #[test]
    fn long_text_preview_is_truncated_but_counted() {
        let long = "x".repeat(text::PREVIEW_LIMIT * 3);
        let c = classify(&clip(&[(uti::PLAIN_TEXT, long.as_bytes())])).unwrap();
        assert_eq!(c.preview_text.len(), text::PREVIEW_LIMIT);
        assert_eq!(c.char_count, (text::PREVIEW_LIMIT * 3) as i64);
    }
}
