//! Project orchestration: discover `.mview` files, compile them, and assemble
//! a single Motoko actor (`src/main.mo`) that wires every page/layout into the
//! MotoView runtime.

use crate::ast::FileKind;
use crate::codegen::{Codegen, EmitMode};
use crate::lint::{self, Severity};
use crate::parser;
use crate::span::{self, SourceMap};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

/// One `@code` function's source-line footprint, gathered while a `.mview` is
/// parsed and later matched against the generated `func <name>(` line in
/// `main.mo` to build the generated->source [`SourceMap`] (R11). `name` is the
/// (mangled-if-needed) emitted function identifier — what `main.mo` actually
/// shows after `func`.
#[derive(Debug, Clone)]
struct FuncSrcInfo {
    /// `.mview` source path (project-relative, e.g. `src/Pages/Counter.mview`).
    file: String,
    /// Emitted function identifier — matched against `    func <name>(` in main.mo.
    name: String,
    /// 1-based `.mview` line of the `func` keyword (the func's first source line).
    src_start_line: usize,
    /// Number of `.mview` lines the whole `func … { … }` spans (>= 1).
    src_line_count: usize,
}

/// Compute the [`FuncSrcInfo`] list for one parsed file: each `@code` func's
/// source line footprint, derived from its FILE-relative span (R11 rebases these
/// in `parse_code_block`). `src` is the file's char slice for `line_col`.
fn func_src_infos(file: &crate::ast::MviewFile, rel: &str, src: &[char]) -> Vec<FuncSrcInfo> {
    let mut out = Vec::new();
    for f in &file.code.funcs {
        let (start_line, _) = span::line_col(src, f.span.start);
        let (end_line, _) = span::line_col(src, f.span.end.saturating_sub(1).max(f.span.start));
        out.push(FuncSrcInfo {
            file: rel.to_string(),
            // Codegen emits the user's func name verbatim for page/layout/component
            // helpers, so the source name is what `main.mo` shows after `func`.
            name: f.name.clone(),
            src_start_line: start_line as usize,
            src_line_count: (end_line as usize).saturating_sub(start_line as usize) + 1,
        });
    }
    out
}

/// One `@code` `var`/`let`/`type` declaration's source-line footprint (R13),
/// matched against the emitted decl line in the page object block to extend the
/// generated->source [`SourceMap`] to var/let/type INITIALIZERS. The `init` (and
/// the rest of the decl) is copied VERBATIM by codegen, so the region is
/// line-for-line — exactly like a `@code` func body.
///
/// `name` is the unique declaration identifier within its page region; the map
/// pairs the i-th queued decl with this name to the i-th emitted decl line
/// carrying it (an ordered queue, so a name that repeats is matched in order).
/// `kind` records what the emitted line begins with (`var`/`let`/`type`/…) so the
/// scanner only matches the right form and never collides with a same-named func.
#[derive(Debug, Clone)]
struct VarSrcInfo {
    /// `.mview` source path (project-relative).
    file: String,
    /// The Motoko keyword the emitted decl line begins with (`var`/`let`/`type`/
    /// `stable`/`class`/`module`/`object`/`public`). Used to filter the scan to
    /// the matching emitted form so a func/render line is never matched.
    kind: String,
    /// The declaration identifier — matched against `    <kind> <name>` in the
    /// emitted page object block.
    name: String,
    /// 1-based `.mview` line of the declaration's first source line.
    src_start_line: usize,
    /// Number of `.mview` lines the whole declaration spans (>= 1).
    src_line_count: usize,
}

/// Compute the [`VarSrcInfo`] list for one parsed PAGE: each `@code` `var` and
/// each `extra` (let/type/…) declaration's source line footprint. Vars carry a
/// FILE-relative span (R11); `extra` strings do not, so they are located by
/// matching their emitted text back to a `@code` line whose decl identifier is
/// the same — see `extra_src_infos`. Only PAGES emit a `var <decl>` /
/// `<extra>` object-field block (layouts/components do not), so this is called
/// for pages only.
fn var_src_infos(file: &crate::ast::MviewFile, rel: &str, src: &[char]) -> Vec<VarSrcInfo> {
    let mut out = Vec::new();
    for v in &file.code.vars {
        // The emitted line is the var's `raw` (`var <name> : <ty> = <init>;` or
        // `stable var …`). Codegen emits it verbatim, indented; we match by the
        // keyword + the var's name. The src span (R11) gives the exact line range.
        if v.span.is_empty() {
            continue; // no span -> cannot anchor reliably; fall back.
        }
        let (start_line, _) = span::line_col(src, v.span.start);
        let (end_line, _) = span::line_col(src, v.span.end.saturating_sub(1).max(v.span.start));
        // The emitted keyword: `stable` for a stable var, else `var`. The scanner
        // matches `    <kind> <name>` so the right emitted line is found.
        let kind = if v.stable { "stable" } else { "var" }.to_string();
        out.push(VarSrcInfo {
            file: rel.to_string(),
            kind,
            name: v.name.clone(),
            src_start_line: start_line as usize,
            src_line_count: (end_line as usize).saturating_sub(start_line as usize) + 1,
        });
    }
    // `extra` decls (let/type/…) carry no span, so we locate them by scanning the
    // raw `@code` body for the same `<kw> <name>` head the emitted line shows. This
    // keeps the pairing reliable (same identifier, same keyword) without inventing
    // spans the parser never recorded.
    out.extend(extra_src_infos(file, rel, src));
    out
}

/// Locate each `extra` (let/type/object/class/module/stable-non-var) declaration
/// in the raw `@code` source so its emitted line can be anchored (R13). The
/// `CodeBlock::extra` strings are normalized (the parser trims + re-adds `;`), so
/// instead of trusting a span we never recorded, we re-scan the FILE for the
/// declaration's `<keyword> <name>` head — the same head the emitted object-field
/// line carries — and record its source line. An extra whose head we cannot
/// confidently locate is simply skipped (it then keeps today's fallback). This is
/// reliability-over-coverage: we only map an extra when its identifier is found.
fn extra_src_infos(file: &crate::ast::MviewFile, rel: &str, src: &[char]) -> Vec<VarSrcInfo> {
    let mut out = Vec::new();
    // Char offset of the `@code {` body in the file (so source lines are right).
    // We scan within the whole file but only accept heads after the code block.
    let src_str: String = src.iter().collect();
    for e in &file.code.extra {
        // Parse the emitted decl head: `<keyword> <name>`. We only handle the
        // forms the page object block emits with a leading keyword + identifier.
        let trimmed = e.trim_start();
        let (kw, after) = match split_keyword(trimmed) {
            Some(x) => x,
            None => continue,
        };
        // For `stable <kw2> …` the emitted object-field line still begins `stable`.
        let head_kw = kw;
        let name = match decl_ident(after) {
            Some(n) => n,
            None => continue,
        };
        // Find a `<kw> <name>` occurrence in the file that starts a token (so we do
        // not match it inside a larger word). We accept the FIRST such occurrence
        // whose preceding char is whitespace/`{`/`;`/start — a heuristic that is
        // robust for top-level @code decls. If the head appears more than once we
        // still anchor to the first (the queue then matches emit-order to decl-
        // order, which is source order — codegen preserves it).
        let needle = format!("{} {}", kw, name);
        if let Some(off) = find_decl_head(&src_str, &needle) {
            let start_off = src_str[..off].chars().count();
            let (start_line, _) = span::line_col(src, start_off);
            out.push(VarSrcInfo {
                file: rel.to_string(),
                kind: head_kw.to_string(),
                name,
                src_start_line: start_line as usize,
                // Extras are emitted on a single object-field line each (codegen
                // joins the decl onto one line via `{};` ), so 1 line is the safe,
                // never-wrong footprint. A multi-line extra still anchors its FIRST
                // line correctly; only lines below the first fall back.
                src_line_count: 1,
            });
        }
    }
    out
}

/// Split a trimmed declaration into `(keyword, rest)` when it begins with one of
/// the decl keywords the page object block can emit. Returns `None` for anything
/// else (so we never try to anchor an expression-only extra).
fn split_keyword(trimmed: &str) -> Option<(&'static str, &str)> {
    for kw in ["stable", "let", "type", "object", "class", "module", "var"] {
        if let Some(rest) = trimmed.strip_prefix(kw) {
            // Must be a real keyword boundary (`let x`, not `lettuce`).
            if rest.starts_with(|c: char| c.is_whitespace()) {
                return Some((kw, rest.trim_start()));
            }
        }
    }
    None
}

/// The declared identifier at the start of `after` (the text following the decl
/// keyword): the leading run of identifier characters. Returns `None` when there
/// is no plain identifier (e.g. a destructuring `let (a, b) = …`), so such a decl
/// is left to fall back rather than be mis-paired.
fn decl_ident(after: &str) -> Option<String> {
    let after = after.trim_start();
    let id: String = after
        .chars()
        .take_while(|c| c.is_alphanumeric() || *c == '_')
        .collect();
    if id.is_empty() {
        None
    } else {
        Some(id)
    }
}

/// Find a `<keyword> <name>` declaration head in `src` that starts a token (its
/// preceding char is start-of-file or a non-identifier char), returning the BYTE
/// offset of the keyword. Used to locate `extra` decls whose span the parser did
/// not record. `None` when no such token-aligned occurrence exists.
fn find_decl_head(src: &str, needle: &str) -> Option<usize> {
    let bytes = src.as_bytes();
    let mut from = 0usize;
    while let Some(rel) = src[from..].find(needle) {
        let at = from + rel;
        let prev_ok = at == 0 || !(bytes[at - 1] as char).is_alphanumeric() && bytes[at - 1] != b'_';
        // The char AFTER the name must also be a non-identifier char so we matched
        // the WHOLE name (`let x` not the `x` in `let xy`).
        let end = at + needle.len();
        let next_ok = end >= bytes.len()
            || !(bytes[end] as char).is_alphanumeric() && bytes[end] != b'_';
        if prev_ok && next_ok {
            return Some(at);
        }
        from = at + needle.len();
    }
    None
}

/// (R13) One page's ordered template-expr spans plus its source chars — the input
/// the source-map builder uses to pair each `@(expr)`/`@raw(expr)` directive to a
/// `b.text(...)`/`b.raw(<expr>)` emit LINE in that page's generated render region.
struct PageExprSpans {
    /// `.mview` source path (project-relative) — the map's file column.
    file: String,
    /// The page name (its object block is `<name>Page`), used to find the render
    /// region in the assembled actor.
    page_name: String,
    /// Ordered spans, one per non-literal text/raw directive emitted (R13). A
    /// `None` entry is counted for ALIGNMENT but never mapped (no parser span).
    spans: Vec<Option<span::Span>>,
    /// The page's `.mview` source chars, for `line_col` on each span.
    src_chars: Vec<char>,
}

/// Whether a trimmed generated render line is a NON-LITERAL `b.text(...)` or
/// `b.raw(<expr>)` emit site — i.e. a dynamic interpolation (what a template
/// `@(expr)`/`@raw(expr)` compiles to), NOT a static `b.raw("…literal…")` chunk.
/// These are the lines the R13 template-expr pairing counts, IN ORDER.
fn is_nonliteral_text_or_raw(trimmed: &str) -> bool {
    // `b.text(` is ALWAYS dynamic (a literal would have been a `b.raw("…")`).
    if trimmed.starts_with("b.text(") {
        return true;
    }
    // `b.raw(` is dynamic only when its arg is NOT a string literal. A literal
    // chunk is `b.raw("…");`; anything else (`b.raw(expr)`, `b.raw(mvBody)`, …)
    // is a non-literal raw. coalesce_raw only ever merges the literal form, so by
    // the time we scan the FINAL actor the literal `b.raw("…")` lines are the
    // coalesced static chunks — we must NOT count them.
    if let Some(rest) = trimmed.strip_prefix("b.raw(") {
        return !rest.starts_with('"');
    }
    false
}

/// Build the generated->source [`SourceMap`] by scanning the FINAL `main.mo`
/// (post-coalesce, post-header) for each `@code` function's emitted
/// `    func <name>(` header line, anchored within the `// mv:src <path>` region
/// it belongs to. Each func maps a line-preserving region: generated header line
/// `G` ↔ source line `src_start_line`, extending `src_line_count` lines (the
/// func body is emitted near-verbatim, so the correspondence is line-for-line).
///
/// This runs on the byte-final actor, so `coalesce_raw`'s line shifts are already
/// baked in — the recorded generated lines are the ones `moc` will actually
/// report. The map is a SIDE artifact (`.mvbuild/main.mo.map`); nothing is added
/// to `main.mo`, so the golden actor stays byte-identical.
fn build_source_map(
    main_mo: &str,
    funcs: &[FuncSrcInfo],
    vars: &[VarSrcInfo],
    page_exprs: &[PageExprSpans],
) -> SourceMap {
    let mut map = SourceMap::new();
    // Index the funcs by their owning file for region-scoped matching.
    let lines: Vec<&str> = main_mo.lines().collect();
    // Track which (file, gen_line) we've already consumed so two funcs with the
    // same name in different files don't collide, and re-emitted helper funcs
    // (mvRender etc.) are never matched (they aren't in `funcs`).
    let mut current_file: Option<String> = None;
    // Per-file cursor: index into that file's not-yet-matched funcs (matched in
    // emission order, which is source order — codegen preserves func order).
    let mut by_file: HashMap<String, std::collections::VecDeque<&FuncSrcInfo>> = HashMap::new();
    for f in funcs {
        by_file.entry(f.file.clone()).or_default().push_back(f);
    }
    // (R13) Per-file ordered queues of var/let/type decl footprints. Matched in
    // emission order, which is source order (codegen emits user vars then extras,
    // each in source order). Keyed by file so two pages don't cross-match.
    let mut vars_by_file: HashMap<String, std::collections::VecDeque<&VarSrcInfo>> = HashMap::new();
    for v in vars {
        vars_by_file.entry(v.file.clone()).or_default().push_back(v);
    }
    for (i, raw) in lines.iter().enumerate() {
        let gen_line = i + 1; // 1-based, matches moc
        let trimmed = raw.trim_start();
        if let Some(rest) = trimmed.strip_prefix("// mv:src ") {
            current_file = Some(rest.trim().to_string());
            continue;
        }
        // Match `func <name>(` (the emitted user helper). The leading indent and
        // exact name disambiguate from the runtime's own `public func ...`.
        if let Some(name) = parse_emitted_func_name(trimmed) {
            if let Some(file) = &current_file {
                if let Some(queue) = by_file.get_mut(file) {
                    // Pop the first queued func with this name (source order).
                    if let Some(pos) = queue.iter().position(|f| f.name == name) {
                        let info = queue.remove(pos).unwrap();
                        let gen_start = gen_line;
                        let gen_end = gen_line + info.src_line_count.saturating_sub(1);
                        map.push(file.clone(), gen_start, gen_end, info.src_start_line);
                    }
                }
            }
            continue;
        }
        // (R13) Match an emitted page-object-block `var`/`let`/`type`/… decl line
        // and anchor it to its `.mview` decl span. We pop the first QUEUED user
        // decl with this `<kind> <name>` head (in source order). Framework-injected
        // fields (`let mvErrors`, `var mvRedirect`, the effect helpers) are NEVER in
        // the queue, so they are skipped — only USER decls are mapped.
        if let Some(file) = &current_file {
            if let Some((kind, name)) = parse_emitted_decl_head(trimmed) {
                if let Some(queue) = vars_by_file.get_mut(file) {
                    if let Some(pos) = queue.iter().position(|v| v.kind == kind && v.name == name) {
                        let info = queue.remove(pos).unwrap();
                        let gen_start = gen_line;
                        let gen_end = gen_line + info.src_line_count.saturating_sub(1);
                        map.push(file.clone(), gen_start, gen_end, info.src_start_line);
                    }
                }
            }
        }
    }
    // (R13) Template expressions: for each page, find its render region in the
    // assembled actor, count the NON-LITERAL `b.text(...)`/`b.raw(<expr>)` emit
    // LINES in ORDER, and pair the i-th with the i-th recorded directive span.
    // The pairing is applied ONLY when the count of emit lines EQUALS the count of
    // recorded spans — any mismatch means an emit site we didn't record (a
    // component/chart/concat/bound-attr expansion), so we DO NOT map that page's
    // template exprs at all (a wrong anchor is worse than the file-marker fallback).
    map_template_exprs(&lines, page_exprs, &mut map);
    map
}

/// (R13) For each page, locate its render region (`public func mvRender` …
/// `b.build();` / `ir.toJson();`) within `<page>Page = object {`, collect the
/// ordered non-literal `b.text(...)`/`b.raw(<expr>)` emit LINES, and pair them
/// 1:1 with the page's recorded directive spans — but only when the counts match
/// EXACTLY. Each mapped emit line becomes a single-line region anchored to its
/// directive's `.mview` line. Mismatches (or `None` spans) fall back silently.
fn map_template_exprs(lines: &[&str], page_exprs: &[PageExprSpans], map: &mut SourceMap) {
    for pe in page_exprs {
        // Find this page's object block, then its render function within it.
        let obj_open = format!("let {}Page = object {{", pe.page_name);
        let obj_start = match lines.iter().position(|l| l.trim_start().starts_with(&obj_open)) {
            Some(i) => i,
            None => continue,
        };
        // The render function opener (a `public func mvRender(` line) after it.
        let render_open = lines[obj_start..]
            .iter()
            .position(|l| l.trim_start().starts_with("public func mvRender("))
            .map(|rel| obj_start + rel);
        let render_open = match render_open {
            Some(i) => i,
            None => continue,
        };
        // The render body ends at the first `b.build();`/`ir.toJson();` after it.
        let render_end = lines[render_open..]
            .iter()
            .position(|l| {
                let t = l.trim_start();
                t.starts_with("b.build();") || t.starts_with("ir.toJson();")
            })
            .map(|rel| render_open + rel);
        let render_end = match render_end {
            Some(i) => i,
            None => continue,
        };
        // Collect the ordered emit-site LINES (1-based) within (render_open, render_end).
        let mut emit_lines: Vec<usize> = Vec::new();
        for (idx, l) in lines.iter().enumerate().take(render_end).skip(render_open + 1) {
            if is_nonliteral_text_or_raw(l.trim_start()) {
                emit_lines.push(idx + 1); // 1-based, matches moc
            }
        }
        // RELIABILITY GATE: only pair when the counts agree exactly. Any mismatch
        // means a non-recorded emit site (component/chart/concat/bound-attr/head),
        // so we decline to map this page's template exprs (fall back).
        if emit_lines.len() != pe.spans.len() {
            continue;
        }
        for (gen_line, span_opt) in emit_lines.iter().zip(pe.spans.iter()) {
            if let Some(span) = span_opt {
                let (src_line, _) = span::line_col(&pe.src_chars, span.start);
                // Single-line region: the directive's expr is on one source line in
                // the common case; if it wraps, only the first line anchors (the
                // emit site is one generated line, so a single-line region is exact).
                map.push(pe.file.clone(), *gen_line, *gen_line, src_line as usize);
            }
        }
    }
}

/// (R13) If `trimmed` (already left-trimmed) is an emitted page-object-block
/// declaration line, return its `(kind, name)` head — e.g. `var count : … = …;`
/// -> `("var", "count")`, `let x = …;` -> `("let","x")`, `stable var s …` ->
/// `("stable","s")`. Only the decl keywords a page object block emits are
/// recognised; anything else (a func, a `public func`, an expression) yields
/// `None`. The `name` is the declared identifier (for `stable var` it is the var
/// name, matching how [`var_src_infos`] records a stable var under kind `stable`).
fn parse_emitted_decl_head(trimmed: &str) -> Option<(String, String)> {
    let (kw, after) = split_keyword(trimmed)?;
    // `stable var <name>` — the name is after the `var`. For a plain decl the
    // name is the leading identifier of `after`.
    let after = if kw == "stable" {
        // expect `var <name>` (a stable var); skip the `var` keyword.
        let a = after.trim_start();
        match a.strip_prefix("var") {
            Some(rest) if rest.starts_with(|c: char| c.is_whitespace()) => rest.trim_start(),
            // `stable <other>` extra (rare) — fall back to the identifier as-is.
            _ => a,
        }
    } else {
        after
    };
    let name = decl_ident(after)?;
    Some((kw.to_string(), name))
}

/// If `trimmed` (already left-trimmed) begins a Motoko function declaration
/// `func <name>(`, return `<name>`. Skips `public func`/`shared func` so it only
/// matches the plain `func name(` form codegen emits for `@code` helpers.
fn parse_emitted_func_name(trimmed: &str) -> Option<String> {
    let rest = trimmed.strip_prefix("func ")?;
    let name: String = rest
        .chars()
        .take_while(|c| c.is_alphanumeric() || *c == '_')
        .collect();
    if name.is_empty() {
        return None;
    }
    // Must be `func name(` — a paren (after optional spaces) follows the name.
    let after = rest[name.len()..].trim_start();
    if after.starts_with('(') {
        Some(name)
    } else {
        None
    }
}

pub struct BuildOptions {
    pub project_dir: PathBuf,
    pub app_name: String,
    pub out: PathBuf,
    /// Target network: "local" (default) uses the local `dfx_test_key` vetKD
    /// key; "ic"/"mainnet" uses the production `key_1`. Selects the key name
    /// baked into the generated actor (see `vetkd_key_name`).
    pub network: String,
    /// Which backend the page render emits. `Html` is the DEFAULT and is the
    /// byte-identical legacy path (every `motoview build`/`check`/`dev` uses it).
    /// `Ir` makes each page's `mvRender` return the portable UINode forest as
    /// JSON Text (via `Ir.Builder`) — the basis of the no-deploy preview.
    pub emit: EmitMode,
    /// Opt-in observability instrumentation (R7). When true, the generated event
    /// dispatch emits a structured `Debug.print` line per event (tag + page +
    /// handler + caller + lastBatch + instruction cost) consumed by the studio
    /// log parser. DEFAULT is false; when false the generated `main.mo` is
    /// BYTE-IDENTICAL to the legacy path (no Debug import, no perf counter).
    pub instrument: bool,
}

impl Default for BuildOptions {
    fn default() -> Self {
        BuildOptions {
            project_dir: PathBuf::from("."),
            app_name: "MotoViewApp".to_string(),
            out: PathBuf::from(".mvbuild/main.mo"),
            network: "local".to_string(),
            emit: EmitMode::Html,
            instrument: false,
        }
    }
}

/// Pick the vetKD key name for a target network. Mainnet (`ic`/`mainnet`) gets
/// the production `key_1`; every other network (local replica, playground) gets
/// the local `dfx_test_key`. Centralised so the network gate has one source of
/// truth and the hard-fail guard below can check it.
fn vetkd_key_name(network: &str) -> &'static str {
    match network.trim().to_ascii_lowercase().as_str() {
        "ic" | "mainnet" => "key_1",
        _ => "dfx_test_key",
    }
}

/// Defensive network gate: refuse to emit the LOCAL `dfx_test_key` for a mainnet
/// target. With `vetkd_key_name` the resolved key is always correct, so this
/// never trips on a real build — but it is a hard backstop against a future
/// regression that lets the local test key leak into a mainnet artifact (the
/// local key does not exist on `ic`, so vetKeys would silently break). Pulled
/// out as a pure function so the Err branch is independently unit-testable.
pub fn enforce_network_gate(network: &str, key: &str) -> Result<(), String> {
    let net = network.trim().to_ascii_lowercase();
    if (net == "ic" || net == "mainnet") && key == "dfx_test_key" {
        return Err(format!(
            "network gate: refusing to emit vetKeys key `dfx_test_key` for network \
             `{}` — mainnet requires the production key `key_1`.",
            network
        ));
    }
    Ok(())
}

pub fn build(opts: &BuildOptions) -> Result<String, String> {
    // ---- network gate (defensive hard-fail) ----
    // Resolve the vetKD key name for the target network. If we are building for
    // mainnet but the resolved key is still the LOCAL `dfx_test_key`, refuse to
    // emit — shipping the local key to `ic` would silently break vetKeys (the
    // local test key does not exist on mainnet). With `vetkd_key_name` this can't
    // happen, but the guard must exist and be testable.
    let vetkd_key = vetkd_key_name(&opts.network);
    enforce_network_gate(&opts.network, vetkd_key)?;

    // Lint diagnostics accumulate across every parsed .mview; an Error aborts the
    // build before any codegen, a Warning is printed but does not block.
    let mut diagnostics: Vec<(String, lint::Diagnostic)> = Vec::new();

    let src = opts.project_dir.join("src");
    // Services/Models are imported relative to the GENERATED actor. It now lives
    // in .mvbuild/, so prefix those imports with the path back to src/ (e.g.
    // "../src/"). For a legacy in-src output this prefix is empty.
    let prefix = {
        let rel = rel_path_between(opts.out.parent().unwrap_or(&opts.project_dir), &src);
        if rel.is_empty() { String::new() } else { format!("{}/", rel) }
    };
    let pages_dir = src.join("Pages");
    let layouts_dir = src.join("Layouts");
    let components_dir = src.join("Components");

    let page_files = list_mview(&pages_dir);
    let layout_files = list_mview(&layouts_dir);
    let component_files = list_mview(&components_dir);

    // Service/Model modules (real Motoko) are imported into the generated actor
    // so page @code can call them (e.g. `Crm.all()`, `Crm.Deal`).
    //
    // Two flavours of service are supported:
    //   * Stateless module  — `module { public func ... }`. Imported directly;
    //     pages call `Name.func(...)`. State (if any) lives in the page @code.
    //   * Stateful service  — a file `Name.mo` that exports `public class Name()`.
    //     The compiler imports the module under a mangled alias and binds ONE
    //     shared instance `let Name = Name__mod.Name();` at actor scope, before
    //     the page objects. Because page objects close over the enclosing actor
    //     scope, every page calls `Name.method(...)` against the same instance —
    //     giving shared, cross-page, canister-lifetime state. This is what makes
    //     real multi-page apps (chat/forum/feed/DMs) possible.
    let mut extra_imports = String::new();
    let mut service_instances = String::new();
    // Services that opt into upgrade-stable persistence by exposing
    // `public func mvStableSave() : Blob` and `public func mvStableLoad(Blob)`.
    let mut persistent_services: Vec<String> = Vec::new();
    for f in list_mo(&src.join("Services")) {
        let n = file_stem(&f);
        let content = fs::read_to_string(&f).unwrap_or_default();
        if is_stateful_service(&content, &n) {
            extra_imports.push_str(&format!("import {n}__mod \"{prefix}Services/{n}\";\n", n = n));
            service_instances.push_str(&format!(
                "  // shared, cross-page, canister-lifetime service instance\n  let {n} = {n}__mod.{n}();\n",
                n = n
            ));
            // A real declaration starts its (trimmed) line — don't be fooled by
            // the string occurring inside an embedded string literal or comment
            // (e.g. a docs page that documents the persistence API).
            if content
                .lines()
                .any(|l| l.trim_start().starts_with("public func mvStableSave"))
            {
                persistent_services.push(n.clone());
            }
        } else {
            extra_imports.push_str(&format!("import {} \"{prefix}Services/{}\";\n", n, n));
        }
    }
    // Generate the stable backing + upgrade hooks. Each persistent service keeps
    // its live state in non-stable collections; on `--mode upgrade` we snapshot
    // it to a `stable var` Blob (preupgrade) and restore it (postupgrade), so
    // state survives upgrades. See is_stateful_service for the service convention.
    // Upgrade hooks. `preupgrade` ALWAYS emits, because every app has security
    // state (epochs/roles/consumed-nonces/velocity) that must be snapshotted to
    // its stable var at the upgrade boundary. Those four maps live in the App
    // instance's in-memory stores during normal operation and are synced here
    // ONCE — at upgrade — rather than after every update call (which copied all
    // four whole maps to stable vars on every request: O(security state)/request
    // overhead that grew with the consumed-nonce store). The matching restore is
    // the eager `mvApp.setEpochs(mvEpochs)` … block that runs on every actor
    // init (fresh install AND post-upgrade), so postupgrade only reloads the
    // per-service state blobs. `preupgrade`/`mvApp` forward-reference fields
    // declared later in the actor body — legal because the bodies run at upgrade
    // time, long after init completes.
    let mut persistence = String::new();
    persistence.push_str("  // ---- upgrade-stable persistence ----\n");
    for n in &persistent_services {
        persistence.push_str(&format!("  stable var {n}__state : Blob = \"\" : Blob;\n", n = n));
    }
    persistence.push_str("  system func preupgrade() {\n");
    // Security maps: snapshot once, at the upgrade boundary (not per request).
    persistence.push_str("    mvEpochs := mvApp.dumpEpochs();\n");
    persistence.push_str("    mvRoles := mvApp.dumpRoles();\n");
    persistence.push_str("    mvConsumed := mvApp.dumpConsumed();\n");
    persistence.push_str("    mvVelocity := mvApp.dumpVelocity();\n");
    for n in &persistent_services {
        persistence.push_str(&format!("    {n}__state := {n}.mvStableSave();\n", n = n));
    }
    persistence.push_str("  };\n  system func postupgrade() {\n");
    for n in &persistent_services {
        // Skip an empty snapshot — `from_candid` traps on a zero-length blob,
        // which is exactly the value the stable var holds the first time an
        // app WITHOUT persistence is upgraded to one WITH it. Skipping keeps
        // the freshly-seeded state.
        persistence.push_str(&format!(
            "    if ({n}__state.size() > 0) {{ {n}.mvStableLoad({n}__state) }};\n",
            n = n
        ));
    }
    persistence.push_str("  };\n");
    for f in list_mo(&src.join("Models")) {
        let n = file_stem(&f);
        extra_imports.push_str(&format!("import {} \"{prefix}Models/{}\";\n", n, n));
    }

    // R7 observability: the instrumented dispatch references `Debug.print` and
    // `ExperimentalInternetComputer.performanceCounter`. These imports are ONLY
    // emitted when `--instrument` is on, so the default build header (and thus
    // every byte) is unchanged. (`Principal` is already imported unconditionally.)
    if opts.instrument {
        extra_imports.push_str("import Debug \"mo:base/Debug\";\n");
        extra_imports.push_str("import ExperimentalIC \"mo:base/ExperimentalInternetComputer\";\n");
    }

    if page_files.is_empty() {
        return Err(format!(
            "no .mview pages found in {}",
            pages_dir.display()
        ));
    }

    // Scan Service/Model record types (type Name = { field : T; ... }) so the
    // codegen can type `@item.field` precisely instead of falling back to
    // debug_show. Service types also register under `Service.Name`.
    let mut models: HashMap<String, HashMap<String, String>> = HashMap::new();
    for f in list_mo(&src.join("Services")) {
        let content = fs::read_to_string(&f).unwrap_or_default();
        scan_types(&content, Some(&file_stem(&f)), &mut models);
    }
    for f in list_mo(&src.join("Models")) {
        let content = fs::read_to_string(&f).unwrap_or_default();
        scan_types(&content, Some(&file_stem(&f)), &mut models);
    }

    // App components: parse src/Components/*.mview, register their params, and
    // generate a render function per component. Pages then compile a `<Card .../>`
    // tag to a call of `mvComponent_Card(...)`.
    let mut components: HashMap<String, crate::codegen::CompInfo> = HashMap::new();
    let mut parsed_components = Vec::new();
    // First `@theme brand="#hex"` seen across ALL .mview files (components,
    // pages, layouts) — the project's brand color. Used to cross-compile the
    // SAME design tokens to native (SwiftUI/Compose) alongside the web CSS, so
    // theming is pixel-identical across web/iOS/Android. Brand themes usually
    // live on a Layout, so every loop below contributes.
    let mut theme_brand: Option<String> = None;
    for cf in &component_files {
        let name = file_stem(cf);
        let source = fs::read_to_string(cf).map_err(|e| format!("{}: {}", cf.display(), e))?;
        let file = parser::parse(&source, &name, FileKind::Component)?;
        if theme_brand.is_none() {
            theme_brand = file.theme_brand.clone();
        }
        let rel = rel_src(&opts.project_dir, cf);
        for d in lint::lint_file(&file, &rel) {
            diagnostics.push((rel.clone(), d));
        }
        let slots = crate::codegen::collect_slot_names(&file.template);
        components.insert(
            name,
            crate::codegen::CompInfo { params: file.code.params.clone(), slots },
        );
        parsed_components.push(file);
    }
    let mut component_funcs = String::new();
    for (cf, file) in component_files.iter().zip(parsed_components.iter()) {
        let mut cg = Codegen::new(&models, &components);
        component_funcs.push_str(&format!("  // mv:src {}\n", rel_src(&opts.project_dir, cf)));
        component_funcs.push_str(&cg.gen_app_component(file));
        component_funcs.push('\n');
    }

    let mut page_objects = String::new();
    let mut page_records = String::new();
    let mut page_idents: Vec<String> = Vec::new();
    let mut routes: Vec<(String, String)> = Vec::new();
    // (R11) Per-`@code` func source-line footprints, gathered across pages/
    // components/layouts; turned into the generated->source map after assembly.
    let mut func_infos: Vec<FuncSrcInfo> = Vec::new();
    // (R13) Per-page `var`/`let`/`type` declaration footprints (anchored to the
    // emitted page-object-block decl line), and the ordered template-expr spans
    // recorded during each page's render walk (for the b.text/b.raw emit-line
    // pairing). Both extend the same generated->source map after assembly.
    let mut var_infos: Vec<VarSrcInfo> = Vec::new();
    let mut page_expr_spans: Vec<PageExprSpans> = Vec::new();
    // (layout-auth-gate lint) Per-page gate metadata + the set of layouts that
    // gate content on `ctx.isAuthenticated`. Correlated after both loops: a page
    // using an auth-gating layout but lacking `@authorize` is reachable unguarded
    // via /_motoview/render + /_motoview/event (which skip the layout).
    let mut page_gate_meta: Vec<(String, String, bool)> = Vec::new(); // (rel, layout, has_authorize)
    let mut auth_gated_layouts: HashSet<String> = HashSet::new();
    // Components were already parsed above; record their func footprints too.
    for (cf, file) in component_files.iter().zip(parsed_components.iter()) {
        let rel = rel_src(&opts.project_dir, cf);
        let csrc = fs::read_to_string(cf).unwrap_or_default();
        let cchars: Vec<char> = csrc.chars().collect();
        func_infos.extend(func_src_infos(file, &rel, &cchars));
    }

    for pf in &page_files {
        let name = file_stem(pf);
        let source = fs::read_to_string(pf).map_err(|e| format!("{}: {}", pf.display(), e))?;
        let file = parser::parse(&source, &name, FileKind::Page)?;
        if theme_brand.is_none() {
            theme_brand = file.theme_brand.clone();
        }
        let rel = rel_src(&opts.project_dir, pf);
        let pchars: Vec<char> = source.chars().collect();
        func_infos.extend(func_src_infos(&file, &rel, &pchars));
        // R13: page `var`/`let`/`type` decl footprints (object-block fields).
        var_infos.extend(var_src_infos(&file, &rel, &pchars));
        for d in lint::lint_file(&file, &rel) {
            diagnostics.push((rel.clone(), d));
        }
        page_gate_meta.push((
            rel.clone(),
            file.layout.clone().unwrap_or_default(),
            file.authorize.is_some(),
        ));
        // Pages honour the requested emit backend: Html (default, byte-identical)
        // or Ir (portable UINode forest, used by the no-deploy preview). Layouts
        // and components are always Html-emitted regardless (they are the document
        // shell / HTML fragments — see gen_layout / gen_app_component).
        let mut cg = Codegen::new_with_emit(&models, &components, opts.emit).with_instrument(opts.instrument);
        let pg = cg.gen_page(&file);
        // R13: stash this page's ordered template-expr spans + source chars so the
        // source-map builder can pair them with the render region's emit lines.
        page_expr_spans.push(PageExprSpans {
            file: rel.clone(),
            page_name: pg.name.clone(),
            spans: pg.expr_spans.clone(),
            src_chars: pchars.clone(),
        });
        page_objects.push_str(&format!("  // mv:src {}\n", rel_src(&opts.project_dir, pf)));
        page_objects.push_str(&pg.object_block);
        page_objects.push('\n');
        page_records.push_str(&pg.page_record);
        page_idents.push(format!("{}Def", pg.name));
        routes.push((pg.route, pg.name));
    }

    let mut layout_funcs = String::new();
    let mut layout_entries: Vec<(String, String)> = Vec::new(); // (name, func)
    for lf in &layout_files {
        let name = file_stem(lf);
        let source = fs::read_to_string(lf).map_err(|e| format!("{}: {}", lf.display(), e))?;
        let file = parser::parse(&source, &name, FileKind::Layout)?;
        if theme_brand.is_none() {
            theme_brand = file.theme_brand.clone();
        }
        let rel = rel_src(&opts.project_dir, lf);
        let lchars: Vec<char> = source.chars().collect();
        func_infos.extend(func_src_infos(&file, &rel, &lchars));
        for d in lint::lint_file(&file, &rel) {
            diagnostics.push((rel.clone(), d));
        }
        if lint::layout_gates_on_auth(&file) {
            auth_gated_layouts.insert(name.clone());
        }
        let mut cg = Codegen::new(&models, &components);
        layout_funcs.push_str(&format!("  // mv:src {}\n", rel_src(&opts.project_dir, lf)));
        layout_funcs.push_str(&cg.gen_layout(&file));
        layout_funcs.push('\n');
        layout_entries.push((name.clone(), format!("mvLayout_{}", name)));
    }

    // (layout-auth-gate) Now that every layout's auth-gate status is known, flag
    // pages that lean on a layout gate without their own `@authorize` — they are
    // reachable unguarded via the layout-less /_motoview/render + /event endpoints.
    for (rel, layout, has_authorize) in &page_gate_meta {
        if !has_authorize && auth_gated_layouts.contains(layout) {
            diagnostics.push((rel.clone(), lint::layout_gate_warning(rel, layout)));
        }
    }

    // Lint gate: if ANY .mview produced an Error-severity diagnostic, abort the
    // build before emitting the actor. All diagnostics (errors AND warnings) are
    // printed, mapped to their source .mview; warnings alone never abort.
    if diagnostics.iter().any(|(_, d)| d.severity == Severity::Error) {
        return Err(format_diagnostics(&diagnostics));
    }
    // Warnings (e.g. @raw) don't block the build, but surface them.
    if !diagnostics.is_empty() {
        eprint!("{}", format_diagnostics(&diagnostics));
    }

    // The real HMAC secret is installed at runtime from raw_rand (see the
    // generated http_request_update). The compile-time value is only an empty
    // placeholder — never a function of public input, never a usable secret.
    let secret = "\"\"".to_string();
    let main = assemble(
        &opts.app_name,
        &page_objects,
        &page_records,
        &page_idents,
        &layout_funcs,
        &layout_entries,
        &secret,
        &extra_imports,
        &service_instances,
        &persistence,
        &component_funcs,
        vetkd_key,
    );
    // Collapse the per-token `b.raw("…")` storm into one call per contiguous
    // static run, so the generated actor reads as readable HTML chunks.
    let main = coalesce_raw(&main);
    // This actor is a BUILD ARTIFACT, like Blazor's obj/ or React's bundle —
    // generated, not source. You never edit or commit it.
    let main = format!(
        "// GENERATED by `motoview build` — do not edit.\n\
         // Edit the .mview files in src/; run `motoview check` to type-check\n\
         // (errors map back to your .mview via the `// mv:src` markers below).\n\n{}",
        main
    );

    if let Some(parent) = opts.out.parent() {
        let _ = fs::create_dir_all(parent);
    }
    fs::write(&opts.out, &main).map_err(|e| format!("writing {}: {}", opts.out.display(), e))?;

    // ---- generated->source line map (R11, SIDE artifact) ----
    // Scan the BYTE-FINAL `main.mo` for each `@code` func's emitted header line and
    // record `generated line -> (.mview file, .mview line)`. Written next to the
    // actor as `<out>.map`; nothing is added to `main.mo` (so it stays byte-
    // identical). `check` reads this back to point `moc` type errors at the
    // originating `.mview` LINE — see `map_moc_errors[_json]`.
    let source_map = build_source_map(&main, &func_infos, &var_infos, &page_expr_spans);
    let map_path = source_map_path(&opts.out);
    let _ = fs::write(&map_path, source_map.to_text());

    // ---- design-token cross-compile (web CSS -> native SwiftUI/Compose) ----
    // When the project declares `@theme brand="#hex"`, emit the SAME brand ramp +
    // alias tokens the web `<style>` carries, as native color sources next to the
    // generated actor. The native emitters reuse color::brand_ramp / BRAND_ALIASES
    // / shade_idx, so the shade selection is byte-identical to the CSS path.
    let mut native_artifacts: Vec<PathBuf> = Vec::new();
    if let Some(brand) = &theme_brand {
        let native_dir = opts
            .out
            .parent()
            .unwrap_or(&opts.project_dir)
            .join("native");
        if let (Some(swift), Some(kotlin)) = (
            crate::color_native::brand_theme_swift(brand),
            crate::color_native::brand_theme_kotlin(brand),
        ) {
            fs::create_dir_all(&native_dir)
                .map_err(|e| format!("creating {}: {}", native_dir.display(), e))?;
            let swift_path = native_dir.join("BrandTokens.swift");
            let kotlin_path = native_dir.join("BrandTokens.kt");
            fs::write(&swift_path, &swift)
                .map_err(|e| format!("writing {}: {}", swift_path.display(), e))?;
            fs::write(&kotlin_path, &kotlin)
                .map_err(|e| format!("writing {}: {}", kotlin_path.display(), e))?;
            native_artifacts.push(swift_path);
            native_artifacts.push(kotlin_path);
        }
    }

    let mut summary = format!(
        "compiled {} page(s), {} layout(s) -> {}\n",
        page_files.len(),
        layout_files.len(),
        opts.out.display()
    );
    for art in &native_artifacts {
        summary.push_str(&format!("native tokens -> {}\n", art.display()));
    }
    summary.push_str("routes:\n");
    for (r, n) in &routes {
        summary.push_str(&format!("  {:<24} {}\n", r, n));
    }
    Ok(summary)
}

// ---- no-deploy preview (motoview preview) --------------------------------

/// One recorded interaction event for deterministic record/replay (R10). It is the
/// exact triple the generated `mvDispatch(ctx, handler, args)` consumes: a handler
/// id, its string args (the same `[Text]` the runtime passes), and the caller
/// principal text (anonymous by default). Replaying the same ordered list through
/// the page's dispatch is byte-deterministic — the IC's determinism makes
/// time-travel near-free.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReplayEvent {
    /// The handler/event id, e.g. `increment` (matches an `@click="increment(..)"`).
    pub handler: String,
    /// The handler's string arguments, in order (e.g. `["1"]` for `increment(1)`).
    pub args: Vec<String>,
    /// The caller principal text. Defaults to the anonymous principal `2vxsx-fae`.
    pub caller: String,
}

impl ReplayEvent {
    /// The anonymous principal — the default caller for a recorded event.
    pub const ANON: &'static str = "2vxsx-fae";
}

/// The result of generating a no-deploy preview driver.
#[derive(Debug, Clone)]
pub struct PreviewBuild {
    /// Absolute/relative path of the generated preview driver program.
    pub driver_path: PathBuf,
    /// The route of the page the driver renders.
    pub route: String,
    /// The page identifier (file stem) the driver renders.
    pub page_name: String,
    /// All discovered routes (route -> page name), for the error message / listing.
    pub routes: Vec<(String, String)>,
}

/// Generate a self-contained preview DRIVER program (`.mvbuild/preview.mo`) that,
/// when run through the Motoko interpreter (`moc -r`), constructs a MOCK request
/// context (anonymous caller, empty params/query/form, isAuthenticated=false,
/// no-op token/role functions) and `Debug.print`s the IR forest produced by the
/// target page's `render(mockCtx)`.
///
/// This is the fast inner loop: NO `dfx deploy`, NO replica. The page objects in
/// the driver are the EXACT ones the real build emits, but in `EmitMode::Ir` so
/// `mvRender` returns the portable UINode forest JSON (see runtime/src/Ir.mo).
/// The driver is a top-level program (not an `actor`) so `moc -r` evaluates it
/// directly; the actor's HTTP/stable/vetKD machinery (which needs the async actor
/// context) is intentionally omitted — the preview only needs the initial render.
///
/// `route` selects which page to render (matched against each page's route, or by
/// page name); `None` renders the first page (after sorting routes for stability).
pub fn build_preview(
    project_dir: &Path,
    route: Option<&str>,
) -> Result<PreviewBuild, String> {
    build_preview_with_events(project_dir, route, &[], false)
}

/// Like [`build_preview`], but BEFORE the final render it applies an ordered list
/// of recorded [`ReplayEvent`]s through the page's `mvDispatch` (R10 deterministic
/// replay). Each event mutates the page-local state exactly as a real interaction
/// would; the forest printed is the page's render AFTER those N dispatches. Passing
/// an empty slice is identical to [`build_preview`] (the initial render). Because
/// the page render + dispatch are pure/deterministic, replaying the same session
/// twice yields byte-identical forests — that is the core record/replay property.
pub fn build_preview_with_events(
    project_dir: &Path,
    route: Option<&str>,
    events: &[ReplayEvent],
    // SELECTION BRIDGE: when true (the studio's `--srcmap` path), each preview IR
    // element is tagged with a `data-mv-src` id and a `.mvbuild/preview.srcmap.json`
    // side-map (id -> .mview file + open-tag/attr/event spans) is written. PREVIEW
    // ONLY — production `main.mo` / native IR bytes are never affected. Default
    // false keeps `motoview preview` output unchanged.
    source_ids: bool,
) -> Result<PreviewBuild, String> {
    let mut diagnostics: Vec<(String, lint::Diagnostic)> = Vec::new();
    let src = project_dir.join("src");
    // The driver lives in .mvbuild/, so service/model imports point back to ../src.
    let out = project_dir.join(".mvbuild").join("preview.mo");
    let prefix = {
        let rel = rel_path_between(out.parent().unwrap_or(project_dir), &src);
        if rel.is_empty() { String::new() } else { format!("{}/", rel) }
    };

    let pages_dir = src.join("Pages");
    let layouts_dir = src.join("Layouts");
    let components_dir = src.join("Components");
    let page_files = list_mview(&pages_dir);
    let layout_files = list_mview(&layouts_dir);
    let component_files = list_mview(&components_dir);

    if page_files.is_empty() {
        return Err(format!("no .mview pages found in {}", pages_dir.display()));
    }

    // ---- services (mirrors build()): stateless modules + stateful classes ----
    let mut extra_imports = String::new();
    let mut service_instances = String::new();
    for f in list_mo(&src.join("Services")) {
        let n = file_stem(&f);
        let content = fs::read_to_string(&f).unwrap_or_default();
        if is_stateful_service(&content, &n) {
            extra_imports.push_str(&format!("import {n}__mod \"{prefix}Services/{n}\";\n", n = n));
            service_instances.push_str(&format!(
                "  let {n} = {n}__mod.{n}();\n",
                n = n
            ));
        } else {
            extra_imports.push_str(&format!("import {} \"{prefix}Services/{}\";\n", n, n));
        }
    }
    for f in list_mo(&src.join("Models")) {
        let n = file_stem(&f);
        extra_imports.push_str(&format!("import {} \"{prefix}Models/{}\";\n", n, n));
    }

    // ---- scan record types (mirrors build()) ----
    let mut models: HashMap<String, HashMap<String, String>> = HashMap::new();
    for f in list_mo(&src.join("Services")) {
        let content = fs::read_to_string(&f).unwrap_or_default();
        scan_types(&content, Some(&file_stem(&f)), &mut models);
    }
    for f in list_mo(&src.join("Models")) {
        let content = fs::read_to_string(&f).unwrap_or_default();
        scan_types(&content, Some(&file_stem(&f)), &mut models);
    }

    // ---- components (always Html-emitted, like build()) ----
    let mut components: HashMap<String, crate::codegen::CompInfo> = HashMap::new();
    let mut parsed_components = Vec::new();
    for cf in &component_files {
        let name = file_stem(cf);
        let source = fs::read_to_string(cf).map_err(|e| format!("{}: {}", cf.display(), e))?;
        let file = parser::parse(&source, &name, FileKind::Component)?;
        let rel = rel_src(project_dir, cf);
        for d in lint::lint_file(&file, &rel) {
            diagnostics.push((rel.clone(), d));
        }
        let slots = crate::codegen::collect_slot_names(&file.template);
        components.insert(
            name,
            crate::codegen::CompInfo { params: file.code.params.clone(), slots },
        );
        parsed_components.push(file);
    }
    let mut component_funcs = String::new();
    for (cf, file) in component_files.iter().zip(parsed_components.iter()) {
        let mut cg = Codegen::new(&models, &components);
        component_funcs.push_str(&format!("  // mv:src {}\n", rel_src(project_dir, cf)));
        component_funcs.push_str(&cg.gen_app_component(file));
        component_funcs.push('\n');
    }

    // ---- pages: emit each in IR mode (the whole point of preview) ----
    let mut page_objects = String::new();
    let mut page_records = String::new();
    let mut routes: Vec<(String, String)> = Vec::new();
    // SELECTION BRIDGE: one JSON object per element across ALL pages, the array
    // index being the `data-mv-src` id (a global counter, so ids stay unique).
    let mut srcmap_json: Vec<String> = Vec::new();
    for pf in &page_files {
        let name = file_stem(pf);
        let source = fs::read_to_string(pf).map_err(|e| format!("{}: {}", pf.display(), e))?;
        let file = parser::parse(&source, &name, FileKind::Page)?;
        let rel = rel_src(project_dir, pf);
        for d in lint::lint_file(&file, &rel) {
            diagnostics.push((rel.clone(), d));
        }
        let mut cg = Codegen::new_with_emit(&models, &components, EmitMode::Ir);
        if source_ids {
            // base = ids already assigned, so this page's ids continue the count.
            cg.enable_source_ids(srcmap_json.len());
        }
        let pg = cg.gen_page(&file);
        page_objects.push_str(&format!("  // mv:src {}\n", rel_src(project_dir, pf)));
        page_objects.push_str(&pg.object_block);
        page_objects.push('\n');
        page_records.push_str(&pg.page_record);
        routes.push((pg.route.clone(), pg.name.clone()));
        if source_ids {
            let chars: Vec<char> = source.chars().collect();
            for e in cg.take_src_spans() {
                srcmap_json.push(src_entry_to_json(&rel, &chars, &e));
            }
        }
    }

    // ---- layouts (always Html-emitted) ----
    let mut layout_funcs = String::new();
    for lf in &layout_files {
        let name = file_stem(lf);
        let source = fs::read_to_string(lf).map_err(|e| format!("{}: {}", lf.display(), e))?;
        let file = parser::parse(&source, &name, FileKind::Layout)?;
        let rel = rel_src(project_dir, lf);
        for d in lint::lint_file(&file, &rel) {
            diagnostics.push((rel.clone(), d));
        }
        let mut cg = Codegen::new(&models, &components);
        layout_funcs.push_str(&format!("  // mv:src {}\n", rel_src(project_dir, lf)));
        layout_funcs.push_str(&cg.gen_layout(&file));
        layout_funcs.push('\n');
    }

    // Lint gate: an Error-severity finding aborts the preview (same gate as build).
    if diagnostics.iter().any(|(_, d)| d.severity == Severity::Error) {
        return Err(format_diagnostics(&diagnostics));
    }

    // Pick the target page: by route (exact match), then by page name, else first.
    let mut sorted = routes.clone();
    sorted.sort();
    let (target_route, target_name) = if let Some(want) = route {
        let w = want.trim();
        routes
            .iter()
            .find(|(r, _)| r == w)
            .or_else(|| routes.iter().find(|(_, n)| n.eq_ignore_ascii_case(w.trim_start_matches('/'))))
            .cloned()
            .ok_or_else(|| {
                let mut msg = format!("no page matches route `{}`. Available routes:\n", want);
                for (r, n) in &sorted {
                    msg.push_str(&format!("  {:<24} {}\n", r, n));
                }
                msg
            })?
    } else {
        sorted
            .first()
            .cloned()
            .ok_or_else(|| "no pages to preview".to_string())?
    };
    let target_obj = format!("{}Page", target_name);

    let program = assemble_preview(
        &page_objects,
        &component_funcs,
        &layout_funcs,
        &extra_imports,
        &service_instances,
        &target_obj,
        &target_route,
        events,
    );
    // Collapse the per-token raw runs (same readability pass the build uses).
    let program = coalesce_raw(&program);

    if let Some(parent) = out.parent() {
        let _ = fs::create_dir_all(parent);
    }
    fs::write(&out, &program).map_err(|e| format!("writing {}: {}", out.display(), e))?;

    // SELECTION BRIDGE: write the node→span side-map next to preview.mo. It is a
    // SIDE artifact (like main.mo.map) — nothing is added to preview.mo beyond the
    // already-emitted `data-mv-src` attrs, and the production build is untouched.
    if source_ids {
        let map_path = out.with_file_name("preview.srcmap.json");
        let body = format!("[{}]\n", srcmap_json.join(","));
        let _ = fs::write(&map_path, body);
    }

    Ok(PreviewBuild {
        driver_path: out,
        route: target_route,
        page_name: target_name,
        routes: sorted,
    })
}

/// Minimal JSON string escaper (the compiler stays serde-free).
fn json_esc(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    for c in s.chars() {
        match c {
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

/// Serialize one [`crate::codegen::SrcEntry`] to a JSON object for the selection
/// side-map: its `.mview` file + open-tag span (char offsets AND 1-based
/// line/col) + per-attr and per-event spans. Array index = the `data-mv-src` id.
fn src_entry_to_json(file: &str, chars: &[char], e: &crate::codegen::SrcEntry) -> String {
    let span_json = |s: &crate::span::Span| {
        let (sl, sc) = crate::span::line_col(chars, s.start);
        let (el, ec) = crate::span::line_col(chars, s.end);
        format!(
            "{{\"start\":{},\"end\":{},\"line\":{},\"col\":{},\"endLine\":{},\"endCol\":{}}}",
            s.start, s.end, sl, sc, el, ec
        )
    };
    let attrs: Vec<String> = e
        .attrs
        .iter()
        .map(|(n, sp)| format!("{{\"name\":\"{}\",\"span\":{}}}", json_esc(n), span_json(sp)))
        .collect();
    let events: Vec<String> = e
        .events
        .iter()
        .map(|(ev, h, sp)| {
            format!(
                "{{\"event\":\"{}\",\"handler\":\"{}\",\"span\":{}}}",
                json_esc(ev),
                json_esc(h),
                span_json(sp)
            )
        })
        .collect();
    format!(
        "{{\"file\":\"{}\",\"tag\":\"{}\",\"secure\":{},\"span\":{},\"attrs\":[{}],\"events\":[{}]}}",
        json_esc(file),
        json_esc(&e.tag),
        e.secure,
        span_json(&e.span),
        attrs.join(","),
        events.join(",")
    )
}

/// Assemble the preview DRIVER as a top-level Motoko PROGRAM (not an actor) that
/// `moc -r` can evaluate directly. It carries the same imports, conversion
/// helpers, services, components, page objects and layout funcs the real actor
/// has — but instead of HTTP wiring it builds a mock `MV.Ctx` and prints the IR
/// forest of the target page's `mvRender(mockCtx)`.
///
/// Why a program and not the actor: the actor's `http_request*`/vetKD/raw_rand
/// machinery is `async` and needs the canister context (`Random.blob()`,
/// `await VetKeys.*`), which `moc -r` cannot drive. The preview only needs the
/// page's INITIAL render, which is a pure `Ctx -> Text` call — fully evaluable in
/// the interpreter with a mock context.
#[allow(clippy::too_many_arguments)]
fn assemble_preview(
    page_objects: &str,
    component_funcs: &str,
    layout_funcs: &str,
    extra_imports: &str,
    service_instances: &str,
    target_obj: &str,
    target_route: &str,
    events: &[ReplayEvent],
) -> String {
    // REPLAY: emit one `mvDispatch(...)` per recorded event, in order, BEFORE the
    // final render. The dispatch reuses `mockCtx` for the anonymous caller and a
    // per-event override otherwise, so a recorded session re-runs through the exact
    // same dispatch+render path the live actor uses (deterministically).
    let mut replay = String::new();
    for ev in events {
        let args_lit = ev
            .args
            .iter()
            .map(|a| mo_string(a))
            .collect::<Vec<_>>()
            .join(", ");
        if ev.caller == ReplayEvent::ANON {
            replay.push_str(&format!(
                "{obj}.mvDispatch(mockCtx, {h}, [{args}]);\n",
                obj = target_obj,
                h = mo_string(&ev.handler),
                args = args_lit
            ));
        } else {
            // A non-anonymous caller: clone mockCtx with the recorded principal so a
            // handler reading `ctx.caller` sees the replayed identity.
            replay.push_str(&format!(
                "{obj}.mvDispatch({{ mockCtx with caller = Principal.fromText({c}) }}, {h}, [{args}]);\n",
                obj = target_obj,
                c = mo_string(&ev.caller),
                h = mo_string(&ev.handler),
                args = args_lit
            ));
        }
    }
    format!(
        r#"// GENERATED by `motoview preview` — do not edit.
// A no-deploy preview DRIVER: run it with `moc -r` to print the IR forest of one
// page's INITIAL render against a MOCK context. No dfx, no replica. Edit the
// .mview source instead, then re-run `motoview preview`.
import Html "mo:motoview/Html";
import Ir "mo:motoview/Ir";
import MV "mo:motoview/Types";
import Charts "mo:motoview/Charts";
import Nat "mo:base/Nat";
import Nat32 "mo:base/Nat32";
import Nat64 "mo:base/Nat64";
import Int "mo:base/Int";
import Float "mo:base/Float";
import Char "mo:base/Char";
import Text "mo:base/Text";
import Buffer "mo:base/Buffer";
import Principal "mo:base/Principal";
import Array "mo:base/Array";
import Iter "mo:base/Iter";
import Option "mo:base/Option";
import Time "mo:base/Time";
import Bool "mo:base/Bool";
import Debug "mo:base/Debug";
{extra_imports}
// `Context` alias (a handler whose first param is `ctx : Context`).
type Context = MV.Ctx;

// ---- conversion helpers (identical to the generated actor's) ----
func mvNat(t : Text) : Nat {{
  var n : Nat = 0;
  for (c in t.chars()) {{
    let d = Char.toNat32(c);
    if (d >= 48 and d <= 57) {{ n := n * 10 + Nat32.toNat(d - 48) }};
  }};
  n;
}};
func mvInt(t : Text) : Int {{
  var n : Int = 0;
  var neg = false;
  for (c in t.chars()) {{
    if (c == '-') {{ neg := true }} else {{
      let d = Char.toNat32(c);
      if (d >= 48 and d <= 57) {{ n := n * 10 + Nat32.toNat(d - 48) }};
    }};
  }};
  if (neg) {{ -n }} else {{ n }};
}};
func mvIsEmail(t : Text) : Bool {{
  Text.contains(t, #char '@') and Text.contains(t, #char '.');
}};
func mvFormGet(ctx : MV.Ctx, k : Text) : ?Text {{
  for ((kk, vv) in ctx.form.vals()) {{ if (kk == k) {{ return ?vv }} }};
  null;
}};
func mvParamGet(ctx : MV.Ctx, k : Text) : Text {{
  for ((kk, vv) in ctx.params.vals()) {{ if (kk == k) {{ return vv }} }};
  "";
}};
// Silence "unused" warnings for helpers a given page may not reference.
ignore mvNat; ignore mvInt; ignore mvIsEmail; ignore mvFormGet; ignore mvParamGet;
ignore Html; ignore Charts;

// ---- shared service instances ----
{service_instances}
{component_funcs}
{page_objects}
{layout_funcs}

// ---- mock request context (anonymous, empty, no-op security) ----
// The anonymous principal (2vxsx-fae) — exactly what a query render sees before
// any Internet Identity login. isAuthenticated=false; all token/role functions
// are inert so a page that *reads* them renders, while a page that *requires* a
// real session simply renders its unauthenticated branch.
let mockCtx : MV.Ctx = {{
  method = "GET";
  path = {target_route:?};
  queryParams = [];
  params = [];
  form = [];
  caller = Principal.fromText("2vxsx-fae");
  isAuthenticated = false;
  lastBatchId = "";
  mintToken = func (_h : Text, _s : Text) : Text {{ "" }};
  mintIntentToken = func (_h : Text, _s : Text, _i : [(Text, Text)]) : Text {{ "" }};
  authorizeSpend = func (_h : Text, _i : [(Text, Text)], _t : Text, _w : Nat) : Bool {{ false }};
  mintSpendToken = func (_h : Text, _i : [(Text, Text)]) : Text {{ "" }};
  hasRole = func (_w : Principal, _r : Text) : Bool {{ false }};
  callerRoles = func () : [Text] {{ [] }};
  grantRole = func (_w : Principal, _r : Text) : () {{ () }};
  revokeRole = func (_w : Principal, _r : Text) : () {{ () }};
  claimRole = func (_r : Text) : Bool {{ false }};
}};

// Run the page's data-loading lifecycle, REPLAY any recorded events through the
// page's dispatch, then print the IR forest. With no recorded events this is the
// INITIAL render; with events it is the render AFTER those N dispatches.
// `mvRender` returns the UINode forest as JSON Text (EmitMode::Ir). This single
// line of stdout IS the preview payload.
{target_obj}.mvOnLoad(mockCtx);
{replay}Debug.print({target_obj}.mvRender(mockCtx));
"#,
        extra_imports = extra_imports,
        service_instances = service_instances,
        component_funcs = component_funcs,
        page_objects = page_objects,
        layout_funcs = layout_funcs,
        target_obj = target_obj,
        target_route = target_route,
        replay = replay,
    )
}

/// Emit a Motoko string literal for `s` (double-quoted, with the escapes Motoko
/// accepts). Used to embed recorded handler ids/args/caller text into the replay
/// driver safely.
fn mo_string(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// Parse every `.mview` in the project (pages/layouts/components) and run the
/// security lint pass over each, returning `(source_path, diagnostic)` pairs.
/// Reuses the same file discovery + parser the build uses, so `motoview lint`
/// sees exactly what `motoview build` would lint. A parse error is surfaced as
/// an `Err` (lint can only run on a tree that parses).
pub fn lint_project(project_dir: &Path) -> Result<Vec<(String, lint::Diagnostic)>, String> {
    let src = project_dir.join("src");
    let mut out: Vec<(String, lint::Diagnostic)> = Vec::new();
    let groups = [
        (list_mview(&src.join("Pages")), FileKind::Page),
        (list_mview(&src.join("Layouts")), FileKind::Layout),
        (list_mview(&src.join("Components")), FileKind::Component),
    ];
    for (files, kind) in groups {
        for f in &files {
            let name = file_stem(f);
            let source = fs::read_to_string(f).map_err(|e| format!("{}: {}", f.display(), e))?;
            let file = parser::parse(&source, &name, kind.clone())?;
            let rel = rel_src(project_dir, f);
            for d in lint::lint_file(&file, &rel) {
                out.push((rel.clone(), d));
            }
        }
    }
    Ok(out)
}

/// Public access to the diagnostic formatter so the `lint` CLI prints the same
/// `error:`/`warning:` lines the build does.
pub fn format_lint(diags: &[(String, lint::Diagnostic)]) -> String {
    format_diagnostics(diags)
}

/// Like [`lint_project`], but returns the machine-readable [`lint::JsonDiagnostic`]
/// for each finding — with the offending node's span resolved to 1-based
/// line/col against that file's source. This is what `motoview lint --json`
/// serializes. Each file is re-parsed exactly as [`lint_project`]/`build` does,
/// so the JSON sees identical findings; we additionally keep the source around to
/// resolve positions (`lint::lint_file` returns raw spans).
pub fn lint_project_json(project_dir: &Path) -> Result<Vec<lint::JsonDiagnostic>, String> {
    let src = project_dir.join("src");
    let mut out: Vec<lint::JsonDiagnostic> = Vec::new();
    let groups = [
        (list_mview(&src.join("Pages")), FileKind::Page),
        (list_mview(&src.join("Layouts")), FileKind::Layout),
        (list_mview(&src.join("Components")), FileKind::Component),
    ];
    for (files, kind) in groups {
        for f in &files {
            let name = file_stem(f);
            let source = fs::read_to_string(f).map_err(|e| format!("{}: {}", f.display(), e))?;
            let file = parser::parse(&source, &name, kind.clone())?;
            let rel = rel_src(project_dir, f);
            let chars: Vec<char> = source.chars().collect();
            for d in lint::lint_file(&file, &rel) {
                out.push(lint::JsonDiagnostic::from_lint(&rel, &d, &chars));
            }
        }
    }
    Ok(out)
}

fn assemble(
    app_name: &str,
    page_objects: &str,
    page_records: &str,
    page_idents: &[String],
    layout_funcs: &str,
    layout_entries: &[(String, String)],
    secret: &str,
    extra_imports: &str,
    service_instances: &str,
    persistence: &str,
    component_funcs: &str,
    vetkd_key: &str,
) -> String {
    let pages_arr = page_idents.join(", ");
    let layouts_arr = layout_entries
        .iter()
        .map(|(n, f)| format!("{{ name = {:?}; render = {} }}", n, f))
        .collect::<Vec<_>>()
        .join(", ");

    format!(
        r#"// GENERATED by the MotoView compiler. Do not edit by hand.
// Edit the .mview files in src/Pages, src/Layouts and src/Components instead,
// then re-run `motoview build`.
import App "mo:motoview/App";
import Html "mo:motoview/Html";
import MV "mo:motoview/Types";
import Charts "mo:motoview/Charts";
import VetKeys "mo:motoview/VetKeys";
import Lib "mo:motoview";
import Nat "mo:base/Nat";
import Nat32 "mo:base/Nat32";
import Nat64 "mo:base/Nat64";
import Int "mo:base/Int";
import Float "mo:base/Float";
import Char "mo:base/Char";
import Text "mo:base/Text";
import Buffer "mo:base/Buffer";
import Principal "mo:base/Principal";
import Array "mo:base/Array";
import Iter "mo:base/Iter";
import Option "mo:base/Option";
import Time "mo:base/Time";
import Bool "mo:base/Bool";
import Random "mo:base/Random";
{extra_imports}
actor {{
  // `Context` is the friendly alias for the request context passed to
  // `onLoad(ctx : Context)` and to any handler whose first parameter is `ctx`.
  type Context = MV.Ctx;

  // ---- conversion helpers used by generated event dispatch ----
  func mvNat(t : Text) : Nat {{
    var n : Nat = 0;
    for (c in t.chars()) {{
      let d = Char.toNat32(c);
      if (d >= 48 and d <= 57) {{ n := n * 10 + Nat32.toNat(d - 48) }};
    }};
    n;
  }};
  func mvInt(t : Text) : Int {{
    var n : Int = 0;
    var neg = false;
    for (c in t.chars()) {{
      if (c == '-') {{ neg := true }} else {{
        let d = Char.toNat32(c);
        if (d >= 48 and d <= 57) {{ n := n * 10 + Nat32.toNat(d - 48) }};
      }};
    }};
    if (neg) {{ -n }} else {{ n }};
  }};
  func mvIsEmail(t : Text) : Bool {{
    Text.contains(t, #char '@') and Text.contains(t, #char '.');
  }};
  func mvFormGet(ctx : MV.Ctx, k : Text) : ?Text {{
    for ((kk, vv) in ctx.form.vals()) {{ if (kk == k) {{ return ?vv }} }};
    null;
  }};
  // Read a route param (e.g. {{id:Nat}}) as Text; "" if absent.
  func mvParamGet(ctx : MV.Ctx, k : Text) : Text {{
    for ((kk, vv) in ctx.params.vals()) {{ if (kk == k) {{ return vv }} }};
    "";
  }};

  // ---- shared service instances (stateful services) ----
{service_instances}
{persistence}
{component_funcs}
{page_objects}
{page_records}
{layout_funcs}

  let mvPages : [MV.Page] = [{pages_arr}];
  let mvLayouts : [MV.Layout] = [{layouts_arr}];
  let mvConfig : MV.Config = {{ appName = {app_name:?}; secret = {secret} : Blob; seo = true; altOrigins = [] }};
  let mvApp = App.App(mvConfig, mvPages, mvLayouts, Lib.defaultAssets());

  // Session / secure-form HMAC secret: cryptographically random per canister
  // (from the IC's raw_rand), kept in a stable var so it survives upgrades, and
  // NEVER present in source. Installed lazily on the first update call below;
  // restored into the app instance here after an upgrade.
  stable var mvSecret : Blob = "" : Blob;
  // Per-principal session epochs (logout-everywhere revocation), kept stable.
  stable var mvEpochs : [(Text, Nat)] = [];
  // Role store (principal -> roles), backing `@authorize role="..."`.
  stable var mvRoles : [(Principal, [Text])] = [];
  // Consumed secure-form nonces (replay protection), kept stable so a consumed
  // nonce cannot be replayed after an upgrade.
  stable var mvConsumed : [(Text, Int)] = [];
  // Per-principal wallet spend-velocity log (Slice 9B), kept stable so the
  // rolling-window spend cap survives an upgrade and a leaked session cannot be
  // drained across a redeploy boundary. The record type matches WalletAuth.Entry.
  stable var mvVelocity : [(Text, [{{ ts : Int; weight : Nat }}])] = [];
  if (mvSecret.size() == 32) {{ mvApp.setSecret(mvSecret) }};
  mvApp.setEpochs(mvEpochs);
  mvApp.setRoles(mvRoles);
  mvApp.setConsumed(mvConsumed);
  mvApp.setVelocity(mvVelocity);

  public shared query (msg) func http_request(req : MV.HttpRequest) : async MV.HttpResponse {{
    // vetKeys endpoints must run as updates (they await the management canister).
    if (Text.startsWith(req.url, #text "/_motoview/vetkd/")) {{
      return {{ status_code = 200; headers = []; body = "" : Blob; upgrade = ?true }};
    }};
    mvApp.httpRequest(req, msg.caller);
  }};

  public shared (msg) func http_request_update(req : MV.HttpRequest) : async MV.HttpResponse {{
    if (mvApp.needsSecret()) {{ mvSecret := await Random.blob(); mvApp.setSecret(mvSecret) }};
    // vetKeys: authorized decryption capability for the SESSION caller. The key
    // is bound to the caller's principal, so only they can unwrap it. Runs here
    // (the async actor context) since the App request handler is synchronous.
    if (Text.startsWith(req.url, #text "/_motoview/vetkd/")) {{
      let mvVetCaller = mvApp.effectiveCaller(req, msg.caller);
      let mvVetCtx = Text.encodeUtf8("motoview");
      let mvVetHdrs = [("content-type", "application/octet-stream"), ("cache-control", "no-store")];
      if (Text.startsWith(req.url, #text "/_motoview/vetkd/public-key")) {{
        let pk = await VetKeys.publicKey("{vetkd_key}", mvVetCtx);
        return {{ status_code = 200; headers = mvVetHdrs; body = pk; upgrade = null }};
      }};
      if (Text.startsWith(req.url, #text "/_motoview/vetkd/derive")) {{
        let ek = await VetKeys.deriveKey("{vetkd_key}", mvVetCtx, Principal.toBlob(mvVetCaller), req.body);
        return {{ status_code = 200; headers = mvVetHdrs; body = ek; upgrade = null }};
      }};
    }};
    // Security state (epochs/roles/consumed-nonces/velocity) is held in the App
    // instance and snapshotted to its stable var in `preupgrade`, NOT here — so a
    // normal update no longer copies all four whole maps to stable vars every
    // call. mvSecret is still installed lazily above (one-time, then a no-op).
    mvApp.httpRequestUpdate(req, msg.caller);
  }};

  // Internet Identity login bridge: an authenticated update call whose caller
  // the IC has verified. Records the principal under the client's nonce so a
  // following GET /mv-session can mint a session token bound to it.
  public shared (msg) func mvEstablish(nonce : Text) : async () {{
    if (mvApp.needsSecret()) {{ mvSecret := await Random.blob(); mvApp.setSecret(mvSecret) }};
    mvApp.establish(nonce, msg.caller);
  }};

  // vetKeys: threshold-derived encryption keys. The derived key is bound to the
  // authenticated caller's principal (per-user). The client unwraps it + runs the
  // IBE encrypt/decrypt (BLS12-381 in the Rust brain). The key name is selected by
  // the build network: local replicas use `dfx_test_key`, mainnet (`--network ic`)
  // uses `key_1` — enforced by the compiler's network gate. See VetKeys.mo +
  // docs/security.md.
  public shared query (msg) func mvVetkdContext() : async Text {{ ignore msg; "motoview" }};
  public shared (msg) func mvVetkdPublicKey() : async Blob {{
    ignore msg;
    await VetKeys.publicKey("{vetkd_key}", Text.encodeUtf8("motoview"));
  }};
  public shared (msg) func mvVetkdDeriveKey(transportKey : Blob) : async Blob {{
    await VetKeys.deriveKey("{vetkd_key}", Text.encodeUtf8("motoview"), Principal.toBlob(msg.caller), transportKey);
  }};
}};
"#,
        app_name = app_name,
        page_objects = page_objects,
        page_records = page_records,
        layout_funcs = layout_funcs,
        pages_arr = pages_arr,
        layouts_arr = layouts_arr,
        secret = secret,
        extra_imports = extra_imports,
        service_instances = service_instances,
        persistence = persistence,
        component_funcs = component_funcs,
        vetkd_key = vetkd_key,
    )
}

/// Format accumulated lint diagnostics for printing: `error:`/`warning:` lines
/// mapped to their `.mview` source, sorted errors-first for readability.
fn format_diagnostics(diags: &[(String, lint::Diagnostic)]) -> String {
    let mut out = String::new();
    let mut ordered: Vec<&(String, lint::Diagnostic)> = diags.iter().collect();
    ordered.sort_by_key(|(_, d)| match d.severity {
        Severity::Error => 0,
        Severity::Warning => 1,
    });
    for (_, d) in ordered {
        let label = match d.severity {
            Severity::Error => "error",
            Severity::Warning => "warning",
        };
        out.push_str(&format!(
            "{}: [{}] {}\n  --> {}\n",
            label, d.rule, d.message, d.location
        ));
    }
    out
}

/// A service file is "stateful" when it exports a `public class <Name>()` whose
/// name matches the file stem. The compiler then instantiates one shared
/// instance at actor scope (see `build`). Otherwise it is a stateless module.
fn is_stateful_service(content: &str, name: &str) -> bool {
    let needle = format!("public class {}", name);
    // A real declaration starts its (trimmed) line — ignore the string occurring
    // inside an embedded string literal/comment (e.g. a docs page about services).
    content.lines().any(|l| {
        let l = l.trim_start();
        l.starts_with(&needle)
            && matches!(
                l[needle.len()..].chars().next(),
                Some('(') | Some(' ') | Some('<') | Some('\t')
            )
    })
}

/// Merge runs of consecutive `b.raw("literal")` statements into a single call,
/// so the generated render code reads as contiguous HTML chunks broken only at
/// real dynamic boundaries (b.text / b.attr / control flow), not one call per
/// token. Purely cosmetic — byte-identical rendered output.
fn coalesce_raw(src: &str) -> String {
    let mut out = String::with_capacity(src.len());
    let mut run: Option<(String, String)> = None; // (indent, accumulated escaped inner)
    fn flush(out: &mut String, run: &mut Option<(String, String)>) {
        if let Some((indent, inner)) = run.take() {
            out.push_str(&indent);
            out.push_str("b.raw(\"");
            out.push_str(&inner);
            out.push_str("\");\n");
        }
    }
    for line in src.lines() {
        match raw_literal(line) {
            Some((indent, inner)) => match &mut run {
                Some((ri, acc)) if ri == indent => acc.push_str(inner),
                _ => {
                    flush(&mut out, &mut run);
                    run = Some((indent.to_string(), inner.to_string()));
                }
            },
            None => {
                flush(&mut out, &mut run);
                out.push_str(line);
                out.push('\n');
            }
        }
    }
    flush(&mut out, &mut run);
    out
}

/// If `line` is exactly `<indent>b.raw("<escaped>");`, return (indent, escaped).
/// Rejects anything else (a `b.raw(ident)` call, a concatenation, etc.).
fn raw_literal(line: &str) -> Option<(&str, &str)> {
    let l = line.trim_end();
    let trimmed = l.trim_start();
    let indent = &l[..l.len() - trimmed.len()];
    let inner = trimmed.strip_prefix("b.raw(\"")?.strip_suffix("\");")?;
    // A single string literal has no UNescaped `"` in its body.
    let bytes = inner.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'\\' => i += 2,
            b'"' => return None,
            _ => i += 1,
        }
    }
    Some((indent, inner))
}

/// Scan Motoko source for record type definitions — `[public ]type Name = { f :
/// T; ... }` — and add `name -> (field -> type)` to `out`. `qualifier` (a service
/// name) also registers a `Qualifier.Name` alias. Best-effort: complex/unparsed
/// types simply don't get an entry (the codegen falls back to debug_show).
fn scan_types(content: &str, qualifier: Option<&str>, out: &mut HashMap<String, HashMap<String, String>>) {
    let bytes = content.as_bytes();
    let mut search = 0usize;
    while let Some(rel) = content[search..].find("type ") {
        let kw = search + rel;
        search = kw + 5;
        // must start a token (preceded by start / whitespace).
        if kw > 0 && !bytes[kw - 1].is_ascii_whitespace() {
            continue;
        }
        let rest = &content[kw + 5..];
        let name_end = rest
            .find(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
            .unwrap_or(rest.len());
        let name = rest[..name_end].trim();
        if name.is_empty() {
            continue;
        }
        let tail = rest[name_end..].trim_start();
        let body = match tail.strip_prefix('=') {
            Some(b) => b.trim_start(),
            None => continue,
        };
        if !body.starts_with('{') {
            continue; // not a record type (alias, variant, etc.)
        }
        if let Some(inner) = read_braced(body) {
            let fields = parse_record_fields(&inner);
            if !fields.is_empty() {
                out.entry(name.to_string()).or_default().extend(fields.clone());
                if let Some(q) = qualifier {
                    out.entry(format!("{}.{}", q, name)).or_default().extend(fields);
                }
            }
        }
    }
}

/// Read a balanced `{ ... }` (s must start with `{`), returning the inner text.
fn read_braced(s: &str) -> Option<String> {
    let mut depth = 0i32;
    let mut inner = String::new();
    for c in s.chars() {
        match c {
            '{' => {
                depth += 1;
                if depth > 1 {
                    inner.push(c);
                }
            }
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(inner);
                }
                inner.push(c);
            }
            _ if depth >= 1 => inner.push(c),
            _ => {}
        }
    }
    None
}

/// Parse `field : type; field : type` (depth-aware on `;`) into a field->type map.
fn parse_record_fields(inner: &str) -> HashMap<String, String> {
    let mut fields = HashMap::new();
    let mut depth = 0i32;
    let mut seg = String::new();
    let mut commit = |seg: &str, fields: &mut HashMap<String, String>| {
        if let Some(colon) = seg.find(':') {
            let name = seg[..colon].trim();
            let ty = seg[colon + 1..].trim();
            if !name.is_empty()
                && !ty.is_empty()
                && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
            {
                fields.insert(name.to_string(), ty.to_string());
            }
        }
    };
    for c in inner.chars() {
        match c {
            '{' | '(' | '[' => {
                depth += 1;
                seg.push(c);
            }
            '}' | ')' | ']' => {
                depth -= 1;
                seg.push(c);
            }
            ';' if depth == 0 => {
                commit(&seg, &mut fields);
                seg.clear();
            }
            _ => seg.push(c),
        }
    }
    commit(&seg, &mut fields);
    fields
}

/// Relative path from `from` dir to `to` dir, e.g. (.mvbuild, src) -> "../src".
/// Empty when they're the same directory.
fn rel_path_between(from: &Path, to: &Path) -> String {
    let f: Vec<_> = from.components().collect();
    let t: Vec<_> = to.components().collect();
    let common = f.iter().zip(t.iter()).take_while(|(a, b)| a == b).count();
    let mut parts: Vec<String> = std::iter::repeat("..".to_string()).take(f.len() - common).collect();
    for c in &t[common..] {
        parts.push(c.as_os_str().to_string_lossy().to_string());
    }
    parts.join("/")
}

/// Sidecar path for the generated->source line map: `<out>.map` (e.g.
/// `.mvbuild/main.mo.map`). A SIDE artifact — never part of `main.mo`.
pub fn source_map_path(out: &Path) -> PathBuf {
    let mut name = out.file_name().map(|s| s.to_os_string()).unwrap_or_default();
    name.push(".map");
    out.with_file_name(name)
}

/// Load the generated->source [`SourceMap`] sidecar for `out` (the generated
/// `main.mo`). A missing/unreadable map yields an EMPTY map — callers then fall
/// back to the `// mv:src` FILE markers, exactly as before R11.
pub fn load_source_map(out: &Path) -> SourceMap {
    let path = source_map_path(out);
    match fs::read_to_string(&path) {
        Ok(text) => SourceMap::parse(&text),
        Err(_) => SourceMap::new(),
    }
}

/// Path of a source file relative to the project dir (for `// mv:src` markers).
fn rel_src(project_dir: &Path, file: &Path) -> String {
    file.strip_prefix(project_dir)
        .unwrap_or(file)
        .to_string_lossy()
        .replace('\\', "/")
}

/// R12 token-anchored column resolution.
///
/// R11 maps a `moc` error's GENERATED line back to the originating `.mview` line.
/// The COLUMN, however, is moc's column into the GENERATED actor text. For most
/// `@code` lines that already equals the source column (codegen preserves source
/// indentation byte-for-byte). It DRIFTS only when the line carries an in-line
/// transform that shifts text horizontally: `await`-stripping (codegen removes
/// `"await "`), `validate { … }` translation, identifier rewriting
/// (`replace_ident_in_code`). After such a transform a token to its right lands
/// at a smaller generated column than its true source column.
///
/// This helper re-anchors the column by the offending TOKEN's text:
///   1. slice the GENERATED line at moc's `[gen_col, gen_end_col)` to get `T`,
///   2. find `T` verbatim in the resolved SOURCE line (the occurrence nearest the
///      column moc reported), and
///   3. report `T`'s 1-based SOURCE column + `end = col + T.chars().count()`.
///
/// It is deliberately CONSERVATIVE: it returns `None` (caller keeps moc's column,
/// never worse than today) whenever `T` is empty/whitespace, the range spans
/// multiple lines, or `T` is not present verbatim in the source line (e.g. a
/// RENAMED identifier — `replace_ident_in_code` made the generated text differ
/// from the source, so there is nothing to anchor to).
///
/// All columns are 1-based and counted in CHARACTERS (matching moc + [`span::line_col`]).
/// `gen_line`/`source_line` are the raw line TEXT (no trailing newline).
fn anchor_token_column(
    gen_line: &str,
    source_line: &str,
    gen_col: usize,
    gen_end_col: usize,
    gen_line_no: usize,
    gen_end_line_no: usize,
) -> Option<(u32, u32)> {
    // Multi-line spans have no single source line to anchor onto.
    if gen_end_line_no != gen_line_no {
        return None;
    }
    if gen_col == 0 {
        return None; // unparsed position — leave the column as-is.
    }
    let gen_chars: Vec<char> = gen_line.chars().collect();
    // moc columns are 1-based; convert to 0-based char indices into the line.
    let start = gen_col.saturating_sub(1);
    // A point span (`col == end_col`) means moc fingered a single position; widen
    // it to the identifier/operator token starting there so we have text to anchor.
    let end = if gen_end_col > gen_col {
        gen_end_col.saturating_sub(1)
    } else {
        // Grow to the end of the token at `start` (alnum/_ run, else one char).
        let mut e = start;
        while e < gen_chars.len() && is_ident_char(gen_chars[e]) {
            e += 1;
        }
        if e == start {
            (start + 1).min(gen_chars.len())
        } else {
            e
        }
    };
    if start >= gen_chars.len() || end <= start || end > gen_chars.len() {
        return None;
    }
    let token: String = gen_chars[start..end].iter().collect();
    if token.trim().is_empty() {
        return None; // whitespace-only — nothing to anchor.
    }

    // Find `token` in the source line. Prefer the occurrence whose source column is
    // nearest the generated column (so a token that appears more than once snaps to
    // the right one). We search by CHAR index to keep columns char-based.
    let src_chars: Vec<char> = source_line.chars().collect();
    let tok_chars: Vec<char> = token.chars().collect();
    let mut best: Option<usize> = None; // 0-based char index of the match start.
    let mut best_dist = usize::MAX;
    let predicted = start; // expect the source token at >= the generated column.
    let mut i = 0usize;
    while i + tok_chars.len() <= src_chars.len() {
        if src_chars[i..i + tok_chars.len()] == tok_chars[..] {
            // The transform only ever shifts text RIGHTWARD in source vs. generated
            // (codegen removes/contracts text), so the true source column is >= the
            // generated one. Bias toward matches at-or-after `predicted`, but accept
            // any if none qualify.
            let dist = if i >= predicted { i - predicted } else { predicted - i + src_chars.len() };
            if dist < best_dist {
                best_dist = dist;
                best = Some(i);
            }
        }
        i += 1;
    }
    let idx = best?;
    let col = (idx + 1) as u32; // 1-based source column.
    let end_col = (idx + 1 + tok_chars.len()) as u32;
    Some((col, end_col))
}

/// Whether `c` can be part of a Motoko identifier (used to widen a point span to
/// the whole token when moc reports `col == end_col`).
fn is_ident_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// Read the `.mview`/source line `line_no` (1-based) of `file` (project-relative)
/// under `project_dir`, returning the raw line TEXT (no newline). Caches files so
/// a report with many errors in one file reads it once. Any IO/range failure
/// yields `None` (the caller then keeps moc's column — never worse than today).
struct SourceLineLookup<'a> {
    project_dir: Option<&'a Path>,
    cache: HashMap<String, Option<Vec<String>>>,
}

impl<'a> SourceLineLookup<'a> {
    fn new(project_dir: Option<&'a Path>) -> Self {
        SourceLineLookup { project_dir, cache: HashMap::new() }
    }

    fn line(&mut self, file: &str, line_no: usize) -> Option<String> {
        let dir = self.project_dir?;
        if line_no == 0 {
            return None;
        }
        let entry = self.cache.entry(file.to_string()).or_insert_with(|| {
            let path = dir.join(file);
            fs::read_to_string(&path)
                .ok()
                .map(|t| t.lines().map(|s| s.to_string()).collect())
        });
        entry.as_ref().and_then(|lines| lines.get(line_no - 1).cloned())
    }
}

/// Rewrite `moc` errors that point at the generated `main.mo` so they name the
/// originating `.mview`/source region instead. Uses the `// mv:src <path>`
/// markers emitted per page/component/layout for the FILE, and (R11) the
/// generated->source [`SourceMap`] for the precise `.mview` LINE of `@code`
/// errors. Returns the mapped report and whether any errors were found.
///
/// When a [`SourceMap`] region covers the generated line, the report names the
/// `.mview` file AND line (`src:LINE`); otherwise it falls back to the file from
/// the `// mv:src` marker (line unknown) and always notes the generated line.
///
/// The human report does not print columns, so `project_dir` (used by the JSON
/// path for R12 token-anchored columns) is accepted for signature symmetry but
/// not consulted here — the output is byte-for-byte the same as before R12.
pub fn map_moc_errors(
    main_mo: &str,
    moc_output: &str,
    source_map: &SourceMap,
    _project_dir: Option<&Path>,
) -> (String, bool) {
    // (generated line number, source path) for each marker, in order.
    let mut markers: Vec<(usize, String)> = Vec::new();
    for (i, line) in main_mo.lines().enumerate() {
        if let Some(rest) = line.trim_start().strip_prefix("// mv:src ") {
            markers.push((i + 1, rest.trim().to_string()));
        }
    }
    let src_for = |gen_line: usize| -> Option<&String> {
        markers
            .iter()
            .rev()
            .find(|(l, _)| *l <= gen_line)
            .map(|(_, p)| p)
    };
    let mut out = String::new();
    let mut had_error = false;
    for line in moc_output.lines() {
        // e.g. ".../main.mo:6519.5-6519.11: syntax error [M0001], unexpected ..."
        if let Some(pos) = line.find("main.mo:") {
            let after = &line[pos + "main.mo:".len()..];
            let gen_line: usize = after
                .split(|c: char| !c.is_ascii_digit())
                .next()
                .and_then(|s| s.parse().ok())
                .unwrap_or(0);
            let is_err = line.contains("error");
            if is_err {
                had_error = true;
            }
            let msg = line.splitn(2, ": ").nth(1).unwrap_or(line);
            // Prefer the precise (.mview file, .mview line) from the source map; it
            // is the headline R11 behaviour. Fall back to the marker FILE, else the
            // raw generated line.
            match source_map.resolve(gen_line) {
                Some((src, src_line)) => out.push_str(&format!(
                    "{}  ({}:{}, generated main.mo:{})\n",
                    msg, src, src_line, gen_line
                )),
                None => match src_for(gen_line) {
                    Some(src) => out.push_str(&format!("{}  ({}, generated main.mo:{})\n", msg, src, gen_line)),
                    None => out.push_str(&format!("{}  (generated main.mo:{})\n", msg, gen_line)),
                },
            }
        } else if !line.trim().is_empty() {
            out.push_str(line);
            out.push('\n');
        }
    }
    (out, had_error)
}

/// Machine-readable variant of [`map_moc_errors`]: parse `moc`'s output into
/// structured [`lint::JsonDiagnostic`]s, mapping each `main.mo:LINE.COL` position
/// back to the originating `.mview` (via the `// mv:src` markers) where possible.
///
/// The shape is IDENTICAL to the lint `--json` diagnostics so the editor (R6) /
/// repair loop (R5) consume one schema. `rule` is `"type-check"` for moc errors
/// and `"moc"` for warnings/notes; `severity` is mapped from moc's own label.
/// `file`/`line` are the `.mview` source path AND LINE when the generated->source
/// [`SourceMap`] (R11) covers the generated line; otherwise `file` falls back to
/// the `// mv:src` marker (line stays moc's generated line).
///
/// `col`/`endCol` are R12 token-anchored to the `.mview` SOURCE column whenever the
/// generated line resolves to a `.mview` line AND the offending token is found
/// verbatim in that source line (so an `await`-stripped / transform-shifted token
/// reports its true source column, not the contracted generated one — see
/// [`anchor_token_column`]). When `project_dir` is `None`, the line cannot be read,
/// or the token is absent (e.g. a RENAMED identifier), they FALL BACK to moc's
/// generated column (never worse than before R12). `endLine`/`endCol` otherwise
/// carry moc's end position when present (`LINE.COL-LINE.COL`), else equal start.
pub fn map_moc_errors_json(
    main_mo: &str,
    moc_output: &str,
    source_map: &SourceMap,
    project_dir: Option<&Path>,
) -> Vec<lint::JsonDiagnostic> {
    // (generated line number, source path) for each marker, in order.
    let mut markers: Vec<(usize, String)> = Vec::new();
    for (i, line) in main_mo.lines().enumerate() {
        if let Some(rest) = line.trim_start().strip_prefix("// mv:src ") {
            markers.push((i + 1, rest.trim().to_string()));
        }
    }
    let src_for = |gen_line: usize| -> Option<&String> {
        markers
            .iter()
            .rev()
            .find(|(l, _)| *l <= gen_line)
            .map(|(_, p)| p)
    };
    // R12: the GENERATED actor lines (to slice the offending token) + a cached
    // reader for the `.mview` SOURCE lines (to re-anchor the column onto them).
    let gen_lines: Vec<&str> = main_mo.lines().collect();
    let mut src_lookup = SourceLineLookup::new(project_dir);
    let mut out: Vec<lint::JsonDiagnostic> = Vec::new();
    for line in moc_output.lines() {
        // e.g. ".../main.mo:6519.5-6519.11: syntax error [M0001], unexpected ..."
        let pos = match line.find("main.mo:") {
            Some(p) => p,
            None => continue, // continuation/blank lines carry no position
        };
        let after = &line[pos + "main.mo:".len()..];
        // Parse `LINE.COL` and an optional `-LINE.COL` end before the `:`.
        let (line_no, col, end_line, end_col) = parse_moc_position(after);
        // moc labels the severity in the message body (`... error ...`/`warning`).
        let lower = line.to_ascii_lowercase();
        let severity = if lower.contains("warning") && !lower.contains("error") {
            lint::Severity::Warning
        } else {
            lint::Severity::Error
        };
        let rule = match severity {
            lint::Severity::Error => "type-check",
            lint::Severity::Warning => "moc",
        };
        let message = line.splitn(2, ": ").nth(1).unwrap_or(line).trim().to_string();
        // R11: remap the generated start/end LINE to the `.mview` line via the
        // source map; the column stays moc's (a per-line approximation — the
        // generated body is indented like the source, so it is usually close).
        // Fall back to the `// mv:src` marker FILE (keeping moc's generated line)
        // when no mapped region covers the error.
        let (file, line_out, end_line_out) = match source_map.resolve(line_no) {
            Some((src, src_line)) => {
                // Map the end line through the same region when it resolves there;
                // otherwise keep it equal to the (now-mapped) start line.
                let end_src = source_map
                    .resolve(end_line)
                    .map(|(_, l)| l)
                    .unwrap_or(src_line);
                (src, src_line as u32, end_src as u32)
            }
            None => (
                src_for(line_no).cloned().unwrap_or_else(|| "main.mo".to_string()),
                line_no as u32,
                end_line as u32,
            ),
        };
        // R12: re-anchor the COLUMN onto the `.mview` source line. Only attempt it
        // when the start line mapped to a `.mview` line (line_out != generated) AND
        // we can read that source line; otherwise keep moc's column (the prior
        // behaviour — never worse). The token is sliced from the GENERATED line.
        let (mut col_out, mut end_col_out) = (col as u32, end_col as u32);
        let mapped_to_source = source_map.resolve(line_no).is_some();
        if mapped_to_source {
            if let Some(gen_text) = gen_lines.get(line_no.saturating_sub(1)) {
                if let Some(src_text) = src_lookup.line(&file, line_out as usize) {
                    if let Some((c, ec)) =
                        anchor_token_column(gen_text, &src_text, col, end_col, line_no, end_line)
                    {
                        col_out = c;
                        end_col_out = ec;
                    }
                }
            }
        }
        out.push(lint::JsonDiagnostic {
            severity,
            rule: rule.to_string(),
            message,
            file,
            line: line_out,
            col: col_out,
            end_line: end_line_out,
            end_col: end_col_out,
        });
    }
    out
}

/// Parse a moc position string of the form `LINE.COL` or `LINE.COL-LINE.COL`
/// (the text right after `main.mo:`), returning `(line, col, end_line, end_col)`.
/// The end defaults to the start when moc gives only a point. Non-numeric or
/// missing fields fall back to `0`.
fn parse_moc_position(after: &str) -> (usize, usize, usize, usize) {
    // Take up to the `:` that ends the position span.
    let head = after.split(':').next().unwrap_or("");
    let mut parts = head.splitn(2, '-');
    let start = parts.next().unwrap_or("");
    let end = parts.next();
    let (line, col) = split_line_col(start);
    let (end_line, end_col) = match end {
        Some(e) => split_line_col(e),
        None => (line, col),
    };
    (line, col, end_line, end_col)
}

/// Split `"LINE.COL"` into `(line, col)`; missing parts are `0`.
fn split_line_col(s: &str) -> (usize, usize) {
    let mut it = s.splitn(2, '.');
    let line = it.next().and_then(|x| x.trim().parse().ok()).unwrap_or(0);
    let col = it.next().and_then(|x| x.trim().parse().ok()).unwrap_or(0);
    (line, col)
}

fn list_mview(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    collect_mview(dir, &mut out);
    out.sort();
    out
}

/// List top-level `.mo` files directly in `dir` (non-recursive).
fn list_mo(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    if let Ok(entries) = fs::read_dir(dir) {
        for e in entries.flatten() {
            let p = e.path();
            if p.is_file() && p.extension().and_then(|s| s.to_str()) == Some("mo") {
                out.push(p);
            }
        }
    }
    out.sort();
    out
}

fn collect_mview(dir: &Path, out: &mut Vec<PathBuf>) {
    if let Ok(entries) = fs::read_dir(dir) {
        for e in entries.flatten() {
            let p = e.path();
            if p.is_dir() {
                collect_mview(&p, out);
            } else if p.extension().and_then(|s| s.to_str()) == Some("mview") {
                out.push(p);
            }
        }
    }
}

fn file_stem(p: &Path) -> String {
    p.file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("Page")
        .to_string()
}

#[cfg(test)]
mod coalesce_tests {
    use super::coalesce_raw;

    #[test]
    fn merges_adjacent_static_raw() {
        let src = "      b.raw(\"</div>\");\n      b.raw(\"\\n   \");\n      b.raw(\"<div>\");\n";
        assert_eq!(coalesce_raw(src), "      b.raw(\"</div>\\n   <div>\");\n");
    }

    #[test]
    fn dynamic_calls_break_the_run() {
        let src = "  b.raw(\"<p>\");\n  b.text(name);\n  b.raw(\"</p>\");\n";
        let out = coalesce_raw(src);
        assert!(out.contains("b.raw(\"<p>\");"));
        assert!(out.contains("b.text(name);"));
        assert!(out.contains("b.raw(\"</p>\");"));
    }

    #[test]
    fn does_not_merge_raw_identifier() {
        let src = "  b.raw(\"<x>\");\n  b.raw(mvBody);\n  b.raw(\"</x>\");\n";
        let out = coalesce_raw(src);
        assert!(out.contains("b.raw(mvBody);"), "mvBody must stay its own call:\n{out}");
        assert!(out.contains("b.raw(\"<x>\");"));
    }

    #[test]
    fn preserves_escaped_quotes() {
        let src = "  b.raw(\"<a href=\\\"/x\\\">\");\n  b.raw(\"y\");\n";
        assert_eq!(coalesce_raw(src), "  b.raw(\"<a href=\\\"/x\\\">y\");\n");
    }

    #[test]
    fn leaves_non_raw_lines_untouched() {
        let src = "  // mv:src src/Pages/Home.mview\n  b.raw(\"<a>\");\n  b.raw(\"<b>\");\n  let x = 1;\n";
        let out = coalesce_raw(src);
        assert!(out.contains("// mv:src src/Pages/Home.mview"));
        assert!(out.contains("let x = 1;"));
        assert!(out.contains("b.raw(\"<a><b>\");"));
    }
}

#[cfg(test)]
mod scan_tests {
    use super::scan_types;
    use std::collections::HashMap;

    #[test]
    fn scans_record_fields_and_qualified_alias() {
        let mut m: HashMap<String, HashMap<String, String>> = HashMap::new();
        scan_types(
            "module { public type Product = { id : Nat; name : Text; tags : [Text]; sub : { a : Nat } }; }",
            Some("Catalog"),
            &mut m,
        );
        assert_eq!(m["Product"]["name"], "Text");
        assert_eq!(m["Product"]["id"], "Nat");
        assert_eq!(m["Product"]["tags"], "[Text]");           // depth-aware split keeps [Text] whole
        assert_eq!(m["Product"]["sub"], "{ a : Nat }");        // nested record kept as-is
        assert!(m.contains_key("Catalog.Product"));            // qualified alias too
    }

    #[test]
    fn ignores_non_record_types() {
        let mut m: HashMap<String, HashMap<String, String>> = HashMap::new();
        scan_types("type Color = { #red; #green }; type Id = Nat;", None, &mut m);
        // variant + alias have no `field : type` pairs we can use
        assert!(m.get("Id").is_none());
    }
}

/// R13 unit tests for the pure decl/expr scan helpers — no IO, no moc. They lock
/// the head-parsing + emit-site recognition the source-map builder relies on.
#[cfg(test)]
mod r13_helper_tests {
    use super::*;

    #[test]
    fn split_keyword_recognises_decl_heads() {
        assert_eq!(split_keyword("var count : Nat = 0;").map(|(k, _)| k), Some("var"));
        assert_eq!(split_keyword("let x = 1;").map(|(k, _)| k), Some("let"));
        assert_eq!(split_keyword("type T = { a : Nat };").map(|(k, _)| k), Some("type"));
        assert_eq!(split_keyword("stable var s : Nat = 0;").map(|(k, _)| k), Some("stable"));
        // Not a keyword boundary -> None (must not match a larger identifier).
        assert_eq!(split_keyword("lettuce := 1;"), None);
        assert_eq!(split_keyword("variable := 1;"), None);
        // An expression / call is not a decl head.
        assert_eq!(split_keyword("foo();"), None);
    }

    #[test]
    fn decl_ident_extracts_the_leading_identifier() {
        assert_eq!(decl_ident("count : Nat = 0").as_deref(), Some("count"));
        assert_eq!(decl_ident("  spaced : Text").as_deref(), Some("spaced"));
        // A destructuring binding has no plain leading identifier -> None.
        assert_eq!(decl_ident("(a, b) = pair"), None);
    }

    #[test]
    fn parse_emitted_decl_head_handles_var_let_type_and_stable() {
        assert_eq!(
            parse_emitted_decl_head("var count : Nat = 0;"),
            Some(("var".to_string(), "count".to_string()))
        );
        assert_eq!(
            parse_emitted_decl_head("let helper = 3;"),
            Some(("let".to_string(), "helper".to_string()))
        );
        assert_eq!(
            parse_emitted_decl_head("type Row = { id : Nat };"),
            Some(("type".to_string(), "Row".to_string()))
        );
        // `stable var s …` -> kind `stable`, name is the VAR name (matches how
        // var_src_infos records a stable var).
        assert_eq!(
            parse_emitted_decl_head("stable var s : Nat = 0;"),
            Some(("stable".to_string(), "s".to_string()))
        );
        // A func / public func is NOT a decl head we anchor as a var.
        assert_eq!(parse_emitted_decl_head("public func mvRender(ctx : MV.Ctx) : Text {"), None);
        assert_eq!(parse_emitted_decl_head("func increment(by : Nat) : () {"), None);
    }

    #[test]
    fn nonliteral_text_or_raw_distinguishes_dynamic_from_literal() {
        // Dynamic interpolations (what @(expr)/@raw(expr) compile to).
        assert!(is_nonliteral_text_or_raw("b.text(Nat.toText(count));"));
        assert!(is_nonliteral_text_or_raw("b.raw(mvBody);"));
        assert!(is_nonliteral_text_or_raw("b.raw(someExpr);"));
        // A static literal chunk (a coalesced b.raw("…")) is NOT counted.
        assert!(!is_nonliteral_text_or_raw("b.raw(\"<div>literal</div>\");"));
        // Even a b.text with a string-literal arg IS counted as dynamic — b.text is
        // only ever emitted for interpolations, never for static text (which is
        // b.raw). The reliability gate handles any over/under-count by falling back.
        assert!(is_nonliteral_text_or_raw("b.text(\"a builtin title literal\");"));
        // Unrelated builder calls are not emit sites.
        assert!(!is_nonliteral_text_or_raw("b.attr(\"value\", x);"));
        assert!(!is_nonliteral_text_or_raw("b.build();"));
    }

    #[test]
    fn find_decl_head_is_token_aligned() {
        let src = "@code {\n  let helper = 3;\n  let helperX = 4;\n}";
        // `let helper` must match the FIRST decl, not the prefix inside `helperX`.
        let off = find_decl_head(src, "let helper").expect("found");
        assert_eq!(&src[off..off + "let helper".len()], "let helper");
        // The matched occurrence's following char is a space (whole-name match), so
        // it is the `let helper = 3;` line, not `let helperX`.
        assert_eq!(src.as_bytes()[off + "let helper".len()], b' ');
    }
}

#[cfg(test)]
mod path_tests {
    use super::rel_path_between;
    use std::path::Path;
    #[test]
    fn mvbuild_to_src_is_dotdot_src() {
        assert_eq!(rel_path_between(Path::new("examples/crm/.mvbuild"), Path::new("examples/crm/src")), "../src");
        assert_eq!(rel_path_between(Path::new("a/b/.mvbuild"), Path::new("a/b/src")), "../src");
    }
    #[test]
    fn same_dir_is_empty() {
        assert_eq!(rel_path_between(Path::new("app/src"), Path::new("app/src")), "");
    }
}

/// R12 unit tests for the pure token-anchor core: given a generated line, the
/// resolved source line, and moc's columns, it returns the SOURCE column of the
/// offending token (or `None` to fall back to moc's column). No file IO here.
#[cfg(test)]
mod anchor_tests {
    use super::anchor_token_column;

    #[test]
    fn await_strip_reanchors_token_to_source_column() {
        // Source has `await ` (codegen strips it), so the token `nope` sits 6 cols
        // further right in the SOURCE than in the GENERATED line. moc reports the
        // generated column; the anchor must report the SOURCE column.
        let gen = "      ignore (mvNoop()); let _b : Nat = nope;";
        let src = "      ignore (await mvNoop()); let _b : Nat = nope;";
        // `nope` in the generated line: find it to compute moc's reported cols.
        let g0 = gen.find("nope").unwrap() + 1; // 1-based
        let g1 = g0 + "nope".len();
        let (col, end_col) = anchor_token_column(gen, src, g0, g1, 10, 10).expect("anchored");
        let s0 = src.find("nope").unwrap() + 1;
        assert_eq!(col as usize, s0, "token re-anchored to its SOURCE column");
        assert_eq!(end_col as usize, s0 + "nope".len());
        assert!(col as usize > g0, "source column is to the RIGHT of the generated one");
    }

    #[test]
    fn simple_line_keeps_exact_source_column() {
        // No transform: generated == source, so the anchored column equals moc's.
        let line = "        let _b : Nat = nope;";
        let g0 = line.find("nope").unwrap() + 1;
        let g1 = g0 + "nope".len();
        let (col, end_col) = anchor_token_column(line, line, g0, g1, 5, 5).expect("anchored");
        assert_eq!(col as usize, g0, "unchanged column for a non-transformed line");
        assert_eq!(end_col as usize, g1);
    }

    #[test]
    fn renamed_identifier_falls_back_to_none() {
        // `replace_ident_in_code` rewrote `oldName` -> `newName` in the generated
        // text; the generated token is absent from the source line, so anchoring
        // must DECLINE (caller keeps moc's column — never worse).
        let gen = "      ignore newName;";
        let src = "      ignore oldName;";
        let g0 = gen.find("newName").unwrap() + 1;
        let g1 = g0 + "newName".len();
        assert_eq!(anchor_token_column(gen, src, g0, g1, 3, 3), None);
    }

    #[test]
    fn multi_line_span_declines() {
        let gen = "      let x = foo(";
        let src = "      let x = foo(";
        // end line != start line -> no single source line to anchor onto.
        assert_eq!(anchor_token_column(gen, src, 7, 4, 3, 4), None);
    }

    #[test]
    fn whitespace_or_empty_token_declines() {
        let gen = "      let x = 1;";
        let src = "      let x = 1;";
        // A zero-width point on a space widens to one char; a pure-space slice declines.
        // Force the start onto a run of spaces with end_col past it.
        assert_eq!(anchor_token_column(gen, src, 1, 4, 1, 1), None);
        // col == 0 (unparsed) -> decline.
        assert_eq!(anchor_token_column(gen, src, 0, 0, 1, 1), None);
    }

    #[test]
    fn point_span_widens_to_whole_token() {
        // moc sometimes reports a point (col == end_col) at the token start; the
        // anchor widens it to the full identifier and re-anchors both ends.
        let gen = "      ignore (mvNoop()); let _b : Nat = nope;";
        let src = "      ignore (await mvNoop()); let _b : Nat = nope;";
        let g0 = gen.find("nope").unwrap() + 1;
        let (col, end_col) = anchor_token_column(gen, src, g0, g0, 9, 9).expect("anchored");
        let s0 = src.find("nope").unwrap() + 1;
        assert_eq!(col as usize, s0);
        assert_eq!(end_col as usize, s0 + "nope".len(), "widened to the whole token");
    }

    #[test]
    fn nearest_occurrence_is_chosen_for_repeated_token() {
        // `x` appears twice in the source; the anchor should pick the occurrence
        // nearest (at-or-after) the generated column, not the first one.
        let gen = "  x + x_marker_here_x;"; // generated col of the 2nd `x` region
        let src = "  x + x_marker_here_x;";
        // Target the standalone leading `x` at col 3.
        let (col, _ec) = anchor_token_column(gen, src, 3, 4, 1, 1).expect("anchored");
        assert_eq!(col, 3, "the leading x at its own column");
    }
}
