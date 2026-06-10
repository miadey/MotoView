//! Native FFI surface — the C-ABI the native MotoView clients (iOS/SwiftUI,
//! Android/Compose, desktop) call into.
//!
//! This module is compiled ONLY under the default-off `ffi` feature. The default
//! web build (`wasm32-unknown-unknown`, no features) never references it, so the
//! shipped web wasm stays byte-identical (84634 bytes) and pulls ZERO extra deps:
//! everything here uses only `std` + the crate's own `ir`/`diff`/`cert_verify`
//! modules. There is no UniFFI/serde dependency — the surface is a hand-written,
//! cbindgen-style C ABI (the lead-approved fallback). UniFFI's bindgen binary is
//! not on this machine's PATH and vendoring its proc-macro/runtime would add a
//! heavy optional dependency graph for no functional gain over a flat C ABI that
//! the Swift/Kotlin sides decode with their own built-in JSON (Codable / kotlinx).
//!
//! ## Wire model
//!
//! Strings cross as NUL-terminated UTF-8 `char *`. The native side passes a C
//! string in; Rust returns a heap-allocated C string that the caller MUST free
//! with [`mv_ffi_string_free`]. Every fallible call returns a JSON envelope
//!
//! ```json
//! { "ok": true,  "value": <result> }
//! { "ok": false, "error": "<reason>" }
//! ```
//!
//! so the native side never has to interpret a null/empty return as success.
//!
//! ## What is exposed (the native equivalent of the brain's host_* contract)
//!
//! * [`mv_ffi_parse_forest`]   — JSON IR forest -> validated, canonical UINode
//!   forest JSON (the Rust [`ir::parse_forest`] ran; the native side decodes the
//!   normalized JSON into its own `UINode` value type). Round-trips the exact
//!   `Ir.mo.toJsonForest` wire format.
//! * [`mv_ffi_parse_node`]     — same, for a single node.
//! * [`mv_ffi_ir_diff`]        — (oldForest, newForest) -> the keyed-diff Plan as
//!   JSON ops ({"plan":"full"} | {"plan":"patch","ops":[...]}), reusing
//!   [`ir::ir_diff`] verbatim (same LIS keyed reconcile as the web brain).
//! * [`mv_ffi_render_forest`]  — UINode forest -> HTML (for a raw/WebView fallback
//!   container or debugging).
//! * [`mv_ffi_verify_response`]— the chain-key cert verifier
//!   ([`cert_verify::verify_response`]) against the pinned NNS root key. Returns
//!   the certified `/time` (ns) on success or a named error. Gated additionally
//!   on `cert-verify` being compiled (the `ffi` feature turns it on, see
//!   Cargo.toml `ffi = [... "cert-verify"]`).
//!
//! The canonical JSON this module emits is deliberately a SUPERSET-free, fixed
//! key order (`t`, then type-specific fields) so the native decoders are trivial
//! and stable.

use crate::ir::{self, UINode};
use std::ffi::{CStr, CString};
use std::os::raw::c_char;

// ---------------------------------------------------------------------------
// C-string plumbing
// ---------------------------------------------------------------------------

/// Free a C string previously returned by any `mv_ffi_*` function.
///
/// # Safety
/// `ptr` must be a pointer returned by an `mv_ffi_*` call (or null). Passing any
/// other pointer, or freeing twice, is undefined behaviour.
#[no_mangle]
pub unsafe extern "C" fn mv_ffi_string_free(ptr: *mut c_char) {
    if ptr.is_null() {
        return;
    }
    // Retake ownership and drop. The CString was created by `into_owned_cstr`.
    drop(CString::from_raw(ptr));
}

/// Borrow an incoming C string as `&str`. Returns `None` for null or non-UTF-8.
///
/// # Safety
/// `ptr`, if non-null, must point to a valid NUL-terminated C string that
/// outlives the borrow.
unsafe fn borrow_str<'a>(ptr: *const c_char) -> Option<&'a str> {
    if ptr.is_null() {
        return None;
    }
    CStr::from_ptr(ptr).to_str().ok()
}

/// Move a Rust `String` out across the ABI as an owned C string the caller frees.
/// A NUL byte in the payload (impossible for our JSON, but be safe) truncates at
/// the NUL via `CString::new` failure -> we fall back to an error envelope.
fn into_owned_cstr(s: String) -> *mut c_char {
    match CString::new(s) {
        Ok(c) => c.into_raw(),
        Err(_) => CString::new(r#"{"ok":false,"error":"interior-nul"}"#)
            .unwrap()
            .into_raw(),
    }
}

fn ok_envelope(value_json: String) -> *mut c_char {
    into_owned_cstr(format!(r#"{{"ok":true,"value":{value_json}}}"#))
}

fn err_envelope(reason: &str) -> *mut c_char {
    into_owned_cstr(format!(r#"{{"ok":false,"error":"{}"}}"#, json_escape(reason)))
}

// ---------------------------------------------------------------------------
// Canonical JSON serialization of UINode (fixed key order)
// ---------------------------------------------------------------------------

/// JSON-escape a string the same way the brain's IR emitter / `Ir.mo.escape`
/// does (the escape set the parser round-trips): `\ " \n \r \t` plus control
/// bytes via `\u00xx`. Mirrors the decode side in `ir.rs::string`.
fn json_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    for ch in s.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out
}

fn pairs_json(pairs: &[(String, String)]) -> String {
    let mut out = String::from("{");
    for (i, (k, v)) in pairs.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        out.push('"');
        out.push_str(&json_escape(k));
        out.push_str("\":\"");
        out.push_str(&json_escape(v));
        out.push('"');
    }
    out.push('}');
    out
}

/// Serialize a `UINode` to the canonical wire JSON the native decoders read.
/// Key order is fixed: `t` first, then type fields, then (for elements)
/// `attrs`, `events`, optional `key`, `children`. This is the SAME schema
/// `Ir.mo.toJson` emits, so a parse->serialize round-trip is stable.
fn node_json(node: &UINode) -> String {
    match node {
        UINode::Text(v) => format!(r#"{{"t":"text","value":"{}"}}"#, json_escape(v)),
        UINode::Raw(h) => format!(r#"{{"t":"raw","html":"{}"}}"#, json_escape(h)),
        UINode::Element {
            tag,
            attrs,
            events,
            key,
            children,
        } => {
            let mut out = format!(r#"{{"t":"el","tag":"{}"#, json_escape(tag));
            out.push('"');
            out.push_str(",\"attrs\":");
            out.push_str(&pairs_json(attrs));
            out.push_str(",\"events\":");
            out.push_str(&pairs_json(events));
            if let Some(k) = key {
                out.push_str(",\"key\":\"");
                out.push_str(&json_escape(k));
                out.push('"');
            }
            out.push_str(",\"children\":");
            out.push_str(&forest_json(children));
            out.push('}');
            out
        }
    }
}

fn forest_json(forest: &[UINode]) -> String {
    let mut out = String::from("[");
    for (i, n) in forest.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        out.push_str(&node_json(n));
    }
    out.push(']');
    out
}

// ---------------------------------------------------------------------------
// Exposed: parse_forest / parse_node / render_forest
// ---------------------------------------------------------------------------

/// Parse a JSON IR forest (the `Ir.mo.toJsonForest` wire format) and return the
/// validated, canonical forest JSON inside an `{ok,value}` envelope. The Rust
/// [`ir::parse_forest`] does the real validation (depth guard, schema, escape
/// decoding); the native side decodes the normalized `value` into its own tree.
///
/// # Safety
/// `json` must be a valid NUL-terminated UTF-8 C string (or null).
#[no_mangle]
pub unsafe extern "C" fn mv_ffi_parse_forest(json: *const c_char) -> *mut c_char {
    let Some(src) = borrow_str(json) else {
        return err_envelope("null-or-invalid-utf8");
    };
    match ir::parse_forest(src) {
        Ok(forest) => ok_envelope(forest_json(&forest)),
        Err(e) => err_envelope(&format!("parse@{}: {}", e.at, e.msg)),
    }
}

/// Parse a single JSON IR node and return its canonical JSON in an envelope.
///
/// # Safety
/// `json` must be a valid NUL-terminated UTF-8 C string (or null).
#[no_mangle]
pub unsafe extern "C" fn mv_ffi_parse_node(json: *const c_char) -> *mut c_char {
    let Some(src) = borrow_str(json) else {
        return err_envelope("null-or-invalid-utf8");
    };
    match ir::parse_node(src) {
        Ok(node) => ok_envelope(node_json(&node)),
        Err(e) => err_envelope(&format!("parse@{}: {}", e.at, e.msg)),
    }
}

/// Render a JSON IR forest to HTML (used by the native `raw`/WebView fallback
/// container and for debugging). Returns `{ok,value:"<html>"}`.
///
/// # Safety
/// `json` must be a valid NUL-terminated UTF-8 C string (or null).
#[no_mangle]
pub unsafe extern "C" fn mv_ffi_render_forest(json: *const c_char) -> *mut c_char {
    let Some(src) = borrow_str(json) else {
        return err_envelope("null-or-invalid-utf8");
    };
    match ir::parse_forest(src) {
        Ok(forest) => ok_envelope(format!("\"{}\"", json_escape(&ir::render_forest(&forest)))),
        Err(e) => err_envelope(&format!("parse@{}: {}", e.at, e.msg)),
    }
}

// ---------------------------------------------------------------------------
// Exposed: ir_diff -> Plan JSON
// ---------------------------------------------------------------------------

/// Diff two JSON IR forests with the SAME keyed-reconcile/LIS the web brain
/// uses ([`ir::ir_diff`]). Returns a Plan JSON:
///
/// ```json
/// { "ok": true, "value": { "plan": "full" } }
/// { "ok": true, "value": { "plan": "patch", "ops": [
///     {"op":"replace","key":"k","html":"<...>"},
///     {"op":"remove","key":"k"},
///     {"op":"insert","html":"<...>","after":"k"|null},
///     {"op":"move","key":"k","after":"k"|null}
/// ] } }
/// ```
///
/// # Safety
/// `old_json` / `new_json` must be valid NUL-terminated UTF-8 C strings (or null).
#[no_mangle]
pub unsafe extern "C" fn mv_ffi_ir_diff(
    old_json: *const c_char,
    new_json: *const c_char,
) -> *mut c_char {
    let (Some(old_src), Some(new_src)) = (borrow_str(old_json), borrow_str(new_json)) else {
        return err_envelope("null-or-invalid-utf8");
    };
    let old = match ir::parse_forest(old_src) {
        Ok(f) => f,
        Err(e) => return err_envelope(&format!("old-parse@{}: {}", e.at, e.msg)),
    };
    let new = match ir::parse_forest(new_src) {
        Ok(f) => f,
        Err(e) => return err_envelope(&format!("new-parse@{}: {}", e.at, e.msg)),
    };
    ok_envelope(plan_json(&ir::ir_diff(&old, &new)))
}

fn opt_after(after: &Option<String>) -> String {
    match after {
        Some(k) => format!("\"{}\"", json_escape(k)),
        None => "null".to_string(),
    }
}

fn plan_json(plan: &crate::diff::Plan) -> String {
    use crate::diff::{Op, Plan};
    match plan {
        Plan::Full => r#"{"plan":"full"}"#.to_string(),
        Plan::Patch(ops) => {
            let mut out = String::from(r#"{"plan":"patch","ops":["#);
            for (i, op) in ops.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                match op {
                    Op::Replace { key, html } => out.push_str(&format!(
                        r#"{{"op":"replace","key":"{}","html":"{}"}}"#,
                        json_escape(key),
                        json_escape(html)
                    )),
                    Op::Remove { key } => out.push_str(&format!(
                        r#"{{"op":"remove","key":"{}"}}"#,
                        json_escape(key)
                    )),
                    Op::Insert { html, after } => out.push_str(&format!(
                        r#"{{"op":"insert","html":"{}","after":{}}}"#,
                        json_escape(html),
                        opt_after(after)
                    )),
                    Op::Move { key, after } => out.push_str(&format!(
                        r#"{{"op":"move","key":"{}","after":{}}}"#,
                        json_escape(key),
                        opt_after(after)
                    )),
                }
            }
            out.push_str("]}");
            out
        }
    }
}

// ---------------------------------------------------------------------------
// Exposed: cert_verify::verify_response (only when cert-verify is compiled)
// ---------------------------------------------------------------------------

/// Verify an IC response against the PINNED NNS root key (mainnet) — the native
/// client's zero-trust gate before acting on any consequential reply. This wraps
/// [`crate::cert_verify::verify_response`]: tree-hash reconstruction, delegation
/// chain, BLS signature, range check, and `certified_data == SHA256(body)`.
///
/// Inputs are passed as raw byte buffers (not C strings, since they are binary):
/// the CBOR certificate, the canister-id principal bytes, the response body, and
/// the certified-data path encoded as a single buffer of length-prefixed
/// segments: `[u32_le seg_len][seg bytes]...`. `now_ns` and `max_offset_ns` are
/// the freshness window (pass `0` for `max_offset_ns` to use
/// [`cert_verify::DEFAULT_MAX_TIME_OFFSET_NS`]).
///
/// Returns an `{ok,value}` envelope as a C string:
/// `{ "ok": true, "value": { "time_ns": "<u128 as string>" } }` on success, or
/// `{ "ok": false, "error": "<CertError variant>" }` on any failure (fails
/// closed). Always Mainnet root key — there is NO way to inject a local key
/// through this surface, so a mainnet caller can never be downgraded.
///
/// # Safety
/// Each `*_ptr` must point to at least `*_len` valid bytes (or be null with len
/// 0). The path buffer must be well-formed length-prefixed segments.
#[cfg(feature = "cert-verify")]
#[no_mangle]
#[allow(clippy::too_many_arguments)]
pub unsafe extern "C" fn mv_ffi_verify_response(
    cert_ptr: *const u8,
    cert_len: usize,
    canister_ptr: *const u8,
    canister_len: usize,
    path_ptr: *const u8,
    path_len: usize,
    body_ptr: *const u8,
    body_len: usize,
    now_ns_hi: u64,
    now_ns_lo: u64,
    max_offset_ns: u64,
) -> *mut c_char {
    use crate::cert_verify::{verify_response, RootKey, DEFAULT_MAX_TIME_OFFSET_NS};

    let cert = slice_or_empty(cert_ptr, cert_len);
    let canister = slice_or_empty(canister_ptr, canister_len);
    let body = slice_or_empty(body_ptr, body_len);
    let path_buf = slice_or_empty(path_ptr, path_len);

    let segments = match decode_path(path_buf) {
        Ok(s) => s,
        Err(_) => return err_envelope("bad-path-encoding"),
    };
    let path_refs: Vec<&[u8]> = segments.iter().map(|s| s.as_slice()).collect();

    // 128-bit now reassembled from two u64 halves (C ABI has no u128).
    let now_ns: u128 = ((now_ns_hi as u128) << 64) | (now_ns_lo as u128);
    let max_offset = if max_offset_ns == 0 {
        DEFAULT_MAX_TIME_OFFSET_NS
    } else {
        max_offset_ns as u128
    };

    match verify_response(
        cert,
        canister,
        &path_refs,
        body,
        RootKey::Mainnet,
        now_ns,
        max_offset,
    ) {
        Ok(time_ns) => ok_envelope(format!(r#"{{"time_ns":"{time_ns}"}}"#)),
        Err(e) => err_envelope(cert_error_name(&e)),
    }
}

#[cfg(feature = "cert-verify")]
fn cert_error_name(e: &crate::cert_verify::CertError) -> &'static str {
    use crate::cert_verify::CertError::*;
    match e {
        Cbor => "Cbor",
        Structure => "Structure",
        BadTree => "BadTree",
        PathAbsent => "PathAbsent",
        TimeDecode => "TimeDecode",
        TimeOutOfRange { .. } => "TimeOutOfRange",
        DerKey => "DerKey",
        Signature => "Signature",
        NestedDelegation => "NestedDelegation",
        BadRanges => "BadRanges",
        CanisterOutOfRange => "CanisterOutOfRange",
    }
}

#[cfg(feature = "cert-verify")]
unsafe fn slice_or_empty<'a>(ptr: *const u8, len: usize) -> &'a [u8] {
    if ptr.is_null() || len == 0 {
        &[]
    } else {
        std::slice::from_raw_parts(ptr, len)
    }
}

/// Decode the length-prefixed path buffer (`[u32_le len][bytes]...`) into owned
/// segments. An empty buffer yields an empty path.
#[cfg(feature = "cert-verify")]
fn decode_path(buf: &[u8]) -> Result<Vec<Vec<u8>>, ()> {
    let mut out = Vec::new();
    let mut i = 0usize;
    while i < buf.len() {
        if i + 4 > buf.len() {
            return Err(());
        }
        let n = u32::from_le_bytes([buf[i], buf[i + 1], buf[i + 2], buf[i + 3]]) as usize;
        i += 4;
        if i + n > buf.len() {
            return Err(());
        }
        out.push(buf[i..i + n].to_vec());
        i += n;
    }
    Ok(out)
}

// ---------------------------------------------------------------------------
// ABI sanity tests (host build / `cargo test --features ffi`)
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::CString;

    /// Helper: call an FFI fn that takes one C string, return the owned result
    /// String (and free the C buffer).
    unsafe fn call1(f: unsafe extern "C" fn(*const c_char) -> *mut c_char, input: &str) -> String {
        let c = CString::new(input).unwrap();
        let out = f(c.as_ptr());
        let s = CStr::from_ptr(out).to_str().unwrap().to_string();
        mv_ffi_string_free(out);
        s
    }

    #[test]
    fn parse_forest_round_trips_canonical_json() {
        // The exact builder-forest golden from IrTest.mo / ir.rs tests.
        let input = "[{\"t\":\"el\",\"tag\":\"li\",\"attrs\":{\"class\":\"row\"},\"events\":{\"click\":\"pick\"},\"key\":\"it-1\",\"children\":[{\"t\":\"text\",\"value\":\"hi\"}]}]";
        let out = unsafe { call1(mv_ffi_parse_forest, input) };
        assert!(out.starts_with(r#"{"ok":true,"value":"#), "envelope: {out}");
        // Canonical re-serialization must contain the key + nested text.
        assert!(out.contains(r#""tag":"li""#));
        assert!(out.contains(r#""key":"it-1""#));
        assert!(out.contains(r#""t":"text","value":"hi""#));
    }

    #[test]
    fn parse_forest_reports_error_envelope() {
        let out = unsafe { call1(mv_ffi_parse_forest, "[") };
        assert!(out.starts_with(r#"{"ok":false,"error":"#), "{out}");
    }

    #[test]
    fn ir_diff_replace_emits_patch() {
        let old = "[{\"t\":\"el\",\"tag\":\"li\",\"attrs\":{},\"events\":{},\"key\":\"a\",\"children\":[{\"t\":\"text\",\"value\":\"A\"}]}]";
        let new = "[{\"t\":\"el\",\"tag\":\"li\",\"attrs\":{},\"events\":{},\"key\":\"a\",\"children\":[{\"t\":\"text\",\"value\":\"B\"}]}]";
        let co = CString::new(old).unwrap();
        let cn = CString::new(new).unwrap();
        let out = unsafe {
            let p = mv_ffi_ir_diff(co.as_ptr(), cn.as_ptr());
            let s = CStr::from_ptr(p).to_str().unwrap().to_string();
            mv_ffi_string_free(p);
            s
        };
        assert!(out.contains(r#""plan":"patch""#), "{out}");
        assert!(out.contains(r#""op":"replace""#), "{out}");
        assert!(out.contains(r#""key":"a""#), "{out}");
    }

    #[test]
    fn render_forest_emits_html() {
        let input = "[{\"t\":\"el\",\"tag\":\"p\",\"attrs\":{},\"events\":{},\"children\":[{\"t\":\"text\",\"value\":\"x & y\"}]}]";
        let out = unsafe { call1(mv_ffi_render_forest, input) };
        assert!(out.contains("<p>x &amp; y</p>"), "{out}");
    }

    #[cfg(feature = "cert-verify")]
    #[test]
    fn verify_response_fails_closed_on_garbage() {
        // Garbage CBOR must fail closed with a named error, never panic/abort.
        let cert = [0u8; 8];
        let canister = [0u8; 10];
        let body = [1u8, 2, 3];
        let path: [u8; 0] = [];
        let out = unsafe {
            let p = mv_ffi_verify_response(
                cert.as_ptr(),
                cert.len(),
                canister.as_ptr(),
                canister.len(),
                path.as_ptr(),
                path.len(),
                body.as_ptr(),
                body.len(),
                0,
                0,
                0,
            );
            let s = CStr::from_ptr(p).to_str().unwrap().to_string();
            mv_ffi_string_free(p);
            s
        };
        assert!(out.starts_with(r#"{"ok":false,"error":"#), "{out}");
    }

    #[cfg(feature = "cert-verify")]
    #[test]
    fn decode_path_round_trips() {
        // Two segments: "canister" + 3 bytes.
        let mut buf = Vec::new();
        let seg0 = b"canister";
        buf.extend_from_slice(&(seg0.len() as u32).to_le_bytes());
        buf.extend_from_slice(seg0);
        let seg1 = [9u8, 9, 9];
        buf.extend_from_slice(&(seg1.len() as u32).to_le_bytes());
        buf.extend_from_slice(&seg1);
        let segs = decode_path(&buf).unwrap();
        assert_eq!(segs.len(), 2);
        assert_eq!(segs[0], seg0);
        assert_eq!(segs[1], seg1);
    }
}
