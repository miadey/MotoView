//! Keyed-region diffing — runs entirely in the brain (Rust→WASM).
//!
//! The server re-renders a target's full HTML on every change. Replacing it
//! wholesale destroys the live state of every node. When a region carries
//! `key="..."` (compiled to `data-mv-key`), we can do better: if the surrounding
//! structure and the set/order of keys are unchanged, only the keyed regions
//! whose HTML actually changed need to be patched — every other node is left
//! untouched, keeping its focus, selection, scroll and media state.
//!
//! This module turns (old_html, new_html) into a `Plan`. The glue ("hands")
//! only executes the resulting primitive ops; all decisions are made here.

/// A keyed region: its key and the exact HTML span of its element.
pub struct Region {
    pub key: String,
    pub html: String,
}

/// What to do with a freshly rendered target.
#[derive(Debug)]
pub enum Plan {
    /// Replace the whole target (no stable keyed structure to exploit).
    Full,
    /// Structure is stable; replace only these keyed regions (key, new html).
    Patch(Vec<(String, String)>),
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

/// Parse the HTML into a "skeleton" (every keyed region replaced by a
/// `\x01key\x01` marker) plus the list of keyed regions. Returns None when there
/// are no keyed regions — nothing to optimize.
pub fn parse(html: &str) -> Option<(String, Vec<Region>)> {
    let b = html.as_bytes();
    let n = b.len();
    let mut regions: Vec<Region> = Vec::new();
    let mut skel = String::new();
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
                    skel.push_str(&html[last..i]);
                    skel.push('\u{1}');
                    skel.push_str(&key);
                    skel.push('\u{1}');
                    regions.push(Region { key, html: html[i..span_end].to_string() });
                    i = span_end;
                    last = span_end;
                    continue;
                } else {
                    // non-keyed open tag: keep scanning its children for keys
                    i = open_end;
                    continue;
                }
            }
        }
        i += 1;
    }
    skel.push_str(&html[last..]);
    if regions.is_empty() {
        None
    } else {
        Some((skel, regions))
    }
}

/// Compute how to update a target whose previous HTML was `old` to `new`.
pub fn plan(old: &str, new: &str) -> Plan {
    let (old_skel, old_regions) = match parse(old) {
        Some(x) => x,
        None => return Plan::Full,
    };
    let (new_skel, new_regions) = match parse(new) {
        Some(x) => x,
        None => return Plan::Full,
    };
    // A matching skeleton means: identical non-keyed structure AND the same keys
    // in the same order. Anything else (added/removed/reordered keys, or changed
    // surrounding chrome) falls back to a correct full swap.
    if old_skel != new_skel {
        return Plan::Full;
    }
    let mut patches: Vec<(String, String)> = Vec::new();
    for (idx, nr) in new_regions.iter().enumerate() {
        // skeletons matched, so old_regions[idx].key == nr.key
        if old_regions[idx].html != nr.html {
            patches.push((nr.key.clone(), nr.html.clone()));
        }
    }
    Plan::Patch(patches)
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

/// Match `name` (lowercased) at b[pos..] as a tag name boundary.
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
    // the next char must end the name (space, >, /, or end)
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

    fn keys(p: &Plan) -> Vec<String> {
        match p {
            Plan::Patch(v) => v.iter().map(|(k, _)| k.clone()).collect(),
            Plan::Full => vec!["<FULL>".into()],
        }
    }

    #[test]
    fn no_keys_is_full() {
        assert!(matches!(plan("<p>a</p>", "<p>b</p>"), Plan::Full));
    }

    #[test]
    fn stable_list_patches_changed_only() {
        let old = r#"<ul><li data-mv-key="a">A</li><li data-mv-key="b">B</li><li data-mv-key="c">C</li></ul>"#;
        let new = r#"<ul><li data-mv-key="a">A</li><li data-mv-key="b">B2</li><li data-mv-key="c">C</li></ul>"#;
        match plan(old, new) {
            Plan::Patch(p) => { assert_eq!(p.len(), 1); assert_eq!(p[0].0, "b"); assert!(p[0].1.contains("B2")); }
            _ => panic!("expected patch, got {:?}", keys(&plan(old, new))),
        }
    }

    #[test]
    fn unchanged_is_empty_patch() {
        let h = r#"<ul><li data-mv-key="a">A</li></ul>"#;
        match plan(h, h) { Plan::Patch(p) => assert_eq!(p.len(), 0), _ => panic!("expected empty patch") }
    }

    #[test]
    fn insert_falls_back_to_full() {
        let old = r#"<ul><li data-mv-key="a">A</li></ul>"#;
        let new = r#"<ul><li data-mv-key="a">A</li><li data-mv-key="b">B</li></ul>"#;
        assert!(matches!(plan(old, new), Plan::Full));
    }

    #[test]
    fn chrome_change_falls_back_to_full() {
        let old = r#"<h1>0</h1><ul><li data-mv-key="a">A</li></ul>"#;
        let new = r#"<h1>1</h1><ul><li data-mv-key="a">A2</li></ul>"#;
        assert!(matches!(plan(old, new), Plan::Full));
    }

    #[test]
    fn nested_keyed_span_captured() {
        let old = r#"<li data-mv-key="a"><div class="x"><span>x</span></div></li><li data-mv-key="b">B</li>"#;
        let new = r#"<li data-mv-key="a"><div class="x"><span>x</span></div></li><li data-mv-key="b">B2</li>"#;
        match plan(old, new) { Plan::Patch(p) => { assert_eq!(p.len(), 1); assert_eq!(p[0].0, "b"); } _ => panic!("expected patch") }
    }

    #[test]
    fn void_keyed_element_then_change() {
        let old = r#"<input data-mv-key="f" value="1"><span data-mv-key="s">A</span>"#;
        let new = r#"<input data-mv-key="f" value="1"><span data-mv-key="s">B</span>"#;
        match plan(old, new) { Plan::Patch(p) => { assert_eq!(p.len(), 1); assert_eq!(p[0].0, "s"); } _ => panic!("expected patch") }
    }
}
