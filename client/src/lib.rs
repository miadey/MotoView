//! The MotoView browser client — the protocol/polling "brain", compiled to
//! `wasm32-unknown-unknown`.
//!
//! Responsibilities (all decision logic lives here, in Rust/WASM):
//!   * adaptive polling state machine (hot / warm / cold / hidden / offline)
//!   * the render-as-query + event-as-update protocol
//!   * event sequencing + idempotency keys
//!   * interpreting render batches (changed / unchanged / redirect / errors)
//!
//! The JS glue ("the hands") provides only the unavoidable DOM/`fetch`/timer
//! primitives the browser exposes solely to JavaScript.

mod abi;
mod json;
mod diff;

use abi::{host, take_string};
use json::Value;
use std::cell::RefCell;
use std::collections::HashMap;

// ---- timing (ms) ----------------------------------------------------------
const HOT_MS: f64 = 350.0;
const WARM_MS: f64 = 2500.0;
const COLD_MS: f64 = 15000.0;
const HIDDEN_MS: f64 = 45000.0;
const HOT_WINDOW_MS: f64 = 3000.0; // stay hot this long after an interaction
const COLD_AFTER_MS: f64 = 20000.0; // idle this long (visible) -> cold
const BACKOFF_START_MS: f64 = 1000.0;
const BACKOFF_MAX_MS: f64 = 30000.0;

#[derive(Clone, Copy, PartialEq)]
enum ReqKind {
    Render,
    Event,
}

struct Bridge {
    client_id: String,
    path: String,
    last_batch_id: String,
    event_seq: u64,
    timer_gen: u32,
    req_gen: u32,
    pending: Vec<(u32, ReqKind)>,
    hidden: bool,
    hot_until: f64,
    last_change: f64,
    backoff: f64,
    started: bool,
    /// Last HTML applied per target, so the brain can diff and patch only the
    /// keyed regions that changed (see `diff`).
    last_html: HashMap<String, String>,
}

impl Bridge {
    fn new() -> Self {
        Bridge {
            client_id: String::new(),
            path: "/".into(),
            last_batch_id: String::new(),
            event_seq: 0,
            timer_gen: 0,
            req_gen: 0,
            pending: Vec::new(),
            hidden: false,
            hot_until: 0.0,
            last_change: 0.0,
            backoff: BACKOFF_START_MS,
            started: false,
            last_html: HashMap::new(),
        }
    }

    fn next_req(&mut self, kind: ReqKind) -> u32 {
        self.req_gen = self.req_gen.wrapping_add(1);
        self.pending.push((self.req_gen, kind));
        if self.pending.len() > 64 {
            self.pending.remove(0);
        }
        self.req_gen
    }

    fn take_kind(&mut self, req_id: u32) -> Option<ReqKind> {
        if let Some(idx) = self.pending.iter().position(|(id, _)| *id == req_id) {
            Some(self.pending.remove(idx).1)
        } else {
            None
        }
    }

    /// Interval for the current mode.
    fn interval(&self, now: f64) -> f64 {
        if self.hidden {
            HIDDEN_MS
        } else if now < self.hot_until {
            HOT_MS
        } else if now - self.last_change > COLD_AFTER_MS {
            COLD_MS
        } else {
            WARM_MS
        }
    }

    fn schedule(&mut self, ms: f64) {
        self.timer_gen = self.timer_gen.wrapping_add(1);
        host::set_timer(self.timer_gen, ms);
    }

    fn go_hot(&mut self, now: f64) {
        self.hot_until = now + HOT_WINDOW_MS;
        self.last_change = now;
        self.backoff = BACKOFF_START_MS;
    }

    fn poll_render(&mut self) {
        let url = format!(
            "/_motoview/render?path={}&lastBatchId={}",
            encode(&self.path),
            encode(&self.last_batch_id)
        );
        let id = self.next_req(ReqKind::Render);
        host::fetch(id, "GET", &url, "");
    }

    /// Build and POST an event. `frag` is a pre-encoded form fragment built by
    /// the glue (handler/event/args/form/secure/token/schema).
    fn send_event(&mut self, frag: &str) {
        self.event_seq += 1;
        let body = format!(
            "{frag}&__mv_path={}&__mv_batch={}&__mv_client={}&__mv_seq={}&__mv_idem={}-{}",
            encode(&self.path),
            encode(&self.last_batch_id),
            encode(&self.client_id),
            self.event_seq,
            encode(&self.client_id),
            self.event_seq,
        );
        let id = self.next_req(ReqKind::Event);
        host::fetch(id, "POST", "/_motoview/event", &body);
    }

    /// Apply a parsed batch. Returns true if the page content changed.
    fn apply(&mut self, v: &Value, now: f64) -> bool {
        let status = v.str_field("status");
        match status.as_str() {
            "unchanged" => {
                let bid = v.str_field("batchId");
                if !bid.is_empty() {
                    self.last_batch_id = bid;
                }
                false
            }
            "redirect" => {
                let loc = v.str_field("location");
                if !loc.is_empty() {
                    host::navigate(&loc);
                }
                false
            }
            "changed" | "validation-error" => {
                self.last_batch_id = v.str_field("batchId");
                let target = {
                    let t = v.str_field("target");
                    if t.is_empty() { "mv-root".to_string() } else { t }
                };
                let html = v.str_field("html");
                // Keyed-region patching: when the structure is stable, replace
                // only the keyed regions that changed; otherwise full swap. All
                // the decision logic is here in the brain — the hands just apply.
                let old = self.last_html.get(&target).cloned().unwrap_or_default();
                match diff::plan(&old, &html) {
                    diff::Plan::Patch(ops) => {
                        for op in &ops {
                            match op {
                                diff::Op::Replace { key, html } => host::replace_keyed(&target, key, html),
                                diff::Op::Remove { key } => host::remove_keyed(&target, key),
                                diff::Op::Insert { html, after } => {
                                    host::insert_keyed(&target, html, after.as_deref().unwrap_or(""))
                                }
                                diff::Op::Move { key, after } => {
                                    host::move_keyed(&target, key, after.as_deref().unwrap_or(""))
                                }
                            }
                        }
                    }
                    diff::Plan::Full => host::apply_html(&target, &html),
                }
                self.last_html.insert(target.clone(), html);
                if let Some(head) = v.get("head") {
                    let title = head.str_field("title");
                    if !title.is_empty() {
                        host::set_title(&title);
                    }
                }
                if let Some(effects) = v.get("effects") {
                    for e in effects.as_array() {
                        host::effect(&e.str_field("type"), &e.str_field("target"), &e.str_field("value"));
                    }
                }
                self.go_hot(now);
                true
            }
            _ => false,
        }
    }
}

thread_local! {
    static BRIDGE: RefCell<Bridge> = RefCell::new(Bridge::new());
}

fn with<R>(f: impl FnOnce(&mut Bridge) -> R) -> R {
    BRIDGE.with(|b| f(&mut b.borrow_mut()))
}

// ---- exports called by the JS glue ----------------------------------------

/// Initialize the bridge for the current page. `path` and `batch` are read
/// from the SSR document by the glue so the client starts already in sync.
///
/// # Safety
/// Pointers must be `mv_alloc`-owned buffers.
#[no_mangle]
pub unsafe extern "C" fn mv_start(
    path_ptr: *mut u8,
    path_len: usize,
    batch_ptr: *mut u8,
    batch_len: usize,
    seed_ptr: *mut u8,
    seed_len: usize,
) {
    let path = take_string(path_ptr, path_len);
    let batch = take_string(batch_ptr, batch_len);
    // The server-rendered root HTML — so the brain can diff the FIRST event
    // against it and keyed-patch from the very first interaction.
    let seed = take_string(seed_ptr, seed_len);
    let now = host::now();
    with(|b| {
        if b.started {
            return;
        }
        b.started = true;
        if !seed.is_empty() {
            b.last_html.insert("mv-root".to_string(), seed);
        }
        b.path = if path.is_empty() { "/".into() } else { path };
        b.last_batch_id = batch;
        b.client_id = format!("c{}", (now as u64));
        b.last_change = now;
        b.go_hot(now);
        let ms = b.interval(now);
        b.schedule(ms);
    });
}

/// A DOM event captured by the glue. `frag` is a pre-encoded form fragment.
///
/// # Safety
/// Pointer must be an `mv_alloc`-owned buffer.
#[no_mangle]
pub unsafe extern "C" fn mv_on_event(frag_ptr: *mut u8, frag_len: usize) {
    let frag = take_string(frag_ptr, frag_len);
    with(|b| b.send_event(&frag));
}

/// A scheduled timer fired. Drives the polling loop.
#[no_mangle]
pub extern "C" fn mv_on_timer(timer_id: u32) {
    with(|b| {
        if timer_id != b.timer_gen {
            return; // stale timer
        }
        b.poll_render();
    });
}

/// A fetch initiated via `host_fetch` completed.
///
/// # Safety
/// `body_ptr`/`body_len` must be an `mv_alloc`-owned buffer (may be empty).
#[no_mangle]
pub unsafe extern "C" fn mv_on_response(req_id: u32, status: u32, body_ptr: *mut u8, body_len: usize) {
    let body = take_string(body_ptr, body_len);
    let now = host::now();
    with(|b| {
        let kind = b.take_kind(req_id);
        if status == 0 || status >= 500 {
            // network / server error -> back off, then resume polling
            let wait = b.backoff;
            b.backoff = (b.backoff * 2.0).min(BACKOFF_MAX_MS);
            b.schedule(wait);
            return;
        }
        let _ = kind; // taken only to evict from the pending set
        b.backoff = BACKOFF_START_MS;
        if let Some(v) = json::parse(&body) {
            let _ = b.apply(&v, now);
        }
        // Always reschedule the next poll. Even if this response was for an
        // evicted/superseded request, the loop must keep going. Stale timers
        // are ignored via the generation check in mv_on_timer.
        let ms = b.interval(now);
        b.schedule(ms);
    });
}

/// Document visibility changed (1 = hidden, 0 = visible).
#[no_mangle]
pub extern "C" fn mv_on_visibility(hidden: u32) {
    let now = host::now();
    with(|b| {
        let was_hidden = b.hidden;
        b.hidden = hidden != 0;
        if was_hidden && !b.hidden {
            // becoming visible: refresh promptly
            b.go_hot(now);
            b.schedule(0.0);
        }
    });
}

/// Single-page navigation requested by app code / a link. Re-points the bridge
/// at a new path and refreshes.
///
/// # Safety
/// Pointer must be an `mv_alloc`-owned buffer.
#[no_mangle]
pub unsafe extern "C" fn mv_navigate(path_ptr: *mut u8, path_len: usize) {
    let path = take_string(path_ptr, path_len);
    let now = host::now();
    with(|b| {
        b.path = if path.is_empty() { "/".into() } else { path };
        b.last_batch_id = String::new();
        b.go_hot(now);
        b.schedule(0.0);
    });
}

// ---- helpers ---------------------------------------------------------------

/// Percent-encode a value for use in a URL query / form body.
fn encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for &byte in s.as_bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(byte as char)
            }
            _ => out.push_str(&format!("%{:02X}", byte)),
        }
    }
    out
}
