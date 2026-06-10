//! `.mview` language server (R3) — a minimal but correct LSP over stdio.
//!
//! This is the in-process language server that backs `motoview lsp`: an editor
//! (R6 / VS Code / any LSP client) opens a `.mview` buffer and gets **live**
//! security diagnostics as they type, plus template/directive completions.
//!
//! Design constraints (matching the rest of the crate):
//!   * **Dependency-free.** The JSON-RPC reader/writer, the Content-Length
//!     framing, the JSON value parser, and the JSON serializer are all
//!     hand-rolled here. No `serde`, no `tower-lsp`, no `lsp-types`.
//!   * **Reuse R1/R2.** Diagnostics come from the exact same path the CLI uses:
//!     `parser::parse` -> `lint::lint_file` -> `JsonDiagnostic::from_lint`. The
//!     only LSP-specific work is (a) converting the 1-based R1/R2 line/col to
//!     **0-based** LSP positions, and (b) framing/transport.
//!   * **Unit-testable.** All protocol logic lives on [`LspServer`] as plain
//!     methods that take a parsed request and return a parsed response/notif,
//!     so the protocol test can drive them directly *and* through real framed
//!     stdio. The transport ([`read_message`]/[`write_message`]) is generic over
//!     `BufRead`/`Write` so a test can feed bytes and capture bytes.
//!
//! What is honestly NOT here (flagged, not faked):
//!   * **Motoko-in-`@code` completion is delegated.** Template/directive/element
//!     completion is implemented for real; completion *inside* a `@code { ... }`
//!     block is the integration seam for the WASM Motoko language server
//!     (vscode-motoko). [`LspServer::completion`] detects an in-`@code` cursor
//!     and returns the delegation marker instead of inventing Motoko items.
//!   * **`moc` type-check in the LSP is a follow-up.** Live diagnostics are the
//!     lint pass only (the must-have, structural, no-moc-needed checks). Running
//!     `moc --check` against an in-memory unsaved buffer would require staging the
//!     buffer + assembling the whole project on every keystroke; that is deferred.
//!     See [`LspServer::diagnostics_for`].

use std::collections::HashMap;
use std::io::{BufRead, Write};

use crate::ast::FileKind;
use crate::lint::{self, JsonDiagnostic, Severity};
use crate::parser;
use crate::span::{self, Span};

// ===========================================================================
// Minimal JSON value model + parser (dependency-free).
// ===========================================================================

/// A parsed JSON value. Deliberately tiny — just what the LSP wire needs.
#[derive(Debug, Clone, PartialEq)]
pub enum Json {
    Null,
    Bool(bool),
    Num(f64),
    Str(String),
    Arr(Vec<Json>),
    /// Object preserves nothing about key order (we only ever look keys up).
    Obj(HashMap<String, Json>),
}

impl Json {
    /// `obj.get(key)` for an `Obj`, else `None`. Non-objects yield `None`.
    pub fn get(&self, key: &str) -> Option<&Json> {
        match self {
            Json::Obj(m) => m.get(key),
            _ => None,
        }
    }

    /// Follow a dotted path of object keys (`"params.textDocument.uri"`).
    pub fn path(&self, dotted: &str) -> Option<&Json> {
        let mut cur = self;
        for k in dotted.split('.') {
            cur = cur.get(k)?;
        }
        Some(cur)
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            Json::Str(s) => Some(s),
            _ => None,
        }
    }

    pub fn as_f64(&self) -> Option<f64> {
        match self {
            Json::Num(n) => Some(*n),
            _ => None,
        }
    }

    pub fn as_array(&self) -> Option<&[Json]> {
        match self {
            Json::Arr(a) => Some(a),
            _ => None,
        }
    }
}

/// Parse a JSON document into a [`Json`]. Returns `Err` with a short message on
/// malformed input. Supports the full JSON grammar the LSP wire uses: objects,
/// arrays, strings (with `\uXXXX` + the standard escapes), numbers, and the three
/// literals. Surrogate pairs in `\u` escapes are decoded.
pub fn parse_json(s: &str) -> Result<Json, String> {
    let chars: Vec<char> = s.chars().collect();
    let mut p = JsonParser { c: &chars, i: 0 };
    p.skip_ws();
    let v = p.value()?;
    p.skip_ws();
    if p.i != p.c.len() {
        return Err(format!("trailing JSON at offset {}", p.i));
    }
    Ok(v)
}

struct JsonParser<'a> {
    c: &'a [char],
    i: usize,
}

impl<'a> JsonParser<'a> {
    fn peek(&self) -> char {
        if self.i < self.c.len() {
            self.c[self.i]
        } else {
            '\0'
        }
    }
    fn bump(&mut self) -> char {
        let ch = self.peek();
        self.i += 1;
        ch
    }
    fn skip_ws(&mut self) {
        while self.i < self.c.len() && self.c[self.i].is_ascii_whitespace() {
            self.i += 1;
        }
    }
    fn value(&mut self) -> Result<Json, String> {
        self.skip_ws();
        match self.peek() {
            '{' => self.object(),
            '[' => self.array(),
            '"' => Ok(Json::Str(self.string()?)),
            't' | 'f' => self.boolean(),
            'n' => self.null(),
            c if c == '-' || c.is_ascii_digit() => self.number(),
            other => Err(format!("unexpected '{}' at offset {}", other, self.i)),
        }
    }
    fn object(&mut self) -> Result<Json, String> {
        self.bump(); // '{'
        let mut map = HashMap::new();
        self.skip_ws();
        if self.peek() == '}' {
            self.bump();
            return Ok(Json::Obj(map));
        }
        loop {
            self.skip_ws();
            if self.peek() != '"' {
                return Err(format!("expected object key at offset {}", self.i));
            }
            let key = self.string()?;
            self.skip_ws();
            if self.bump() != ':' {
                return Err(format!("expected ':' at offset {}", self.i));
            }
            let val = self.value()?;
            map.insert(key, val);
            self.skip_ws();
            match self.bump() {
                ',' => continue,
                '}' => break,
                other => return Err(format!("expected ',' or '}}' got '{}'", other)),
            }
        }
        Ok(Json::Obj(map))
    }
    fn array(&mut self) -> Result<Json, String> {
        self.bump(); // '['
        let mut arr = Vec::new();
        self.skip_ws();
        if self.peek() == ']' {
            self.bump();
            return Ok(Json::Arr(arr));
        }
        loop {
            let val = self.value()?;
            arr.push(val);
            self.skip_ws();
            match self.bump() {
                ',' => continue,
                ']' => break,
                other => return Err(format!("expected ',' or ']' got '{}'", other)),
            }
        }
        Ok(Json::Arr(arr))
    }
    fn string(&mut self) -> Result<String, String> {
        self.bump(); // opening '"'
        let mut out = String::new();
        loop {
            if self.i >= self.c.len() {
                return Err("unterminated string".to_string());
            }
            let ch = self.bump();
            match ch {
                '"' => break,
                '\\' => {
                    let esc = self.bump();
                    match esc {
                        '"' => out.push('"'),
                        '\\' => out.push('\\'),
                        '/' => out.push('/'),
                        'n' => out.push('\n'),
                        't' => out.push('\t'),
                        'r' => out.push('\r'),
                        'b' => out.push('\u{08}'),
                        'f' => out.push('\u{0c}'),
                        'u' => {
                            let cp = self.hex4()?;
                            // Decode a UTF-16 surrogate pair if present.
                            if (0xD800..=0xDBFF).contains(&cp) {
                                if self.peek() == '\\' {
                                    self.bump();
                                    if self.bump() != 'u' {
                                        return Err("expected \\u low surrogate".into());
                                    }
                                    let lo = self.hex4()?;
                                    let c = 0x10000 + ((cp - 0xD800) << 10) + (lo - 0xDC00);
                                    if let Some(ch) = char::from_u32(c) {
                                        out.push(ch);
                                    }
                                } else {
                                    return Err("lone high surrogate".into());
                                }
                            } else if let Some(ch) = char::from_u32(cp) {
                                out.push(ch);
                            }
                        }
                        other => return Err(format!("bad escape '\\{}'", other)),
                    }
                }
                c => out.push(c),
            }
        }
        Ok(out)
    }
    fn hex4(&mut self) -> Result<u32, String> {
        let mut v = 0u32;
        for _ in 0..4 {
            let c = self.bump();
            let d = c
                .to_digit(16)
                .ok_or_else(|| format!("bad hex digit '{}'", c))?;
            v = v * 16 + d;
        }
        Ok(v)
    }
    fn boolean(&mut self) -> Result<Json, String> {
        if self.starts_with("true") {
            self.i += 4;
            Ok(Json::Bool(true))
        } else if self.starts_with("false") {
            self.i += 5;
            Ok(Json::Bool(false))
        } else {
            Err("invalid literal".into())
        }
    }
    fn null(&mut self) -> Result<Json, String> {
        if self.starts_with("null") {
            self.i += 4;
            Ok(Json::Null)
        } else {
            Err("invalid literal".into())
        }
    }
    fn number(&mut self) -> Result<Json, String> {
        let start = self.i;
        if self.peek() == '-' {
            self.bump();
        }
        while self.peek().is_ascii_digit() {
            self.bump();
        }
        if self.peek() == '.' {
            self.bump();
            while self.peek().is_ascii_digit() {
                self.bump();
            }
        }
        if self.peek() == 'e' || self.peek() == 'E' {
            self.bump();
            if self.peek() == '+' || self.peek() == '-' {
                self.bump();
            }
            while self.peek().is_ascii_digit() {
                self.bump();
            }
        }
        let s: String = self.c[start..self.i].iter().collect();
        s.parse::<f64>()
            .map(Json::Num)
            .map_err(|_| format!("bad number '{}'", s))
    }
    fn starts_with(&self, s: &str) -> bool {
        let want: Vec<char> = s.chars().collect();
        if self.i + want.len() > self.c.len() {
            return false;
        }
        self.c[self.i..self.i + want.len()] == want[..]
    }
}

// ===========================================================================
// JSON serialization (hand-rolled, matching lint.rs's json string escaping).
// ===========================================================================

/// Escape a string into a JSON string literal (with surrounding quotes). Same
/// rules as `lint::json_string` (which is private to that module); kept here so
/// the LSP stays self-contained and dependency-free.
pub fn json_string(s: &str) -> String {
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

impl Json {
    /// Serialize this value back to a compact JSON string.
    pub fn to_string(&self) -> String {
        match self {
            Json::Null => "null".to_string(),
            Json::Bool(b) => b.to_string(),
            Json::Num(n) => {
                // Emit integers without a trailing `.0` (LSP ids/positions are ints).
                if n.fract() == 0.0 && n.is_finite() {
                    format!("{}", *n as i64)
                } else {
                    format!("{}", n)
                }
            }
            Json::Str(s) => json_string(s),
            Json::Arr(a) => {
                let body = a.iter().map(|v| v.to_string()).collect::<Vec<_>>().join(",");
                format!("[{}]", body)
            }
            Json::Obj(m) => {
                // Sort keys for deterministic output (tests + readability).
                let mut keys: Vec<&String> = m.keys().collect();
                keys.sort();
                let body = keys
                    .iter()
                    .map(|k| format!("{}:{}", json_string(k), m[*k].to_string()))
                    .collect::<Vec<_>>()
                    .join(",");
                format!("{{{}}}", body)
            }
        }
    }
}

/// Tiny builder helpers so handler code reads like the wire shape.
fn obj(pairs: Vec<(&str, Json)>) -> Json {
    let mut m = HashMap::new();
    for (k, v) in pairs {
        m.insert(k.to_string(), v);
    }
    Json::Obj(m)
}
fn num(n: i64) -> Json {
    Json::Num(n as f64)
}
fn s(v: &str) -> Json {
    Json::Str(v.to_string())
}

// ===========================================================================
// Content-Length framing (the LSP transport).
// ===========================================================================

/// Read one Content-Length-framed JSON-RPC message from `r`. Returns `Ok(None)`
/// at clean end-of-stream (no more headers), `Ok(Some(body))` with the raw JSON
/// body string, or `Err` on a malformed frame / IO error.
///
/// We parse the header block line-by-line (CRLF-terminated per LSP), read the
/// `Content-Length`, then read exactly that many **bytes** of body.
pub fn read_message<R: BufRead>(r: &mut R) -> Result<Option<String>, String> {
    let mut content_length: Option<usize> = None;
    loop {
        let mut line = String::new();
        let n = r.read_line(&mut line).map_err(|e| e.to_string())?;
        if n == 0 {
            // EOF before any header -> clean end of stream.
            return Ok(None);
        }
        let trimmed = line.trim_end_matches(['\r', '\n']);
        if trimmed.is_empty() {
            // Blank line: end of headers.
            break;
        }
        if let Some(rest) = trimmed
            .strip_prefix("Content-Length:")
            .or_else(|| trimmed.strip_prefix("content-length:"))
        {
            content_length = Some(
                rest.trim()
                    .parse::<usize>()
                    .map_err(|_| format!("bad Content-Length: {:?}", rest))?,
            );
        }
        // Other headers (Content-Type) are ignored.
    }
    let len = content_length.ok_or("missing Content-Length header")?;
    let mut buf = vec![0u8; len];
    read_exact(r, &mut buf)?;
    String::from_utf8(buf).map_err(|e| e.to_string()).map(Some)
}

/// Read exactly `buf.len()` bytes, erroring on premature EOF. `BufRead` is also
/// `Read`, but we go through the buffered reader so a partially-consumed header
/// buffer is drained first.
fn read_exact<R: BufRead>(r: &mut R, buf: &mut [u8]) -> Result<(), String> {
    // `BufRead: Read`, so `read_exact` is in scope via the `Write`/`Read` prelude
    // import at the top of this module.
    r.read_exact(buf).map_err(|e| e.to_string())
}

/// Write one Content-Length-framed JSON-RPC message (body is the JSON text).
pub fn write_message<W: Write>(w: &mut W, body: &str) -> Result<(), String> {
    let bytes = body.as_bytes();
    write!(w, "Content-Length: {}\r\n\r\n", bytes.len()).map_err(|e| e.to_string())?;
    w.write_all(bytes).map_err(|e| e.to_string())?;
    w.flush().map_err(|e| e.to_string())?;
    Ok(())
}

// ===========================================================================
// Position conversion: R1/R2 (1-based) -> LSP (0-based).
// ===========================================================================

/// Convert a 1-based (line, col) — the R1/R2 convention, where `0` means "no
/// position" — to a 0-based LSP `character`/`line`. We subtract 1, clamping at 0
/// so a zeroed (no-span) diagnostic maps to the start of the document rather
/// than underflowing.
pub fn to_lsp_pos(one_based: u32) -> u32 {
    one_based.saturating_sub(1)
}

/// Build an LSP `Range` object from a 1-based R2 [`JsonDiagnostic`]'s positions.
fn lsp_range(d: &JsonDiagnostic) -> Json {
    let start = obj(vec![
        ("line", num(to_lsp_pos(d.line) as i64)),
        ("character", num(to_lsp_pos(d.col) as i64)),
    ]);
    // A zeroed end (no precise span) collapses to the start so the range is valid.
    let (el, ec) = if d.end_line == 0 && d.end_col == 0 {
        (to_lsp_pos(d.line), to_lsp_pos(d.col))
    } else {
        (to_lsp_pos(d.end_line), to_lsp_pos(d.end_col))
    };
    let end = obj(vec![
        ("line", num(el as i64)),
        ("character", num(ec as i64)),
    ]);
    obj(vec![("start", start), ("end", end)])
}

/// Map an R2 [`Severity`] to the LSP `DiagnosticSeverity` enum (1=Error, 2=Warning).
fn lsp_severity(sev: Severity) -> i64 {
    match sev {
        Severity::Error => 1,
        Severity::Warning => 2,
    }
}

/// Convert one R2 [`JsonDiagnostic`] to an LSP `Diagnostic` object.
fn lsp_diagnostic(d: &JsonDiagnostic) -> Json {
    obj(vec![
        ("range", lsp_range(d)),
        ("severity", num(lsp_severity(d.severity))),
        ("code", s(&d.rule)),
        ("source", s("motoview")),
        ("message", s(&d.message)),
    ])
}

// ===========================================================================
// FileKind inference + diagnostics for a buffer.
// ===========================================================================

/// Decide the `.mview` [`FileKind`] from the document URI/path. Mirrors the
/// directory convention the CLI uses (`compile`/`project`): `Layouts/` =>
/// Layout, `Components/` => Component, anything else => Page.
pub fn kind_from_uri(uri: &str) -> FileKind {
    if uri.contains("Layouts") {
        FileKind::Layout
    } else if uri.contains("Components") {
        FileKind::Component
    } else {
        FileKind::Page
    }
}

/// Derive a module-ish name + display path from a `file://` URI (or a bare path).
/// The name is only used by the parser for diagnostics; the path is what we put
/// in the diagnostic `file` field.
fn name_and_path(uri: &str) -> (String, String) {
    let path = uri.strip_prefix("file://").unwrap_or(uri).to_string();
    let stem = path
        .rsplit('/')
        .next()
        .and_then(|f| f.strip_suffix(".mview").or(Some(f)))
        .unwrap_or("Page")
        .to_string();
    (stem, path)
}

/// The result of linting a buffer: the LSP `Diagnostic` array (already converted
/// to 0-based), ready to drop into a `publishDiagnostics` notification.
///
/// NOTE: this is **lint only** — the structural security pass (`secure-form`,
/// `raw-html`) that needs no Motoko type info. `moc --check` against an in-memory
/// unsaved buffer is a deliberate follow-up (it would require staging the buffer
/// and assembling the whole project per keystroke). A parse error is itself
/// surfaced as a diagnostic so the editor shows *something* on broken syntax.
pub fn diagnostics_for(uri: &str, text: &str) -> Vec<Json> {
    let kind = kind_from_uri(uri);
    let (name, path) = name_and_path(uri);
    match parser::parse(text, &name, kind) {
        Ok(file) => {
            let chars: Vec<char> = text.chars().collect();
            lint::lint_file(&file, &path)
                .iter()
                .map(|d| JsonDiagnostic::from_lint(&path, d, &chars))
                .map(|jd| lsp_diagnostic(&jd))
                .collect()
        }
        Err(e) => {
            // R11: parse failure -> one diagnostic at the parser's REAL offset
            // (mapped to a 0-based LSP line/char), falling back to (0,0) only when
            // the error carries no position. The message carries the parser detail.
            let (line0, char0) = match e.offset {
                Some(off) => {
                    let chars: Vec<char> = text.chars().collect();
                    let (l, c) = span::line_col(&chars, off);
                    // line_col is 1-based; LSP positions are 0-based.
                    (l.saturating_sub(1), c.saturating_sub(1))
                }
                None => (0, 0),
            };
            let range = obj(vec![
                (
                    "start",
                    obj(vec![("line", num(line0 as i64)), ("character", num(char0 as i64))]),
                ),
                (
                    "end",
                    obj(vec![("line", num(line0 as i64)), ("character", num(char0 as i64))]),
                ),
            ]);
            vec![obj(vec![
                ("range", range),
                ("severity", num(1)),
                ("code", s("parse-error")),
                ("source", s("motoview")),
                ("message", s(&format!("parse error: {}", e))),
            ])]
        }
    }
}

// ===========================================================================
// Completion: template/directive completions + the Motoko delegation seam.
// ===========================================================================

/// A completion item we offer (label + a short detail + an LSP `kind`).
/// LSP CompletionItemKind: 14 = Keyword, 7 = Class, 10 = Property, 25 = TypeParameter.
struct Completion {
    label: &'static str,
    detail: &'static str,
    kind: i64,
}

/// The template-level directives a `.mview` author can type (outside `@code`).
/// These are the REAL completions the server returns — they mirror the directives
/// the parser/codegen actually recognize.
const DIRECTIVES: &[Completion] = &[
    Completion { label: "@page", detail: "declare a page route", kind: 14 },
    Completion { label: "@layout", detail: "use a named layout", kind: 14 },
    Completion { label: "@title", detail: "page <title>", kind: 14 },
    Completion { label: "@description", detail: "page meta description", kind: 14 },
    Completion { label: "@canonical", detail: "canonical URL", kind: 14 },
    Completion { label: "@authorize", detail: "gate the page by role", kind: 14 },
    Completion { label: "@cacheable", detail: "serve as a certified query", kind: 14 },
    Completion { label: "@theme", detail: "apply a theme preset / brand ramp", kind: 14 },
    Completion { label: "@code", detail: "Motoko code block", kind: 14 },
    Completion { label: "@if", detail: "conditional block", kind: 14 },
    Completion { label: "@elseif", detail: "else-if branch", kind: 14 },
    Completion { label: "@else", detail: "else branch", kind: 14 },
    Completion { label: "@for", detail: "iterate a collection", kind: 14 },
    Completion { label: "@switch", detail: "pattern-match block", kind: 14 },
    Completion { label: "@case", detail: "switch case", kind: 14 },
    Completion { label: "@expr", detail: "escaped expression output", kind: 14 },
    Completion { label: "@raw", detail: "TRUSTED unescaped HTML (XSS sink)", kind: 14 },
    Completion { label: "@yield", detail: "layout slot for the page body", kind: 14 },
    Completion { label: "@head", detail: "layout <head> injection point", kind: 14 },
];

/// Element / attribute keywords commonly typed inside the template markup. The
/// security-critical `secure` keyword is here so authors discover it (it's what
/// the `secure-form` lint demands).
const TEMPLATE_KEYWORDS: &[Completion] = &[
    Completion { label: "secure", detail: "mark a mutating <form> CSRF-safe (required)", kind: 14 },
    Completion { label: "@submit", detail: "form submit handler", kind: 10 },
    Completion { label: "@click", detail: "click handler", kind: 10 },
    Completion { label: "@input", detail: "input handler", kind: 10 },
    Completion { label: "@change", detail: "change handler", kind: 10 },
    Completion { label: "bind", detail: "two-way bind to a Motoko lvalue", kind: 10 },
    Completion { label: "form", detail: "<form> element", kind: 7 },
    Completion { label: "button", detail: "<button> element", kind: 7 },
    Completion { label: "input", detail: "<input> element", kind: 7 },
    Completion { label: "div", detail: "<div> element", kind: 7 },
];

/// Find the byte/char offset in `text` corresponding to a 0-based LSP
/// (line, character). Clamps past-EOF to the document end.
fn offset_of(text: &str, line: u32, character: u32) -> usize {
    let mut cur_line = 0u32;
    let mut cur_col = 0u32;
    for (idx, ch) in text.chars().enumerate() {
        if cur_line == line && cur_col == character {
            return idx;
        }
        if ch == '\n' {
            cur_line += 1;
            cur_col = 0;
        } else {
            cur_col += 1;
        }
    }
    text.chars().count()
}

/// Whether the cursor at `offset` sits inside a `@code { ... }` block. We scan
/// for the last `@code` before the cursor and check brace depth. This is the
/// **delegation seam**: inside `@code`, Motoko completion belongs to the WASM
/// Motoko language server, not to us.
pub fn cursor_in_code(text: &str, offset: usize) -> bool {
    let chars: Vec<char> = text.chars().collect();
    let upto = &chars[..offset.min(chars.len())];
    // Find the start of the most recent `@code` keyword before the cursor.
    let s: String = upto.iter().collect();
    let code_pos = match s.rfind("@code") {
        Some(p) => p,
        None => return false,
    };
    // Count braces from just after `@code` to the cursor. Inside the block when
    // we've seen an opening `{` and depth is still > 0.
    let mut depth = 0i32;
    let mut seen_open = false;
    for ch in s[code_pos..].chars() {
        match ch {
            '{' => {
                depth += 1;
                seen_open = true;
            }
            '}' => depth -= 1,
            _ => {}
        }
    }
    seen_open && depth > 0
}

/// Build the completion list for a position WITHOUT a URI (template/directive
/// completion only; the in-`@code` path yields just the delegation marker because
/// there is no project to bind against). Kept for callers/tests that only have the
/// buffer text. Prefer [`completion_at_uri`] from the server so in-`@code`
/// completion is **backend-bound**.
pub fn completion_at(text: &str, line: u32, character: u32) -> Json {
    completion_at_uri("", text, line, character)
}

/// Build the completion list for a position, BOUND TO THE PROJECT BACKEND (R10).
///
/// Outside `@code`: template/directive completion (R3, unchanged).
///
/// Inside `@code`: the differentiator. We still emit the Motoko delegation marker
/// (the WASM Motoko LS owns general Motoko completion), but we ALSO offer completion
/// items for the PROJECT'S OWN service surface — every `public func/type/let` of
/// every `src/Services/*.mo` stateful service, extracted in-process by
/// [`crate::services`]. Each item's label is the decl name, its detail is the real
/// signature, and its kind reflects func/type/let. This is the "no client/server
/// drift" property: the palette IS the backend. The project root is found from
/// `uri`; if there is no project (empty uri / no services), only the delegation
/// marker is returned — we never fabricate symbols.
pub fn completion_at_uri(uri: &str, text: &str, line: u32, character: u32) -> Json {
    let offset = offset_of(text, line, character);
    if cursor_in_code(text, offset) {
        let mut items = Vec::new();
        // DELEGATION SEAM: do not invent *general* Motoko completions. Surface a
        // marker item so a client/editor can route to the WASM Motoko LS.
        items.push(obj(vec![
            ("label", s("(Motoko — delegated)")),
            ("kind", num(1)),
            (
                "detail",
                s("general @code completion is provided by the Motoko language server"),
            ),
            (
                "documentation",
                s("MotoView delegates in-@code Motoko completion to the WASM \
                   Motoko LS (vscode-motoko). This is the integration seam."),
            ),
            // A data marker the client can branch on to trigger the delegated LS.
            ("data", obj(vec![("delegate", s("motoko"))])),
        ]));
        // BACKEND-BOUND: append the project's real service surface so the author can
        // only call functions/types that actually EXIST, with the types they have.
        for d in backend_surface(uri) {
            items.push(service_completion_item(&d));
        }
        return obj(vec![
            ("isIncomplete", Json::Bool(true)),
            ("items", Json::Arr(items)),
        ]);
    }
    let mut items = Vec::new();
    for c in DIRECTIVES.iter().chain(TEMPLATE_KEYWORDS.iter()) {
        items.push(obj(vec![
            ("label", s(c.label)),
            ("kind", num(c.kind)),
            ("detail", s(c.detail)),
        ]));
    }
    obj(vec![
        ("isIncomplete", Json::Bool(false)),
        ("items", Json::Arr(items)),
    ])
}

/// Extract the project's service surface for a document URI. Empty when there is no
/// resolvable project root or no stateful services. Pure pass-through to
/// [`crate::services`] so the LSP stays dependency-free.
fn backend_surface(uri: &str) -> Vec<crate::services::Decl> {
    if uri.is_empty() {
        return Vec::new();
    }
    match crate::services::project_root_for(uri) {
        Some(root) => crate::services::project_decls(&root),
        None => Vec::new(),
    }
}

/// Build an LSP `CompletionItem` for one service declaration. label = the decl
/// name, detail = `<Service>.<kw> <signature>` (so the palette reads as the real
/// backend surface), kind = Function/Struct/Constant. The `data.source` marker lets
/// a client tell these backend-bound items apart from the delegation marker.
fn service_completion_item(d: &crate::services::Decl) -> Json {
    let detail = format!("{}.{} {}", d.service, d.kind.keyword(), d.signature);
    obj(vec![
        ("label", s(&d.name)),
        ("kind", num(d.kind.lsp_kind())),
        ("detail", s(&detail)),
        (
            "documentation",
            s(&format!(
                "From service `{}` (src/Services/{}.mo). Backend-bound: this symbol \
                 is part of the project's real Motoko service surface.",
                d.service, d.service
            )),
        ),
        // The author wants the call, not the full signature, inserted.
        ("insertText", s(&d.name)),
        ("data", obj(vec![("source", s("motoview-service"))])),
    ])
}

// ===========================================================================
// Code actions (quick-fixes): turn a `secure-form` diagnostic into a one-click
// edit that adds `secure` to the offending <form> open tag.
// ===========================================================================

/// A single text edit: a 0-based LSP `Range` plus the replacement `newText`.
/// For an INSERTION the range is zero-width (`start == end`).
#[derive(Debug, Clone, PartialEq)]
pub struct TextEdit {
    pub start_line: u32,
    pub start_char: u32,
    pub end_line: u32,
    pub end_char: u32,
    pub new_text: String,
}

impl TextEdit {
    fn to_json(&self) -> Json {
        let range = obj(vec![
            (
                "start",
                obj(vec![
                    ("line", num(self.start_line as i64)),
                    ("character", num(self.start_char as i64)),
                ]),
            ),
            (
                "end",
                obj(vec![
                    ("line", num(self.end_line as i64)),
                    ("character", num(self.end_char as i64)),
                ]),
            ),
        ]);
        obj(vec![("range", range), ("newText", s(&self.new_text))])
    }
}

/// Compute the edit that adds ` secure` to a `<form>` open tag, given the form's
/// open-tag span (the R1 char-offset span carried on the `secure-form`
/// diagnostic). The new keyword is inserted just BEFORE the closing `>` (or `/>`)
/// of the open tag, so the result is a well-formed `<form ... secure>`.
///
/// Why just-before-`>`: the span end is one-past the `>`; inserting ` secure`
/// there is the minimal change that the parser reads as a boolean `secure` attr
/// (see parser.rs: a bare `secure` attribute sets `Element::secure = true`), which
/// is exactly what clears the lint. Returns `None` if the span does not actually
/// end with `>`/`/>` (defensive — never emit a malformed edit).
pub fn secure_form_edit(text: &str, span: Span) -> Option<TextEdit> {
    let chars: Vec<char> = text.chars().collect();
    let end = span.end.min(chars.len());
    if end == 0 {
        return None;
    }
    // The open tag ends with `>` at end-1, possibly preceded by `/` for `/>`.
    if chars[end - 1] != '>' {
        return None;
    }
    let mut insert_at = end - 1; // before the `>`
    if insert_at > 0 && chars[insert_at - 1] == '/' {
        insert_at -= 1; // before the `/` of a `/>` (a <form/> is unusual but be safe)
    }
    let (line, col) = span::line_col(&chars, insert_at);
    let pos_line = to_lsp_pos(line);
    let pos_char = to_lsp_pos(col);
    Some(TextEdit {
        start_line: pos_line,
        start_char: pos_char,
        end_line: pos_line,
        end_char: pos_char,
        // Leading space so it never abuts the previous token (`...="save" secure>`).
        new_text: " secure".to_string(),
    })
}

/// Apply a single [`TextEdit`] to `text` (used by tests to prove the quick-fix
/// produces a buffer that lints clean). Operates on the same 0-based line/char
/// model the editor uses.
pub fn apply_edit(text: &str, edit: &TextEdit) -> String {
    // Translate the 0-based (line, char) start back to a char offset, insert, done.
    // Only insertions (zero-width range) are produced by `secure_form_edit`, but
    // this handles a general single-range replacement for completeness.
    let chars: Vec<char> = text.chars().collect();
    let start = lc_to_offset(&chars, edit.start_line, edit.start_char);
    let end = lc_to_offset(&chars, edit.end_line, edit.end_char);
    let mut out: String = chars[..start.min(chars.len())].iter().collect();
    out.push_str(&edit.new_text);
    out.extend(chars[end.min(chars.len())..].iter());
    out
}

/// Map a 0-based (line, character) LSP position to a char offset into `chars`.
fn lc_to_offset(chars: &[char], line: u32, character: u32) -> usize {
    let mut cur_line = 0u32;
    let mut i = 0usize;
    // Advance to the start of the target line.
    while i < chars.len() && cur_line < line {
        if chars[i] == '\n' {
            cur_line += 1;
        }
        i += 1;
    }
    // Then advance `character` columns (without crossing a newline).
    let mut col = 0u32;
    while i < chars.len() && col < character && chars[i] != '\n' {
        i += 1;
        col += 1;
    }
    i
}

/// Build the `textDocument/codeAction` result array for a buffer + requested
/// range. Re-lints the buffer (the SAME pass the editor squiggles come from),
/// finds every `secure-form` Error whose span overlaps the requested range, and
/// emits a `quickfix` code action carrying a `WorkspaceEdit` that inserts `secure`.
///
/// The action's edit is also tied to the diagnostic (the `diagnostics` field), so
/// the client can show "Fix" next to the squiggle. Returns an empty array when no
/// fixable diagnostic overlaps the range (the LSP-correct "no actions" reply).
pub fn code_actions_for(uri: &str, text: &str, range: Option<(u32, u32, u32, u32)>) -> Vec<Json> {
    let kind = kind_from_uri(uri);
    let (name, path) = name_and_path(uri);
    let file = match parser::parse(text, &name, kind) {
        Ok(f) => f,
        Err(_) => return Vec::new(), // can't offer fixes on unparseable text
    };
    let chars: Vec<char> = text.chars().collect();
    let mut actions = Vec::new();
    for d in lint::lint_file(&file, &path) {
        if d.rule != "secure-form" {
            continue;
        }
        let span = match d.span {
            Some(sp) => sp,
            None => continue,
        };
        // If the client passed a range, only offer the fix when the diagnostic
        // overlaps it (so right-clicking elsewhere doesn't surface the action).
        if let Some((sl, sc, el, ec)) = range {
            let jd = JsonDiagnostic::from_lint(&path, &d, &chars);
            let dsl = to_lsp_pos(jd.line);
            let dsc = to_lsp_pos(jd.col);
            let del = to_lsp_pos(jd.end_line);
            let dec = to_lsp_pos(jd.end_col);
            if !ranges_overlap((dsl, dsc, del, dec), (sl, sc, el, ec)) {
                continue;
            }
        }
        let edit = match secure_form_edit(text, span) {
            Some(e) => e,
            None => continue,
        };
        let jd = JsonDiagnostic::from_lint(&path, &d, &chars);
        let diag_json = lsp_diagnostic(&jd);
        // WorkspaceEdit: { changes: { <uri>: [TextEdit] } }
        let workspace_edit = obj(vec![(
            "changes",
            obj(vec![(uri, Json::Arr(vec![edit.to_json()]))]),
        )]);
        actions.push(obj(vec![
            ("title", s("Add `secure` to this form")),
            ("kind", s("quickfix")),
            ("diagnostics", Json::Arr(vec![diag_json])),
            // Mark as the preferred fix so editors can apply it with one keystroke.
            ("isPreferred", Json::Bool(true)),
            ("edit", workspace_edit),
        ]));
    }
    actions
}

/// Whether two 0-based (line, char, end_line, end_char) ranges overlap. A
/// zero-width range counts as overlapping when it sits within the other.
fn ranges_overlap(a: (u32, u32, u32, u32), b: (u32, u32, u32, u32)) -> bool {
    // Compare as (line, char) lexicographic positions: a.start <= b.end && b.start <= a.end.
    let le = |p: (u32, u32), q: (u32, u32)| p.0 < q.0 || (p.0 == q.0 && p.1 <= q.1);
    let a_start = (a.0, a.1);
    let a_end = (a.2, a.3);
    let b_start = (b.0, b.1);
    let b_end = (b.2, b.3);
    le(a_start, b_end) && le(b_start, a_end)
}

// ===========================================================================
// The server: per-uri buffer store + request handlers.
// ===========================================================================

/// The in-memory `.mview` language server. Holds the latest full text per open
/// document (full-sync model) and turns JSON-RPC requests into responses +
/// notifications. Pure logic — no IO — so it is directly unit-testable.
pub struct LspServer {
    /// uri -> latest full buffer text.
    docs: HashMap<String, String>,
    /// Set once `initialize` has been handled (LSP requires it first).
    initialized: bool,
    /// Set on `shutdown`; the loop exits on the following `exit`.
    shutting_down: bool,
}

/// What a handled message produced: zero or more outgoing JSON messages
/// (responses and/or notifications), plus whether the loop should stop.
#[derive(Default)]
pub struct Reply {
    /// Outgoing JSON-RPC messages to frame + write, in order.
    pub messages: Vec<Json>,
    /// True when the server should exit the read loop (after `exit`).
    pub exit: bool,
}

impl LspServer {
    pub fn new() -> Self {
        LspServer {
            docs: HashMap::new(),
            initialized: false,
            shutting_down: false,
        }
    }

    /// The current text of an open document, if any (test/observability hook).
    #[allow(dead_code)]
    pub fn document_text(&self, uri: &str) -> Option<&str> {
        self.docs.get(uri).map(|s| s.as_str())
    }

    /// Dispatch one parsed JSON-RPC message and return the outgoing messages.
    /// `msg` is the already-parsed request/notification body.
    pub fn handle(&mut self, msg: &Json) -> Reply {
        let method = msg.get("method").and_then(|m| m.as_str()).unwrap_or("");
        let id = msg.get("id").cloned();
        match method {
            "initialize" => self.on_initialize(id),
            "initialized" => Reply::default(), // notification, no reply
            "shutdown" => {
                self.shutting_down = true;
                Reply {
                    messages: vec![response(id, Json::Null)],
                    exit: false,
                }
            }
            "exit" => Reply {
                messages: vec![],
                exit: true,
            },
            "textDocument/didOpen" => self.on_did_open(msg),
            "textDocument/didChange" => self.on_did_change(msg),
            "textDocument/didClose" => self.on_did_close(msg),
            "textDocument/completion" => self.on_completion(msg, id),
            "textDocument/codeAction" => self.on_code_action(msg, id),
            // Unknown request (has an id) -> MethodNotFound error so clients don't
            // hang. Unknown notification (no id) -> silently ignored per LSP.
            _ => {
                if let Some(id) = id {
                    Reply {
                        messages: vec![error_response(
                            Some(id),
                            -32601,
                            &format!("method not found: {}", method),
                        )],
                        exit: false,
                    }
                } else {
                    Reply::default()
                }
            }
        }
    }

    fn on_initialize(&mut self, id: Option<Json>) -> Reply {
        self.initialized = true;
        let capabilities = obj(vec![
            // 1 = full document sync (we keep the whole buffer text per change).
            ("textDocumentSync", num(1)),
            (
                "completionProvider",
                obj(vec![(
                    "triggerCharacters",
                    Json::Arr(vec![s("@"), s("<"), s(" ")]),
                )]),
            ),
            // We PUSH diagnostics via publishDiagnostics on open/change. Advertise
            // it so a client knows diagnostics are server-driven.
            (
                "diagnosticProvider",
                obj(vec![
                    ("interFileDependencies", Json::Bool(false)),
                    ("workspaceDiagnostics", Json::Bool(false)),
                ]),
            ),
            // Quick-fixes: the `secure-form` -> "Add `secure`" code action. Advertise
            // the `quickfix` kind so clients show the lightbulb on the squiggle.
            (
                "codeActionProvider",
                obj(vec![("codeActionKinds", Json::Arr(vec![s("quickfix")]))]),
            ),
        ]);
        let result = obj(vec![
            ("capabilities", capabilities),
            (
                "serverInfo",
                obj(vec![("name", s("motoview-lsp")), ("version", s(env!("CARGO_PKG_VERSION")))]),
            ),
        ]);
        Reply {
            messages: vec![response(id, result)],
            exit: false,
        }
    }

    fn on_did_open(&mut self, msg: &Json) -> Reply {
        let td = msg.path("params.textDocument");
        let uri = td.and_then(|t| t.get("uri")).and_then(|u| u.as_str());
        let text = td.and_then(|t| t.get("text")).and_then(|t| t.as_str());
        if let (Some(uri), Some(text)) = (uri, text) {
            self.docs.insert(uri.to_string(), text.to_string());
            return Reply {
                messages: vec![self.publish(uri)],
                exit: false,
            };
        }
        Reply::default()
    }

    fn on_did_change(&mut self, msg: &Json) -> Reply {
        let uri = msg
            .path("params.textDocument.uri")
            .and_then(|u| u.as_str());
        // Full-sync: the last content change's `text` is the entire new buffer.
        let changes = msg
            .path("params.contentChanges")
            .and_then(|c| c.as_array());
        let new_text = changes
            .and_then(|cs| cs.last())
            .and_then(|c| c.get("text"))
            .and_then(|t| t.as_str());
        if let (Some(uri), Some(text)) = (uri, new_text) {
            self.docs.insert(uri.to_string(), text.to_string());
            return Reply {
                messages: vec![self.publish(uri)],
                exit: false,
            };
        }
        Reply::default()
    }

    fn on_did_close(&mut self, msg: &Json) -> Reply {
        if let Some(uri) = msg.path("params.textDocument.uri").and_then(|u| u.as_str()) {
            self.docs.remove(uri);
            // Clear diagnostics for the closed doc (publish an empty array).
            return Reply {
                messages: vec![publish_notification(uri, vec![])],
                exit: false,
            };
        }
        Reply::default()
    }

    fn on_completion(&mut self, msg: &Json, id: Option<Json>) -> Reply {
        let uri = msg
            .path("params.textDocument.uri")
            .and_then(|u| u.as_str())
            .unwrap_or("");
        let line = msg
            .path("params.position.line")
            .and_then(|n| n.as_f64())
            .unwrap_or(0.0) as u32;
        let character = msg
            .path("params.position.character")
            .and_then(|n| n.as_f64())
            .unwrap_or(0.0) as u32;
        let text = self.docs.get(uri).cloned().unwrap_or_default();
        // BACKEND-BOUND completion: pass the uri so in-@code completion can bind to
        // the project's real service surface (no client/server drift).
        let result = completion_at_uri(uri, &text, line, character);
        Reply {
            messages: vec![response(id, result)],
            exit: false,
        }
    }

    /// Handle `textDocument/codeAction`: return the quick-fix actions available for
    /// the requested range (currently the `secure-form` -> add-`secure` fix). The
    /// result is a `CodeAction[]`; an empty array means "no actions here".
    fn on_code_action(&mut self, msg: &Json, id: Option<Json>) -> Reply {
        let uri = msg
            .path("params.textDocument.uri")
            .and_then(|u| u.as_str())
            .unwrap_or("")
            .to_string();
        let text = self.docs.get(&uri).cloned().unwrap_or_default();
        // The requested range (0-based). Absent -> offer fixes for the whole buffer.
        let range = msg.path("params.range").map(|r| {
            let g = |dotted: &str| {
                r.path(dotted).and_then(|n| n.as_f64()).unwrap_or(0.0) as u32
            };
            (
                g("start.line"),
                g("start.character"),
                g("end.line"),
                g("end.character"),
            )
        });
        let actions = code_actions_for(&uri, &text, range);
        Reply {
            messages: vec![response(id, Json::Arr(actions))],
            exit: false,
        }
    }

    /// Build a `publishDiagnostics` notification for `uri` from its current text.
    fn publish(&self, uri: &str) -> Json {
        let text = self.docs.get(uri).cloned().unwrap_or_default();
        let diags = diagnostics_for(uri, &text);
        publish_notification(uri, diags)
    }
}

impl Default for LspServer {
    fn default() -> Self {
        Self::new()
    }
}

/// Build a JSON-RPC success response with the given id + result.
fn response(id: Option<Json>, result: Json) -> Json {
    obj(vec![
        ("jsonrpc", s("2.0")),
        ("id", id.unwrap_or(Json::Null)),
        ("result", result),
    ])
}

/// Build a JSON-RPC error response.
fn error_response(id: Option<Json>, code: i64, message: &str) -> Json {
    obj(vec![
        ("jsonrpc", s("2.0")),
        ("id", id.unwrap_or(Json::Null)),
        (
            "error",
            obj(vec![("code", num(code)), ("message", s(message))]),
        ),
    ])
}

/// Build a `textDocument/publishDiagnostics` notification.
fn publish_notification(uri: &str, diagnostics: Vec<Json>) -> Json {
    obj(vec![
        ("jsonrpc", s("2.0")),
        ("method", s("textDocument/publishDiagnostics")),
        (
            "params",
            obj(vec![
                ("uri", s(uri)),
                ("diagnostics", Json::Arr(diagnostics)),
            ]),
        ),
    ])
}

// ===========================================================================
// The stdio loop (the `motoview lsp` entry point).
// ===========================================================================

/// Run the language server over the given reader/writer until `exit` (or EOF).
/// Generic so tests can drive it with in-memory byte buffers; `run_stdio` wires
/// it to real stdin/stdout.
pub fn serve<R: BufRead, W: Write>(reader: &mut R, writer: &mut W) -> Result<(), String> {
    let mut server = LspServer::new();
    loop {
        let body = match read_message(reader)? {
            Some(b) => b,
            None => break, // clean EOF
        };
        let msg = match parse_json(&body) {
            Ok(m) => m,
            Err(_) => continue, // skip malformed frame, keep serving
        };
        let reply = server.handle(&msg);
        for out in &reply.messages {
            write_message(writer, &out.to_string())?;
        }
        if reply.exit {
            break;
        }
    }
    Ok(())
}

/// `motoview lsp` — run the server loop on real stdin/stdout. Returns the process
/// exit code.
pub fn run_stdio() -> i32 {
    let stdin = std::io::stdin();
    let mut reader = stdin.lock();
    let stdout = std::io::stdout();
    let mut writer = stdout.lock();
    match serve(&mut reader, &mut writer) {
        Ok(()) => 0,
        Err(e) => {
            eprintln!("motoview lsp: {}", e);
            1
        }
    }
}

// ===========================================================================
// Tests.
// ===========================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    /// Frame a JSON body the way an LSP client would on the wire.
    fn frame(body: &str) -> Vec<u8> {
        format!("Content-Length: {}\r\n\r\n{}", body.as_bytes().len(), body).into_bytes()
    }

    #[test]
    fn one_based_to_zero_based_conversion() {
        // 1-based -> 0-based: subtract 1.
        assert_eq!(to_lsp_pos(1), 0);
        assert_eq!(to_lsp_pos(2), 1);
        assert_eq!(to_lsp_pos(42), 41);
        // 0 ("no position") clamps to 0 rather than underflowing.
        assert_eq!(to_lsp_pos(0), 0);
    }

    #[test]
    fn framing_writer_then_reader_round_trips() {
        let body = r#"{"jsonrpc":"2.0","id":1,"method":"ping"}"#;
        let mut buf: Vec<u8> = Vec::new();
        write_message(&mut buf, body).expect("write");
        // The framed bytes start with the Content-Length header.
        let framed = String::from_utf8(buf.clone()).unwrap();
        assert!(framed.starts_with("Content-Length: "), "{framed}");
        assert!(framed.contains("\r\n\r\n"), "{framed}");
        // And read_message recovers exactly the body.
        let mut cur = Cursor::new(buf);
        let got = read_message(&mut cur).expect("read").expect("some");
        assert_eq!(got, body);
        // A second read at EOF returns None (clean end).
        assert_eq!(read_message(&mut cur).expect("read2"), None);
    }

    #[test]
    fn reader_parses_multiple_back_to_back_frames() {
        let a = r#"{"a":1}"#;
        let b = r#"{"b":2}"#;
        let mut bytes = frame(a);
        bytes.extend(frame(b));
        let mut cur = Cursor::new(bytes);
        assert_eq!(read_message(&mut cur).unwrap().unwrap(), a);
        assert_eq!(read_message(&mut cur).unwrap().unwrap(), b);
        assert_eq!(read_message(&mut cur).unwrap(), None);
    }

    #[test]
    fn json_parser_handles_nested_objects_arrays_strings_numbers() {
        let v = parse_json(
            r#"{"id":1,"method":"x","params":{"a":[true,false,null,3.5],"s":"hi\n\"q\""}}"#,
        )
        .expect("parse");
        assert_eq!(v.path("id").and_then(|n| n.as_f64()), Some(1.0));
        assert_eq!(v.path("method").and_then(|s| s.as_str()), Some("x"));
        assert_eq!(
            v.path("params.s").and_then(|s| s.as_str()),
            Some("hi\n\"q\"")
        );
        let arr = v.path("params.a").and_then(|a| a.as_array()).unwrap();
        assert_eq!(arr.len(), 4);
        assert_eq!(arr[3].as_f64(), Some(3.5));
    }

    #[test]
    fn json_round_trip_object_is_deterministic() {
        // Keys are sorted on output, so two parses of the same doc serialize equal.
        let v = parse_json(r#"{"b":2,"a":1}"#).unwrap();
        assert_eq!(v.to_string(), r#"{"a":1,"b":2}"#);
    }

    #[test]
    fn kind_inference_from_uri() {
        assert!(matches!(
            kind_from_uri("file:///proj/src/Pages/Home.mview"),
            FileKind::Page
        ));
        assert!(matches!(
            kind_from_uri("file:///proj/src/Layouts/Main.mview"),
            FileKind::Layout
        ));
        assert!(matches!(
            kind_from_uri("file:///proj/src/Components/Card.mview"),
            FileKind::Component
        ));
    }

    #[test]
    fn diagnostics_for_insecure_form_flags_secure_form_at_zero_based_range() {
        // <form @submit> on line 2 (0-based line 1), col 1 (0-based char 0).
        let text = "@page \"/\"\n<form @submit=\"save\"><button>Go</button></form>\n@code { func save(ctx : Context) : async () { ignore ctx; }; }";
        let diags = diagnostics_for("file:///proj/src/Pages/T.mview", text);
        assert_eq!(diags.len(), 1, "expected exactly the secure-form diagnostic");
        let d = &diags[0];
        assert_eq!(d.path("code").and_then(|c| c.as_str()), Some("secure-form"));
        assert_eq!(d.path("severity").and_then(|n| n.as_f64()), Some(1.0)); // Error
        // 1-based (2,1) -> 0-based (1,0).
        assert_eq!(d.path("range.start.line").and_then(|n| n.as_f64()), Some(1.0));
        assert_eq!(
            d.path("range.start.character").and_then(|n| n.as_f64()),
            Some(0.0)
        );
        // The end is past the start on the same line (covers the open tag).
        assert_eq!(d.path("range.end.line").and_then(|n| n.as_f64()), Some(1.0));
        assert!(
            d.path("range.end.character").and_then(|n| n.as_f64()).unwrap() > 0.0
        );
    }

    #[test]
    fn diagnostics_for_clean_buffer_is_empty() {
        let text = "@page \"/\"\n<div>Hello</div>";
        let diags = diagnostics_for("file:///proj/src/Pages/T.mview", text);
        assert!(diags.is_empty(), "clean buffer should have no diagnostics: {:?}", diags);
    }

    #[test]
    fn diagnostics_for_parse_error_uses_the_real_offset_not_zero_zero() {
        // R11: an `@if` whose body brace is never closed -> a parse error. The LSP
        // must place the diagnostic at the parser's REAL offset (past line 1), not
        // the old (0,0) fallback. The failure is in the `@if` body on line 2+.
        let text = "@page \"/\"\n@if cond {\n  <p>x</p>\n";
        let diags = diagnostics_for("file:///proj/src/Pages/T.mview", text);
        assert_eq!(diags.len(), 1, "expected one parse-error diagnostic: {diags:?}");
        let d = &diags[0];
        assert_eq!(d.path("code").and_then(|c| c.as_str()), Some("parse-error"));
        // 0-based line; the real offset is past line 0 (the `@page` line).
        let line0 = d.path("range.start.line").and_then(|n| n.as_f64()).unwrap();
        assert!(line0 >= 1.0, "parse error must not anchor at line 0, got {line0}");
    }

    #[test]
    fn completion_outside_code_returns_template_directives() {
        let text = "@p";
        let list = completion_at(text, 0, 2);
        let items = list.path("items").and_then(|i| i.as_array()).unwrap();
        let labels: Vec<&str> = items
            .iter()
            .filter_map(|i| i.get("label").and_then(|l| l.as_str()))
            .collect();
        assert!(labels.contains(&"@page"), "{:?}", labels);
        assert!(labels.contains(&"@for"), "{:?}", labels);
        assert!(labels.contains(&"secure"), "{:?}", labels);
    }

    #[test]
    fn completion_inside_code_delegates_to_motoko() {
        // Cursor inside an open @code block.
        let text = "@page \"/\"\n@code {\n  let x = ";
        // line 2 (0-based), some column after `let x = `.
        let in_code = cursor_in_code(text, text.chars().count());
        assert!(in_code, "cursor should be detected inside @code");
        let list = completion_at(text, 2, 10);
        let items = list.path("items").and_then(|i| i.as_array()).unwrap();
        assert_eq!(items.len(), 1, "delegation marker only");
        let data = items[0].path("data.delegate").and_then(|d| d.as_str());
        assert_eq!(data, Some("motoko"), "must flag delegation, not fake Motoko");
    }

    #[test]
    fn completion_inside_code_is_backend_bound_to_project_services() {
        // R10 differentiator: open a .mview whose project has a real service, then
        // request completion INSIDE @code and assert the project's own service
        // surface (the `add` func + the `Note` type) is offered with the right
        // signature + kind. Drives the REAL on_completion handler via handle().
        use std::io::Write;
        // Stage a throwaway project on disk: <tmp>/src/Services/Notes.mo + dfx.json.
        let root = std::env::temp_dir().join(format!("mv-lsp-backend-{}", std::process::id()));
        let services = root.join("src").join("Services");
        std::fs::create_dir_all(&services).unwrap();
        std::fs::write(root.join("dfx.json"), "{}").unwrap();
        let mut f = std::fs::File::create(services.join("Notes.mo")).unwrap();
        write!(
            f,
            "module {{\n  public class Notes() {{\n    public type Note = {{ id : Nat; body : Text }};\n    public func add(body : Text) : Nat {{ 0 }};\n    func secret() : Nat {{ 1 }};\n  }};\n}}\n"
        )
        .unwrap();

        // The page lives under <root>/src/Pages so project_root_for resolves to <root>.
        let page = root.join("src").join("Pages").join("Notes.mview");
        let uri = format!("file://{}", page.display());
        let text = "@page \"/\"\n@code {\n  let n = ";

        let mut server = LspServer::new();
        // didOpen so the server has the buffer text.
        let open = parse_json(&format!(
            r#"{{"jsonrpc":"2.0","method":"textDocument/didOpen","params":{{"textDocument":{{"uri":{},"text":{}}}}}}}"#,
            json_string(&uri),
            json_string(text)
        ))
        .unwrap();
        server.handle(&open);
        // completion at a position inside the open @code block (line 2).
        let comp = parse_json(&format!(
            r#"{{"jsonrpc":"2.0","id":7,"method":"textDocument/completion","params":{{"textDocument":{{"uri":{}}},"position":{{"line":2,"character":10}}}}}}"#,
            json_string(&uri)
        ))
        .unwrap();
        let reply = server.handle(&comp);
        let result = reply.messages[0].path("result").expect("completion result");
        let items = result.path("items").and_then(|i| i.as_array()).expect("items");

        // Collect labels + (label -> detail) so we can assert names AND signatures.
        let labels: Vec<&str> = items
            .iter()
            .filter_map(|i| i.get("label").and_then(|l| l.as_str()))
            .collect();
        // The delegation marker is still present (general Motoko stays delegated).
        assert!(
            labels.iter().any(|l| l.contains("Motoko")),
            "delegation marker present: {:?}",
            labels
        );
        // The project's OWN service func is offered — no client/server drift.
        let add = items
            .iter()
            .find(|i| i.get("label").and_then(|l| l.as_str()) == Some("add"))
            .expect("the project's `add` func is in the palette");
        assert_eq!(
            add.path("detail").and_then(|d| d.as_str()),
            Some("Notes.func add(body : Text) : Nat"),
            "the palette carries the REAL signature"
        );
        // kind = Function (3).
        assert_eq!(add.path("kind").and_then(|n| n.as_f64()), Some(3.0));
        // The service's public type is offered too, as a Struct (22).
        let note = items
            .iter()
            .find(|i| i.get("label").and_then(|l| l.as_str()) == Some("Note"))
            .expect("the project's `Note` type is in the palette");
        assert_eq!(note.path("kind").and_then(|n| n.as_f64()), Some(22.0));
        assert!(note
            .path("detail")
            .and_then(|d| d.as_str())
            .unwrap()
            .contains("Note = {"));
        // A PRIVATE func never leaks into the backend-bound palette.
        assert!(!labels.contains(&"secret"), "private func leaked: {:?}", labels);

        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn cursor_in_code_false_before_and_after_block() {
        let text = "@page \"/\"\n@code { let x = 1; }\n<div></div>";
        // Before @code (in the @page line) -> not in code.
        assert!(!cursor_in_code(text, 3));
        // After the closing brace (in the <div>) -> not in code.
        assert!(!cursor_in_code(text, text.chars().count()));
    }

    /// THE protocol-level test: drive the server over real framed JSON-RPC stdio.
    /// initialize -> didOpen (INSECURE) -> expect publishDiagnostics with a
    /// secure-form error at the right 0-based range; didChange (CLEAN) -> expect
    /// publishDiagnostics with an EMPTY array; shutdown -> exit.
    #[test]
    fn protocol_initialize_open_insecure_then_change_clean() {
        let uri = "file:///proj/src/Pages/Login.mview";
        let insecure = "@page \"/\"\n<form @submit=\"save\"><button>Go</button></form>\n@code { func save(ctx : Context) : async () { ignore ctx; }; }";
        let clean = "@page \"/\"\n<div>Hello</div>";

        // Build the input stream of framed messages.
        let initialize = r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"capabilities":{}}}"#.to_string();
        let did_open = format!(
            r#"{{"jsonrpc":"2.0","method":"textDocument/didOpen","params":{{"textDocument":{{"uri":{},"languageId":"mview","version":1,"text":{}}}}}}}"#,
            json_string(uri),
            json_string(insecure)
        );
        let did_change = format!(
            r#"{{"jsonrpc":"2.0","method":"textDocument/didChange","params":{{"textDocument":{{"uri":{},"version":2}},"contentChanges":[{{"text":{}}}]}}}}"#,
            json_string(uri),
            json_string(clean)
        );
        let shutdown = r#"{"jsonrpc":"2.0","id":2,"method":"shutdown"}"#.to_string();
        let exit = r#"{"jsonrpc":"2.0","method":"exit"}"#.to_string();

        let mut input: Vec<u8> = Vec::new();
        for body in [&initialize, &did_open, &did_change, &shutdown, &exit] {
            input.extend(frame(body));
        }

        let mut reader = Cursor::new(input);
        let mut output: Vec<u8> = Vec::new();
        serve(&mut reader, &mut output).expect("serve");

        // Decode the output frames back into JSON values.
        let mut out_cur = Cursor::new(output);
        let mut messages: Vec<Json> = Vec::new();
        while let Some(body) = read_message(&mut out_cur).expect("read out") {
            messages.push(parse_json(&body).expect("parse out"));
        }

        // 1) initialize result with capabilities.
        let init_resp = messages
            .iter()
            .find(|m| m.path("id").and_then(|n| n.as_f64()) == Some(1.0))
            .expect("initialize response");
        assert!(
            init_resp.path("result.capabilities.textDocumentSync").is_some(),
            "capabilities present"
        );
        assert!(
            init_resp
                .path("result.capabilities.completionProvider")
                .is_some(),
            "completionProvider advertised"
        );

        // 2) the FIRST publishDiagnostics (from didOpen on the insecure buffer)
        //    has a secure-form error mapped to the <form> at 0-based (1, 0).
        let publishes: Vec<&Json> = messages
            .iter()
            .filter(|m| {
                m.path("method").and_then(|s| s.as_str())
                    == Some("textDocument/publishDiagnostics")
            })
            .collect();
        assert!(publishes.len() >= 2, "open + change each publish: {:?}", publishes);

        let first = publishes[0];
        assert_eq!(
            first.path("params.uri").and_then(|u| u.as_str()),
            Some(uri)
        );
        let diags = first
            .path("params.diagnostics")
            .and_then(|d| d.as_array())
            .expect("diagnostics array");
        assert_eq!(diags.len(), 1, "exactly the secure-form error");
        assert_eq!(
            diags[0].path("code").and_then(|c| c.as_str()),
            Some("secure-form")
        );
        assert_eq!(
            diags[0].path("severity").and_then(|n| n.as_f64()),
            Some(1.0)
        );
        // The squiggle is on the <form> open tag: 0-based line 1, char 0.
        assert_eq!(
            diags[0]
                .path("range.start.line")
                .and_then(|n| n.as_f64()),
            Some(1.0)
        );
        assert_eq!(
            diags[0]
                .path("range.start.character")
                .and_then(|n| n.as_f64()),
            Some(0.0)
        );

        // 3) the SECOND publishDiagnostics (from didChange to the clean buffer)
        //    is an EMPTY array — the squiggle is gone.
        let second = publishes[1];
        let diags2 = second
            .path("params.diagnostics")
            .and_then(|d| d.as_array())
            .expect("diagnostics array");
        assert!(diags2.is_empty(), "clean buffer clears diagnostics: {:?}", diags2);

        // 4) shutdown got a null result response.
        let shutdown_resp = messages
            .iter()
            .find(|m| m.path("id").and_then(|n| n.as_f64()) == Some(2.0))
            .expect("shutdown response");
        assert_eq!(shutdown_resp.path("result"), Some(&Json::Null));
    }

    #[test]
    fn unknown_request_gets_method_not_found() {
        let mut server = LspServer::new();
        let req = parse_json(r#"{"jsonrpc":"2.0","id":9,"method":"textDocument/foo"}"#).unwrap();
        let reply = server.handle(&req);
        assert_eq!(reply.messages.len(), 1);
        let err = &reply.messages[0];
        assert_eq!(err.path("error.code").and_then(|n| n.as_f64()), Some(-32601.0));
        assert_eq!(err.path("id").and_then(|n| n.as_f64()), Some(9.0));
    }

    #[test]
    fn did_close_clears_diagnostics() {
        let uri = "file:///proj/src/Pages/T.mview";
        let mut server = LspServer::new();
        let open = parse_json(&format!(
            r#"{{"jsonrpc":"2.0","method":"textDocument/didOpen","params":{{"textDocument":{{"uri":{},"text":{}}}}}}}"#,
            json_string(uri),
            json_string("@page \"/\"\n<div>ok</div>")
        ))
        .unwrap();
        server.handle(&open);
        assert!(server.document_text(uri).is_some());
        let close = parse_json(&format!(
            r#"{{"jsonrpc":"2.0","method":"textDocument/didClose","params":{{"textDocument":{{"uri":{}}}}}}}"#,
            json_string(uri)
        ))
        .unwrap();
        let reply = server.handle(&close);
        assert!(server.document_text(uri).is_none());
        // It published an empty diagnostics array for the closed doc.
        let pub_ = &reply.messages[0];
        assert_eq!(
            pub_.path("params.diagnostics").and_then(|d| d.as_array()).map(|a| a.len()),
            Some(0)
        );
    }
}
