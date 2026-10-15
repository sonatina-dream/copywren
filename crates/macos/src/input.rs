//! Accessibility trust and synthetic ⌘V (SPEC §4.1, §8).

use std::os::raw::c_void;

use core_foundation::base::{CFRelease, CFTypeRef, TCFType};
use core_foundation::boolean::CFBoolean;
use core_foundation::dictionary::CFDictionary;
use core_foundation::string::{CFString, CFStringRef};
use core_graphics::event::{CGEvent, CGEventFlags, CGEventTapLocation, CGKeyCode};
use core_graphics::event_source::{CGEventSource, CGEventSourceStateID};

type AXUIElementRef = *const c_void;

#[repr(C)]
#[derive(Default, Clone, Copy)]
struct CGRectRaw {
    x: f64,
    y: f64,
    width: f64,
    height: f64,
}

#[link(name = "ApplicationServices", kind = "framework")]
unsafe extern "C" {
    fn AXIsProcessTrustedWithOptions(options: core_foundation::dictionary::CFDictionaryRef)
    -> bool;
    static kAXTrustedCheckOptionPrompt: CFStringRef;
    fn AXUIElementCreateSystemWide() -> AXUIElementRef;
    fn AXUIElementSetMessagingTimeout(element: AXUIElementRef, seconds: f32) -> i32;
    fn AXUIElementCopyAttributeValue(
        element: AXUIElementRef,
        attribute: CFStringRef,
        value: *mut CFTypeRef,
    ) -> i32;
    fn AXUIElementCopyParameterizedAttributeValue(
        element: AXUIElementRef,
        attribute: CFStringRef,
        parameter: CFTypeRef,
        value: *mut CFTypeRef,
    ) -> i32;
    fn AXValueGetValue(value: CFTypeRef, kind: u32, out: *mut c_void) -> bool;
}

const AX_VALUE_CG_POINT: u32 = 1;
const AX_VALUE_CG_SIZE: u32 = 2;
const AX_VALUE_CG_RECT: u32 = 3;
/// Text fields taller than this are text views; their frame says nothing
/// about where the caret is.
const MAX_FIELD_HEIGHT: f64 = 120.0;

/// Virtual key code of the V key on an ANSI layout.
const KEY_V: CGKeyCode = 9;
/// Device-dependent "left Command" bit, set by real keyboards alongside the
/// device-independent Command flag; some apps check for it.
const LEFT_COMMAND_DEVICE_BIT: u64 = 0x8;

/// Whether this process may post keyboard events to other apps. With
/// `prompt`, macOS shows its "allow accessibility access" dialog when the
/// process isn't trusted yet.
pub fn is_trusted(prompt: bool) -> bool {
    unsafe {
        let key = CFString::wrap_under_get_rule(kAXTrustedCheckOptionPrompt);
        let value = if prompt {
            CFBoolean::true_value()
        } else {
            CFBoolean::false_value()
        };
        let options = CFDictionary::from_CFType_pairs(&[(key.as_CFType(), value.as_CFType())]);
        AXIsProcessTrustedWithOptions(options.as_concrete_TypeRef())
    }
}

/// Posts ⌘V to whichever app is frontmost.
pub fn post_paste() -> bool {
    let Ok(source) = CGEventSource::new(CGEventSourceStateID::CombinedSessionState) else {
        return false;
    };
    let flags =
        CGEventFlags::CGEventFlagCommand | CGEventFlags::from_bits_retain(LEFT_COMMAND_DEVICE_BIT);
    for key_down in [true, false] {
        let Ok(event) = CGEvent::new_keyboard_event(source.clone(), KEY_V, key_down) else {
            return false;
        };
        event.set_flags(flags);
        event.post(CGEventTapLocation::Session);
    }
    true
}

/// A CoreFoundation object we own (released on drop).
struct Owned(CFTypeRef);

impl Drop for Owned {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe { CFRelease(self.0) };
        }
    }
}

unsafe fn copy_attribute(element: CFTypeRef, name: &str) -> Option<Owned> {
    let name = CFString::new(name);
    let mut value: CFTypeRef = std::ptr::null();
    let status =
        unsafe { AXUIElementCopyAttributeValue(element, name.as_concrete_TypeRef(), &mut value) };
    (status == 0 && !value.is_null()).then_some(Owned(value))
}

/// Where the focused app's text cursor is, in top-left-origin screen
/// coordinates: `(x, y, width, height)`. Falls back to the frame of a
/// focused single-line field. `None` when the app doesn't say (or
/// Accessibility access isn't granted).
pub fn focused_caret_rect() -> Option<(f64, f64, f64, f64)> {
    unsafe {
        let system = Owned(AXUIElementCreateSystemWide());
        // Never let a hung app stall the popup.
        AXUIElementSetMessagingTimeout(system.0, 0.2);
        let focused = copy_attribute(system.0, "AXFocusedUIElement")?;

        if let Some(range) = copy_attribute(focused.0, "AXSelectedTextRange") {
            let name = CFString::new("AXBoundsForRange");
            let mut bounds: CFTypeRef = std::ptr::null();
            let status = AXUIElementCopyParameterizedAttributeValue(
                focused.0,
                name.as_concrete_TypeRef(),
                range.0,
                &mut bounds,
            );
            if status == 0 && !bounds.is_null() {
                let bounds = Owned(bounds);
                let mut rect = CGRectRaw::default();
                if AXValueGetValue(
                    bounds.0,
                    AX_VALUE_CG_RECT,
                    &mut rect as *mut _ as *mut c_void,
                ) && rect.height > 0.0
                    && (rect.x != 0.0 || rect.y != 0.0)
                {
                    return Some((rect.x, rect.y, rect.width.max(1.0), rect.height));
                }
            }
        }

        let position = copy_attribute(focused.0, "AXPosition")?;
        let size = copy_attribute(focused.0, "AXSize")?;
        let mut origin = (0.0f64, 0.0f64);
        let mut extent = (0.0f64, 0.0f64);
        let ok = AXValueGetValue(
            position.0,
            AX_VALUE_CG_POINT,
            &mut origin as *mut _ as *mut c_void,
        ) && AXValueGetValue(
            size.0,
            AX_VALUE_CG_SIZE,
            &mut extent as *mut _ as *mut c_void,
        );
        (ok && extent.1 > 0.0 && extent.1 <= MAX_FIELD_HEIGHT)
            .then_some((origin.0, origin.1, extent.0, extent.1))
    }
}
