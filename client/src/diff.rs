//! Keyed-region diffing — runs entirely in the brain (Rust→WASM).
//!
//! The server re-renders a target's full HTML on every change. Replacing it
//! wholesale destroys the live state of every node. When regions carry
//! `key="..."` (compiled to `data-mv-key`), we can do better: diff the keyed
//! regions and emit primitive ops — replace a changed region, and (when the keys
//! form one contiguous run inside stable chrome) insert / remove / move regions
//! to reconcile the list. Untouched nodes keep their focus, selection, scroll
//! and media state. A reorder moves the minimum number of nodes (LIS), so a
//! node that doesn't need to move — including a focused one — is never touched.
//!
//! All decisions are made here; the glue ("hands") only executes the ops.

use std::collections::{HashMap, HashSet};

/// A keyed region: its key and the exact HTML span of its element.
pub struct Region {
    pub key: String,
    pub html: String,
}

/// A primitive DOM operation the hands execute. `after` is the key to position
/// after, or None for the start of the parent.
#[derive(Debug, PartialEq)]
pub enum Op {
    Replace { key: String, html: String },
    Remove { key: String },
    Insert { html: String, after: Option<String> },
    Move { key: String, after: Option<String> },
}

/// What to do with a freshly rendered target.
#[derive(Debug, PartialEq)]
pub enum Plan {
    /// Replace the whole target (no stable keyed structure to exploit).
    Full,
    /// Apply these ops in order.
    Patch(Vec<Op>),
}

const VOID: &[&str] = &[
    "area", "base", "br", "col", "embed", "hr", "img", "input", "link", "meta", "param", "source",
    "track", "wbr",
];

fn is_name_start(c: u8) -> bool {
    c.is_ascii_alphabetic()
}
fn is_name_char(c: u8) -> bool {
    c.is_ascii_alphanumeric() || c == b'-' || c == b':'
}

/// Parse the HTML into keyed regions plus the non-keyed text segments around
/// them: `segments.len() == regions.len() + 1`, where `segments[i]` is the text
/// before `regions[i]` and `segments[last]` is the text after the last region.
/// Returns None when there are no keyed regions — nothing to optimize.
pub fn parse(html: &str) -> Option<(Vec<Region>, Vec<String>)> {
    let b = html.as_bytes();
    let n = b.len();
    let mut regions: Vec<Region> = Vec::new();
    let mut segments: Vec<String> = Vec::new();
    let mut i = 0usize;
    let mut last = 0usize;
    while i < n {
        if b[i] == b'<' && i + 1 < n && is_name_start(b[i + 1]) {
            if let Some((name, open_end, self_closing)) = read_open_tag(b, i) {
                let open = &html[i..open_end];
                if let Some(key) = find_key(open) {
                    let void = self_closing || VOID.contains(&name.to_ascii_lowercase().as_str());
                    let span_end = if void {
                        open_end
                    } else {
                        match find_close(b, open_end, &name) {
                            Some(e) => e,
                            None => {
                                i += 1;
                                continue;
                            }
                        }
                    };
                    segments.push(html[last..i].to_string());
                    regions.push(Region { key, html: html[i..span_end].to_string() });
                    i = span_end;
                    last = span_end;
                    continue;
                } else {
                    i = open_end;
                    continue;
                }
            }
        }
        i += 1;
    }
    if regions.is_empty() {
        return None;
    }
    segments.push(html[last..].to_string());
    Some((regions, segments))
}

/// Compute how to update a target whose previous HTML was `old` to `new`.
pub fn plan(old: &str, new: &str) -> Plan {
    let (old_regions, old_segs) = match parse(old) {
        Some(x) => x,
        None => return Plan::Full,
    };
    let (new_regions, new_segs) = match parse(new) {
        Some(x) => x,
        None => return Plan::Full,
    };

    let same_keys = old_regions.len() == new_regions.len()
        && old_regions
            .iter()
            .zip(new_regions.iter())
            .all(|(o, n)| o.key == n.key);

    // Fast path: identical non-keyed structure AND same keys in same order ->
    // only the changed regions need replacing.
    if same_keys && old_segs == new_segs {
        let mut ops = Vec::new();
        for (o, n) in old_regions.iter().zip(new_regions.iter()) {
            if o.html != n.html {
                ops.push(Op::Replace { key: n.key.clone(), html: n.html.clone() });
            }
        }
        return Plan::Patch(ops);
    }

    // Structural path: only when the keys form ONE contiguous run inside stable
    // chrome — i.e. identical prefix + suffix and whitespace-only text between
    // the keyed siblings (both before and after). Anything else is a full swap.
    let prefix_ok = old_segs.first() == new_segs.first();
    let suffix_ok = old_segs.last() == new_segs.last();
    let middles_ws = old_segs[1..old_segs.len() - 1].iter().all(|s| s.trim().is_empty())
        && new_segs[1..new_segs.len() - 1].iter().all(|s| s.trim().is_empty());
    if !prefix_ok || !suffix_ok || !middles_ws {
        return Plan::Full;
    }
    // Need at least one surviving keyed node to anchor inserts/moves against;
    // otherwise (whole list replaced) let the full swap handle it.
    let new_set: HashSet<&str> = new_regions.iter().map(|r| r.key.as_str()).collect();
    if !old_regions.iter().any(|r| new_set.contains(r.key.as_str())) {
        return Plan::Full;
    }
    Plan::Patch(reconcile(&old_regions, &new_regions))
}

/// Reconcile two keyed sequences into ops, moving the minimum number of nodes.
fn reconcile(old: &[Region], new: &[Region]) -> Vec<Op> {
    let old_index: HashMap<&str, usize> =
        old.iter().enumerate().map(|(i, r)| (r.key.as_str(), i)).collect();
    let old_html: HashMap<&str, &str> = old.iter().map(|r| (r.key.as_str(), r.html.as_str())).collect();
    let new_set: HashSet<&str> = new.iter().map(|r| r.key.as_str()).collect();

    let mut ops: Vec<Op> = Vec::new();
    // Removals first, so anchoring only references surviving nodes.
    for r in old {
        if !new_set.contains(r.key.as_str()) {
            ops.push(Op::Remove { key: r.key.clone() });
        }
    }
    // old-index of each new item (-1 for inserted items).
    let kept_old_idx: Vec<i64> = new
        .iter()
        .map(|r| old_index.get(r.key.as_str()).map(|&i| i as i64).unwrap_or(-1))
        .collect();
    // The longest run already in increasing old order stays put; everything
    // else moves. Keeps a stable (e.g. focused) node untouched.
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

/// Indices (into the new sequence) of a longest increasing subsequence over the
/// kept items' old positions — the items that need not move.
///
/// Shared with the UI-IR keyed diff (`ir::ir_diff`): the structural reconcile is
/// identical whether the keyed regions are HTML spans or `UINode` subtrees, so
/// both paths reuse this one LIS implementation rather than reinventing it.
pub(crate) fn lis_stable_set(seq: &[i64]) -> HashSet<usize> {
    let cand: Vec<usize> = (0..seq.len()).filter(|&j| seq[j] >= 0).collect();
    let m = cand.len();
    let mut set = HashSet::new();
    if m == 0 {
        return set;
    }
    let vals: Vec<i64> = cand.iter().map(|&j| seq[j]).collect();
    let mut len = vec![1usize; m];
    let mut prev = vec![usize::MAX; m];
    let mut best = 0usize;
    for i in 0..m {
        for k in 0..i {
            if vals[k] < vals[i] && len[k] + 1 > len[i] {
                len[i] = len[k] + 1;
                prev[i] = k;
            }
        }
        if len[i] > len[best] {
            best = i;
        }
    }
    let mut i = best;
    loop {
        set.insert(cand[i]);
        if prev[i] == usize::MAX {
            break;
        }
        i = prev[i];
    }
    set
}

// ---- low-level HTML scanning (safe for server-generated, escaped HTML) ----

fn read_open_tag(b: &[u8], start: usize) -> Option<(String, usize, bool)> {
    let n = b.len();
    let mut i = start + 1;
    let name_start = i;
    while i < n && is_name_char(b[i]) {
        i += 1;
    }
    if i == name_start {
        return None;
    }
    let name = String::from_utf8_lossy(&b[name_start..i]).to_string();
    let mut quote = 0u8;
    while i < n {
        let c = b[i];
        if quote != 0 {
            if c == quote {
                quote = 0;
            }
        } else if c == b'"' || c == b'\'' {
            quote = c;
        } else if c == b'>' {
            let self_closing = i > start && b[i - 1] == b'/';
            return Some((name, i + 1, self_closing));
        }
        i += 1;
    }
    None
}

fn find_key(open: &str) -> Option<String> {
    let p = open.find("data-mv-key")?;
    let rest = open[p + "data-mv-key".len()..].trim_start();
    let rest = rest.strip_prefix('=')?.trim_start();
    let q = rest.as_bytes().first().copied()?;
    if q == b'"' || q == b'\'' {
        let qc = q as char;
        let end = rest[1..].find(qc)?;
        Some(rest[1..1 + end].to_string())
    } else {
        let end = rest
            .find(|c: char| c.is_whitespace() || c == '>' || c == '/')
            .unwrap_or(rest.len());
        Some(rest[..end].to_string())
    }
}

fn tag_name_matches(b: &[u8], pos: usize, lname: &str) -> bool {
    let ln = lname.as_bytes();
    if pos + ln.len() > b.len() {
        return false;
    }
    for (k, &c) in ln.iter().enumerate() {
        if b[pos + k].to_ascii_lowercase() != c {
            return false;
        }
    }
    let after = pos + ln.len();
    after >= b.len() || !is_name_char(b[after])
}

fn find_close(b: &[u8], from: usize, name: &str) -> Option<usize> {
    let n = b.len();
    let lname = name.to_ascii_lowercase();
    let void = VOID.contains(&lname.as_str());
    let mut depth = 1i32;
    let mut i = from;
    while i < n {
        if b[i] == b'<' {
            if i + 1 < n && b[i + 1] == b'/' {
                if tag_name_matches(b, i + 2, &lname) {
                    depth -= 1;
                    let mut j = i + 2;
                    while j < n && b[j] != b'>' {
                        j += 1;
                    }
                    if depth == 0 {
                        return Some(j + 1);
                    }
                    i = j + 1;
                    continue;
                }
            } else if i + 1 < n && is_name_start(b[i + 1]) && tag_name_matches(b, i + 1, &lname) {
                if let Some((_, oe, sc)) = read_open_tag(b, i) {
                    if !sc && !void {
                        depth += 1;
                    }
                    i = oe;
                    continue;
                }
            }
        }
        i += 1;
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn li(k: &str, body: &str) -> String {
        format!("<li data-mv-key=\"{}\">{}</li>", k, body)
    }
    fn list(items: &[(&str, &str)]) -> String {
        let inner: String = items.iter().map(|(k, b)| li(k, b)).collect::<Vec<_>>().join("");
        format!("<ul>{}</ul>", inner)
    }

    #[test]
    fn no_keys_is_full() {
        assert_eq!(plan("<p>a</p>", "<p>b</p>"), Plan::Full);
    }

    #[test]
    fn replace_changed_only() {
        let old = list(&[("a", "A"), ("b", "B"), ("c", "C")]);
        let new = list(&[("a", "A"), ("b", "B2"), ("c", "C")]);
        assert_eq!(
            plan(&old, &new),
            Plan::Patch(vec![Op::Replace { key: "b".into(), html: li("b", "B2") }])
        );
    }

    #[test]
    fn unchanged_is_empty_patch() {
        let h = list(&[("a", "A")]);
        assert_eq!(plan(&h, &h), Plan::Patch(vec![]));
    }

    #[test]
    fn append_inserts_after_last() {
        let old = list(&[("a", "A"), ("b", "B")]);
        let new = list(&[("a", "A"), ("b", "B"), ("c", "C")]);
        assert_eq!(
            plan(&old, &new),
            Plan::Patch(vec![Op::Insert { html: li("c", "C"), after: Some("b".into()) }])
        );
    }

    #[test]
    fn prepend_inserts_at_start() {
        let old = list(&[("b", "B")]);
        let new = list(&[("a", "A"), ("b", "B")]);
        assert_eq!(
            plan(&old, &new),
            Plan::Patch(vec![Op::Insert { html: li("a", "A"), after: None }])
        );
    }

    #[test]
    fn remove_middle() {
        let old = list(&[("a", "A"), ("b", "B"), ("c", "C")]);
        let new = list(&[("a", "A"), ("c", "C")]);
        assert_eq!(plan(&old, &new), Plan::Patch(vec![Op::Remove { key: "b".into() }]));
    }

    #[test]
    fn reorder_moves_minimum() {
        // a,b,c -> c,a,b : the LIS is (a,b); only c moves to the front.
        let old = list(&[("a", "A"), ("b", "B"), ("c", "C")]);
        let new = list(&[("c", "C"), ("a", "A"), ("b", "B")]);
        assert_eq!(
            plan(&old, &new),
            Plan::Patch(vec![Op::Move { key: "c".into(), after: None }])
        );
    }

    #[test]
    fn insert_and_change_mixed() {
        let old = list(&[("a", "A"), ("b", "B")]);
        let new = list(&[("a", "A2"), ("x", "X"), ("b", "B")]);
        assert_eq!(
            plan(&old, &new),
            Plan::Patch(vec![
                Op::Replace { key: "a".into(), html: li("a", "A2") },
                Op::Insert { html: li("x", "X"), after: Some("a".into()) },
            ])
        );
    }

    #[test]
    fn chrome_change_falls_back_to_full() {
        let old = format!("<h1>0</h1>{}", list(&[("a", "A")]));
        let new = format!("<h1>1</h1>{}", list(&[("a", "A2")]));
        assert_eq!(plan(&old, &new), Plan::Full);
    }

    #[test]
    fn two_lists_fall_back_to_full() {
        let old = format!("{}{}", list(&[("a", "A")]), list(&[("b", "B")]));
        let new = format!("{}{}", list(&[("a", "A")]), list(&[("b", "B"), ("c", "C")]));
        assert_eq!(plan(&old, &new), Plan::Full);
    }

    #[test]
    fn whole_list_replaced_falls_back() {
        let old = list(&[("a", "A"), ("b", "B")]);
        let new = list(&[("x", "X"), ("y", "Y")]);
        assert_eq!(plan(&old, &new), Plan::Full);
    }
}
