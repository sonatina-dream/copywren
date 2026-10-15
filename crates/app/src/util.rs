use copywren_macos::window::NativeWindow;
use gpui::Window;
use raw_window_handle::{HasWindowHandle, RawWindowHandle};

/// The `NSWindow` behind a GPUI window.
pub fn native_window(window: &Window) -> Option<NativeWindow> {
    let handle = HasWindowHandle::window_handle(window).ok()?;
    match handle.as_raw() {
        // SAFETY: GPUI hands out the window's live content view.
        RawWindowHandle::AppKit(appkit) => unsafe {
            NativeWindow::from_ns_view(appkit.ns_view.as_ptr())
        },
        _ => None,
    }
}
