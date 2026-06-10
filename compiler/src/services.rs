//! Project Motoko **service-surface** scanner (R10 — backend-bound completion).
//!
//! This is the Rust-side mirror of `tools/studio/signatures.js`: given a project
//! directory it scans every `src/Services/*.mo`, finds the stateful-service
//! `public class <Name>(...)` body (the MotoView convention), and extracts the
//! PUBLIC declarations (`public func` / `public type` / `public let`) into a flat
//! list of [`Decl`] entries (name + kind + a clean one-line signature).
//!
//! Why a Rust scanner and not a call out to the JS: the LSP ([`crate::lsp`]) must
//! offer these completions IN-PROCESS while the editor types, with no Node
//! dependency on the hot path. The "no drift" differentiator is that the in-`@code`
//! completion palette IS the project's real backend surface — so we read it from
//! the same files the compiler imports, with the same shape `signatures.js` emits.
//!
//! Honest scope (flagged, not faked): this is a **line/brace-oriented scanner**,
//! NOT a full Motoko parser. It recognizes the conventional service shape (a
//! `public class Name(...)` body containing `public func/type/let` decls), which is
//! exactly the convention every MotoView stateful service follows. A service file
//! that does not use that shape (e.g. a bare `module { }` of free helpers, like the
//! stateless Blog service) yields no completions — by design, since only the
//! stateful-class surface is the page-callable palette.

use std::path::{Path, PathBuf};

/// The kind of a service declaration. Maps to an LSP `CompletionItemKind`:
/// Function (3), Struct (22) for a `type`, Constant (21) for a `let`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeclKind {
    Func,
    Type,
    Let,
}

impl DeclKind {
    /// The LSP `CompletionItemKind` integer for this declaration kind.
    /// 3 = Function, 22 = Struct, 21 = Constant.
    pub fn lsp_kind(self) -> i64 {
        match self {
            DeclKind::Func => 3,
            DeclKind::Type => 22,
            DeclKind::Let => 21,
        }
    }

    /// The Motoko keyword for this declaration kind (used in detail text).
    pub fn keyword(self) -> &'static str {
        match self {
            DeclKind::Func => "func",
            DeclKind::Type => "type",
            DeclKind::Let => "let",
        }
    }
}

/// One PUBLIC declaration extracted from a service class.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Decl {
    /// The declared name (the completion `label`), e.g. `add`, `Deal`, `stages`.
    pub name: String,
    /// What it is (drives the LSP CompletionItemKind + the detail keyword).
    pub kind: DeclKind,
    /// A clean one-line signature, e.g. `add(deals : [Deal], id : Nat) : [Deal]`.
    pub signature: String,
    /// The service class this decl belongs to (its `public class` name).
    pub service: String,
}

/// A scanned service: the `public class` name plus its public declarations.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Service {
    pub name: String,
    pub decls: Vec<Decl>,
}

/// Persistence plumbing that is compiler-wired, not page-facing — skipped just like
/// `signatures.js` does, so the completion palette stays the author-callable surface.
const PERSISTENCE: &[&str] = &["mvStableSave", "mvStableLoad"];

/// Strip a `// …` line comment tail WITHOUT eating a `//` inside a string literal.
fn strip_line_comment(line: &str) -> String {
    let bytes: Vec<char> = line.chars().collect();
    let mut in_str = false;
    let mut i = 0;
    while i + 1 < bytes.len() {
        let c = bytes[i];
        if c == '"' && (i == 0 || bytes[i - 1] != '\\') {
            in_str = !in_str;
        }
        if !in_str && c == '/' && bytes[i + 1] == '/' {
            return bytes[..i].iter().collect();
        }
        i += 1;
    }
    line.to_string()
}

/// Collapse runs of whitespace (incl. newlines) into single spaces and trim.
fn normalize_ws(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn is_opener(c: char) -> bool {
    c == '(' || c == '{' || c == '['
}
fn is_closer(c: char) -> bool {
    c == ')' || c == '}' || c == ']'
}

/// Find the body of the `public class <Name>(...)` in a service source. Returns
/// `(name, body)` (body excludes the outer braces) or `None` when the file has no
/// service class (e.g. a stateless `module { }` of helpers — by design these have
/// no page-callable palette). Mirrors `signatures.js::findServiceClass`.
pub fn find_service_class(src: &str) -> Option<(String, String)> {
    let chars: Vec<char> = src.chars().collect();
    // Locate `public class Name(` by a small hand scan (no regex dependency).
    let needle: Vec<char> = "public".chars().collect();
    let mut i = 0;
    while i < chars.len() {
        if matches_at(&chars, i, &needle) && is_word_boundary(&chars, i, needle.len()) {
            // skip `public` + ws
            let mut j = i + needle.len();
            j = skip_ws(&chars, j);
            let class_kw: Vec<char> = "class".chars().collect();
            if matches_at(&chars, j, &class_kw) && is_word_boundary(&chars, j, class_kw.len()) {
                j += class_kw.len();
                j = skip_ws(&chars, j);
                // read the class name (identifier)
                let name_start = j;
                while j < chars.len() && is_ident_char(chars[j]) {
                    j += 1;
                }
                if j > name_start {
                    let name: String = chars[name_start..j].iter().collect();
                    j = skip_ws(&chars, j);
                    if j < chars.len() && chars[j] == '(' {
                        // skip the constructor param list to its matching ')'
                        let mut depth = 1;
                        let mut k = j + 1;
                        let mut in_str = false;
                        while k < chars.len() && depth > 0 {
                            let c = chars[k];
                            if in_str {
                                if c == '"' && chars[k - 1] != '\\' {
                                    in_str = false;
                                }
                            } else if c == '"' {
                                in_str = true;
                            } else if c == '(' {
                                depth += 1;
                            } else if c == ')' {
                                depth -= 1;
                            }
                            k += 1;
                        }
                        // find the class-body opening '{'
                        while k < chars.len() && chars[k] != '{' {
                            k += 1;
                        }
                        if k < chars.len() {
                            // capture to the matching '}'
                            let body = capture_braced(&chars, k);
                            return Some((name, body));
                        }
                    }
                }
            }
        }
        i += 1;
    }
    None
}

/// Capture the brace-balanced body starting at the `{` at `open_idx` (exclusive of
/// the outer braces). String literals are skipped so a `"}"` inside a string does
/// not close the body early.
fn capture_braced(chars: &[char], open_idx: usize) -> String {
    let mut depth = 0i32;
    let mut in_str = false;
    let mut j = open_idx;
    let mut body_start = open_idx + 1;
    while j < chars.len() {
        let c = chars[j];
        if in_str {
            if c == '"' && chars[j - 1] != '\\' {
                in_str = false;
            }
        } else if c == '"' {
            in_str = true;
        } else if c == '{' {
            if depth == 0 {
                body_start = j + 1;
            }
            depth += 1;
        } else if c == '}' {
            depth -= 1;
            if depth == 0 {
                return chars[body_start..j].iter().collect();
            }
        }
        j += 1;
    }
    chars[body_start..].iter().collect()
}

/// Walk a service body from `start`, returning the decl text up to the `;` that
/// terminates it at brace-depth 0 (string literals respected), plus the index just
/// past that `;`. Mirrors `signatures.js::readDecl`.
fn read_decl(body: &[char], start: usize) -> (String, usize) {
    let mut depth = 0i32;
    let mut in_str = false;
    let mut i = start;
    while i < body.len() {
        let c = body[i];
        if in_str {
            if c == '"' && body[i - 1] != '\\' {
                in_str = false;
            }
            i += 1;
            continue;
        }
        if c == '"' {
            in_str = true;
        } else if is_opener(c) {
            depth += 1;
        } else if is_closer(c) {
            depth -= 1;
        } else if c == ';' && depth == 0 {
            return (body[start..i].iter().collect(), i + 1);
        }
        i += 1;
    }
    (body[start..i].iter().collect(), i)
}

/// Produce a clean `name(params) : Ret` signature from a `func` decl (body dropped).
/// `decl_text` begins at the `func` keyword. Mirrors `signatures.js::funcSignature`
/// for the shapes MotoView services use.
fn func_signature(decl_text: &str) -> Option<(String, String)> {
    let after = decl_text.trim_start();
    let after = after.strip_prefix("func")?.trim_start();
    let chars: Vec<char> = after.chars().collect();
    // name
    let mut i = 0;
    let name_start = i;
    while i < chars.len() && is_ident_char(chars[i]) {
        i += 1;
    }
    if i == name_start {
        return None;
    }
    let name: String = chars[name_start..i].iter().collect();
    // optional generics <...>
    let mut generics = String::new();
    let j = skip_ws(&chars, i);
    if j < chars.len() && chars[j] == '<' {
        if let Some(end) = find_char(&chars, j + 1, '>') {
            generics = chars[j..=end].iter().collect();
            i = end + 1;
        }
    } else {
        i = j;
    }
    i = skip_ws(&chars, i);
    if i >= chars.len() || chars[i] != '(' {
        // no param list — not a shape we sign; return bare name.
        return Some((name.clone(), name));
    }
    // capture the param list to its matching ')'
    let paren_start = i;
    let mut depth = 0i32;
    let mut in_str = false;
    let mut k = i;
    while k < chars.len() {
        let c = chars[k];
        if in_str {
            if c == '"' && chars[k - 1] != '\\' {
                in_str = false;
            }
        } else if c == '"' {
            in_str = true;
        } else if c == '(' {
            depth += 1;
        } else if c == ')' {
            depth -= 1;
            if depth == 0 {
                k += 1;
                break;
            }
        }
        k += 1;
    }
    let params_raw: String = chars[paren_start..k].iter().collect();
    let params = normalize_ws(&params_raw)
        .replace(", )", " )")
        .replace("( ", "(")
        .replace(" )", ")");
    // optional `: ReturnType` — capture ONE type-expression, dropping the body.
    let rest: Vec<char> = chars[k..].iter().copied().collect();
    let mut ret_clean = String::new();
    let mut p = skip_ws(&rest, 0);
    if p < rest.len() && rest[p] == ':' {
        p = skip_ws(&rest, p + 1);
        let mut d2 = 0i32;
        let mut seen = false;
        let mut in_str2 = false;
        let mut ret = String::new();
        while p < rest.len() {
            let c = rest[p];
            if in_str2 {
                ret.push(c);
                if c == '"' && rest[p - 1] != '\\' {
                    in_str2 = false;
                }
                p += 1;
                continue;
            }
            if c == '"' {
                in_str2 = true;
                ret.push(c);
                seen = true;
                p += 1;
                continue;
            }
            if d2 == 0 && seen && (c == '{' || c == '=' || c == ';') {
                break;
            }
            if is_opener(c) {
                d2 += 1;
            } else if is_closer(c) {
                d2 -= 1;
            }
            if !c.is_whitespace() {
                seen = true;
            }
            ret.push(c);
            if d2 == 0 && (c == '}' || c == ']' || c == ')') && !ret.trim().is_empty() {
                let first = ret.trim().chars().next().unwrap();
                if first == '{' || first == '[' || first == '(' {
                    break;
                }
            }
            p += 1;
        }
        ret_clean = normalize_ws(&ret);
    }
    let signature = if ret_clean.is_empty() {
        normalize_ws(&format!("{}{}{}", name, generics, params))
    } else {
        normalize_ws(&format!("{}{}{} : {}", name, generics, params, ret_clean))
    };
    Some((name, signature))
}

/// Signature for a `type` decl: `Name = <rhs>`. Mirrors `signatures.js::typeSignature`.
fn type_signature(decl_text: &str) -> Option<(String, String)> {
    let after = decl_text.trim_start().strip_prefix("type")?.trim_start();
    let chars: Vec<char> = after.chars().collect();
    let mut i = 0;
    while i < chars.len() && is_ident_char(chars[i]) {
        i += 1;
    }
    if i == 0 {
        return None;
    }
    let name: String = chars[..i].iter().collect();
    // optional generics
    let mut generics = String::new();
    let j = skip_ws(&chars, i);
    if j < chars.len() && chars[j] == '<' {
        if let Some(end) = find_char(&chars, j + 1, '>') {
            generics = chars[j..=end].iter().collect();
        }
    }
    let eq = after.find('=')?;
    let rhs = normalize_ws(&after[eq + 1..]);
    Some((name.clone(), normalize_ws(&format!("{}{} = {}", name, generics, rhs))))
}

/// Signature for a `let` decl: `Name : Type` (value/body dropped). Mirrors
/// `signatures.js::letSignature`.
fn let_signature(decl_text: &str) -> Option<(String, String)> {
    let after = decl_text.trim_start().strip_prefix("let")?.trim_start();
    let chars: Vec<char> = after.chars().collect();
    let mut i = 0;
    while i < chars.len() && is_ident_char(chars[i]) {
        i += 1;
    }
    if i == 0 {
        return None;
    }
    let name: String = chars[..i].iter().collect();
    let head = match after.find('=') {
        Some(eq) => &after[..eq],
        None => after,
    };
    // type annotation between name and `=`
    let ty = head
        .find(':')
        .map(|c| normalize_ws(&head[c + 1..]))
        .unwrap_or_default();
    let signature = if ty.is_empty() {
        name.clone()
    } else {
        format!("{} : {}", name, ty)
    };
    Some((name, signature))
}

/// Extract all PUBLIC declarations from a service-class body. Skips upgrade
/// persistence plumbing. Mirrors `signatures.js::extractDecls`.
pub fn extract_decls(body: &str, service: &str) -> Vec<Decl> {
    let chars: Vec<char> = body.chars().collect();
    let pub_kw: Vec<char> = "public".chars().collect();
    let mut decls = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        if matches_at(&chars, i, &pub_kw) && is_word_boundary(&chars, i, pub_kw.len()) {
            let mut j = skip_ws(&chars, i + pub_kw.len());
            let kind = if matches_kw(&chars, j, "func") {
                Some((DeclKind::Func, 4))
            } else if matches_kw(&chars, j, "type") {
                Some((DeclKind::Type, 4))
            } else if matches_kw(&chars, j, "let") {
                Some((DeclKind::Let, 3))
            } else {
                None
            };
            if let Some((dk, _kwlen)) = kind {
                // decl text starts at the keyword (func/type/let).
                let (raw, end) = read_decl(&chars, j);
                // strip line comments per line
                let stripped: String = raw
                    .lines()
                    .map(strip_line_comment)
                    .collect::<Vec<_>>()
                    .join("\n");
                let sig = match dk {
                    DeclKind::Func => func_signature(&stripped),
                    DeclKind::Type => type_signature(&stripped),
                    DeclKind::Let => let_signature(&stripped),
                };
                if let Some((name, signature)) = sig {
                    let skip = dk == DeclKind::Func && PERSISTENCE.contains(&name.as_str());
                    if !skip {
                        decls.push(Decl {
                            name,
                            kind: dk,
                            signature,
                            service: service.to_string(),
                        });
                    }
                }
                i = j.max(end);
                continue;
            }
        }
        i += 1;
    }
    decls
}

/// Scan one `Services/*.mo` file. Returns `Some(Service)` if it has a service class.
pub fn extract_file(path: &Path) -> Option<Service> {
    let src = std::fs::read_to_string(path).ok()?;
    let (name, body) = find_service_class(&src)?;
    let decls = extract_decls(&body, &name);
    Some(Service { name, decls })
}

/// Scan a whole project's `src/Services/*.mo` for service surfaces. Returns every
/// stateful service with its public declarations (sorted by file name for
/// determinism). Stateless `module {}` services contribute nothing (no class).
pub fn extract_project(project_dir: &Path) -> Vec<Service> {
    let services_dir = project_dir.join("src").join("Services");
    let mut files: Vec<PathBuf> = match std::fs::read_dir(&services_dir) {
        Ok(rd) => rd
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| p.extension().map(|x| x == "mo").unwrap_or(false))
            .collect(),
        Err(_) => return Vec::new(),
    };
    files.sort();
    let mut out = Vec::new();
    for f in files {
        if let Some(svc) = extract_file(&f) {
            out.push(svc);
        }
    }
    out
}

/// All declarations across a project's services, flattened (the completion palette).
pub fn project_decls(project_dir: &Path) -> Vec<Decl> {
    extract_project(project_dir)
        .into_iter()
        .flat_map(|s| s.decls)
        .collect()
}

/// Find the project root for a `.mview` document URI/path: walk up from the file's
/// directory looking for a `dfx.json` (the MotoView/dfx project marker) or a
/// `src/Services` directory. Returns the first ancestor that has either. Falls back
/// to the `src/`-parent heuristic (`…/src/Pages/Foo.mview` -> `…`).
pub fn project_root_for(uri: &str) -> Option<PathBuf> {
    let path = uri.strip_prefix("file://").unwrap_or(uri);
    let mut dir = Path::new(path).parent();
    while let Some(d) = dir {
        if d.join("dfx.json").is_file() || d.join("src").join("Services").is_dir() {
            return Some(d.to_path_buf());
        }
        dir = d.parent();
    }
    // Heuristic fallback: a path like `…/src/Pages/Foo.mview` has the project root
    // two levels above the `Pages` dir (the parent of `src`).
    let p = Path::new(path);
    let mut cur = p.parent();
    while let Some(d) = cur {
        if d.file_name().map(|n| n == "src").unwrap_or(false) {
            return d.parent().map(|x| x.to_path_buf());
        }
        cur = d.parent();
    }
    None
}

// ---- tiny char-scan helpers -------------------------------------------------

fn is_ident_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_'
}

fn skip_ws(chars: &[char], mut i: usize) -> usize {
    while i < chars.len() && chars[i].is_whitespace() {
        i += 1;
    }
    i
}

fn matches_at(chars: &[char], at: usize, needle: &[char]) -> bool {
    if at + needle.len() > chars.len() {
        return false;
    }
    chars[at..at + needle.len()] == *needle
}

/// True when `needle` placed at `at` (length `len`) is bounded by non-identifier
/// chars on both sides — so `public` matches the keyword, not `republicans`.
fn is_word_boundary(chars: &[char], at: usize, len: usize) -> bool {
    let before_ok = at == 0 || !is_ident_char(chars[at - 1]);
    let after_idx = at + len;
    let after_ok = after_idx >= chars.len() || !is_ident_char(chars[after_idx]);
    before_ok && after_ok
}

fn matches_kw(chars: &[char], at: usize, kw: &str) -> bool {
    let n: Vec<char> = kw.chars().collect();
    matches_at(chars, at, &n) && is_word_boundary(chars, at, n.len())
}

fn find_char(chars: &[char], from: usize, target: char) -> Option<usize> {
    (from..chars.len()).find(|&i| chars[i] == target)
}

#[cfg(test)]
mod tests {
    use super::*;

    const CRM_LIKE: &str = r#"
import Array "mo:base/Array";
module {
  public class Crm() {
    public type Deal = { id : Nat; title : Text; stage : Text };
    public let stages : [Text] = ["Lead", "Contacted", "Won"];
    public func seed() : [Deal] { [] };
    public func add(deals : [Deal], id : Nat, title : Text) : [Deal] {
      Array.append(deals, [{ id; title; stage = "Lead" }]);
    };
    // not public — must be skipped
    func internalHelper() : Nat { 0 };
    public func mvStableSave() : Blob { "" };  // persistence — skipped
  };
}
"#;

    #[test]
    fn finds_service_class_and_name() {
        let (name, body) = find_service_class(CRM_LIKE).expect("service class");
        assert_eq!(name, "Crm");
        assert!(body.contains("public func add"), "body captured");
    }

    #[test]
    fn extracts_public_func_type_let_skips_private_and_persistence() {
        let (name, body) = find_service_class(CRM_LIKE).unwrap();
        let decls = extract_decls(&body, &name);
        let names: Vec<&str> = decls.iter().map(|d| d.name.as_str()).collect();
        assert!(names.contains(&"add"), "{:?}", names);
        assert!(names.contains(&"seed"), "{:?}", names);
        assert!(names.contains(&"Deal"), "{:?}", names);
        assert!(names.contains(&"stages"), "{:?}", names);
        // private + persistence are NOT in the palette.
        assert!(!names.contains(&"internalHelper"), "private leaked: {:?}", names);
        assert!(!names.contains(&"mvStableSave"), "persistence leaked: {:?}", names);
    }

    #[test]
    fn func_signature_keeps_params_and_return_drops_body() {
        let (_n, body) = find_service_class(CRM_LIKE).unwrap();
        let decls = extract_decls(&body, "Crm");
        let add = decls.iter().find(|d| d.name == "add").unwrap();
        assert_eq!(add.kind, DeclKind::Func);
        assert_eq!(
            add.signature,
            "add(deals : [Deal], id : Nat, title : Text) : [Deal]"
        );
    }

    #[test]
    fn type_and_let_signatures() {
        let (_n, body) = find_service_class(CRM_LIKE).unwrap();
        let decls = extract_decls(&body, "Crm");
        let deal = decls.iter().find(|d| d.name == "Deal").unwrap();
        assert_eq!(deal.kind, DeclKind::Type);
        assert!(deal.signature.starts_with("Deal = {"), "{}", deal.signature);
        let stages = decls.iter().find(|d| d.name == "stages").unwrap();
        assert_eq!(stages.kind, DeclKind::Let);
        assert_eq!(stages.signature, "stages : [Text]");
    }

    #[test]
    fn stateless_module_has_no_service_class() {
        let stateless = "module { public func all() : [Nat] { [] }; }";
        assert!(find_service_class(stateless).is_none());
    }

    #[test]
    fn project_root_from_src_pages_heuristic() {
        // No fs needed: the heuristic walks the path to the parent of `src`.
        let root =
            project_root_for("file:///home/me/proj/src/Pages/Counter.mview").expect("root");
        assert_eq!(root, PathBuf::from("/home/me/proj"));
    }
}
