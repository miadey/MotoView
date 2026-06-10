//! `motoview fmt` — a CONSERVATIVE, SEMANTICS-PRESERVING `.mview` formatter.
//!
//! ## Why this formatter is so cautious
//!
//! A `.mview` file is template markup + a Motoko `@code` block. The template is
//! whitespace-sensitive: inter-element whitespace and text become [`Node::Text`]
//! nodes that the codegen emits verbatim into the rendered HTML. So *reflowing*
//! template text — re-indenting tags, collapsing spaces between elements, moving
//! a `</p>` onto its own line — would silently change what the page renders. That
//! is a correctness bug, not a style choice.
//!
//! The HARD GATE this module upholds:
//!
//! > Formatting must NEVER change the generated Motoko. For every file,
//! > `codegen(fmt(x)) == codegen(x)` — byte-for-byte — and `fmt` is idempotent
//! > (`fmt(fmt(x)) == fmt(x)`).
//!
//! ## How the gate is enforced (self-verification)
//!
//! Rather than hand-reason about which whitespace is safe to touch (fragile), the
//! formatter is **self-checking**. It produces a candidate formatting, then re-runs
//! the real parser + codegen on BOTH the original and the candidate and compares
//! the generated Motoko ([`codegen_signature`]). If they differ in any byte — or if
//! the candidate fails to parse — the transformation is REJECTED and the original
//! text is kept unchanged. A smaller, correct formatter beats a pretty, wrong one.
//!
//! ## What it normalizes (all whitespace-only, all gate-checked)
//!
//!   * CRLF / lone-CR line endings -> LF.
//!   * Trailing whitespace stripped from every line.
//!   * Runs of 3+ blank lines collapsed to a single blank line.
//!   * Exactly one trailing newline (no missing newline, no blank padding at EOF).
//!
//! It DELIBERATELY does NOT: reflow or re-indent template text/markup, change
//! attribute quoting or directive spacing, or reformat the Motoko inside `@code`
//! (that is `moc`/the Motoko LSP's job — we pass it through untouched). Those could
//! all alter [`Node::Text`] content or `@code` body bytes and thus the output.

use crate::ast::FileKind;
use crate::codegen::Codegen;
use crate::parser;
use std::collections::HashMap;

/// The generated-Motoko "signature" of a `.mview` source: the full codegen output
/// for the file. Two sources with the same signature produce byte-identical
/// `main.mo` (codegen is a pure function of the parsed AST). `None` means the
/// source failed to parse — used so the formatter never accepts a candidate that
/// broke parsing.
///
/// `kind` selects the emit path (`gen_page`/`gen_layout`/`gen_app_component`) the
/// real project build uses for that file, so the signature matches the bytes that
/// would land in `main.mo`.
pub fn codegen_signature(source: &str, kind: FileKind) -> Option<String> {
    // Models/components are empty here: the formatter compares the SAME source
    // before/after under the SAME (empty) context, so any model/component-driven
    // codegen is identical on both sides. The signature need only be a faithful,
    // deterministic function of the parsed file — not the whole-project assembly.
    let models: HashMap<String, HashMap<String, String>> = HashMap::new();
    let comps: HashMap<String, crate::codegen::CompInfo> = HashMap::new();
    let file = parser::parse(source, "FmtProbe", kind.clone()).ok()?;
    let mut cg = Codegen::new(&models, &comps);
    let out = match kind {
        FileKind::Layout => cg.gen_layout(&file),
        FileKind::Component => cg.gen_app_component(&file),
        FileKind::Page => {
            let pg = cg.gen_page(&file);
            // Concatenate every codegen artifact a page contributes so the
            // signature captures all of them (route, record, object block).
            format!("{}\n{}\n{}\n{}", pg.name, pg.route, pg.object_block, pg.page_record)
        }
    };
    Some(out)
}

/// Infer the [`FileKind`] from a file path, mirroring the directory convention
/// the project build uses (`Layouts/` => Layout, `Components/` => Component, else
/// Page). Used so `fmt` checks each file under the same emit path the build would.
pub fn kind_from_path(path: &str) -> FileKind {
    if path.contains("Layouts") {
        FileKind::Layout
    } else if path.contains("Components") {
        FileKind::Component
    } else {
        FileKind::Page
    }
}

/// The pure, whitespace-only normalization pass (NO verification yet). Applies the
/// transformations described in the module docs. The caller ([`format_source`])
/// is responsible for gate-checking the result; this function is also reused by
/// the idempotency tests directly.
fn normalize(source: &str) -> String {
    // 1) Normalize line endings to LF (handles CRLF and lone CR).
    let unified = source.replace("\r\n", "\n").replace('\r', "\n");

    // 2) Per-line: strip trailing spaces/tabs.
    let stripped: Vec<&str> = unified
        .split('\n')
        .map(|line| line.trim_end_matches([' ', '\t']))
        .collect();

    // 3) Collapse runs of 3+ blank lines into a single blank line, and trim
    //    trailing blank lines entirely (the final-newline step re-adds exactly one).
    let mut out_lines: Vec<&str> = Vec::with_capacity(stripped.len());
    let mut blank_run = 0usize;
    for line in &stripped {
        if line.is_empty() {
            blank_run += 1;
            // Keep at most ONE blank line in a row.
            if blank_run <= 1 {
                out_lines.push(line);
            }
        } else {
            blank_run = 0;
            out_lines.push(line);
        }
    }
    // Drop trailing blank lines (we re-add a single final newline below).
    while out_lines.last().map(|l| l.is_empty()).unwrap_or(false) {
        out_lines.pop();
    }
    // Drop leading blank lines too (purely cosmetic header whitespace).
    while out_lines.first().map(|l| l.is_empty()).unwrap_or(false) {
        out_lines.remove(0);
    }

    // 4) Join with LF and end with exactly one trailing newline. An empty file
    //    stays empty (no spurious newline).
    if out_lines.is_empty() {
        String::new()
    } else {
        let mut s = out_lines.join("\n");
        s.push('\n');
        s
    }
}

/// Format `source` for a file of the given `kind`, upholding the hard gate.
///
/// Produces the normalized text ONLY IF it parses and yields a byte-identical
/// codegen signature to the original. Otherwise the original `source` is returned
/// unchanged. This is what makes the formatter provably semantics-preserving: a
/// transformation that would change the generated Motoko can never be emitted.
pub fn format_source(source: &str, kind: FileKind) -> String {
    let candidate = normalize(source);
    if candidate == source {
        // Already formatted (fast path; also the idempotent fixpoint).
        return candidate;
    }
    let before = codegen_signature(source, kind.clone());
    let after = codegen_signature(&candidate, kind.clone());
    match (before, after) {
        // Accept ONLY when both parse and the generated Motoko is byte-identical.
        (Some(b), Some(a)) if a == b => candidate,
        // Any divergence (or a candidate that broke parsing) -> keep the original.
        // The formatter must never change behaviour; correctness over prettiness.
        _ => source.to_string(),
    }
}

/// Whether `source` is already formatted (its [`format_source`] output equals it).
/// Drives `fmt --check`: a "clean" file needs no rewrite.
pub fn is_formatted(source: &str, kind: FileKind) -> bool {
    format_source(source, kind) == source
}

#[cfg(test)]
mod fmt_self_tests {
    use super::*;

    #[test]
    fn normalize_strips_trailing_ws_and_crlf() {
        let src = "@page \"/\"\r\n<p>hi</p>   \r\n";
        let out = normalize(src);
        assert!(!out.contains('\r'), "CRLF should be gone");
        assert!(!out.contains("   \n"), "trailing ws should be gone");
        assert!(out.ends_with("</p>\n"), "exactly one final newline: {:?}", out);
    }

    #[test]
    fn normalize_collapses_blank_runs_and_is_idempotent() {
        let src = "@page \"/\"\n\n\n\n<p>x</p>\n\n\n";
        let once = normalize(src);
        // 3+ blanks -> a single blank line between the directive and the markup.
        assert_eq!(once, "@page \"/\"\n\n<p>x</p>\n");
        // idempotent
        assert_eq!(normalize(&once), once);
    }

    #[test]
    fn empty_file_stays_empty() {
        assert_eq!(normalize(""), "");
        assert_eq!(normalize("   \n  \n"), "");
    }
}
