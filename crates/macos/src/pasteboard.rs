//! The general pasteboard (SPEC §3.1–5.3, §6.1).

use copywren_core::model::{ClipContents, PasteboardEntry, Representation};
use objc::rc::autoreleasepool;
use objc::{class, msg_send, sel, sel_impl};

use crate::ns::{self, Id};

unsafe fn general() -> Id {
    unsafe { msg_send![class!(NSPasteboard), generalPasteboard] }
}

/// Increments whenever any app (including Copywren) changes the clipboard.
pub fn change_count() -> i64 {
    unsafe { msg_send![general(), changeCount] }
}

/// Every item's type identifiers, with empty data. Cheap: no app is asked to
/// render promised data, so privacy checks can run before `read`.
pub fn read_types() -> ClipContents {
    read_with(false)
}

/// Every item with every representation's bytes.
pub fn read() -> ClipContents {
    read_with(true)
}

fn read_with(include_data: bool) -> ClipContents {
    autoreleasepool(|| unsafe {
        let items: Id = msg_send![general(), pasteboardItems];
        let mut entries = Vec::with_capacity(ns::array_count(items));
        for i in 0..ns::array_count(items) {
            let item = ns::array_get(items, i);
            let types: Id = msg_send![item, types];
            let mut representations = Vec::with_capacity(ns::array_count(types));
            for j in 0..ns::array_count(types) {
                let uti_ns = ns::array_get(types, j);
                let Some(uti) = ns::to_string(uti_ns) else {
                    continue;
                };
                let data = if include_data {
                    let data: Id = msg_send![item, dataForType: uti_ns];
                    if data.is_null() {
                        continue;
                    }
                    ns::data_to_vec(data)
                } else {
                    Vec::new()
                };
                representations.push(Representation { uti, data });
            }
            entries.push(PasteboardEntry { representations });
        }
        ClipContents { entries }
    })
}

/// Replaces the clipboard with `contents`. Returns the new change count so
/// the capture loop can recognise its own write.
pub fn write(contents: &ClipContents) -> i64 {
    autoreleasepool(|| unsafe {
        let pasteboard = general();
        let _: isize = msg_send![pasteboard, clearContents];
        let array: Id = msg_send![class!(NSMutableArray), array];
        for entry in &contents.entries {
            if entry.representations.is_empty() {
                continue;
            }
            let item: Id = msg_send![class!(NSPasteboardItem), new];
            for rep in &entry.representations {
                let _: bool = msg_send![item,
                    setData: ns::data(&rep.data)
                    forType: ns::string(&rep.uti)];
            }
            let _: () = msg_send![array, addObject: item];
            let _: () = msg_send![item, release];
        }
        let _: bool = msg_send![pasteboard, writeObjects: array];
        msg_send![pasteboard, changeCount]
    })
}

pub fn write_plain_text(text: &str) -> i64 {
    write(&ClipContents::plain_text(text))
}
