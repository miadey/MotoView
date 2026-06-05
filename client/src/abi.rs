//! Low-level ABI between the WASM "brain" and the JS "hands".
//!
//! Strings cross the boundary as (ptr, len) byte slices over the shared linear
//! memory. JS allocates with [`mv_alloc`] before calling an export that takes
//! ownership; WASM passes borrowed (ptr, len) when calling host imports, and JS
//! copies out synchronously.

use std::alloc::{alloc, dealloc, Layout};

/// Allocate `len` bytes in WASM linear memory (align 1). Called by JS.
#[no_mangle]
pub extern "C" fn mv_alloc(len: usize) -> *mut u8 {
    if len == 0 {
        return std::ptr::null_mut();
    }
    // SAFETY: layout is valid for len >= 1, align 1.
    unsafe { alloc(Layout::from_size_align(len, 1).unwrap()) }
}

/// Free a buffer previously returned by [`mv_alloc`]. Called by JS.
#[no_mangle]
pub extern "C" fn mv_dealloc(ptr: *mut u8, len: usize) {
    if ptr.is_null() || len == 0 {
        return;
    }
    // SAFETY: ptr/len came from mv_alloc with align 1.
    unsafe { dealloc(ptr, Layout::from_size_align(len, 1).unwrap()) }
}

/// Take ownership of a (ptr, len) buffer that JS allocated via `mv_alloc`,
/// returning it as an owned `String` (freed when dropped).
///
/// # Safety
/// `ptr` must have been produced by `mv_alloc(len)` and not yet freed.
pub unsafe fn take_string(ptr: *mut u8, len: usize) -> String {
    if ptr.is_null() || len == 0 {
        return String::new();
    }
    let v = Vec::from_raw_parts(ptr, len, len);
    String::from_utf8_lossy(&v).into_owned()
}

/// Host functions implemented by the JS glue.
pub mod host {
    extern "C" {
        fn host_log(ptr: *const u8, len: usize);
        fn host_now() -> f64;
        fn host_fetch(
            req_id: u32,
            method_ptr: *const u8,
            method_len: usize,
            url_ptr: *const u8,
            url_len: usize,
            body_ptr: *const u8,
            body_len: usize,
        );
        fn host_set_timer(timer_id: u32, ms: f64);
        fn host_apply_html(
            target_ptr: *const u8,
            target_len: usize,
            html_ptr: *const u8,
            html_len: usize,
        );
        fn host_replace_keyed(
            target_ptr: *const u8,
            target_len: usize,
            key_ptr: *const u8,
            key_len: usize,
            html_ptr: *const u8,
            html_len: usize,
        );
        fn host_effect(
            kind_ptr: *const u8,
            kind_len: usize,
            target_ptr: *const u8,
            target_len: usize,
            value_ptr: *const u8,
            value_len: usize,
        );
        fn host_navigate(url_ptr: *const u8, url_len: usize);
        fn host_set_title(ptr: *const u8, len: usize);
    }

    pub fn log(s: &str) {
        unsafe { host_log(s.as_ptr(), s.len()) }
    }
    pub fn now() -> f64 {
        unsafe { host_now() }
    }
    pub fn fetch(req_id: u32, method: &str, url: &str, body: &str) {
        unsafe {
            host_fetch(
                req_id,
                method.as_ptr(),
                method.len(),
                url.as_ptr(),
                url.len(),
                body.as_ptr(),
                body.len(),
            )
        }
    }
    pub fn set_timer(timer_id: u32, ms: f64) {
        unsafe { host_set_timer(timer_id, ms) }
    }
    pub fn apply_html(target: &str, html: &str) {
        unsafe { host_apply_html(target.as_ptr(), target.len(), html.as_ptr(), html.len()) }
    }
    /// Replace just the keyed region `key` inside `target` with `html`. The
    /// brain decides which regions changed; the hands only swap the node.
    pub fn replace_keyed(target: &str, key: &str, html: &str) {
        unsafe {
            host_replace_keyed(
                target.as_ptr(),
                target.len(),
                key.as_ptr(),
                key.len(),
                html.as_ptr(),
                html.len(),
            )
        }
    }
    pub fn effect(kind: &str, target: &str, value: &str) {
        unsafe {
            host_effect(
                kind.as_ptr(),
                kind.len(),
                target.as_ptr(),
                target.len(),
                value.as_ptr(),
                value.len(),
            )
        }
    }
    pub fn navigate(url: &str) {
        unsafe { host_navigate(url.as_ptr(), url.len()) }
    }
    pub fn set_title(t: &str) {
        unsafe { host_set_title(t.as_ptr(), t.len()) }
    }
}
