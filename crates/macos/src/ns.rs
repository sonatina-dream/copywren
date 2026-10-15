//! Foundation conversions shared by the other modules.

use std::ffi::CStr;
use std::os::raw::{c_char, c_void};

use objc::runtime::Object;
use objc::{class, msg_send, sel, sel_impl};

pub type Id = *mut Object;

const NS_UTF8_STRING_ENCODING: usize = 4;

/// An autoreleased `NSString` with the contents of `s`.
pub unsafe fn string(s: &str) -> Id {
    unsafe {
        let alloc: Id = msg_send![class!(NSString), alloc];
        let ns: Id = msg_send![alloc,
            initWithBytes: s.as_ptr() as *const c_void
            length: s.len()
            encoding: NS_UTF8_STRING_ENCODING];
        msg_send![ns, autorelease]
    }
}

pub unsafe fn to_string(ns: Id) -> Option<String> {
    if ns.is_null() {
        return None;
    }
    unsafe {
        let ptr: *const c_char = msg_send![ns, UTF8String];
        if ptr.is_null() {
            return None;
        }
        Some(CStr::from_ptr(ptr).to_string_lossy().into_owned())
    }
}

/// An autoreleased `NSData` copy of `bytes`.
pub unsafe fn data(bytes: &[u8]) -> Id {
    unsafe {
        msg_send![class!(NSData),
            dataWithBytes: bytes.as_ptr() as *const c_void
            length: bytes.len()]
    }
}

pub unsafe fn data_to_vec(data: Id) -> Vec<u8> {
    if data.is_null() {
        return Vec::new();
    }
    unsafe {
        let len: usize = msg_send![data, length];
        if len == 0 {
            return Vec::new();
        }
        let ptr: *const u8 = msg_send![data, bytes];
        std::slice::from_raw_parts(ptr, len).to_vec()
    }
}

pub unsafe fn array_count(array: Id) -> usize {
    if array.is_null() {
        return 0;
    }
    unsafe { msg_send![array, count] }
}

pub unsafe fn array_get(array: Id, index: usize) -> Id {
    unsafe { msg_send![array, objectAtIndex: index] }
}
