//! AST lint pass — security gates that run BEFORE codegen.
//!
//! These are *structural* checks on the parsed `.mview` tree (no Motoko type
//! info needed). They catch security footguns the type-checker can't see:
//!
//!   * `secure-form` (Error): a state-mutating `<form @submit=...>` that is NOT
//!     marked `secure`. Secure forms mint an HMAC token binding the request, so
//!     an unsecured mutating form is a CSRF + over-posting hole. The build is
//!     aborted (see `project::build`).
//!   * `raw-html` (Warning): every `@raw(...)` is an unescaped-HTML / XSS sink;
//!     advisory only — it never blocks the build.
//!
//! The walker mirrors the node/element shape `codegen.rs` uses, so forms/raw
//! nested anywhere in the tree (inside `@if`/`@for`/`@switch`/components/slots)
//! are still seen.

use crate::ast::{MviewFile, Node};
use crate::span::{self, Span};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Severity {
    Error,
    Warning,
}

impl Severity {
    /// The lowercase label used in both the human (`error:`/`warning:`) output
    /// and the machine-readable `--json` `severity` field, so the two never drift.
    pub fn label(self) -> &'static str {
        match self {
            Severity::Error => "error",
            Severity::Warning => "warning",
        }
    }
}

#[derive(Debug, Clone)]
pub struct Diagnostic {
    pub severity: Severity,
    pub message: String,
    /// Source location: file path plus the element/handler that triggered it.
    /// This is the HUMAN-facing string the text formatter prints; it is kept
    /// byte-for-byte as it was, independent of the structured `span` below.
    pub location: String,
    /// Stable rule id (e.g. "secure-form", "raw-html").
    pub rule: String,
    /// Source span of the offending node (file-relative char offsets), when the
    /// rule can point at a precise AST node. `None` for rules whose node carries
    /// no span yet (e.g. `raw-html`, since `Node::Raw` has no span — see ast.rs).
    /// Resolved to line/col on demand via [`Position::from_span`] for `--json`.
    pub span: Option<Span>,
}

/// A 1-based line/column range, the structured position surfaced in `--json`
/// diagnostics. Derived from a [`Span`] + the source via [`span::line_col`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Position {
    pub line: u32,
    pub col: u32,
    pub end_line: u32,
    pub end_col: u32,
}

impl Position {
    /// Resolve a span against the source character slice the offsets index into
    /// (`source.chars().collect()`), yielding 1-based start/end line+column.
    pub fn from_span(span: Span, src: &[char]) -> Position {
        let (line, col) = span::line_col(src, span.start);
        let (end_line, end_col) = span::line_col(src, span.end);
        Position { line, col, end_line, end_col }
    }
}

/// A fully-resolved, machine-readable diagnostic — the JSON shape emitted by
/// both `motoview lint --json` and `motoview check --json`. Positions are 1-based
/// (`line`/`col` = start, `endLine`/`endCol` = end); `0` means "no position"
/// (the rule could not be tied to a precise source span). This struct is the
/// single source of truth for the `--json` schema across both commands.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JsonDiagnostic {
    pub severity: Severity,
    pub rule: String,
    pub message: String,
    pub file: String,
    pub line: u32,
    pub col: u32,
    pub end_line: u32,
    pub end_col: u32,
}

impl JsonDiagnostic {
    /// Build a JSON diagnostic from a lint [`Diagnostic`] + its source file, where
    /// `src` is the file's character slice (`source.chars().collect()`) used to
    /// resolve the node span to line/col. A rule with no span gets zeroed
    /// positions (so the editor knows there is no precise location).
    pub fn from_lint(file: &str, d: &Diagnostic, src: &[char]) -> JsonDiagnostic {
        let pos = d.span.map(|s| Position::from_span(s, src));
        JsonDiagnostic {
            severity: d.severity,
            rule: d.rule.clone(),
            message: d.message.clone(),
            file: file.to_string(),
            line: pos.map(|p| p.line).unwrap_or(0),
            col: pos.map(|p| p.col).unwrap_or(0),
            end_line: pos.map(|p| p.end_line).unwrap_or(0),
            end_col: pos.map(|p| p.end_col).unwrap_or(0),
        }
    }

    /// Serialize ONE diagnostic to a JSON object string.
    pub fn to_json(&self) -> String {
        format!(
            "{{\"severity\":{},\"rule\":{},\"message\":{},\"file\":{},\
             \"line\":{},\"col\":{},\"endLine\":{},\"endCol\":{}}}",
            json_string(self.severity.label()),
            json_string(&self.rule),
            json_string(&self.message),
            json_string(&self.file),
            self.line,
            self.col,
            self.end_line,
            self.end_col,
        )
    }
}

/// Serialize a slice of diagnostics as a JSON array (`[]` when empty). This is
/// the exact stdout of `lint --json` / `check --json`.
pub fn diagnostics_to_json(diags: &[JsonDiagnostic]) -> String {
    let body = diags
        .iter()
        .map(|d| d.to_json())
        .collect::<Vec<_>>()
        .join(",");
    format!("[{}]", body)
}

/// Escape a string into a JSON string literal (including the surrounding quotes).
/// Handcrafted so the compiler keeps zero dependencies (matching the rest of the
/// crate, which hand-rolls its JSON). Covers the control characters JSON requires
/// to be escaped plus `"` and `\`.
fn json_string(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{08}' => out.push_str("\\b"),
            '\u{0c}' => out.push_str("\\f"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// Run all lint rules over a parsed file, returning every diagnostic found.
/// `path` is the `.mview` source path, used to build the `location` string.
pub fn lint_file(file: &MviewFile, path: &str) -> Vec<Diagnostic> {
    let mut diags = Vec::new();
    walk_nodes(&file.template, path, &mut diags);
    for (_, body) in &file.sections {
        walk_nodes(body, path, &mut diags);
    }
    diags
}

fn walk_nodes(nodes: &[Node], path: &str, diags: &mut Vec<Diagnostic>) {
    for node in nodes {
        walk_node(node, path, diags);
    }
}

fn walk_node(node: &Node, path: &str, diags: &mut Vec<Diagnostic>) {
    match node {
        Node::Raw(_) => {
            // RULE raw-html (advisory): @raw emits unescaped HTML.
            diags.push(Diagnostic {
                severity: Severity::Warning,
                message: "@raw emits unescaped HTML — ensure the expression is \
                          trusted server-generated HTML, never user input (XSS sink)."
                    .to_string(),
                location: format!("{} (@raw)", path),
                rule: "raw-html".to_string(),
                // `Node::Raw` carries no span yet (see ast.rs TODO), so this rule
                // has no precise position; `--json` emits a zeroed/omitted span.
                span: None,
            });
        }
        Node::Element(el) => {
            // RULE secure-form: a <form> that MUTATES (has an @submit event) must
            // be `secure`. A <form> WITHOUT @submit (e.g. a GET search form) is
            // not mutating and is never flagged.
            // Case-insensitive matching mirrors codegen (codegen.rs) exactly:
            // the parser already lowercases element tags and event names, but we
            // match case-insensitively here too so the lint can never diverge
            // from what codegen wires as a live submit form (CSRF-relevant).
            let submit = el.events.iter().find(|e| e.event.eq_ignore_ascii_case("submit"));
            if el.tag.eq_ignore_ascii_case("form") {
                if let Some(ev) = submit {
                    if !el.secure {
                        diags.push(Diagnostic {
                            severity: Severity::Error,
                            message:
                                "state-mutating <form @submit=...> must be marked \
                                 `secure` (or remove the submit handler). Secure forms \
                                 mint an HMAC token binding the request; an unsecured \
                                 mutating form is a CSRF + over-posting hole."
                                    .to_string(),
                            location: format!("{} (<form @submit=\"{}\">)", path, ev.handler),
                            rule: "secure-form".to_string(),
                            // Point at the offending `<form ...>` open tag so the
                            // editor / repair loop can place a squiggle exactly there.
                            span: Some(el.span),
                        });
                    }
                }
            }
            walk_nodes(&el.children, path, diags);
        }
        Node::Component(c) => {
            // @raw / forms can live inside a component's default slot or named slots.
            walk_nodes(&c.children, path, diags);
            for (_, body) in &c.slots {
                walk_nodes(body, path, diags);
            }
        }
        Node::If(branches) => {
            for br in branches {
                walk_nodes(&br.body, path, diags);
            }
        }
        Node::For { body, .. } => walk_nodes(body, path, diags),
        Node::Switch { cases, .. } => {
            for c in cases {
                walk_nodes(&c.body, path, diags);
            }
        }
        // Leaf / non-container nodes carry no nested template to walk.
        Node::Text(_)
        | Node::Expr(_)
        | Node::Yield
        | Node::Head
        | Node::SectionRef(_)
        | Node::Slot(_)
        | Node::Effect { .. } => {}
    }
}

#[cfg(test)]
mod json_tests {
    use super::*;
    use crate::ast::FileKind;
    use crate::parser;

    fn chars(s: &str) -> Vec<char> {
        s.chars().collect()
    }

    #[test]
    fn json_string_escapes_quotes_backslashes_and_controls() {
        assert_eq!(json_string("a\"b\\c"), "\"a\\\"b\\\\c\"");
        assert_eq!(json_string("line1\nline2\there"), "\"line1\\nline2\\there\"");
        // a sub-0x20 control char becomes a \u escape
        assert_eq!(json_string("\u{1}"), "\"\\u0001\"");
    }

    #[test]
    fn empty_diagnostics_serialize_to_empty_array() {
        assert_eq!(diagnostics_to_json(&[]), "[]");
    }

    #[test]
    fn secure_form_json_has_severity_rule_and_position() {
        // <form> sits on line 2, column 1 (after the `@page "/"\n` line).
        let src = "@page \"/\"\n<form @submit=\"save\"><button>Go</button></form>\n@code { func save(ctx : Context) : async () { ignore ctx; }; }";
        let file = parser::parse(src, "T", FileKind::Page).expect("parse");
        let diags = lint_file(&file, "src/Pages/T.mview");
        let src_chars = chars(src);
        let json: Vec<JsonDiagnostic> = diags
            .iter()
            .map(|d| JsonDiagnostic::from_lint("src/Pages/T.mview", d, &src_chars))
            .collect();
        let secure = json
            .iter()
            .find(|d| d.rule == "secure-form")
            .expect("a secure-form diagnostic");
        assert_eq!(secure.severity, Severity::Error);
        assert_eq!(secure.file, "src/Pages/T.mview");
        // The <form ...> open tag starts at line 2, col 1.
        assert_eq!((secure.line, secure.col), (2, 1));
        assert!(secure.end_line >= secure.line);
        // Round-trip through the JSON object string: the fields are present.
        let s = secure.to_json();
        assert!(s.contains("\"severity\":\"error\""), "{s}");
        assert!(s.contains("\"rule\":\"secure-form\""), "{s}");
        assert!(s.contains("\"line\":2"), "{s}");
        assert!(s.contains("\"col\":1"), "{s}");
    }

    #[test]
    fn raw_html_json_warning_has_zeroed_position() {
        let src = "@page \"/\"\n<div>@raw(body)</div>\n@code { var body : Text = \"<b>x</b>\"; }";
        let file = parser::parse(src, "T", FileKind::Page).expect("parse");
        let diags = lint_file(&file, "src/Pages/T.mview");
        let src_chars = chars(src);
        let json: Vec<JsonDiagnostic> = diags
            .iter()
            .map(|d| JsonDiagnostic::from_lint("src/Pages/T.mview", d, &src_chars))
            .collect();
        let raw = json
            .iter()
            .find(|d| d.rule == "raw-html")
            .expect("a raw-html diagnostic");
        assert_eq!(raw.severity, Severity::Warning);
        // @raw carries no span -> positions are zeroed.
        assert_eq!((raw.line, raw.col, raw.end_line, raw.end_col), (0, 0, 0, 0));
    }
}
