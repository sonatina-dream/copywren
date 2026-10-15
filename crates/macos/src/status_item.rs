//! The menu bar item and its menu (SPEC §7).

use std::cell::RefCell;
use std::sync::Once;

use objc::declare::ClassDecl;
use objc::rc::autoreleasepool;
use objc::runtime::{Class, Object, Sel};
use objc::{class, msg_send, sel, sel_impl};

use crate::ns::{self, Id};

pub enum MenuEntry {
    Item {
        title: String,
        /// Delivered to the selection callback.
        tag: i64,
        enabled: bool,
        /// Display-only key equivalent, e.g. ("v", ⌘⇧).
        key_equivalent: Option<(String, KeyMask)>,
    },
    Submenu {
        title: String,
        entries: Vec<MenuEntry>,
    },
    Separator,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct KeyMask {
    pub cmd: bool,
    pub shift: bool,
    pub alt: bool,
    pub ctrl: bool,
}

impl KeyMask {
    fn bits(self) -> usize {
        let mut bits = 0;
        if self.shift {
            bits |= 1 << 17;
        }
        if self.ctrl {
            bits |= 1 << 18;
        }
        if self.alt {
            bits |= 1 << 19;
        }
        if self.cmd {
            bits |= 1 << 20;
        }
        bits
    }
}

impl MenuEntry {
    pub fn item(title: impl Into<String>, tag: i64) -> Self {
        MenuEntry::Item {
            title: title.into(),
            tag,
            enabled: true,
            key_equivalent: None,
        }
    }
}

type SelectCallback = Box<dyn Fn(i64)>;

thread_local! {
    static ON_SELECT: RefCell<Option<SelectCallback>> = const { RefCell::new(None) };
    static TARGET: RefCell<Id> = const { RefCell::new(std::ptr::null_mut()) };
}

extern "C" fn menu_item_selected(_this: &Object, _sel: Sel, sender: Id) {
    let tag: isize = unsafe { msg_send![sender, tag] };
    ON_SELECT.with(|cb| {
        if let Some(cb) = cb.borrow().as_ref() {
            cb(tag as i64);
        }
    });
}

fn target_class() -> &'static Class {
    static REGISTER: Once = Once::new();
    REGISTER.call_once(|| {
        let mut decl = ClassDecl::new("CopywrenMenuTarget", class!(NSObject))
            .expect("CopywrenMenuTarget registered twice");
        unsafe {
            decl.add_method(
                sel!(menuItemSelected:),
                menu_item_selected as extern "C" fn(&Object, Sel, Id),
            );
        }
        decl.register();
    });
    Class::get("CopywrenMenuTarget").expect("CopywrenMenuTarget class")
}

fn target() -> Id {
    TARGET.with(|t| {
        let mut t = t.borrow_mut();
        if t.is_null() {
            *t = unsafe { msg_send![target_class(), new] };
        }
        *t
    })
}

pub struct StatusItem {
    item: Id,
}

impl StatusItem {
    /// Adds an item to the menu bar. `on_select` receives the `tag` of the
    /// chosen menu entry, on the main thread.
    pub fn new(symbol: &str, on_select: impl Fn(i64) + 'static) -> Self {
        ON_SELECT.with(|cb| *cb.borrow_mut() = Some(Box::new(on_select)));
        let item: Id = unsafe {
            let bar: Id = msg_send![class!(NSStatusBar), systemStatusBar];
            // NSVariableStatusItemLength
            let item: Id = msg_send![bar, statusItemWithLength: -1.0f64];
            msg_send![item, retain]
        };
        let status = StatusItem { item };
        status.set_symbol(symbol, "Copywren");
        status
    }

    /// Sets the button image to an SF Symbol.
    pub fn set_symbol(&self, symbol: &str, description: &str) {
        autoreleasepool(|| unsafe {
            let button: Id = msg_send![self.item, button];
            let image: Id = msg_send![class!(NSImage),
                imageWithSystemSymbolName: ns::string(symbol)
                accessibilityDescription: ns::string(description)];
            if !image.is_null() {
                let _: () = msg_send![image, setTemplate: true];
                let _: () = msg_send![button, setImage: image];
            } else {
                let _: () = msg_send![button, setTitle: ns::string("P2")];
            }
        })
    }

    /// Sets the button image to a template PNG drawn at `points` points.
    pub fn set_template_png(&self, png: &[u8], points: f64, description: &str) {
        autoreleasepool(|| unsafe {
            let button: Id = msg_send![self.item, button];
            let image: Id = msg_send![class!(NSImage), alloc];
            let image: Id = msg_send![image, initWithData: ns::data(png)];
            if image.is_null() {
                return;
            }
            let _: () = msg_send![image, setSize: cocoa::foundation::NSSize::new(points, points)];
            let _: () = msg_send![image, setTemplate: true];
            let _: () = msg_send![image, setAccessibilityDescription: ns::string(description)];
            let _: () = msg_send![button, setImage: image];
            let _: () = msg_send![image, release];
        })
    }

    /// Greys the button out, e.g. while capture is paused.
    pub fn set_dimmed(&self, dimmed: bool) {
        unsafe {
            let button: Id = msg_send![self.item, button];
            let _: () = msg_send![button, setAppearsDisabled: dimmed];
        }
    }

    pub fn set_visible(&self, visible: bool) {
        unsafe {
            let _: () = msg_send![self.item, setVisible: visible];
        }
    }

    pub fn set_menu(&self, entries: &[MenuEntry]) {
        autoreleasepool(|| unsafe {
            let menu = build_menu(entries);
            let _: () = msg_send![self.item, setMenu: menu];
        })
    }
}

unsafe fn build_menu(entries: &[MenuEntry]) -> Id {
    unsafe {
        let menu: Id = msg_send![class!(NSMenu), new];
        let _: () = msg_send![menu, setAutoenablesItems: false];
        for entry in entries {
            let item: Id = match entry {
                MenuEntry::Separator => msg_send![class!(NSMenuItem), separatorItem],
                MenuEntry::Item {
                    title,
                    tag,
                    enabled,
                    key_equivalent,
                } => {
                    let (key, mask) = key_equivalent
                        .as_ref()
                        .map(|(k, m)| (k.as_str(), *m))
                        .unwrap_or(("", KeyMask::default()));
                    let item: Id = msg_send![class!(NSMenuItem), alloc];
                    let item: Id = msg_send![item,
                        initWithTitle: ns::string(title)
                        action: sel!(menuItemSelected:)
                        keyEquivalent: ns::string(key)];
                    let _: () = msg_send![item, setKeyEquivalentModifierMask: mask.bits()];
                    let _: () = msg_send![item, setTarget: target()];
                    let _: () = msg_send![item, setTag: *tag as isize];
                    let _: () = msg_send![item, setEnabled: *enabled];
                    msg_send![item, autorelease]
                }
                MenuEntry::Submenu { title, entries } => {
                    let item: Id = msg_send![class!(NSMenuItem), new];
                    let _: () = msg_send![item, setTitle: ns::string(title)];
                    let submenu = build_menu(entries);
                    let _: () = msg_send![item, setSubmenu: submenu];
                    msg_send![item, autorelease]
                }
            };
            let _: () = msg_send![menu, addItem: item];
        }
        msg_send![menu, autorelease]
    }
}
