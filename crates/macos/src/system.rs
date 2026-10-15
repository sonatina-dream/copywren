//! App-wide settings: activation policy, login item, sounds.

use objc::rc::autoreleasepool;
use objc::runtime::Object;
use objc::{class, msg_send, sel, sel_impl};

use crate::ns::{self, Id};

#[link(name = "ServiceManagement", kind = "framework")]
unsafe extern "C" {}

/// Runs without a Dock icon or app menu, like a menu bar utility.
pub fn become_accessory_app() {
    unsafe {
        let app: Id = msg_send![class!(NSApplication), sharedApplication];
        // NSApplicationActivationPolicyAccessory
        let _: bool = msg_send![app, setActivationPolicy: 1isize];
    }
}

/// Brings Copywren itself forward (used before showing the Settings window).
pub fn activate_self() {
    unsafe {
        let app: Id = msg_send![class!(NSApplication), sharedApplication];
        let _: () = msg_send![app, activateIgnoringOtherApps: true];
    }
}

/// Plays a built-in system sound such as "Pop" or "Tink".
pub fn play_sound(name: &str) {
    autoreleasepool(|| unsafe {
        let sound: Id = msg_send![class!(NSSound), soundNamed: ns::string(name)];
        if !sound.is_null() {
            let _: bool = msg_send![sound, play];
        }
    })
}

/// True when running from a `.app` bundle (login items need one).
pub fn is_bundled() -> bool {
    autoreleasepool(|| unsafe {
        let bundle: Id = msg_send![class!(NSBundle), mainBundle];
        let id: Id = msg_send![bundle, bundleIdentifier];
        !id.is_null()
    })
}

unsafe fn main_app_service() -> Option<Id> {
    let class = objc::runtime::Class::get("SMAppService")?;
    let service: Id = unsafe { msg_send![class, mainAppService] };
    (!service.is_null()).then_some(service)
}

pub fn is_open_at_login() -> bool {
    autoreleasepool(|| unsafe {
        // SMAppServiceStatusEnabled = 1
        main_app_service().is_some_and(|s| {
            let status: isize = msg_send![s, status];
            status == 1
        })
    })
}

pub fn set_open_at_login(enabled: bool) -> Result<(), String> {
    if !is_bundled() {
        return Err("Open at login needs Copywren to run from its .app bundle".into());
    }
    autoreleasepool(|| unsafe {
        let Some(service) = main_app_service() else {
            return Err("SMAppService is unavailable".into());
        };
        let mut error: *mut Object = std::ptr::null_mut();
        let ok: bool = if enabled {
            msg_send![service, registerAndReturnError: &mut error]
        } else {
            msg_send![service, unregisterAndReturnError: &mut error]
        };
        if ok {
            Ok(())
        } else {
            let description: Id = if error.is_null() {
                std::ptr::null_mut()
            } else {
                msg_send![error, localizedDescription]
            };
            Err(ns::to_string(description).unwrap_or_else(|| "unknown error".into()))
        }
    })
}

pub fn open_accessibility_settings() {
    crate::workspace::open_url(
        "x-apple.systempreferences:com.apple.preference.security?Privacy_Accessibility",
    );
}
