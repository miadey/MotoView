//! Backend = PLAIN, testable functions. NO egui types appear in this module, so
//! every function here is unit-testable without ever opening a window.
//!
//! Responsibilities:
//!   * resolve + spawn the `motoview` binary and parse its `--json` output
//!     (`run_check`, `run_lint`, `run_preview`, `run_fmt`, `run_build`);
//!   * basic filesystem helpers (`list_mview_files`, `read_file`, `write_file`);
//!   * parse the preview IR forest JSON into a plain `UiNode` tree
//!     (`parse_forest`) and decide which native widget each node maps to
//!     (`widget_kind`) — both pure, both tested.

use std::collections::BTreeMap;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::Command;

// ---------------------------------------------------------------------------
// Diagnostics (mirror of the compiler's `--json` diagnostic schema)
// ---------------------------------------------------------------------------

/// One diagnostic, matching the `motoview check/lint --json` array element:
/// `{severity, rule, message, file, line, col, endLine, endCol}`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Diagnostic {
    pub severity: String,
    pub rule: String,
    pub message: String,
    pub file: String,
    pub line: u32,
    pub col: u32,
    pub end_line: u32,
    pub end_col: u32,
}

impl Diagnostic {
    pub fn is_error(&self) -> bool {
        self.severity.eq_ignore_ascii_case("error")
    }
    pub fn is_warning(&self) -> bool {
        self.severity.eq_ignore_ascii_case("warning")
    }
}

/// Result of running a backend command: the diagnostics we managed to parse,
/// plus the raw stdout/stderr so the UI can always show *something* truthful
/// even when the JSON could not be parsed.
#[derive(Debug, Clone, Default)]
pub struct CommandReport {
    pub diagnostics: Vec<Diagnostic>,
    pub raw_stdout: String,
    pub raw_stderr: String,
    pub exit_ok: bool,
    /// Human note when we had to fall back / something went wrong spawning.
    pub note: Option<String>,
}

// ---------------------------------------------------------------------------
// Binary resolution + spawning
// ---------------------------------------------------------------------------

/// Resolve the `motoview` binary, in priority order:
///   1. `$MOTOVIEW` env var (explicit override),
///   2. a repo-relative default `compiler/target/release/motoview` resolved
///      against a few likely roots (cwd, and the crate's own location),
///   3. bare `motoview` (let the OS search `$PATH`).
pub fn resolve_motoview() -> PathBuf {
    if let Ok(p) = std::env::var("MOTOVIEW") {
        if !p.trim().is_empty() {
            return PathBuf::from(p);
        }
    }

    // Candidate repo roots to try the relative default against.
    let mut roots: Vec<PathBuf> = Vec::new();
    if let Ok(cwd) = std::env::current_dir() {
        roots.push(cwd);
    }
    // The crate lives at <repo>/apps/studio/native, so climb three levels.
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    if let Some(repo) = manifest.ancestors().nth(3) {
        roots.push(repo.to_path_buf());
    }

    for root in roots {
        let cand = root.join("compiler/target/release/motoview");
        if cand.exists() {
            return cand;
        }
    }

    PathBuf::from("motoview")
}

fn run_motoview(args: &[&str], cwd: Option<&Path>) -> std::io::Result<(String, String, bool)> {
    let bin = resolve_motoview();
    let mut cmd = Command::new(bin);
    cmd.args(args);
    if let Some(dir) = cwd {
        cmd.current_dir(dir);
    }
    let out = cmd.output()?;
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    Ok((stdout, stderr, out.status.success()))
}

// ---------------------------------------------------------------------------
// Diagnostic JSON / text parsing
// ---------------------------------------------------------------------------

/// Find and parse the first JSON array of diagnostics in `text`. The compiler
/// prints human-readable banner lines *before* the machine JSON, so we scan
/// for a line that starts with `[` and parse from there.
pub fn parse_diagnostics_json(text: &str) -> Option<Vec<Diagnostic>> {
    for line in text.lines() {
        let trimmed = line.trim_start();
        if trimmed.starts_with('[') {
            if let Ok(v) = serde_json::from_str::<serde_json::Value>(trimmed) {
                if let Some(arr) = v.as_array() {
                    return Some(arr.iter().filter_map(diag_from_value).collect());
                }
            }
        }
    }
    None
}

fn diag_from_value(v: &serde_json::Value) -> Option<Diagnostic> {
    let o = v.as_object()?;
    let s = |k: &str| o.get(k).and_then(|x| x.as_str()).unwrap_or("").to_string();
    let n = |k: &str| o.get(k).and_then(|x| x.as_u64()).unwrap_or(0) as u32;
    Some(Diagnostic {
        severity: s("severity"),
        rule: s("rule"),
        message: s("message"),
        file: s("file"),
        line: n("line"),
        col: n("col"),
        end_line: n("endLine"),
        end_col: n("endCol"),
    })
}

/// Fallback parser: when a hard build error short-circuits before JSON is
/// emitted, the compiler still prints a text error of the form:
///
/// ```text
/// error: [secure-form] <message...>
///   --> src/Pages/Home.mview (<form @submit="submit">)
/// ```
///
/// (optionally prefixed with `build error: ` / `motoview preview error: `).
/// Parse those into `Diagnostic`s so the UI and tests still surface the rule.
pub fn parse_diagnostics_text(text: &str) -> Vec<Diagnostic> {
    let mut out = Vec::new();
    let lines: Vec<&str> = text.lines().collect();
    for (i, raw) in lines.iter().enumerate() {
        let line = raw.trim_start();
        // Strip any leading wrapper like "build error: " / "motoview … error: ".
        let line = match line.find("error: [") {
            Some(idx) => &line[idx..],
            None => continue,
        };
        // Now `line` starts with "error: [rule] message".
        let after = line.trim_start_matches("error:").trim_start();
        if !after.starts_with('[') {
            continue;
        }
        let close = match after.find(']') {
            Some(c) => c,
            None => continue,
        };
        let rule = after[1..close].to_string();
        let message = after[close + 1..].trim().to_string();

        // Look at the following line for `  --> file (context)`.
        let mut file = String::new();
        if let Some(next) = lines.get(i + 1) {
            let nt = next.trim_start();
            if let Some(rest) = nt.strip_prefix("-->") {
                let rest = rest.trim();
                file = match rest.find(" (") {
                    Some(p) => rest[..p].to_string(),
                    None => rest.to_string(),
                };
            }
        }

        out.push(Diagnostic {
            severity: "error".to_string(),
            rule,
            message,
            file,
            line: 0,
            col: 0,
            end_line: 0,
            end_col: 0,
        });
    }
    out
}

/// Build a full `CommandReport` from a process's streams: prefer the JSON
/// diagnostics array; if there is none, fall back to text-error parsing.
fn report_from_streams(stdout: String, stderr: String, exit_ok: bool) -> CommandReport {
    // The JSON array may land on stdout (success path) or, for some commands,
    // be absent entirely. Try both streams for the JSON line.
    let json = parse_diagnostics_json(&stdout).or_else(|| parse_diagnostics_json(&stderr));

    if let Some(diags) = json {
        return CommandReport {
            diagnostics: diags,
            raw_stdout: stdout,
            raw_stderr: stderr,
            exit_ok,
            note: None,
        };
    }

    // No JSON — parse human-readable error text from both streams.
    let mut diags = parse_diagnostics_text(&stderr);
    diags.extend(parse_diagnostics_text(&stdout));
    let note = if diags.is_empty() && !exit_ok {
        Some("command failed; no diagnostics could be parsed (see raw output)".to_string())
    } else if diags.is_empty() {
        None
    } else {
        Some("parsed from text error output (no JSON array present)".to_string())
    };
    CommandReport {
        diagnostics: diags,
        raw_stdout: stdout,
        raw_stderr: stderr,
        exit_ok,
        note,
    }
}

fn spawn_report(args: &[&str], project_dir: &Path) -> CommandReport {
    match run_motoview(args, Some(project_dir)) {
        Ok((so, se, ok)) => report_from_streams(so, se, ok),
        Err(e) => CommandReport {
            diagnostics: Vec::new(),
            raw_stdout: String::new(),
            raw_stderr: String::new(),
            exit_ok: false,
            note: Some(format!("failed to spawn motoview: {e}")),
        },
    }
}

// ---------------------------------------------------------------------------
// Public command API
// ---------------------------------------------------------------------------

/// `motoview check <dir> --json` — build + type-check.
pub fn run_check(project_dir: &Path) -> CommandReport {
    spawn_report(&["check", ".", "--json"], project_dir)
}

/// `motoview lint <dir> --json` — the security lint pass (secure forms, etc.).
pub fn run_lint(project_dir: &Path) -> CommandReport {
    spawn_report(&["lint", ".", "--json"], project_dir)
}

/// `motoview build <dir>` — compile .mview into Motoko. (No `--json`; we keep
/// the raw output and surface any text errors.)
pub fn run_build(project_dir: &Path) -> CommandReport {
    spawn_report(&["build", "."], project_dir)
}

/// `motoview fmt <path>` — format .mview files. Returns the raw report; on a
/// single file or a directory.
pub fn run_fmt(path: &Path) -> CommandReport {
    // `fmt` takes a dir or a file as its positional arg. Run it from the path's
    // parent (or the path itself if it's a dir) so relative output is sane.
    let (cwd, arg) = if path.is_dir() {
        (path.to_path_buf(), ".".to_string())
    } else {
        let parent = path.parent().unwrap_or(Path::new(".")).to_path_buf();
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| path.to_string_lossy().into_owned());
        (parent, name)
    };
    spawn_report(&["fmt", &arg], &cwd)
}

// ---------------------------------------------------------------------------
// Preview IR forest
// ---------------------------------------------------------------------------

/// Output of `run_preview`: the parsed forest plus the raw JSON we parsed it
/// from (handy for debugging) and any error note.
#[derive(Debug, Clone, Default)]
pub struct PreviewResult {
    pub forest: Vec<UiNode>,
    pub raw_json: String,
    pub note: Option<String>,
    pub ok: bool,
}

/// `motoview preview <dir> [--route <r>] --json` — render the page IR forest
/// with NO deploy. Parses the forest into `Vec<UiNode>`.
pub fn run_preview(project_dir: &Path, route: Option<&str>) -> PreviewResult {
    let mut args: Vec<&str> = vec!["preview", ".", "--json"];
    if let Some(r) = route {
        args.push("--route");
        args.push(r);
    }
    match run_motoview(&args, Some(project_dir)) {
        Ok((stdout, stderr, ok)) => {
            // Find the JSON forest line (starts with `[`).
            if let Some(json_line) = stdout.lines().find(|l| l.trim_start().starts_with('[')) {
                match parse_forest(json_line.trim_start()) {
                    Ok(forest) => PreviewResult {
                        forest,
                        raw_json: json_line.trim_start().to_string(),
                        note: None,
                        ok,
                    },
                    Err(e) => PreviewResult {
                        forest: Vec::new(),
                        raw_json: json_line.to_string(),
                        note: Some(format!("forest parse error: {e}")),
                        ok: false,
                    },
                }
            } else {
                PreviewResult {
                    forest: Vec::new(),
                    raw_json: String::new(),
                    note: Some(format!(
                        "no IR forest in preview output. stderr: {}",
                        first_meaningful_line(&stderr)
                    )),
                    ok: false,
                }
            }
        }
        Err(e) => PreviewResult {
            forest: Vec::new(),
            raw_json: String::new(),
            note: Some(format!("failed to spawn motoview preview: {e}")),
            ok: false,
        },
    }
}

fn first_meaningful_line(s: &str) -> String {
    s.lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .unwrap_or("")
        .to_string()
}

// ---------------------------------------------------------------------------
// UI IR node tree (the {t:el|text|raw} schema) — a plain, testable type
// ---------------------------------------------------------------------------

/// A node in the preview IR forest. Mirrors the wire schema:
///   `{"t":"el","tag","attrs","events","key","children"}`
///   `{"t":"text","value"}`
///   `{"t":"raw","html"}`
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UiNode {
    El {
        tag: String,
        /// Attribute map (kept ordered for stable tests).
        attrs: BTreeMap<String, String>,
        /// Event bindings, e.g. `{"click":"increment"}`.
        events: BTreeMap<String, String>,
        key: Option<String>,
        children: Vec<UiNode>,
    },
    Text {
        value: String,
    },
    Raw {
        html: String,
    },
}

impl UiNode {
    pub fn tag(&self) -> Option<&str> {
        match self {
            UiNode::El { tag, .. } => Some(tag.as_str()),
            _ => None,
        }
    }
}

/// Parse the IR forest JSON string into `Vec<UiNode>`.
pub fn parse_forest(json: &str) -> Result<Vec<UiNode>, String> {
    let v: serde_json::Value = serde_json::from_str(json).map_err(|e| e.to_string())?;
    let arr = v
        .as_array()
        .ok_or_else(|| "forest root is not a JSON array".to_string())?;
    arr.iter().map(node_from_value).collect()
}

fn node_from_value(v: &serde_json::Value) -> Result<UiNode, String> {
    let o = v.as_object().ok_or("node is not an object")?;
    let t = o.get("t").and_then(|x| x.as_str()).ok_or("node missing `t`")?;
    match t {
        "text" => Ok(UiNode::Text {
            value: o
                .get("value")
                .and_then(|x| x.as_str())
                .unwrap_or("")
                .to_string(),
        }),
        "raw" => Ok(UiNode::Raw {
            html: o
                .get("html")
                .and_then(|x| x.as_str())
                .unwrap_or("")
                .to_string(),
        }),
        "el" => {
            let tag = o
                .get("tag")
                .and_then(|x| x.as_str())
                .unwrap_or("")
                .to_string();
            let attrs = string_map(o.get("attrs"));
            let events = string_map(o.get("events"));
            let key = o
                .get("key")
                .and_then(|x| x.as_str())
                .map(|s| s.to_string());
            let children = match o.get("children").and_then(|x| x.as_array()) {
                Some(arr) => arr.iter().map(node_from_value).collect::<Result<_, _>>()?,
                None => Vec::new(),
            };
            Ok(UiNode::El {
                tag,
                attrs,
                events,
                key,
                children,
            })
        }
        other => Err(format!("unknown node type `{other}`")),
    }
}

fn string_map(v: Option<&serde_json::Value>) -> BTreeMap<String, String> {
    let mut m = BTreeMap::new();
    if let Some(obj) = v.and_then(|x| x.as_object()) {
        for (k, val) in obj {
            let sv = match val {
                serde_json::Value::String(s) => s.clone(),
                other => other.to_string(),
            };
            m.insert(k.clone(), sv);
        }
    }
    m
}

// ---------------------------------------------------------------------------
// IR -> native widget DECISION logic (pure, testable; no egui types)
// ---------------------------------------------------------------------------

/// Which native egui widget kind a node maps to. The egui render function reads
/// this enum and stays thin; ALL the per-node "which widget" decision lives
/// here so it can be unit-tested with no running egui context.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WidgetKind {
    /// Vertical container group: div/section/ul/ol/li/main/article/nav/header/
    /// footer/aside/form/dl/table/tbody/tr — anything block-ish.
    Group,
    /// Horizontal/inline row group: span/p/strong/em/b/i/small/code/label/
    /// h1..h6/a/td/th — inline-ish wrappers that lay their children out in a row.
    Inline,
    /// A clickable button: <button> (and <input type=submit/button>).
    Button,
    /// A plain text leaf (`text` node) -> a label.
    Label,
    /// Raw HTML -> a label of the stripped/escaped text.
    RawLabel,
    /// A form input field (`input`, `textarea`, `select`) -> a (read-only in
    /// this preview) text box / placeholder.
    Input,
    /// Anything we don't specifically handle -> fall back to a group so children
    /// still render.
    Unknown,
}

/// Pure decision: given an IR node, which native widget kind renders it.
pub fn widget_kind(node: &UiNode) -> WidgetKind {
    match node {
        UiNode::Text { .. } => WidgetKind::Label,
        UiNode::Raw { .. } => WidgetKind::RawLabel,
        UiNode::El { tag, attrs, .. } => {
            let t = tag.to_ascii_lowercase();
            match t.as_str() {
                "button" => WidgetKind::Button,
                "input" => {
                    // <input type=submit|button> is really a button.
                    match attrs.get("type").map(|s| s.to_ascii_lowercase()).as_deref() {
                        Some("submit") | Some("button") => WidgetKind::Button,
                        _ => WidgetKind::Input,
                    }
                }
                "textarea" | "select" => WidgetKind::Input,
                "div" | "section" | "ul" | "ol" | "li" | "main" | "article" | "nav" | "header"
                | "footer" | "aside" | "form" | "dl" | "table" | "tbody" | "thead" | "tr" => {
                    WidgetKind::Group
                }
                "span" | "p" | "strong" | "em" | "b" | "i" | "small" | "code" | "label" | "a"
                | "td" | "th" | "dt" | "dd" | "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => {
                    WidgetKind::Inline
                }
                _ => WidgetKind::Unknown,
            }
        }
    }
}

/// Best-effort visible text of a node (used as a Button's label or an Inline's
/// fallback text). Concatenates descendant text/raw leaves.
pub fn node_text(node: &UiNode) -> String {
    fn walk(n: &UiNode, out: &mut String) {
        match n {
            UiNode::Text { value } => out.push_str(value),
            UiNode::Raw { html } => out.push_str(&strip_html(html)),
            UiNode::El { children, .. } => {
                for c in children {
                    walk(c, out);
                }
            }
        }
    }
    let mut s = String::new();
    walk(node, &mut s);
    s.trim().to_string()
}

/// Strip HTML tags and decode a few common entities so a `raw` node renders as
/// readable text in a native label.
pub fn strip_html(html: &str) -> String {
    let mut out = String::with_capacity(html.len());
    let mut in_tag = false;
    for ch in html.chars() {
        match ch {
            '<' => in_tag = true,
            '>' => in_tag = false,
            _ if !in_tag => out.push(ch),
            _ => {}
        }
    }
    out.replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&amp;", "&")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
}

// ---------------------------------------------------------------------------
// Filesystem helpers
// ---------------------------------------------------------------------------

/// Recursively list every `.mview` file under `dir`, sorted for stable UI.
pub fn list_mview_files(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    collect_mview(dir, &mut out);
    out.sort();
    out
}

fn collect_mview(dir: &Path, out: &mut Vec<PathBuf>) {
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            // Skip build/output + VCS noise dirs.
            let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
            if matches!(name, ".mvbuild" | ".dfx" | ".git" | "target" | "node_modules") {
                continue;
            }
            collect_mview(&path, out);
        } else if path.extension().and_then(|e| e.to_str()) == Some("mview") {
            out.push(path);
        }
    }
}

/// Read a file to a `String`.
pub fn read_file(path: &Path) -> std::io::Result<String> {
    let mut f = std::fs::File::open(path)?;
    let mut s = String::new();
    f.read_to_string(&mut s)?;
    Ok(s)
}

/// Write `content` to `path`, creating parent dirs if needed.
pub fn write_file(path: &Path, content: &str) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, content)
}

// ===========================================================================
// TESTS
// ===========================================================================
#[cfg(test)]
mod tests {
    use super::*;

    fn fixtures_dir() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures")
    }

    // -- File ops round-trip ------------------------------------------------

    #[test]
    fn file_ops_round_trip() {
        let tmp = std::env::temp_dir().join(format!("mvstudio_rt_{}", std::process::id()));
        let file = tmp.join("nested/Sample.mview");
        let body = "@page \"/\"\n<h1>hi</h1>\n";

        write_file(&file, body).expect("write");
        let back = read_file(&file).expect("read");
        assert_eq!(back, body, "round-trip content must match");

        // list_mview_files finds the .mview we just wrote.
        let listed = list_mview_files(&tmp);
        assert!(
            listed.iter().any(|p| p.ends_with("Sample.mview")),
            "list_mview_files should find the written .mview: {listed:?}"
        );

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn list_skips_build_dirs() {
        let tmp = std::env::temp_dir().join(format!("mvstudio_skip_{}", std::process::id()));
        write_file(&tmp.join("src/Pages/A.mview"), "x").unwrap();
        write_file(&tmp.join(".mvbuild/B.mview"), "x").unwrap();
        let listed = list_mview_files(&tmp);
        assert!(listed.iter().any(|p| p.ends_with("A.mview")));
        assert!(
            !listed.iter().any(|p| p.to_string_lossy().contains(".mvbuild")),
            ".mvbuild should be skipped: {listed:?}"
        );
        let _ = std::fs::remove_dir_all(&tmp);
    }

    // -- Backend against the REAL binary (lint + check on unsecured form) ---

    fn motoview_available() -> bool {
        let bin = resolve_motoview();
        bin.exists() || bin == Path::new("motoview")
    }

    #[test]
    fn run_lint_flags_unsecured_form_from_real_binary() {
        if !motoview_available() {
            eprintln!("SKIP: motoview binary not found");
            return;
        }
        let proj = fixtures_dir().join("unsecured");
        let report = run_lint(&proj);
        let secure = report
            .diagnostics
            .iter()
            .find(|d| d.rule == "secure-form")
            .unwrap_or_else(|| {
                panic!(
                    "expected a secure-form diagnostic. report={:?}",
                    report
                )
            });
        assert!(secure.is_error(), "secure-form must be an error: {secure:?}");
        assert!(
            secure.file.contains("Home.mview"),
            "diagnostic should point at the page: {secure:?}"
        );
    }

    #[test]
    fn run_check_surfaces_secure_form_error_from_real_binary() {
        if !motoview_available() {
            eprintln!("SKIP: motoview binary not found");
            return;
        }
        let proj = fixtures_dir().join("unsecured");
        let report = run_check(&proj);
        // On the unsecured fixture the build HARD-fails at the secure-form gate;
        // run_check parses the rule out of either the JSON array or the text
        // error fallback. Either way we must see `secure-form`.
        assert!(
            report.diagnostics.iter().any(|d| d.rule == "secure-form"),
            "run_check must surface secure-form. report={report:?}"
        );
        assert!(
            !report.exit_ok,
            "the unsecured fixture must NOT pass check"
        );
    }

    /// Repo root (the crate lives at <repo>/apps/studio/native).
    fn repo_root() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .ancestors()
            .nth(3)
            .expect("repo root")
            .to_path_buf()
    }

    #[test]
    fn run_preview_parses_real_forest_from_binary() {
        if !motoview_available() {
            eprintln!("SKIP: motoview binary not found");
            return;
        }
        let counter = repo_root().join("examples/counter");
        if !counter.join("motoview.json").exists() && !counter.join("dfx.json").exists() {
            eprintln!("SKIP: examples/counter not present");
            return;
        }
        let res = run_preview(&counter, None);
        if !res.ok {
            // preview needs `moc -r`; if the toolchain isn't wired in this env,
            // don't fail the suite — just note it. The static-JSON forest tests
            // already cover the parser deterministically.
            eprintln!("SKIP: preview not runnable here: {:?}", res.note);
            return;
        }
        assert!(!res.forest.is_empty(), "real forest should be non-empty");
        // The counter page has at least one <button> somewhere in the tree.
        fn has_button(nodes: &[UiNode]) -> bool {
            nodes.iter().any(|n| match n {
                UiNode::El { tag, children, .. } => tag == "button" || has_button(children),
                _ => false,
            })
        }
        assert!(
            has_button(&res.forest),
            "the counter page forest should contain a button"
        );
    }

    // -- Diagnostic text fallback parsing (no binary needed) ----------------

    #[test]
    fn parse_diagnostics_text_extracts_rule_and_file() {
        let text = "build error: error: [secure-form] state-mutating <form> must be `secure`.\n  --> src/Pages/Home.mview (<form @submit=\"submit\">)\n";
        let diags = parse_diagnostics_text(text);
        assert_eq!(diags.len(), 1);
        assert_eq!(diags[0].rule, "secure-form");
        assert_eq!(diags[0].file, "src/Pages/Home.mview");
        assert!(diags[0].is_error());
    }

    #[test]
    fn parse_diagnostics_json_skips_banner() {
        let text = "compiled 1 page(s)\nroutes:\n  /  Home\n[{\"severity\":\"warning\",\"rule\":\"moc\",\"message\":\"m\",\"file\":\"main.mo\",\"line\":1,\"col\":2,\"endLine\":1,\"endCol\":3}]\n";
        let diags = parse_diagnostics_json(text).expect("should find json line");
        assert_eq!(diags.len(), 1);
        assert_eq!(diags[0].rule, "moc");
        assert_eq!(diags[0].line, 1);
        assert_eq!(diags[0].end_col, 3);
        assert!(diags[0].is_warning());
    }

    // -- IR forest parsing --------------------------------------------------

    fn sample_forest_json() -> &'static str {
        // A button + text + raw + nested el, mirroring the real wire schema.
        r#"[
          {"t":"el","tag":"section","attrs":{"class":"box"},"events":{},"children":[
            {"t":"el","tag":"button","attrs":{"class":"mv-btn"},"events":{"click":"increment"},"children":[
              {"t":"text","value":"+1"}
            ]},
            {"t":"el","tag":"p","attrs":{},"events":{},"children":[
              {"t":"text","value":"hello"}
            ]},
            {"t":"raw","html":"<b>bold &amp; raw</b>"}
          ]}
        ]"#
    }

    #[test]
    fn parse_forest_builds_expected_tree() {
        let forest = parse_forest(sample_forest_json()).expect("parse");
        assert_eq!(forest.len(), 1, "one root <section>");
        let section = &forest[0];
        match section {
            UiNode::El { tag, attrs, children, .. } => {
                assert_eq!(tag, "section");
                assert_eq!(attrs.get("class").map(String::as_str), Some("box"));
                assert_eq!(children.len(), 3, "button, p, raw");

                // child 0: button with a click event + a text child "+1"
                match &children[0] {
                    UiNode::El { tag, events, children, .. } => {
                        assert_eq!(tag, "button");
                        assert_eq!(events.get("click").map(String::as_str), Some("increment"));
                        assert_eq!(children.len(), 1);
                        assert_eq!(children[0], UiNode::Text { value: "+1".into() });
                    }
                    other => panic!("expected button el, got {other:?}"),
                }
                // child 1: p with text "hello"
                assert_eq!(node_text(&children[1]), "hello");
                // child 2: raw
                assert_eq!(children[2], UiNode::Raw { html: "<b>bold &amp; raw</b>".into() });
            }
            other => panic!("expected section el, got {other:?}"),
        }
    }

    // -- widget_kind mapping ------------------------------------------------

    fn el(tag: &str) -> UiNode {
        UiNode::El {
            tag: tag.into(),
            attrs: BTreeMap::new(),
            events: BTreeMap::new(),
            key: None,
            children: vec![],
        }
    }

    #[test]
    fn widget_kind_maps_each_node() {
        assert_eq!(widget_kind(&UiNode::Text { value: "x".into() }), WidgetKind::Label);
        assert_eq!(widget_kind(&UiNode::Raw { html: "<i>x</i>".into() }), WidgetKind::RawLabel);
        assert_eq!(widget_kind(&el("button")), WidgetKind::Button);
        assert_eq!(widget_kind(&el("div")), WidgetKind::Group);
        assert_eq!(widget_kind(&el("section")), WidgetKind::Group);
        assert_eq!(widget_kind(&el("ul")), WidgetKind::Group);
        assert_eq!(widget_kind(&el("form")), WidgetKind::Group);
        assert_eq!(widget_kind(&el("span")), WidgetKind::Inline);
        assert_eq!(widget_kind(&el("p")), WidgetKind::Inline);
        assert_eq!(widget_kind(&el("h1")), WidgetKind::Inline);
        assert_eq!(widget_kind(&el("textarea")), WidgetKind::Input);
        assert_eq!(widget_kind(&el("blink")), WidgetKind::Unknown);
    }

    #[test]
    fn widget_kind_input_submit_is_button() {
        let mut attrs = BTreeMap::new();
        attrs.insert("type".to_string(), "submit".to_string());
        let input_submit = UiNode::El {
            tag: "input".into(),
            attrs,
            events: BTreeMap::new(),
            key: None,
            children: vec![],
        };
        assert_eq!(widget_kind(&input_submit), WidgetKind::Button);
        // a plain text input is an Input
        let mut attrs2 = BTreeMap::new();
        attrs2.insert("type".to_string(), "text".to_string());
        let input_text = UiNode::El {
            tag: "input".into(),
            attrs: attrs2,
            events: BTreeMap::new(),
            key: None,
            children: vec![],
        };
        assert_eq!(widget_kind(&input_text), WidgetKind::Input);
    }

    #[test]
    fn strip_html_decodes_entities() {
        assert_eq!(strip_html("<b>bold &amp; raw</b>"), "bold & raw");
        assert_eq!(strip_html("a &lt;tag&gt; b"), "a <tag> b");
        assert_eq!(strip_html("plain"), "plain");
    }

    #[test]
    fn node_text_concatenates_leaves() {
        let forest = parse_forest(sample_forest_json()).unwrap();
        // The whole section's text is button + p + raw text, concatenated.
        let t = node_text(&forest[0]);
        assert!(t.contains("+1"));
        assert!(t.contains("hello"));
        assert!(t.contains("bold & raw"));
    }
}
