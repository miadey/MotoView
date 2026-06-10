//! Portable UI-IR in the brain — the Rust mirror of `runtime/src/Ir.mo`.
//!
//! Slice 6 added a SECOND compiler backend that emits a JSON UI node tree
//! (a "forest" of top-level nodes) instead of (or alongside) HTML. This module
//! is the client-side counterpart: it
//!
//!   1. parses that JSON forest into a [`UINode`] tree (matching the LOCKED
//!      schema `Ir.mo.toJson` produces — exactly),
//!   2. structurally diffs two forests using the SAME keyed-reconcile / LIS the
//!      HTML path uses ([`crate::diff::lis_stable_set`]), and
//!   3. renders nodes to the same HTML the hands already know how to apply, so a
//!      `ui` batch flows through the unchanged `host_*` keyed-op ABI.
//!
//! The HTML apply path in `diff.rs` is left completely intact; this is a
//! PARALLEL path that the web brain does not use today (it still ships HTML).
//! It exists so native renderers (and a future IR-driven web brain) can consume
//! the portable tree without a second reconcile implementation.
//!
//! Why a bespoke parser instead of `json.rs`? `json::Value` stores objects in a
//! `BTreeMap`, which sorts (and so loses) the order of `attrs`/`events`. The IR
//! contract preserves attribute/event order, so we parse straight into the
//! ordered `Vec<(String,String)>` the node type demands. The string-escape
//! decoding mirrors `json.rs` byte-for-byte.
//!
//! This module ALWAYS compiles (and is always tested), but it is only wired into
//! the live apply loop under the `ui-ir` feature. In the default web build it is
//! unreferenced from non-test code, so it is `allow(dead_code)` there and LTO
//! drops it entirely from the shipped wasm.
#![cfg_attr(not(any(feature = "ui-ir", test)), allow(dead_code))]

use crate::diff::{lis_stable_set, Op, Plan};
use std::collections::{HashMap, HashSet};

/// A portable UI node — the Rust image of `Ir.mo`'s `UINode`.
///
/// * `Element` — a tag with ordered attrs/events, an optional keyed-region key,
///   and child nodes.
/// * `Text` — dynamic text (already JSON-unescaped; HTML-escaped on render).
/// * `Raw` — literal HTML the IR could not yet model natively (emitted verbatim).
// `Clone` is needed by the live apply path (`Bridge::last_ui` is cloned to diff
// against). `Debug`/`PartialEq` are only used by the test asserts, so they are
// gated to `cfg(test)` — keeping the shipped web wasm free of the `core::fmt`
// Debug machinery they would otherwise pull in.
#[derive(Clone)]
#[cfg_attr(test, derive(Debug, PartialEq))]
pub enum UINode {
    Element {
        tag: String,
        attrs: Vec<(String, String)>,
        events: Vec<(String, String)>,
        key: Option<String>,
        children: Vec<UINode>,
    },
    Text(String),
    Raw(String),
}

impl UINode {
    /// The keyed-region key, if this node is a keyed element.
    pub fn key(&self) -> Option<&str> {
        match self {
            UINode::Element { key: Some(k), .. } => Some(k.as_str()),
            _ => None,
        }
    }
}

// ---- parsing --------------------------------------------------------------

/// A parse error with a 0-based byte offset into the input — enough to surface
/// "the IR was malformed" without pulling in a formatting dependency.
#[cfg_attr(test, derive(Debug, PartialEq))]
pub struct ParseError {
    pub at: usize,
    pub msg: &'static str,
}

/// Maximum nesting depth the parser will descend before bailing with an error.
///
/// The parser is recursive-descent (a frame per nested `children`/object/array),
/// and the `ui` field it consumes is attacker-controlled (whatever the canister
/// put in `Batch.ui`). Without a cap, a deeply nested forest would overflow the
/// wasm linear-memory stack and **abort** the whole brain (SIGABRT — not a panic
/// `catch_unwind` could trap), defeating the "malformed IR degrades safely"
/// contract. We cap depth well below the empirical overflow threshold (thousands
/// of `node()` frames on a 1 MB stack) so over-deep input returns `Err` and the
/// caller leaves the DOM untouched instead of crashing. No legitimate UI tree
/// approaches this depth; the iterative HTML path (`diff.rs`) has no such guard
/// because it never recurses.
const MAX_DEPTH: usize = 256;

/// Parse a page's IR — a FOREST, i.e. a JSON array of nodes — into `Vec<UINode>`.
///
/// This is what `Ir.mo.toJsonForest` emits and what the runtime stores in
/// `Batch.ui`.
pub fn parse_forest(json: &str) -> Result<Vec<UINode>, ParseError> {
    let mut p = P { b: json.as_bytes(), i: 0, depth: 0 };
    p.ws();
    let nodes = p.array_of_nodes()?;
    p.ws();
    if p.i != p.b.len() {
        return Err(p.err("trailing data after forest"));
    }
    Ok(nodes)
}

/// Parse a single node (one object of the LOCKED schema) into a `UINode`.
/// Public API (the per-node counterpart to [`parse_forest`]); the live apply
/// loop only needs `parse_forest`, so this is `allow(dead_code)` when wired.
#[allow(dead_code)]
pub fn parse_node(json: &str) -> Result<UINode, ParseError> {
    let mut p = P { b: json.as_bytes(), i: 0, depth: 0 };
    p.ws();
    let node = p.node()?;
    p.ws();
    if p.i != p.b.len() {
        return Err(p.err("trailing data after node"));
    }
    Ok(node)
}

struct P<'a> {
    b: &'a [u8],
    i: usize,
    /// Current recursion depth, incremented on entry to any recursive production
    /// (`node`, `array_of_nodes`, `skip_value`) and decremented on exit. Guards
    /// against stack-overflow aborts on hostile, deeply nested input.
    depth: usize,
}

impl<'a> P<'a> {
    fn err(&self, msg: &'static str) -> ParseError {
        ParseError { at: self.i, msg }
    }
    /// Charge one level of recursion, failing (rather than aborting) past the cap.
    fn enter(&mut self) -> Result<(), ParseError> {
        self.depth += 1;
        if self.depth > MAX_DEPTH {
            return Err(self.err("nesting too deep"));
        }
        Ok(())
    }
    fn peek(&self) -> u8 {
        if self.i < self.b.len() { self.b[self.i] } else { 0 }
    }
    fn ws(&mut self) {
        while self.i < self.b.len() {
            match self.b[self.i] {
                b' ' | b'\t' | b'\n' | b'\r' => self.i += 1,
                _ => break,
            }
        }
    }
    fn expect(&mut self, c: u8, msg: &'static str) -> Result<(), ParseError> {
        if self.peek() == c {
            self.i += 1;
            Ok(())
        } else {
            Err(self.err(msg))
        }
    }

    /// `[ node (, node)* ]` (also accepts the empty array). Depth-guarded: each
    /// nested array (via `children`) charges a level, then releases it on return.
    fn array_of_nodes(&mut self) -> Result<Vec<UINode>, ParseError> {
        self.enter()?;
        let r = self.array_of_nodes_inner();
        self.depth -= 1;
        r
    }

    fn array_of_nodes_inner(&mut self) -> Result<Vec<UINode>, ParseError> {
        self.expect(b'[', "expected '[' to start forest")?;
        let mut out = Vec::new();
        self.ws();
        if self.peek() == b']' {
            self.i += 1;
            return Ok(out);
        }
        loop {
            self.ws();
            out.push(self.node()?);
            self.ws();
            match self.peek() {
                b',' => self.i += 1,
                b']' => {
                    self.i += 1;
                    return Ok(out);
                }
                _ => return Err(self.err("expected ',' or ']' in forest")),
            }
        }
    }

    /// Parse one node object, dispatching on the `"t"` discriminant. Field order
    /// in the wire format is fixed (`t` first) but we don't rely on it: we read
    /// every key and slot it in, so the parser is robust to reordering.
    /// Depth-guarded: a node nests via its `children` array, so each `node`
    /// charges a level (and releases it on return) to cap recursion.
    fn node(&mut self) -> Result<UINode, ParseError> {
        self.enter()?;
        let r = self.node_inner();
        self.depth -= 1;
        r
    }

    fn node_inner(&mut self) -> Result<UINode, ParseError> {
        self.expect(b'{', "expected '{' to start node")?;

        let mut t: Option<String> = None;
        let mut tag: Option<String> = None;
        let mut value: Option<String> = None; // text
        let mut raw_html: Option<String> = None; // raw
        let mut attrs: Vec<(String, String)> = Vec::new();
        let mut events: Vec<(String, String)> = Vec::new();
        let mut key: Option<String> = None;
        let mut children: Vec<UINode> = Vec::new();

        self.ws();
        if self.peek() != b'}' {
            loop {
                self.ws();
                let field = self.string()?;
                self.ws();
                self.expect(b':', "expected ':' after field name")?;
                self.ws();
                match field.as_str() {
                    "t" => t = Some(self.string()?),
                    "tag" => tag = Some(self.string()?),
                    "value" => value = Some(self.string()?),
                    "html" => raw_html = Some(self.string()?),
                    "key" => key = Some(self.string()?),
                    "attrs" => attrs = self.pairs()?,
                    "events" => events = self.pairs()?,
                    "children" => children = self.array_of_nodes()?,
                    _ => self.skip_value()?, // forward-compat: ignore unknown fields
                }
                self.ws();
                match self.peek() {
                    b',' => self.i += 1,
                    b'}' => {
                        self.i += 1;
                        break;
                    }
                    _ => return Err(self.err("expected ',' or '}' in node")),
                }
            }
        } else {
            self.i += 1; // empty object
        }

        match t.as_deref() {
            Some("el") => Ok(UINode::Element {
                tag: tag.ok_or_else(|| self.err("element missing 'tag'"))?,
                attrs,
                events,
                key,
                children,
            }),
            Some("text") => Ok(UINode::Text(
                value.ok_or_else(|| self.err("text missing 'value'"))?,
            )),
            Some("raw") => Ok(UINode::Raw(
                raw_html.ok_or_else(|| self.err("raw missing 'html'"))?,
            )),
            _ => Err(self.err("node missing/unknown 't' discriminant")),
        }
    }

    /// `{ "k":"v" (, "k":"v")* }` — an order-preserving object of string pairs,
    /// exactly how `attrs`/`events` ride the wire. Empty object yields `[]`.
    fn pairs(&mut self) -> Result<Vec<(String, String)>, ParseError> {
        self.expect(b'{', "expected '{' for attrs/events object")?;
        let mut out = Vec::new();
        self.ws();
        if self.peek() == b'}' {
            self.i += 1;
            return Ok(out);
        }
        loop {
            self.ws();
            let k = self.string()?;
            self.ws();
            self.expect(b':', "expected ':' in attrs/events")?;
            self.ws();
            let v = self.string()?;
            out.push((k, v));
            self.ws();
            match self.peek() {
                b',' => self.i += 1,
                b'}' => {
                    self.i += 1;
                    return Ok(out);
                }
                _ => return Err(self.err("expected ',' or '}' in attrs/events")),
            }
        }
    }

    /// Decode a JSON string. Mirrors `json.rs::string`: collects raw bytes so
    /// multi-byte UTF-8 survives, and decodes the same escape set `Ir.mo.escape`
    /// produces (`\\ \" \n \r \t`) plus the rest of the JSON escapes for safety.
    fn string(&mut self) -> Result<String, ParseError> {
        if self.peek() != b'"' {
            return Err(self.err("expected '\"' to start string"));
        }
        self.i += 1;
        let mut out: Vec<u8> = Vec::new();
        while self.i < self.b.len() {
            let c = self.b[self.i];
            self.i += 1;
            match c {
                b'"' => return Ok(String::from_utf8_lossy(&out).into_owned()),
                b'\\' => {
                    let e = self.b.get(self.i).copied().unwrap_or(0);
                    self.i += 1;
                    match e {
                        b'"' => out.push(b'"'),
                        b'\\' => out.push(b'\\'),
                        b'/' => out.push(b'/'),
                        b'n' => out.push(b'\n'),
                        b'r' => out.push(b'\r'),
                        b't' => out.push(b'\t'),
                        b'b' => out.push(0x08),
                        b'f' => out.push(0x0C),
                        b'u' => {
                            let end = (self.i + 4).min(self.b.len());
                            let hex = match std::str::from_utf8(&self.b[self.i..end]) {
                                Ok(h) => h,
                                Err(_) => return Err(self.err("bad \\u escape")),
                            };
                            let code = match u32::from_str_radix(hex, 16) {
                                Ok(c) => c,
                                Err(_) => return Err(self.err("bad \\u hex")),
                            };
                            self.i = end;
                            // Ir.mo never emits `\u` (astral chars ride as raw
                            // UTF-8), so this only matters for hostile/non-
                            // conformant producers. Substitute U+FFFD for an
                            // invalid scalar (e.g. a lone surrogate \ud800) so
                            // decode is deterministic instead of silently dropping
                            // the character.
                            let ch = char::from_u32(code).unwrap_or('\u{FFFD}');
                            let mut buf = [0u8; 4];
                            out.extend_from_slice(ch.encode_utf8(&mut buf).as_bytes());
                        }
                        _ => {}
                    }
                }
                _ => out.push(c),
            }
        }
        Err(self.err("unterminated string"))
    }

    /// Skip any JSON value without materializing it — used only to ignore
    /// unknown forward-compatible fields. Depth-guarded: nested unknown
    /// objects/arrays recurse here, so each charges a level (released on return).
    fn skip_value(&mut self) -> Result<(), ParseError> {
        self.enter()?;
        let r = self.skip_value_inner();
        self.depth -= 1;
        r
    }

    fn skip_value_inner(&mut self) -> Result<(), ParseError> {
        self.ws();
        match self.peek() {
            b'"' => {
                self.string()?;
                Ok(())
            }
            b'{' => {
                self.i += 1;
                self.ws();
                if self.peek() == b'}' {
                    self.i += 1;
                    return Ok(());
                }
                loop {
                    self.ws();
                    self.string()?;
                    self.ws();
                    self.expect(b':', "expected ':' while skipping object")?;
                    self.skip_value()?;
                    self.ws();
                    match self.peek() {
                        b',' => self.i += 1,
                        b'}' => {
                            self.i += 1;
                            return Ok(());
                        }
                        _ => return Err(self.err("bad object while skipping")),
                    }
                }
            }
            b'[' => {
                self.i += 1;
                self.ws();
                if self.peek() == b']' {
                    self.i += 1;
                    return Ok(());
                }
                loop {
                    self.skip_value()?;
                    self.ws();
                    match self.peek() {
                        b',' => self.i += 1,
                        b']' => {
                            self.i += 1;
                            return Ok(());
                        }
                        _ => return Err(self.err("bad array while skipping")),
                    }
                }
            }
            0 => Err(self.err("unexpected end while skipping value")),
            _ => {
                // number / bool / null: consume until a structural delimiter.
                while self.i < self.b.len() {
                    match self.b[self.i] {
                        b',' | b'}' | b']' | b' ' | b'\t' | b'\n' | b'\r' => break,
                        _ => self.i += 1,
                    }
                }
                Ok(())
            }
        }
    }
}

// ---- rendering (UINode -> HTML) -------------------------------------------

const VOID: &[&str] = &[
    "area", "base", "br", "col", "embed", "hr", "img", "input", "link", "meta", "param", "source",
    "track", "wbr",
];

/// HTML-escape attribute values / text content. Matches the HTML backend's
/// escaping so an IR-rendered node is byte-identical to the HTML one for the
/// keys the hands key on (and safe against attribute/markup injection).
fn esc(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            _ => out.push(ch),
        }
    }
    out
}

/// Render a `UINode` to HTML. Keyed elements carry `data-mv-key="..."` so the
/// rendered span is reconcilable by the SAME keyed-region machinery the HTML
/// path uses; events become `data-mv-on-<event>="handler"` attributes, matching
/// the HTML backend's event encoding.
pub fn render(node: &UINode) -> String {
    let mut s = String::new();
    render_into(node, &mut s);
    s
}

/// Render a whole forest to HTML (top-level nodes concatenated).
pub fn render_forest(nodes: &[UINode]) -> String {
    let mut s = String::new();
    for n in nodes {
        render_into(n, &mut s);
    }
    s
}

fn render_into(node: &UINode, out: &mut String) {
    match node {
        UINode::Text(t) => out.push_str(&esc(t)),
        UINode::Raw(h) => out.push_str(h),
        UINode::Element { tag, attrs, events, key, children } => {
            out.push('<');
            out.push_str(tag);
            if let Some(k) = key {
                out.push_str(" data-mv-key=\"");
                out.push_str(&esc(k));
                out.push('"');
            }
            for (name, value) in attrs {
                out.push(' ');
                out.push_str(name);
                out.push_str("=\"");
                out.push_str(&esc(value));
                out.push('"');
            }
            for (name, handler) in events {
                out.push_str(" data-mv-on-");
                out.push_str(name);
                out.push_str("=\"");
                out.push_str(&esc(handler));
                out.push('"');
            }
            out.push('>');
            if VOID.contains(&tag.to_ascii_lowercase().as_str()) && children.is_empty() {
                return;
            }
            for c in children {
                render_into(c, out);
            }
            out.push_str("</");
            out.push_str(tag);
            out.push('>');
        }
    }
}

// ---- keyed structural diff over UINode ------------------------------------

/// Compute how to update a target whose previous IR forest was `old` to `new`,
/// producing the SAME [`Op`] kinds the HTML path emits (so the unchanged
/// `host_*` apply loop drives both).
///
/// The structural strategy mirrors `diff::plan` exactly, lifted to nodes:
///   * Fast path — same keys in the same order AND identical non-keyed
///     surroundings: replace only the keyed subtrees whose render changed.
///   * Structural path — the keyed nodes form one contiguous run inside stable
///     surroundings: reconcile via [`lis_stable_set`], moving the minimum number
///     of nodes (a stable/focused node is never touched).
///   * Otherwise — [`Plan::Full`]; the caller re-renders the whole target.
pub fn ir_diff(old: &[UINode], new: &[UINode]) -> Plan {
    // Split each forest into (keyed regions in order, the non-keyed segments
    // around them). `segments.len() == regions.len() + 1`, identical in spirit
    // to `diff::parse` but over nodes instead of HTML spans.
    let old = match split(old) {
        Some(x) => x,
        None => return Plan::Full,
    };
    let new = match split(new) {
        Some(x) => x,
        None => return Plan::Full,
    };

    let same_keys = old.regions.len() == new.regions.len()
        && old
            .regions
            .iter()
            .zip(new.regions.iter())
            .all(|(o, n)| o.key == n.key);

    // Fast path: identical surroundings AND same keys/order -> only changed
    // regions need replacing.
    if same_keys && old.segments == new.segments {
        let mut ops = Vec::new();
        for (o, n) in old.regions.iter().zip(new.regions.iter()) {
            if o.html != n.html {
                ops.push(Op::Replace { key: n.key.clone(), html: n.html.clone() });
            }
        }
        return Plan::Patch(ops);
    }

    // Structural path: stable prefix/suffix and whitespace-only text between the
    // keyed siblings (rendered segments compared, same as the HTML path).
    let prefix_ok = old.segments.first() == new.segments.first();
    let suffix_ok = old.segments.last() == new.segments.last();
    let middles_ws = old.segments[1..old.segments.len() - 1]
        .iter()
        .all(|s| s.trim().is_empty())
        && new.segments[1..new.segments.len() - 1]
            .iter()
            .all(|s| s.trim().is_empty());
    if !prefix_ok || !suffix_ok || !middles_ws {
        return Plan::Full;
    }
    // Need at least one surviving keyed node to anchor inserts/moves against.
    let new_set: HashSet<&str> = new.regions.iter().map(|r| r.key.as_str()).collect();
    if !old.regions.iter().any(|r| new_set.contains(r.key.as_str())) {
        return Plan::Full;
    }
    Plan::Patch(reconcile(&old.regions, &new.regions))
}

/// A keyed region of a forest: its key plus the rendered HTML of its subtree.
struct NodeRegion {
    key: String,
    html: String,
}

struct Split {
    regions: Vec<NodeRegion>,
    segments: Vec<String>,
}

/// Walk the TOP-LEVEL forest, pulling out keyed elements as regions and the
/// rendered non-keyed nodes between them as text segments. Returns `None` when
/// there are no keyed regions — nothing to optimize, caller does a full swap.
///
/// Keyed regions only matter at the level they appear; nested keyed children are
/// reconciled as part of their parent's render, exactly as in the HTML path
/// (which scans only the outermost keyed spans of a target).
fn split(forest: &[UINode]) -> Option<Split> {
    let mut regions: Vec<NodeRegion> = Vec::new();
    let mut segments: Vec<String> = Vec::new();
    let mut cur = String::new();
    for node in forest {
        if let Some(k) = node.key() {
            segments.push(std::mem::take(&mut cur));
            regions.push(NodeRegion { key: k.to_string(), html: render(node) });
        } else {
            render_into(node, &mut cur);
        }
    }
    if regions.is_empty() {
        return None;
    }
    segments.push(cur);
    Some(Split { regions, segments })
}

/// Reconcile two keyed sequences into ops, moving the minimum number of nodes.
/// Byte-for-byte the same algorithm as `diff::reconcile`, over `NodeRegion`
/// instead of `Region` — including reusing the shared [`lis_stable_set`].
fn reconcile(old: &[NodeRegion], new: &[NodeRegion]) -> Vec<Op> {
    let old_index: HashMap<&str, usize> =
        old.iter().enumerate().map(|(i, r)| (r.key.as_str(), i)).collect();
    let old_html: HashMap<&str, &str> =
        old.iter().map(|r| (r.key.as_str(), r.html.as_str())).collect();
    let new_set: HashSet<&str> = new.iter().map(|r| r.key.as_str()).collect();

    let mut ops: Vec<Op> = Vec::new();
    // Removals first, so anchoring only references surviving nodes.
    for r in old {
        if !new_set.contains(r.key.as_str()) {
            ops.push(Op::Remove { key: r.key.clone() });
        }
    }
    let kept_old_idx: Vec<i64> = new
        .iter()
        .map(|r| old_index.get(r.key.as_str()).map(|&i| i as i64).unwrap_or(-1))
        .collect();
    let stay = lis_stable_set(&kept_old_idx);

    let mut prev: Option<String> = None;
    for (j, r) in new.iter().enumerate() {
        let existing = old_index.contains_key(r.key.as_str());
        if !existing {
            ops.push(Op::Insert { html: r.html.clone(), after: prev.clone() });
        } else {
            if !stay.contains(&j) {
                ops.push(Op::Move { key: r.key.clone(), after: prev.clone() });
            }
            if old_html.get(r.key.as_str()) != Some(&r.html.as_str()) {
                ops.push(Op::Replace { key: r.key.clone(), html: r.html.clone() });
            }
        }
        prev = Some(r.key.clone());
    }
    ops
}

#[cfg(test)]
mod tests {
    use super::*;

    // ------------------------------------------------------------------
    // 1. Parsing — fixtures are the EXACT strings runtime/test/IrTest.mo
    //    asserts `Ir.mo.toJson`/`toJsonForest` emit, so the parser is verified
    //    against the real Slice 6 wire format, not a paraphrase of it.
    // ------------------------------------------------------------------

    #[test]
    fn parse_text_leaf_with_escapes() {
        // Ir.toJson(#text("a \"q\" & <b>\n\tend"))
        let json = "{\"t\":\"text\",\"value\":\"a \\\"q\\\" & <b>\\n\\tend\"}";
        assert_eq!(
            parse_node(json).unwrap(),
            UINode::Text("a \"q\" & <b>\n\tend".to_string())
        );
    }

    #[test]
    fn parse_raw_leaf() {
        // Ir.toJson(#raw("<button class=\"x\">Go</button>"))
        let json = "{\"t\":\"raw\",\"html\":\"<button class=\\\"x\\\">Go</button>\"}";
        assert_eq!(
            parse_node(json).unwrap(),
            UINode::Raw("<button class=\"x\">Go</button>".to_string())
        );
    }

    #[test]
    fn parse_element_tree_preserves_order_and_key() {
        // The "element-tree" golden from IrTest.mo.
        let json = "{\"t\":\"el\",\"tag\":\"li\",\"attrs\":{\"class\":\"row\"},\"events\":{},\"key\":\"row-7\",\"children\":[\
            {\"t\":\"el\",\"tag\":\"button\",\"attrs\":{\"data-mv-arg0\":\"7\"},\"events\":{\"click\":\"pick\"},\"children\":[\
            {\"t\":\"text\",\"value\":\"Pick\"}]}]}";
        let want = UINode::Element {
            tag: "li".into(),
            attrs: vec![("class".into(), "row".into())],
            events: vec![],
            key: Some("row-7".into()),
            children: vec![UINode::Element {
                tag: "button".into(),
                attrs: vec![("data-mv-arg0".into(), "7".into())],
                events: vec![("click".into(), "pick".into())],
                key: None,
                children: vec![UINode::Text("Pick".into())],
            }],
        };
        assert_eq!(parse_node(json).unwrap(), want);
    }

    #[test]
    fn parse_key_omitted_when_null() {
        // Ir.mo omits the "key" field entirely when null.
        let json = "{\"t\":\"el\",\"tag\":\"p\",\"attrs\":{},\"events\":{},\"children\":[]}";
        assert_eq!(
            parse_node(json).unwrap(),
            UINode::Element {
                tag: "p".into(),
                attrs: vec![],
                events: vec![],
                key: None,
                children: vec![],
            }
        );
    }

    #[test]
    fn parse_full_builder_forest_golden() {
        // The "builder-forest-json" golden from IrTest.mo — a full forest.
        let json = "[{\"t\":\"el\",\"tag\":\"section\",\"attrs\":{},\"events\":{},\"children\":[\
            {\"t\":\"el\",\"tag\":\"h1\",\"attrs\":{},\"events\":{},\"children\":[\
            {\"t\":\"raw\",\"html\":\"Hi \"},\
            {\"t\":\"text\",\"value\":\"Ada & Co\"}]},\
            {\"t\":\"el\",\"tag\":\"li\",\"attrs\":{},\"events\":{},\"key\":\"it-1\",\"children\":[\
            {\"t\":\"el\",\"tag\":\"button\",\"attrs\":{\"data-mv-arg0\":\"it-1\"},\"events\":{\"click\":\"pick\"},\"children\":[\
            {\"t\":\"text\",\"value\":\"it-1\"}]}]}]}]";
        let forest = parse_forest(json).unwrap();
        assert_eq!(forest.len(), 1);
        let want = vec![UINode::Element {
            tag: "section".into(),
            attrs: vec![],
            events: vec![],
            key: None,
            children: vec![
                UINode::Element {
                    tag: "h1".into(),
                    attrs: vec![],
                    events: vec![],
                    key: None,
                    children: vec![
                        UINode::Raw("Hi ".into()),
                        UINode::Text("Ada & Co".into()),
                    ],
                },
                UINode::Element {
                    tag: "li".into(),
                    attrs: vec![],
                    events: vec![],
                    key: Some("it-1".into()),
                    children: vec![UINode::Element {
                        tag: "button".into(),
                        attrs: vec![("data-mv-arg0".into(), "it-1".into())],
                        events: vec![("click".into(), "pick".into())],
                        key: None,
                        children: vec![UINode::Text("it-1".into())],
                    }],
                },
            ],
        }];
        assert_eq!(forest, want);
    }

    #[test]
    fn parse_empty_forest() {
        assert_eq!(parse_forest("[]").unwrap(), vec![]);
        assert_eq!(parse_forest("  [ ]  ").unwrap(), vec![]);
    }

    #[test]
    fn parse_rejects_malformed() {
        assert!(parse_forest("[").is_err());
        assert!(parse_node("{\"t\":\"el\"}").is_err()); // missing tag
        assert!(parse_node("{\"t\":\"bogus\"}").is_err());
        assert!(parse_forest("[{\"t\":\"text\",\"value\":\"x\"}] trailing").is_err());
    }

    #[test]
    fn parse_ignores_unknown_fields_forward_compat() {
        let json = "{\"t\":\"text\",\"future\":{\"a\":[1,2]},\"value\":\"hi\",\"n\":3.5}";
        assert_eq!(parse_node(json).unwrap(), UINode::Text("hi".into()));
    }

    #[test]
    fn parse_deeply_nested_forest_errs_not_aborts() {
        // A forest nested ~10k levels via repeated `children` arrays. Without the
        // depth guard this recurses one stack frame per level and aborts the
        // whole process (SIGABRT) — which catch_unwind CANNOT trap. With the
        // guard it must return Err and leave the caller free to no-op. The `ui`
        // field is attacker-controlled, so this is the real DoS regression.
        const N: usize = 10_000;
        let mut s = String::new();
        for _ in 0..N {
            s.push_str("[{\"t\":\"el\",\"tag\":\"div\",\"children\":");
        }
        s.push_str("[]");
        for _ in 0..N {
            s.push_str("}]");
        }
        let r = parse_forest(&s);
        assert!(r.is_err(), "deep forest must be rejected, not parsed/aborted");
        assert_eq!(r.unwrap_err().msg, "nesting too deep");
    }

    #[test]
    fn parse_deeply_nested_unknown_field_errs_not_aborts() {
        // skip_value() recurses on nested unknown objects/arrays; it must be
        // guarded too. Build a single node carrying a ~10k-deep unknown array.
        const N: usize = 10_000;
        let mut s = String::from("{\"t\":\"text\",\"value\":\"x\",\"future\":");
        for _ in 0..N {
            s.push('[');
        }
        for _ in 0..N {
            s.push(']');
        }
        s.push('}');
        let r = parse_node(&s);
        assert!(r.is_err(), "deep unknown nesting must be rejected, not aborted");
        assert_eq!(r.unwrap_err().msg, "nesting too deep");
    }

    #[test]
    fn parse_shallow_nesting_within_cap_ok() {
        // A legitimately nested-but-shallow tree must still parse fine — the cap
        // is far above any real UI depth.
        let mut s = String::new();
        let depth = 20;
        for _ in 0..depth {
            s.push_str("{\"t\":\"el\",\"tag\":\"div\",\"children\":[");
        }
        s.push_str("{\"t\":\"text\",\"value\":\"leaf\"}");
        for _ in 0..depth {
            s.push_str("]}");
        }
        assert!(parse_node(&s).is_ok(), "shallow nesting must parse");
    }

    #[test]
    fn parse_bad_codepoint_becomes_replacement_char() {
        // A lone surrogate \ud800 is not a valid scalar; we substitute U+FFFD so
        // decode is deterministic rather than silently dropping the character.
        // (Ir.mo never emits \u, so this is purely a hostile-producer guard.)
        let json = "{\"t\":\"text\",\"value\":\"a\\ud800b\"}";
        assert_eq!(parse_node(json).unwrap(), UINode::Text("a\u{FFFD}b".into()));
    }

    // ------------------------------------------------------------------
    // 2. Rendering — keyed elements carry data-mv-key, events become
    //    data-mv-on-*, text/attrs are HTML-escaped.
    // ------------------------------------------------------------------

    #[test]
    fn render_keyed_element_with_event() {
        let n = UINode::Element {
            tag: "li".into(),
            attrs: vec![("class".into(), "row".into())],
            events: vec![("click".into(), "pick".into())],
            key: Some("r1".into()),
            children: vec![UINode::Text("A & B".into())],
        };
        assert_eq!(
            render(&n),
            "<li data-mv-key=\"r1\" class=\"row\" data-mv-on-click=\"pick\">A &amp; B</li>"
        );
    }

    #[test]
    fn render_void_and_raw() {
        let img = UINode::Element {
            tag: "img".into(),
            attrs: vec![("src".into(), "/a.png".into())],
            events: vec![],
            key: None,
            children: vec![],
        };
        assert_eq!(render(&img), "<img src=\"/a.png\">");
        assert_eq!(render(&UINode::Raw("<b>x</b>".into())), "<b>x</b>");
    }

    // ------------------------------------------------------------------
    // 3. Keyed structural diff over UINode forests. These MIRROR the HTML
    //    golden spec in diff.rs (same scenarios, same op kinds), proving the
    //    generalized path reconciles identically.
    // ------------------------------------------------------------------

    /// A keyed <li> whose text body is `body` — the node analogue of diff.rs's
    /// `li(k, body)` HTML helper, so the rendered region HTML matches exactly.
    fn li(k: &str, body: &str) -> UINode {
        UINode::Element {
            tag: "li".into(),
            attrs: vec![],
            events: vec![],
            key: Some(k.into()),
            children: vec![UINode::Text(body.into())],
        }
    }
    /// The forest of a keyed list, wrapped in a <ul> like diff.rs's `list(..)`.
    /// The <ul> is the (unkeyed) stable chrome; its keyed <li> children are the
    /// regions — so `split` sees the <li>s by recursing? No: split scans the
    /// TOP level, so we expose the <li>s as the top-level forest and treat the
    /// <ul> separately. To match diff.rs semantics we keep the keyed run at the
    /// top level here.
    fn list(items: &[(&str, &str)]) -> Vec<UINode> {
        items.iter().map(|(k, b)| li(k, b)).collect()
    }
    /// Rendered HTML of a single keyed region — what an Op payload carries.
    fn li_html(k: &str, body: &str) -> String {
        render(&li(k, body))
    }

    #[test]
    fn diff_no_keys_is_full() {
        let old = vec![UINode::Element {
            tag: "p".into(),
            attrs: vec![],
            events: vec![],
            key: None,
            children: vec![UINode::Text("a".into())],
        }];
        let new = vec![UINode::Element {
            tag: "p".into(),
            attrs: vec![],
            events: vec![],
            key: None,
            children: vec![UINode::Text("b".into())],
        }];
        assert_eq!(ir_diff(&old, &new), Plan::Full);
    }

    #[test]
    fn diff_replace_changed_only() {
        let old = list(&[("a", "A"), ("b", "B"), ("c", "C")]);
        let new = list(&[("a", "A"), ("b", "B2"), ("c", "C")]);
        assert_eq!(
            ir_diff(&old, &new),
            Plan::Patch(vec![Op::Replace { key: "b".into(), html: li_html("b", "B2") }])
        );
    }

    #[test]
    fn diff_unchanged_is_empty_patch() {
        let h = list(&[("a", "A")]);
        assert_eq!(ir_diff(&h, &h), Plan::Patch(vec![]));
    }

    #[test]
    fn diff_append_inserts_after_last() {
        let old = list(&[("a", "A"), ("b", "B")]);
        let new = list(&[("a", "A"), ("b", "B"), ("c", "C")]);
        assert_eq!(
            ir_diff(&old, &new),
            Plan::Patch(vec![Op::Insert { html: li_html("c", "C"), after: Some("b".into()) }])
        );
    }

    #[test]
    fn diff_prepend_inserts_at_start() {
        let old = list(&[("b", "B")]);
        let new = list(&[("a", "A"), ("b", "B")]);
        assert_eq!(
            ir_diff(&old, &new),
            Plan::Patch(vec![Op::Insert { html: li_html("a", "A"), after: None }])
        );
    }

    #[test]
    fn diff_remove_middle() {
        let old = list(&[("a", "A"), ("b", "B"), ("c", "C")]);
        let new = list(&[("a", "A"), ("c", "C")]);
        assert_eq!(ir_diff(&old, &new), Plan::Patch(vec![Op::Remove { key: "b".into() }]));
    }

    #[test]
    fn diff_reorder_moves_minimum() {
        // a,b,c -> c,a,b : LIS is (a,b); only c moves to the front.
        let old = list(&[("a", "A"), ("b", "B"), ("c", "C")]);
        let new = list(&[("c", "C"), ("a", "A"), ("b", "B")]);
        assert_eq!(
            ir_diff(&old, &new),
            Plan::Patch(vec![Op::Move { key: "c".into(), after: None }])
        );
    }

    #[test]
    fn diff_insert_and_change_mixed() {
        let old = list(&[("a", "A"), ("b", "B")]);
        let new = list(&[("a", "A2"), ("x", "X"), ("b", "B")]);
        assert_eq!(
            ir_diff(&old, &new),
            Plan::Patch(vec![
                Op::Replace { key: "a".into(), html: li_html("a", "A2") },
                Op::Insert { html: li_html("x", "X"), after: Some("a".into()) },
            ])
        );
    }

    #[test]
    fn diff_chrome_change_falls_back_to_full() {
        // A leading unkeyed <h1> whose content changes -> stable prefix differs.
        let h1 = |t: &str| UINode::Element {
            tag: "h1".into(),
            attrs: vec![],
            events: vec![],
            key: None,
            children: vec![UINode::Text(t.into())],
        };
        let mut old = vec![h1("0")];
        old.extend(list(&[("a", "A")]));
        let mut new = vec![h1("1")];
        new.extend(list(&[("a", "A2")]));
        assert_eq!(ir_diff(&old, &new), Plan::Full);
    }

    #[test]
    fn diff_whole_list_replaced_falls_back() {
        let old = list(&[("a", "A"), ("b", "B")]);
        let new = list(&[("x", "X"), ("y", "Y")]);
        assert_eq!(ir_diff(&old, &new), Plan::Full);
    }
}
