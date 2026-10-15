//! Search with smart tokens (SPEC §8): type words filter by kind, words that
//! start a known source-app name filter by app, everything else must appear
//! in the item's text. `"quoted phrases"` are always content terms.

use crate::model::{Item, ItemKind};

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Query {
    pub kinds: Vec<ItemKind>,
    /// Lowercased tokens matched against source-app names.
    pub apps: Vec<String>,
    /// Lowercased substrings that must all occur in the item.
    pub terms: Vec<String>,
}

impl Query {
    pub fn is_empty(&self) -> bool {
        self.kinds.is_empty() && self.apps.is_empty() && self.terms.is_empty()
    }

    pub fn matches(&self, item: &Item) -> bool {
        if !self.kinds.is_empty() && !self.kinds.contains(&item.kind) {
            return false;
        }
        let app = item
            .source_app_name
            .as_deref()
            .unwrap_or_default()
            .to_lowercase();
        if !self.apps.iter().all(|token| app_matches(&app, token)) {
            return false;
        }
        if self.terms.is_empty() {
            return true;
        }
        let haystack = format!(
            "{}\n{}\n{}",
            item.preview_text.to_lowercase(),
            item.title.as_deref().unwrap_or_default().to_lowercase(),
            app
        );
        self.terms.iter().all(|t| haystack.contains(t.as_str()))
    }

    /// The same query with app tokens treated as plain content terms.
    fn without_app_filters(&self) -> Query {
        let mut terms = self.terms.clone();
        terms.extend(self.apps.iter().cloned());
        Query {
            kinds: self.kinds.clone(),
            apps: Vec::new(),
            terms,
        }
    }
}

pub fn kind_for_word(word: &str) -> Option<ItemKind> {
    Some(match word {
        "text" | "texts" => ItemKind::Text,
        "link" | "links" | "url" | "urls" => ItemKind::Link,
        "image" | "images" | "img" | "picture" | "pictures" | "screenshot" | "screenshots" => {
            ItemKind::Image
        }
        "file" | "files" => ItemKind::File,
        "color" | "colors" | "colour" | "colours" => ItemKind::Color,
        _ => return None,
    })
}

/// A token matches an app when some word of the app's name starts with it.
fn app_matches(app_lower: &str, token: &str) -> bool {
    app_lower
        .split(|c: char| !c.is_alphanumeric())
        .any(|word| word.starts_with(token))
}

pub fn parse(input: &str, known_apps: &[String]) -> Query {
    let known: Vec<String> = known_apps.iter().map(|a| a.to_lowercase()).collect();
    let mut query = Query::default();
    for (token, quoted) in tokenize(input) {
        let lower = token.to_lowercase();
        if quoted {
            query.terms.push(lower);
        } else if let Some(kind) = kind_for_word(&lower) {
            if !query.kinds.contains(&kind) {
                query.kinds.push(kind);
            }
        } else if lower.chars().count() >= 3 && known.iter().any(|app| app_matches(app, &lower)) {
            query.apps.push(lower);
        } else {
            query.terms.push(lower);
        }
    }
    query
}

/// Splits on whitespace, keeping `"quoted phrases"` together.
fn tokenize(input: &str) -> Vec<(String, bool)> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    let mut in_quotes = false;
    for ch in input.chars() {
        match ch {
            '"' | '“' | '”' => {
                if in_quotes {
                    if !current.is_empty() {
                        tokens.push((std::mem::take(&mut current), true));
                    }
                    in_quotes = false;
                } else {
                    if !current.is_empty() {
                        tokens.push((std::mem::take(&mut current), false));
                    }
                    in_quotes = true;
                }
            }
            c if c.is_whitespace() && !in_quotes => {
                if !current.is_empty() {
                    tokens.push((std::mem::take(&mut current), false));
                }
            }
            c => current.push(c),
        }
    }
    if !current.is_empty() {
        tokens.push((current, in_quotes));
    }
    tokens
}

/// Indices of `items` that match `input`, preserving order. If treating some
/// words as app names finds nothing, they are retried as content words.
pub fn filter(items: &[Item], input: &str, known_apps: &[String]) -> Vec<usize> {
    let query = parse(input, known_apps);
    if query.is_empty() {
        return (0..items.len()).collect();
    }
    let run = |q: &Query| -> Vec<usize> {
        items
            .iter()
            .enumerate()
            .filter(|(_, item)| q.matches(item))
            .map(|(i, _)| i)
            .collect()
    };
    let hits = run(&query);
    if hits.is_empty() && !query.apps.is_empty() {
        return run(&query.without_app_filters());
    }
    hits
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(kind: ItemKind, text: &str, app: &str) -> Item {
        Item {
            id: 0,
            kind,
            title: None,
            preview_text: text.into(),
            checksum: String::new(),
            created_at: 0,
            timestamp: 0,
            source_bundle_id: None,
            source_app_name: Some(app.into()),
            pinboard_id: None,
            pin_order: 0,
            char_count: 0,
            image_size: None,
            file_count: 0,
        }
    }

    fn apps() -> Vec<String> {
        vec!["Mail".into(), "Google Chrome".into(), "Notes".into()]
    }

    fn sample() -> Vec<Item> {
        vec![
            item(ItemKind::Link, "https://slides.example.com/meeting", "Mail"),
            item(ItemKind::Text, "meeting slides draft", "Mail"),
            item(
                ItemKind::Link,
                "https://example.com/meeting-slides",
                "Google Chrome",
            ),
            item(ItemKind::Text, "Notes about mail servers", "Notes"),
        ]
    }

    #[test]
    fn smart_tokens_like_the_onboarding_example() {
        let q = parse("mail link meeting slides", &apps());
        assert_eq!(q.kinds, vec![ItemKind::Link]);
        assert_eq!(q.apps, vec!["mail"]);
        assert_eq!(q.terms, vec!["meeting", "slides"]);
        assert_eq!(
            filter(&sample(), "mail link meeting slides", &apps()),
            vec![0]
        );
    }

    #[test]
    fn app_word_prefixes() {
        assert_eq!(filter(&sample(), "chro", &apps()), vec![2]);
        // Two letters are too short to be an app filter: plain substring.
        assert_eq!(filter(&sample(), "ch", &apps()), vec![2]);
    }

    #[test]
    fn falls_back_to_content_when_app_filter_finds_nothing() {
        let items = sample();
        assert_eq!(filter(&items, "notes", &apps()), vec![3]);
        // No Mail item mentions "servers", so "mail" is retried as a content
        // word and finds the Notes item that talks about mail servers.
        assert_eq!(filter(&items, "servers mail", &apps()), vec![3]);
    }

    #[test]
    fn quoted_phrases_and_case() {
        let items = sample();
        assert_eq!(filter(&items, "\"Slides Draft\"", &apps()), vec![1]);
        assert_eq!(filter(&items, "\"link\"", &apps()), Vec::<usize>::new());
        assert_eq!(filter(&items, "MEETING", &apps()), vec![0, 1, 2]);
    }

    #[test]
    fn empty_query_returns_everything_and_titles_are_searched() {
        let mut items = sample();
        assert_eq!(filter(&items, "   ", &apps()), vec![0, 1, 2, 3]);
        items[3].title = Some("Server checklist".into());
        assert_eq!(filter(&items, "checklist", &apps()), vec![3]);
        assert_eq!(filter(&items, "text", &apps()), vec![1, 3]);
        assert_eq!(
            filter(&items, "nothing-matches", &apps()),
            Vec::<usize>::new()
        );
    }

    #[test]
    fn tokenizer() {
        assert_eq!(
            tokenize(r#" a "b c"  d"#),
            vec![
                ("a".into(), false),
                ("b c".into(), true),
                ("d".into(), false)
            ]
        );
        assert_eq!(tokenize("\"open"), vec![("open".into(), true)]);
    }
}
