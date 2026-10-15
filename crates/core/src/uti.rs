//! Uniform Type Identifiers that Copywren reads or writes.

pub const PLAIN_TEXT: &str = "public.utf8-plain-text";
pub const UTF16_TEXT: &str = "public.utf16-external-plain-text";
pub const LEGACY_STRING: &str = "NSStringPboardType";
pub const URL: &str = "public.url";
pub const FILE_URL: &str = "public.file-url";
pub const RTF: &str = "public.rtf";
pub const RTFD: &str = "com.apple.flat-rtfd";
pub const HTML: &str = "public.html";

pub const PNG: &str = "public.png";
pub const TIFF: &str = "public.tiff";
pub const JPEG: &str = "public.jpeg";
pub const GIF: &str = "com.compuserve.gif";
pub const BMP: &str = "com.microsoft.bmp";
pub const WEBP: &str = "org.webmproject.webp";
pub const HEIC: &str = "public.heic";

/// Image types in the order Copywren prefers them as the "primary" image.
pub const IMAGE_TYPES: &[&str] = &[PNG, TIFF, JPEG, GIF, BMP, WEBP, HEIC];

/// Marker some apps add to say which app really produced the clip
/// (see nspasteboard.org).
pub const SOURCE: &str = "org.nspasteboard.source";
