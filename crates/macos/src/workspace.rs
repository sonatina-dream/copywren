//! Running apps, app icons, screens, and opening things (SPEC §3.4, §6).

use cocoa::foundation::{NSPoint, NSRect, NSSize};
use objc::rc::autoreleasepool;
use objc::runtime::{NO, YES};
use objc::{class, msg_send, sel, sel_impl};

use crate::ScreenRect;
use crate::ns::{self, Id};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunningApp {
    pub pid: i32,
    pub bundle_id: Option<String>,
    pub name: Option<String>,
}

unsafe fn shared() -> Id {
    unsafe { msg_send![class!(NSWorkspace), sharedWorkspace] }
}

pub fn frontmost_app() -> Option<RunningApp> {
    autoreleasepool(|| unsafe {
        let app: Id = msg_send![shared(), frontmostApplication];
        if app.is_null() {
            return None;
        }
        let pid: i32 = msg_send![app, processIdentifier];
        let bundle: Id = msg_send![app, bundleIdentifier];
        let name: Id = msg_send![app, localizedName];
        Some(RunningApp {
            pid,
            bundle_id: ns::to_string(bundle),
            name: ns::to_string(name),
        })
    })
}

/// Brings the app with `pid` to the front. Returns false if it's gone.
pub fn activate(pid: i32) -> bool {
    autoreleasepool(|| unsafe {
        let app: Id = msg_send![class!(NSRunningApplication),
            runningApplicationWithProcessIdentifier: pid];
        if app.is_null() {
            return false;
        }
        // NSApplicationActivateAllWindows; "ignoring other apps" is a no-op
        // on macOS 14+.
        let ok: bool = msg_send![app, activateWithOptions: 1usize];
        ok
    })
}

/// Display name of an installed app, e.g. "Keychain Access".
pub fn app_name(bundle_id: &str) -> Option<String> {
    autoreleasepool(|| unsafe {
        let url: Id =
            msg_send![shared(), URLForApplicationWithBundleIdentifier: ns::string(bundle_id)];
        if url.is_null() {
            return None;
        }
        let manager: Id = msg_send![class!(NSFileManager), defaultManager];
        let path: Id = msg_send![url, path];
        let name: Id = msg_send![manager, displayNameAtPath: path];
        ns::to_string(name).map(|n| n.trim_end_matches(".app").to_string())
    })
}

/// The app's icon rendered to a `pixels`×`pixels` PNG.
pub fn app_icon_png(bundle_id: &str, pixels: usize) -> Option<Vec<u8>> {
    autoreleasepool(|| unsafe {
        let workspace = shared();
        let url: Id =
            msg_send![workspace, URLForApplicationWithBundleIdentifier: ns::string(bundle_id)];
        if url.is_null() {
            return None;
        }
        let path: Id = msg_send![url, path];
        let icon: Id = msg_send![workspace, iconForFile: path];
        if icon.is_null() {
            return None;
        }
        let rep: Id = msg_send![class!(NSBitmapImageRep), alloc];
        let rep: Id = msg_send![rep,
            initWithBitmapDataPlanes: std::ptr::null_mut::<*mut u8>()
            pixelsWide: pixels as isize
            pixelsHigh: pixels as isize
            bitsPerSample: 8isize
            samplesPerPixel: 4isize
            hasAlpha: YES
            isPlanar: NO
            colorSpaceName: ns::string("NSDeviceRGBColorSpace")
            bytesPerRow: 0isize
            bitsPerPixel: 0isize];
        if rep.is_null() {
            return None;
        }
        let context: Id =
            msg_send![class!(NSGraphicsContext), graphicsContextWithBitmapImageRep: rep];
        let _: () = msg_send![class!(NSGraphicsContext), saveGraphicsState];
        let _: () = msg_send![class!(NSGraphicsContext), setCurrentContext: context];
        let target = NSRect::new(
            NSPoint::new(0., 0.),
            NSSize::new(pixels as f64, pixels as f64),
        );
        let whole = NSRect::new(NSPoint::new(0., 0.), NSSize::new(0., 0.));
        // NSCompositingOperationSourceOver
        let _: () =
            msg_send![icon, drawInRect: target fromRect: whole operation: 2usize fraction: 1.0f64];
        let _: () = msg_send![class!(NSGraphicsContext), restoreGraphicsState];
        let properties: Id = msg_send![class!(NSDictionary), dictionary];
        // NSBitmapImageFileTypePNG
        let png: Id = msg_send![rep, representationUsingType: 4usize properties: properties];
        let bytes = ns::data_to_vec(png);
        let _: () = msg_send![rep, release];
        (!bytes.is_empty()).then_some(bytes)
    })
}

/// Frame and visible frame (minus menu bar and Dock) of the screen under the
/// mouse pointer.
pub fn screen_under_mouse() -> Option<(ScreenRect, ScreenRect)> {
    let (x, y) = mouse_location();
    screen_containing(x, y)
}

/// Frame and visible frame of the screen containing the point, falling back
/// to the main screen.
pub fn screen_containing(x: f64, y: f64) -> Option<(ScreenRect, ScreenRect)> {
    autoreleasepool(|| unsafe {
        let screens: Id = msg_send![class!(NSScreen), screens];
        let mut chosen: Id = std::ptr::null_mut();
        for i in 0..ns::array_count(screens) {
            let screen = ns::array_get(screens, i);
            let frame: NSRect = msg_send![screen, frame];
            if rect(frame).contains(x, y) {
                chosen = screen;
                break;
            }
        }
        if chosen.is_null() {
            chosen = msg_send![class!(NSScreen), mainScreen];
        }
        if chosen.is_null() {
            return None;
        }
        let frame: NSRect = msg_send![chosen, frame];
        let visible: NSRect = msg_send![chosen, visibleFrame];
        Some((rect(frame), rect(visible)))
    })
}

/// Height of the primary display, for converting top-left-origin
/// coordinates (Accessibility, CoreGraphics) to Cocoa's.
pub fn primary_screen_height() -> f64 {
    autoreleasepool(|| unsafe {
        let screens: Id = msg_send![class!(NSScreen), screens];
        if ns::array_count(screens) == 0 {
            return 0.0;
        }
        let frame: NSRect = msg_send![ns::array_get(screens, 0), frame];
        frame.size.height
    })
}

/// Mouse pointer position in Cocoa screen coordinates.
pub fn mouse_location() -> (f64, f64) {
    let point: NSPoint = unsafe { msg_send![class!(NSEvent), mouseLocation] };
    (point.x, point.y)
}

fn rect(r: NSRect) -> ScreenRect {
    ScreenRect::new(r.origin.x, r.origin.y, r.size.width, r.size.height)
}

pub fn open_url(url: &str) -> bool {
    autoreleasepool(|| unsafe {
        let ns_url: Id = msg_send![class!(NSURL), URLWithString: ns::string(url)];
        if ns_url.is_null() {
            return false;
        }
        msg_send![shared(), openURL: ns_url]
    })
}

pub fn open_path(path: &str) -> bool {
    autoreleasepool(|| unsafe {
        let ns_url: Id = msg_send![class!(NSURL), fileURLWithPath: ns::string(path)];
        msg_send![shared(), openURL: ns_url]
    })
}

pub fn reveal_in_finder(paths: &[String]) {
    autoreleasepool(|| unsafe {
        let urls: Id = msg_send![class!(NSMutableArray), array];
        for path in paths {
            let url: Id = msg_send![class!(NSURL), fileURLWithPath: ns::string(path)];
            let _: () = msg_send![urls, addObject: url];
        }
        let _: () = msg_send![shared(), activateFileViewerSelectingURLs: urls];
    })
}
