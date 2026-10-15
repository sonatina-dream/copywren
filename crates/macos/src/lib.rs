//! Thin, safe wrappers over the macOS APIs Copywren needs. Every function here
//! must be called on the main thread, the way AppKit expects.
#![cfg(target_os = "macos")]
// objc 0.2's msg_send! expands to cfg(feature = "cargo-clippy").
#![allow(unexpected_cfgs)]
// cocoa 0.26 is deprecated in favour of objc2, but it is what gpui 0.2.2 pins.
#![allow(deprecated)]

pub mod input;
pub mod pasteboard;
pub mod status_item;
pub mod system;
pub mod window;
pub mod workspace;

mod ns;

/// A rectangle in Cocoa screen coordinates (origin at the bottom-left of the
/// primary display).
pub use copywren_core::placement::Rect as ScreenRect;
