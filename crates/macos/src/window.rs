//! Direct control of the `NSWindow` behind a GPUI window, for what GPUI
//! doesn't expose: hiding without closing, exact placement, and exclusion
//! from screen capture (SPEC §4).

use std::os::raw::c_void;

use cocoa::foundation::{NSPoint, NSRect, NSSize};
use objc::runtime::{NO, YES};
use objc::{msg_send, sel, sel_impl};

use crate::ScreenRect;
use crate::ns::{self, Id};

#[repr(C)]
#[derive(Clone, Copy)]
struct CGPoint {
    x: f64,
    y: f64,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct CGSize {
    width: f64,
    height: f64,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct CGRect {
    origin: CGPoint,
    size: CGSize,
}

#[link(name = "CoreGraphics", kind = "framework")]
unsafe extern "C" {
    fn CGWindowListCreateImage(
        bounds: CGRect,
        options: u32,
        window: u32,
        image_options: u32,
    ) -> *mut c_void;
    fn CGImageRelease(image: *mut c_void);
}

/// Handle to an `NSWindow`, obtained from the window's content `NSView`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NativeWindow(Id);

impl NativeWindow {
    /// # Safety
    /// `ns_view` must be a live `NSView` that is installed in a window.
    pub unsafe fn from_ns_view(ns_view: *mut c_void) -> Option<Self> {
        if ns_view.is_null() {
            return None;
        }
        let window: Id = unsafe { msg_send![ns_view as Id, window] };
        (!window.is_null()).then_some(NativeWindow(window))
    }

    /// Places the window and shows it above everything. With `make_key`, it
    /// also takes keyboard focus without activating the app (the panel is
    /// non-activating).
    pub fn show(&self, frame: ScreenRect, make_key: bool) {
        unsafe {
            let rect = NSRect::new(
                NSPoint::new(frame.x, frame.y),
                NSSize::new(frame.width, frame.height),
            );
            let _: () = msg_send![self.0, setFrame: rect display: YES animate: NO];
            let _: () = msg_send![self.0, orderFrontRegardless];
            if make_key {
                let _: () = msg_send![self.0, makeKeyWindow];
            }
        }
    }

    /// Changes the frame without changing visibility or focus.
    pub fn set_frame(&self, frame: ScreenRect) {
        unsafe {
            let rect = NSRect::new(
                NSPoint::new(frame.x, frame.y),
                NSSize::new(frame.width, frame.height),
            );
            let _: () = msg_send![self.0, setFrame: rect display: YES animate: NO];
        }
    }

    pub fn hide(&self) {
        unsafe {
            let _: () = msg_send![self.0, orderOut: std::ptr::null_mut::<c_void>()];
        }
    }

    pub fn is_visible(&self) -> bool {
        unsafe { msg_send![self.0, isVisible] }
    }

    pub fn frame(&self) -> ScreenRect {
        let rect: NSRect = unsafe { msg_send![self.0, frame] };
        ScreenRect::new(
            rect.origin.x,
            rect.origin.y,
            rect.size.width,
            rect.size.height,
        )
    }

    /// PNG of this window's current contents. A process may always capture
    /// its own windows, so this works without Screen Recording permission.
    /// Used by the `COPYWREN_SNAPSHOT_DIR` debugging hook.
    pub fn snapshot_png(&self) -> Option<Vec<u8>> {
        unsafe {
            let number: isize = msg_send![self.0, windowNumber];
            let null_rect = CGRect {
                origin: CGPoint {
                    x: f64::INFINITY,
                    y: f64::INFINITY,
                },
                size: CGSize {
                    width: 0.0,
                    height: 0.0,
                },
            };
            // kCGWindowListOptionIncludingWindow, kCGWindowImageBoundsIgnoreFraming
            let image = CGWindowListCreateImage(null_rect, 1 << 3, number as u32, 1);
            if image.is_null() {
                return None;
            }
            let rep: Id = msg_send![objc::class!(NSBitmapImageRep), alloc];
            let rep: Id = msg_send![rep, initWithCGImage: image];
            CGImageRelease(image);
            let properties: Id = msg_send![objc::class!(NSDictionary), dictionary];
            let png: Id = msg_send![rep, representationUsingType: 4usize properties: properties];
            let bytes = ns::data_to_vec(png);
            let _: () = msg_send![rep, release];
            (!bytes.is_empty()).then_some(bytes)
        }
    }

    /// `false` keeps the window out of screenshots and screen sharing.
    pub fn set_visible_in_screen_capture(&self, visible: bool) {
        // NSWindowSharingNone = 0, NSWindowSharingReadOnly = 1
        let sharing: usize = if visible { 1 } else { 0 };
        unsafe {
            let _: () = msg_send![self.0, setSharingType: sharing];
        }
    }

    /// Keeps the panel from vanishing when another app activates; Copywren
    /// hides it itself when it loses key status.
    pub fn configure_panel(&self) {
        unsafe {
            let _: () = msg_send![self.0, setHidesOnDeactivate: NO];
            let _: () = msg_send![self.0, setMovable: NO];
            let _: () = msg_send![self.0, setHasShadow: YES];
            // NSWindowCollectionBehaviorCanJoinAllSpaces | Transient |
            // FullScreenAuxiliary | IgnoresCycle
            let behavior: usize = (1 << 0) | (1 << 3) | (1 << 8) | (1 << 6);
            let _: () = msg_send![self.0, setCollectionBehavior: behavior];
        }
    }
}
