use serde::{Deserialize, Serialize};

pub type ItemId = i64;
pub type PinboardId = i64;

/// What a clip is, decided once at capture time (SPEC §3.6).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ItemKind {
    Text,
    Link,
    Image,
    File,
    Color,
}

impl ItemKind {
    pub const ALL: [ItemKind; 5] = [
        ItemKind::Text,
        ItemKind::Link,
        ItemKind::Image,
        ItemKind::File,
        ItemKind::Color,
    ];

    pub fn to_i64(self) -> i64 {
        match self {
            ItemKind::Text => 0,
            ItemKind::Link => 1,
            ItemKind::Image => 2,
            ItemKind::File => 3,
            ItemKind::Color => 4,
        }
    }

    pub fn from_i64(value: i64) -> Option<Self> {
        Some(match value {
            0 => ItemKind::Text,
            1 => ItemKind::Link,
            2 => ItemKind::Image,
            3 => ItemKind::File,
            4 => ItemKind::Color,
            _ => return None,
        })
    }

    pub fn label(self) -> &'static str {
        match self {
            ItemKind::Text => "Text",
            ItemKind::Link => "Link",
            ItemKind::Image => "Image",
            ItemKind::File => "File",
            ItemKind::Color => "Color",
        }
    }
}

/// One representation (type + bytes) of a pasteboard item.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Representation {
    pub uti: String,
    pub data: Vec<u8>,
}

impl Representation {
    pub fn new(uti: impl Into<String>, data: impl Into<Vec<u8>>) -> Self {
        Self {
            uti: uti.into(),
            data: data.into(),
        }
    }
}

/// One `NSPasteboardItem`: every representation the source app offered.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PasteboardEntry {
    pub representations: Vec<Representation>,
}

impl PasteboardEntry {
    pub fn new(representations: Vec<Representation>) -> Self {
        Self { representations }
    }

    pub fn get(&self, uti: &str) -> Option<&[u8]> {
        self.representations
            .iter()
            .find(|r| r.uti == uti)
            .map(|r| r.data.as_slice())
    }

    pub fn has(&self, uti: &str) -> bool {
        self.representations.iter().any(|r| r.uti == uti)
    }

    /// Decodes a UTF-8 string representation, dropping a trailing NUL that
    /// some Carbon-era apps append.
    pub fn string(&self, uti: &str) -> Option<String> {
        let data = self.get(uti)?;
        let data = data.strip_suffix(&[0]).unwrap_or(data);
        Some(String::from_utf8_lossy(data).into_owned())
    }
}

/// The full pasteboard as captured: what gets stored and replayed on paste.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ClipContents {
    pub entries: Vec<PasteboardEntry>,
}

impl ClipContents {
    pub fn new(entries: Vec<PasteboardEntry>) -> Self {
        Self { entries }
    }

    /// A clip holding only a UTF-8 plain-text representation.
    pub fn plain_text(text: &str) -> Self {
        Self::new(vec![PasteboardEntry::new(vec![Representation::new(
            crate::uti::PLAIN_TEXT,
            text.as_bytes(),
        )])])
    }

    pub fn is_empty(&self) -> bool {
        self.entries.iter().all(|e| e.representations.is_empty())
    }

    pub fn total_bytes(&self) -> usize {
        self.entries
            .iter()
            .flat_map(|e| &e.representations)
            .map(|r| r.data.len())
            .sum()
    }

    pub fn types(&self) -> impl Iterator<Item = &str> {
        self.entries
            .iter()
            .flat_map(|e| &e.representations)
            .map(|r| r.uti.as_str())
    }

    pub fn has_type(&self, uti: &str) -> bool {
        self.types().any(|t| t == uti)
    }

    /// First string representation of `uti` across all entries.
    pub fn first_string(&self, uti: &str) -> Option<String> {
        self.entries.iter().find_map(|e| e.string(uti))
    }
}

/// A stored clipboard item (without its payload, which is loaded on demand).
#[derive(Debug, Clone, PartialEq)]
pub struct Item {
    pub id: ItemId,
    pub kind: ItemKind,
    /// User-assigned name, shown instead of the type label.
    pub title: Option<String>,
    /// Searchable, displayable text (truncated; see `text::PREVIEW_LIMIT`).
    pub preview_text: String,
    pub checksum: String,
    pub created_at: i64,
    /// Last copied or used. Orders the history, newest first.
    pub timestamp: i64,
    pub source_bundle_id: Option<String>,
    pub source_app_name: Option<String>,
    /// `None` means the item lives in the history.
    pub pinboard_id: Option<PinboardId>,
    pub pin_order: i64,
    pub char_count: i64,
    pub image_size: Option<(u32, u32)>,
    pub file_count: i64,
}

impl Item {
    pub fn display_title(&self) -> String {
        match &self.title {
            Some(title) if !title.trim().is_empty() => title.clone(),
            _ => self.kind.label().to_string(),
        }
    }

    pub fn is_pinned(&self) -> bool {
        self.pinboard_id.is_some()
    }

    /// Footer text, e.g. "1,204 characters", "3 files", "1920×1080".
    pub fn footer(&self) -> String {
        match self.kind {
            ItemKind::Text => crate::text::count_label(self.char_count, "character"),
            ItemKind::Link => crate::text::link_host(&self.preview_text)
                .unwrap_or_default()
                .to_string(),
            ItemKind::Image => match self.image_size {
                Some((w, h)) => format!("{w}×{h}"),
                None => String::new(),
            },
            ItemKind::File => crate::text::count_label(self.file_count, "file"),
            ItemKind::Color => String::new(),
        }
    }
}

/// A user-created collection of pinned items.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pinboard {
    pub id: PinboardId,
    pub name: String,
    /// Index into the UI's pinboard palette.
    pub color: u8,
    pub position: i64,
}

/// A tab in the panel: the history, or one pinboard.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Tab {
    History,
    Pinboard(PinboardId),
}

/// The app a clip came from.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SourceApp {
    pub bundle_id: Option<String>,
    pub name: Option<String>,
}
