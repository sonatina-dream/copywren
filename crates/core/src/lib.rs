//! Platform-independent core of Copywren: the clip model, classification, privacy
//! filtering, persistence, search, retention, Paste Queue and settings.
//!
//! Nothing in this crate touches AppKit; the macOS layer feeds it raw pasteboard
//! contents and the GPUI layer renders what it returns. See `SPEC.md` at the
//! repository root for the behavior these modules implement.

pub mod appcolor;
pub mod classify;
pub mod color;
pub mod contents;
pub mod model;
pub mod pause;
pub mod placement;
pub mod privacy;
pub mod queue;
pub mod retention;
pub mod search;
pub mod settings;
pub mod shortcut;
pub mod store;
pub mod text;
pub mod timefmt;
pub mod uti;

pub use model::*;

/// Milliseconds since the Unix epoch, the time unit used throughout the store.
pub fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}
