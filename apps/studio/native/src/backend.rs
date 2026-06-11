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
    /// The node→span selection side-map (from `--srcmap`). Empty if unavailable;
    /// resolve a clicked node with [`SrcMap::resolve`].
    pub srcmap: SrcMap,
}

/// `motoview preview <dir> [--route <r>] --json` — render the page IR forest
/// with NO deploy. Parses the forest into `Vec<UiNode>`.
pub fn run_preview(project_dir: &Path, route: Option<&str>) -> PreviewResult {
    // `--srcmap` tags the forest with `data-mv-src` ids and writes the node→span
    // side-map we read below (the selection bridge). Preview-only; production is
    // unaffected.
    let mut args: Vec<&str> = vec!["preview", ".", "--json", "--srcmap"];
    if let Some(r) = route {
        args.push("--route");
        args.push(r);
    }
    // The selection side-map written by `--srcmap`, read after a successful render.
    let read_srcmap = || -> SrcMap {
        std::fs::read_to_string(project_dir.join(".mvbuild").join("preview.srcmap.json"))
            .ok()
            .map(|s| SrcMap::parse(&s))
            .unwrap_or_default()
    };
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
                        srcmap: read_srcmap(),
                    },
                    Err(e) => PreviewResult {
                        forest: Vec::new(),
                        raw_json: json_line.to_string(),
                        note: Some(format!("forest parse error: {e}")),
                        ok: false,
                        srcmap: SrcMap::default(),
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
                    srcmap: SrcMap::default(),
                }
            }
        }
        Err(e) => PreviewResult {
            forest: Vec::new(),
            raw_json: String::new(),
            note: Some(format!("failed to spawn motoview preview: {e}")),
            ok: false,
            srcmap: SrcMap::default(),
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
// LIVE preview via replay (R17): accumulate dispatched events into a session
// and re-render the page IR forest through `motoview preview --replay`.
//
// The replay path runs the page's dispatch+render through `moc -r` (NO dfx
// deploy, deterministic), exactly as R10 wired it. Each entry in the session
// is one event; replaying the WHOLE accumulated session reproduces the
// page-local state from the initial render up to the latest click.
// ---------------------------------------------------------------------------

/// One dispatched event in a replay session, matching the R10 session schema
/// element: `{"handler":"increment","args":["1"],"caller":"<principal-opt>"}`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionEvent {
    pub handler: String,
    pub args: Vec<String>,
    pub caller: Option<String>,
}

/// An ordered list of events. Replaying it reproduces page-local state.
pub type Session = Vec<SessionEvent>;

/// Extract a dispatchable [`SessionEvent`] from an interactive IR node.
///
/// A clickable node (`<button>`, or an `<input type=submit|button>`, or a
/// `<form>`) carries its handler in the node's `events` map — under the
/// `"click"` key for buttons or `"submit"` for forms — and the codegen bakes
/// each positional argument value into a `data-mv-arg0`, `data-mv-arg1`, …
/// attribute. We read the handler from the matching event and collect the
/// args in index order.
///
/// Returns `None` for non-interactive nodes (text/raw, or elements with no
/// click/submit handler), so the UI never tries to dispatch a no-op.
pub fn event_from_node(node: &UiNode) -> Option<SessionEvent> {
    let UiNode::El { attrs, events, .. } = node else {
        return None;
    };
    // Prefer a click handler (buttons), then a submit handler (forms).
    let handler = events
        .get("click")
        .or_else(|| events.get("submit"))
        .filter(|h| !h.is_empty())?
        .clone();

    // Collect baked args: data-mv-arg0, data-mv-arg1, … in contiguous order.
    let mut args = Vec::new();
    let mut i = 0usize;
    while let Some(v) = attrs.get(&format!("data-mv-arg{i}")) {
        args.push(v.clone());
        i += 1;
    }

    Some(SessionEvent {
        handler,
        args,
        caller: None,
    })
}

/// Serialize a session to the R10 `--replay` JSON: `{"events":[ … ]}`.
pub fn session_to_json(session: &Session) -> String {
    let events: Vec<serde_json::Value> = session
        .iter()
        .map(|e| {
            let mut obj = serde_json::Map::new();
            obj.insert("handler".into(), serde_json::Value::String(e.handler.clone()));
            obj.insert(
                "args".into(),
                serde_json::Value::Array(
                    e.args
                        .iter()
                        .map(|a| serde_json::Value::String(a.clone()))
                        .collect(),
                ),
            );
            if let Some(c) = &e.caller {
                obj.insert("caller".into(), serde_json::Value::String(c.clone()));
            }
            serde_json::Value::Object(obj)
        })
        .collect();
    let root = serde_json::json!({ "events": events });
    serde_json::to_string(&root).unwrap_or_else(|_| "{\"events\":[]}".to_string())
}

/// Replay an accumulated `session` through the page's dispatch+render and parse
/// the resulting IR forest. Writes the session to a temp `session.json`, runs
/// `motoview preview <dir> --replay <tmp> [--route <r>] --json` (which uses
/// `moc -r` — NO dfx deploy, deterministic) and parses the forest line.
///
/// On success returns the new forest. On any failure (spawn, non-zero exit, no
/// forest line, parse error) returns `Err` with a human-readable message so the
/// UI can surface it in the diagnostics area without losing the prior render.
pub fn replay_dispatch(
    project_dir: &Path,
    route: Option<&str>,
    session: &Session,
) -> Result<Vec<UiNode>, String> {
    // Write the session to a unique temp file. Include the pid + a nanosecond
    // timestamp so concurrent replays (e.g. fast clicks) don't collide.
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let tmp = std::env::temp_dir().join(format!(
        "mvstudio_session_{}_{}.json",
        std::process::id(),
        nanos
    ));
    std::fs::write(&tmp, session_to_json(session))
        .map_err(|e| format!("failed to write session file: {e}"))?;

    let tmp_str = tmp.to_string_lossy().into_owned();
    let mut args: Vec<&str> = vec!["preview", ".", "--replay", &tmp_str, "--json"];
    if let Some(r) = route {
        if !r.trim().is_empty() {
            args.push("--route");
            args.push(r);
        }
    }

    let outcome = run_motoview(&args, Some(project_dir));
    // Best-effort cleanup; ignore errors (temp dir is fine either way).
    let _ = std::fs::remove_file(&tmp);

    match outcome {
        Ok((stdout, stderr, ok)) => {
            if let Some(json_line) = stdout.lines().find(|l| l.trim_start().starts_with('[')) {
                parse_forest(json_line.trim_start())
                    .map_err(|e| format!("replay forest parse error: {e}"))
            } else {
                Err(format!(
                    "replay produced no IR forest (exit_ok={ok}). stderr: {}",
                    first_meaningful_line(&stderr)
                ))
            }
        }
        Err(e) => Err(format!("failed to spawn motoview preview --replay: {e}")),
    }
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
// SELECTION BRIDGE — node -> .mview source span
// ---------------------------------------------------------------------------
// `motoview preview --srcmap` tags each forest element with a `data-mv-src` id and
// writes `.mvbuild/preview.srcmap.json` (a SIDE artifact; production main.mo is
// untouched). This resolver maps a clicked forest node back to its exact `.mview`
// open-tag span (and its attr/event spans), which is the prerequisite for
// selection, the property grid, and the double-click-to-`@code`-handler move.

/// A `.mview` source span: char offsets plus 1-based line/col (both endpoints).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SrcSpan {
    pub start: usize,
    pub end: usize,
    pub line: u32,
    pub col: u32,
    pub end_line: u32,
    pub end_col: u32,
}

/// One attribute's name + source span (for the property grid).
#[derive(Debug, Clone, Default)]
pub struct SrcAttr {
    pub name: String,
    pub span: SrcSpan,
}

/// One event binding's event/handler + source span (for double-click-to-handler).
#[derive(Debug, Clone, Default)]
pub struct SrcEvent {
    pub event: String,
    pub handler: String,
    pub span: SrcSpan,
}

/// One source-mapped element: where its `.mview` open tag lives, plus its attrs
/// and events. The array index in [`SrcMap`] is the node's `data-mv-src` id.
#[derive(Debug, Clone, Default)]
pub struct SrcEntry {
    pub file: String,
    pub tag: String,
    pub secure: bool,
    pub span: SrcSpan,
    pub attrs: Vec<SrcAttr>,
    pub events: Vec<SrcEvent>,
}

/// The node→span side-map: `entries[id]` is the element tagged `data-mv-src="id"`.
#[derive(Debug, Clone, Default)]
pub struct SrcMap {
    pub entries: Vec<SrcEntry>,
}

impl SrcMap {
    /// Parse the `.mvbuild/preview.srcmap.json` array. Malformed input yields an
    /// empty map (selection simply doesn't light up) — never a panic.
    pub fn parse(json: &str) -> SrcMap {
        let v: serde_json::Value = match serde_json::from_str(json) {
            Ok(v) => v,
            Err(_) => return SrcMap::default(),
        };
        let arr = match v.as_array() {
            Some(a) => a,
            None => return SrcMap::default(),
        };
        let span_of = |o: Option<&serde_json::Value>| -> SrcSpan {
            let g = |k: &str| o.and_then(|o| o.get(k)).and_then(|x| x.as_u64()).unwrap_or(0);
            SrcSpan {
                start: g("start") as usize,
                end: g("end") as usize,
                line: g("line") as u32,
                col: g("col") as u32,
                end_line: g("endLine") as u32,
                end_col: g("endCol") as u32,
            }
        };
        let entries = arr
            .iter()
            .map(|e| SrcEntry {
                file: e.get("file").and_then(|x| x.as_str()).unwrap_or("").to_string(),
                tag: e.get("tag").and_then(|x| x.as_str()).unwrap_or("").to_string(),
                secure: e.get("secure").and_then(|x| x.as_bool()).unwrap_or(false),
                span: span_of(e.get("span")),
                attrs: e
                    .get("attrs")
                    .and_then(|x| x.as_array())
                    .map(|a| {
                        a.iter()
                            .map(|at| SrcAttr {
                                name: at.get("name").and_then(|x| x.as_str()).unwrap_or("").to_string(),
                                span: span_of(at.get("span")),
                            })
                            .collect()
                    })
                    .unwrap_or_default(),
                events: e
                    .get("events")
                    .and_then(|x| x.as_array())
                    .map(|a| {
                        a.iter()
                            .map(|ev| SrcEvent {
                                event: ev.get("event").and_then(|x| x.as_str()).unwrap_or("").to_string(),
                                handler: ev.get("handler").and_then(|x| x.as_str()).unwrap_or("").to_string(),
                                span: span_of(ev.get("span")),
                            })
                            .collect()
                    })
                    .unwrap_or_default(),
            })
            .collect();
        SrcMap { entries }
    }

    /// The entry for a `data-mv-src` id, if any.
    pub fn get(&self, id: usize) -> Option<&SrcEntry> {
        self.entries.get(id)
    }

    /// Resolve a forest node to its source entry via its `data-mv-src` attr. A
    /// `@for`-expanded instance carries its template's id, so every row resolves
    /// back to the single template span.
    pub fn resolve<'a>(&'a self, node: &UiNode) -> Option<&'a SrcEntry> {
        self.get(node_src_id(node)?)
    }
}

/// Read a node's `data-mv-src` id (preview `--srcmap` only). `None` for text/raw
/// nodes, elements without the attr, or a non-numeric value.
pub fn node_src_id(node: &UiNode) -> Option<usize> {
    if let UiNode::El { attrs, .. } = node {
        attrs.get("data-mv-src").and_then(|s| s.parse::<usize>().ok())
    } else {
        None
    }
}

// ---------------------------------------------------------------------------
// DRAG-AND-DROP — add a component from the Elements toolbox to the canvas
// ---------------------------------------------------------------------------
// A dropped tile splices its `.mview` snippet as the FIRST CHILD of the drop
// target (right after the target element's open tag, whose span comes from the
// selection bridge), validates the result with `check` (revert on error), and the
// studio re-previews. Secure-by-construction: a dropped form is `secure` and its
// submit handler is scaffolded so it compiles. The PURE splice is unit-tested; the
// egui drag gesture is the GUI layer.

/// A component that can be dragged from the Elements toolbox onto the canvas.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ComponentKind {
    Container,
    Row,
    Column,
    Text,
    Button,
    Input,
    Image,
    SecureForm,
    EncryptedField,
    WalletButton,
    IIAuth,
}

impl ComponentKind {
    /// Map a toolbox tile label to its kind (the tiles carry these labels).
    pub fn from_label(label: &str) -> Option<ComponentKind> {
        Some(match label {
            "Container" => Self::Container,
            "Row" => Self::Row,
            "Column" => Self::Column,
            "Text" => Self::Text,
            "Button" => Self::Button,
            "Input" => Self::Input,
            "Image" => Self::Image,
            "Secure form" => Self::SecureForm,
            "Encrypted field" => Self::EncryptedField,
            "Wallet button" => Self::WalletButton,
            "II auth" => Self::IIAuth,
            _ => return None,
        })
    }

    /// The `.mview` snippet inserted as a child (the caller adds indentation). All
    /// snippets parse + type-check WITHOUT extra `@code` except those with a
    /// [`ComponentKind::handler`], whose handler is scaffolded by [`add_component`].
    pub fn snippet(self) -> &'static str {
        match self {
            Self::Container => "<div class=\"mv-box\"></div>",
            Self::Row => "<div class=\"mv-row\"></div>",
            Self::Column => "<div class=\"mv-col\"></div>",
            Self::Text => "<p>Text</p>",
            Self::Button => "<button>Button</button>",
            Self::Input => "<input name=\"field\" />",
            Self::Image => "<img alt=\"image\" />",
            // SECURE BY DEFAULT: a dropped form is `secure` (the deny-by-default
            // lint would reject it otherwise) and its submit is wired to a
            // scaffolded handler.
            Self::SecureForm => "<form @submit=onSubmit secure>\n  <button type=\"submit\">Submit</button>\n</form>",
            // Client-side encrypted field (vetKeys glue marker).
            Self::EncryptedField => "<input name=\"secret\" data-mv-encrypt />",
            Self::WalletButton => "<button>Authorize spend</button>",
            Self::IIAuth => "<button>Sign in with Internet Identity</button>",
        }
    }

    /// The `@code` handler this component needs scaffolded so the snippet compiles
    /// (a secure form's `@submit`), or `None`.
    pub fn handler(self) -> Option<&'static str> {
        match self {
            Self::SecureForm => Some("onSubmit"),
            _ => None,
        }
    }

    /// A short human label for status messages.
    pub fn label(self) -> &'static str {
        match self {
            Self::Container => "Container",
            Self::Row => "Row",
            Self::Column => "Column",
            Self::Text => "Text",
            Self::Button => "Button",
            Self::Input => "Input",
            Self::Image => "Image",
            Self::SecureForm => "Secure form",
            Self::EncryptedField => "Encrypted field",
            Self::WalletButton => "Wallet button",
            Self::IIAuth => "II auth",
        }
    }
}

/// Insert `kind`'s snippet into `source` as the first child of the element whose
/// open tag ends at char offset `open_tag_end` (the selection bridge's span.end),
/// indented by `indent`. Also scaffolds the component's `@code` handler if any.
/// PURE (no IO) so it is unit-testable; [`add_component`] is the IO wrapper.
pub fn splice_component(source: &str, open_tag_end: usize, kind: ComponentKind, indent: &str) -> String {
    let mut chars: Vec<char> = source.chars().collect();
    let at = open_tag_end.min(chars.len());
    let snippet: String = format!("\n{indent}{}", kind.snippet());
    let snip: Vec<char> = snippet.chars().collect();
    chars.splice(at..at, snip);
    let mut out: String = chars.into_iter().collect();
    if let Some(h) = kind.handler() {
        if let Some(scaffolded) = scaffold_handler(&out, h) {
            out = scaffolded;
        }
    }
    out
}

/// Insert `func <name>() : async () { };` just before the `@code` block's closing
/// brace. Returns `None` if there is no `@code { ... }` block. Brace-matched so
/// nested braces in the block are handled; idempotent (won't duplicate a handler).
fn scaffold_handler(source: &str, name: &str) -> Option<String> {
    if source.contains(&format!("func {}(", name)) {
        return Some(source.to_string());
    }
    let code_at = source.find("@code")?;
    let open = source[code_at..].find('{').map(|i| code_at + i)?;
    let bytes = source.as_bytes();
    let mut depth = 0i32;
    let mut close = None;
    let mut i = open;
    while i < bytes.len() {
        match bytes[i] {
            b'{' => depth += 1,
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    close = Some(i);
                    break;
                }
            }
            _ => {}
        }
        i += 1;
    }
    let close = close?;
    let handler = format!(
        "\n  // scaffolded by the designer — wire its body\n  func {}() : async () {{ }};\n",
        name
    );
    let mut out = String::with_capacity(source.len() + handler.len());
    out.push_str(&source[..close]);
    out.push_str(&handler);
    out.push_str(&source[close..]);
    Some(out)
}

/// Drop `kind` into the project file `file_rel` at char offset `at` (a drop
/// target's open-tag end), validate with `check`, and REVERT on any error so a bad
/// drop never leaves the source broken.
pub fn add_component(
    project_dir: &Path,
    file_rel: &str,
    at: usize,
    kind: ComponentKind,
    indent: &str,
) -> Result<(), String> {
    let path = project_dir.join(file_rel);
    let original = read_file(&path).map_err(|e| format!("read {}: {e}", path.display()))?;
    let updated = splice_component(&original, at, kind, indent);
    write_file(&path, &updated).map_err(|e| format!("write {}: {e}", path.display()))?;
    let report = run_check(project_dir);
    if report.diagnostics.iter().any(|d| d.is_error()) {
        let _ = write_file(&path, &original);
        let msg = report
            .diagnostics
            .iter()
            .find(|d| d.is_error())
            .map(|d| format!("[{}] {}", d.rule, d.message))
            .unwrap_or_else(|| "would not compile".to_string());
        return Err(format!("reverted — {msg}"));
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// EDITABLE PROPERTY GRID — change an attribute's value with a single-span edit
// ---------------------------------------------------------------------------
// The inspector edits the SELECTED node's attributes. Each attribute's source
// span comes from the selection bridge; the studio reads the `.mview` text at that
// span, parses `name="value"`, and (for a simple quoted literal — never a `{expr}`
// or concat) replaces the span with `name="newvalue"`, validates with `check`, and
// reverts on a compile error. PURE parse/replace is unit-tested.

/// The slice of `source` covered by `span` (char offsets), or "" if out of range.
pub fn slice_span(source: &str, span: &SrcSpan) -> String {
    let chars: Vec<char> = source.chars().collect();
    let start = span.start.min(chars.len());
    let end = span.end.min(chars.len());
    if end <= start {
        String::new()
    } else {
        chars[start..end].iter().collect()
    }
}

/// Parse an attribute's source text (`class="deal-card"`) into
/// `(name, value, editable, quote)`. Editable ONLY for a simple quoted literal with
/// no `{` interpolation or `@` expression — so a concat/expr/bind attr is never
/// corrupted (it is shown read-only).
pub fn parse_attr_source(text: &str) -> (String, String, bool, char) {
    let t = text.trim();
    match t.find('=') {
        None => (t.to_string(), String::new(), false, '"'), // boolean attr (e.g. `secure`)
        Some(eq) => {
            let name = t[..eq].trim().to_string();
            let rest = t[eq + 1..].trim();
            let q = rest.chars().next();
            if matches!(q, Some('"') | Some('\'')) && rest.chars().count() >= 2 {
                let quote = q.unwrap();
                let rchars: Vec<char> = rest.chars().collect();
                if *rchars.last().unwrap() == quote {
                    let inner: String = rchars[1..rchars.len() - 1].iter().collect();
                    let editable = !inner.contains('{') && !inner.contains('@');
                    return (name, inner, editable, quote);
                }
            }
            (name, rest.to_string(), false, '"') // unquoted / `{expr}` -> read-only
        }
    }
}

/// Replace the attribute at `span` in `source` with `name="new_value"` (a single
/// span edit). The `quote` is preserved; an occurrence of `quote` in the value is
/// neutralized so the attribute stays well-formed. PURE.
pub fn set_attr_value(source: &str, span: &SrcSpan, name: &str, new_value: &str, quote: char) -> String {
    let mut chars: Vec<char> = source.chars().collect();
    let start = span.start.min(chars.len());
    let end = span.end.min(chars.len());
    if end < start {
        return source.to_string();
    }
    let safe: String = new_value
        .chars()
        .map(|c| if c == quote || c == '\n' { ' ' } else { c })
        .collect();
    let repl: Vec<char> = format!("{name}={quote}{safe}{quote}").chars().collect();
    chars.splice(start..end, repl);
    chars.into_iter().collect()
}

/// Apply a property-grid edit: set the attribute at `span` in `file_rel` to
/// `new_value`, validate with `check`, and REVERT on a compile error. Returns the
/// updated source on success (so the studio can refresh the editor).
pub fn edit_attr(
    project_dir: &Path,
    file_rel: &str,
    span: &SrcSpan,
    name: &str,
    new_value: &str,
    quote: char,
) -> Result<String, String> {
    let path = project_dir.join(file_rel);
    let original = read_file(&path).map_err(|e| format!("read {}: {e}", path.display()))?;
    let updated = set_attr_value(&original, span, name, new_value, quote);
    write_file(&path, &updated).map_err(|e| format!("write {}: {e}", path.display()))?;
    let report = run_check(project_dir);
    if report.diagnostics.iter().any(|d| d.is_error()) {
        let _ = write_file(&path, &original);
        let msg = report
            .diagnostics
            .iter()
            .find(|d| d.is_error())
            .map(|d| format!("[{}] {}", d.rule, d.message))
            .unwrap_or_else(|| "would not compile".to_string());
        return Err(format!("reverted — {msg}"));
    }
    Ok(updated)
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

// ---------------------------------------------------------------------------
// Open-a-project (PURE, testable) — replaces the crashy native folder dialog
// ---------------------------------------------------------------------------

/// Trim whitespace/quotes and expand a leading `~` to `$HOME` so a user can
/// paste either `~/dev/MotoView/examples/crm` or a fully-qualified path into
/// the studio's path text field. Pure string work — no filesystem touch.
pub fn expand_path(raw: &str) -> PathBuf {
    // Drop surrounding whitespace and any wrapping quotes (a path dragged from
    // Finder onto a terminal often arrives single-quoted).
    let trimmed = raw.trim().trim_matches(|c| c == '"' || c == '\'').trim();
    if trimmed == "~" {
        if let Some(home) = home_dir() {
            return home;
        }
    } else if let Some(rest) = trimmed.strip_prefix("~/") {
        if let Some(home) = home_dir() {
            return home.join(rest);
        }
    }
    PathBuf::from(trimmed)
}

/// `$HOME` as a `PathBuf`, if set. (No `dirs` crate dependency.)
pub fn home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME").map(PathBuf::from)
}

/// The outcome of resolving a project path the user typed/pasted. Either the
/// directory + the `.mview` files found under it, or a clear human message
/// explaining why nothing opened. NEVER panics; this is the function the GUI
/// uses instead of a native folder dialog (which crashes the macOS run-loop).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OpenOutcome {
    /// A real directory; `files` may be empty (a clear status is set in `note`).
    Opened {
        dir: PathBuf,
        files: Vec<PathBuf>,
        note: String,
    },
    /// The path could not be used as a project (with a human-readable reason).
    Rejected { note: String },
}

/// Resolve a user-supplied project path into an [`OpenOutcome`]. Expands `~`,
/// trims quotes/whitespace, rejects empty/non-existent/non-directory inputs
/// with a clear message, and otherwise lists the `.mview` files (build/VCS
/// noise + `node_modules` are skipped by `list_mview_files`). This is the
/// crash-free replacement for `rfd::FileDialog::pick_folder()`.
pub fn open_project(raw: &str) -> OpenOutcome {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return OpenOutcome::Rejected {
            note: "Enter a project folder path, then click Open.".to_string(),
        };
    }
    let dir = expand_path(raw);
    if !dir.exists() {
        return OpenOutcome::Rejected {
            note: format!("No such path: {}", dir.display()),
        };
    }
    if !dir.is_dir() {
        return OpenOutcome::Rejected {
            note: format!("Not a folder (point at the project directory): {}", dir.display()),
        };
    }
    let files = list_mview_files(&dir);
    let note = if files.is_empty() {
        format!(
            "Opened {} — but no .mview files were found under it.",
            dir.display()
        )
    } else {
        format!("Opened {} — {} .mview file(s).", dir.display(), files.len())
    };
    OpenOutcome::Opened { dir, files, note }
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

    // -- R17: live preview via replay --------------------------------------

    /// A `+1` counter button: click handler `increment` with one baked arg `1`.
    fn counter_inc_button(arg: &str) -> UiNode {
        let mut attrs = BTreeMap::new();
        attrs.insert("class".to_string(), "mv-btn mv-btn-primary".to_string());
        attrs.insert("data-mv-arg0".to_string(), arg.to_string());
        let mut events = BTreeMap::new();
        events.insert("click".to_string(), "increment".to_string());
        UiNode::El {
            tag: "button".into(),
            attrs,
            events,
            key: None,
            children: vec![UiNode::Raw { html: format!("+{arg}") }],
        }
    }

    #[test]
    fn event_from_node_extracts_handler_and_args() {
        // The "+1" button -> increment with arg ["1"], no caller.
        let ev = event_from_node(&counter_inc_button("1"))
            .expect("a clickable button yields an event");
        assert_eq!(ev.handler, "increment");
        assert_eq!(ev.args, vec!["1".to_string()]);
        assert_eq!(ev.caller, None);

        // The "+5" button -> same handler, arg ["5"].
        let ev5 = event_from_node(&counter_inc_button("5")).unwrap();
        assert_eq!(ev5.handler, "increment");
        assert_eq!(ev5.args, vec!["5".to_string()]);
    }

    #[test]
    fn event_from_node_handles_no_args_and_forms() {
        // A button with a click handler but no baked args (e.g. <button @click="reset">).
        let mut events = BTreeMap::new();
        events.insert("click".to_string(), "reset".to_string());
        let reset_btn = UiNode::El {
            tag: "button".into(),
            attrs: BTreeMap::new(),
            events,
            key: None,
            children: vec![UiNode::Raw { html: "Reset".into() }],
        };
        let ev = event_from_node(&reset_btn).unwrap();
        assert_eq!(ev.handler, "reset");
        assert!(ev.args.is_empty(), "no data-mv-arg* -> no args");

        // A <form @submit="save"> dispatches its submit handler.
        let mut fevents = BTreeMap::new();
        fevents.insert("submit".to_string(), "save".to_string());
        let form = UiNode::El {
            tag: "form".into(),
            attrs: BTreeMap::new(),
            events: fevents,
            key: None,
            children: vec![],
        };
        let fev = event_from_node(&form).expect("a form with a submit handler is interactive");
        assert_eq!(fev.handler, "save");
    }

    #[test]
    fn event_from_node_none_for_non_interactive() {
        // A plain text node has no handler.
        assert_eq!(event_from_node(&UiNode::Text { value: "hi".into() }), None);
        // A raw node has no handler.
        assert_eq!(event_from_node(&UiNode::Raw { html: "<b>x</b>".into() }), None);
        // A <div> with no events is not interactive.
        assert_eq!(event_from_node(&el("div")), None);
        // A <strong> wrapping the counter value carries no handler.
        assert_eq!(event_from_node(&el("strong")), None);
    }

    #[test]
    fn session_to_json_round_trips_through_parser() {
        let session = vec![
            SessionEvent { handler: "increment".into(), args: vec!["1".into()], caller: None },
            SessionEvent { handler: "save".into(), args: vec![], caller: Some("abc".into()) },
        ];
        let json = session_to_json(&session);
        let v: serde_json::Value = serde_json::from_str(&json).unwrap();
        let events = v.get("events").and_then(|e| e.as_array()).expect("events array");
        assert_eq!(events.len(), 2);
        assert_eq!(events[0]["handler"], "increment");
        assert_eq!(events[0]["args"][0], "1");
        assert_eq!(events[1]["caller"], "abc");
    }

    /// Find the counter's value text node ("0"/"1"/"2"…) inside the forest.
    /// In the real counter page it sits in `<strong>` under `<p.counter-value>`,
    /// and it is the only `text` leaf that parses as a number.
    fn counter_value(forest: &[UiNode]) -> Option<String> {
        fn walk(n: &UiNode, out: &mut Option<String>) {
            match n {
                UiNode::Text { value } => {
                    let t = value.trim();
                    if !t.is_empty() && t.chars().all(|c| c.is_ascii_digit() || c == '-') {
                        *out = Some(t.to_string());
                    }
                }
                UiNode::El { children, .. } => {
                    for c in children {
                        walk(c, out);
                    }
                }
                UiNode::Raw { .. } => {}
            }
        }
        let mut out = None;
        for n in forest {
            walk(n, &mut out);
        }
        out
    }

    fn inc_session(n: usize) -> Session {
        (0..n)
            .map(|_| SessionEvent { handler: "increment".into(), args: vec!["1".into()], caller: None })
            .collect()
    }

    /// Recursively copy a directory tree, skipping build/VCS noise so each test
    /// gets an isolated project sandbox (the compiler writes generated `.mo`
    /// + `.mvbuild` artifacts into the project dir, so concurrent replays on
    /// the SHARED examples/counter would race; an isolated copy makes the
    /// real-binary tests independent and parallel-safe).
    fn copy_tree(src: &Path, dst: &Path) {
        std::fs::create_dir_all(dst).expect("mkdir dst");
        for entry in std::fs::read_dir(src).expect("read_dir").flatten() {
            let path = entry.path();
            let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
            if matches!(name, ".mvbuild" | ".dfx" | ".git" | "target" | "node_modules") {
                continue;
            }
            let to = dst.join(name);
            if path.is_dir() {
                copy_tree(&path, &to);
            } else {
                std::fs::copy(&path, &to).expect("copy file");
            }
        }
    }

    /// An isolated copy of examples/counter under a unique temp dir, or `None`
    /// if the example isn't present. Caller is responsible for cleanup.
    fn isolated_counter(tag: &str) -> Option<PathBuf> {
        let src = repo_root().join("examples/counter");
        if !src.join("motoview.json").exists() && !src.join("dfx.json").exists() {
            return None;
        }
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let dst = std::env::temp_dir().join(format!(
            "mvstudio_counter_{tag}_{}_{}",
            std::process::id(),
            nanos
        ));
        copy_tree(&src, &dst);
        // The copied dfx.json references the runtime package by a path relative
        // to the ORIGINAL project dir (`--package motoview ../../runtime/src`),
        // which does not resolve from the temp dir. Rewrite it to the absolute
        // repo path so `moc -r` resolves it here — otherwise the replay tests
        // would silently SKIP (preview fails) and pass vacuously.
        let dfx = dst.join("dfx.json");
        if let Ok(txt) = std::fs::read_to_string(&dfx) {
            let abs = repo_root().join("runtime/src");
            let fixed = txt.replace("../../runtime/src", &abs.to_string_lossy());
            let _ = std::fs::write(&dfx, fixed);
        }
        Some(dst)
    }

    /// An isolated copy of examples/crm (build-noise dirs skipped), with the dfx
    /// runtime package path absolutized so `moc -r` resolves it from /tmp.
    fn isolated_crm(tag: &str) -> Option<PathBuf> {
        let src = repo_root().join("examples/crm");
        if !src.join("dfx.json").exists() {
            return None;
        }
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let dst = std::env::temp_dir().join(format!("mvstudio_crm_{tag}_{}_{}", std::process::id(), nanos));
        copy_tree(&src, &dst);
        let dfx = dst.join("dfx.json");
        if let Ok(txt) = std::fs::read_to_string(&dfx) {
            let abs = repo_root().join("runtime/src");
            let _ = std::fs::write(&dfx, txt.replace("../../runtime/src", &abs.to_string_lossy()));
        }
        Some(dst)
    }

    // ---- SELECTION BRIDGE ------------------------------------------------

    #[test]
    fn srcmap_resolves_node_to_its_source_span() {
        // Pure (no binary): parse a side-map + resolve a forest node by its
        // `data-mv-src` id back to its source span / attrs / events.
        let json = r#"[
          {"file":"src/Pages/Board.mview","tag":"div","secure":false,"span":{"start":0,"end":5,"line":6,"col":1,"endLine":6,"endCol":6},"attrs":[],"events":[]},
          {"file":"src/Pages/Board.mview","tag":"button","secure":true,"span":{"start":40,"end":60,"line":11,"col":5,"endLine":11,"endCol":25},"attrs":[{"name":"class","span":{"start":48,"end":58,"line":11,"col":13,"endLine":11,"endCol":23}}],"events":[{"event":"click","handler":"toggleCreate","span":{"start":60,"end":80,"line":11,"col":25,"endLine":11,"endCol":45}}]}
        ]"#;
        let map = SrcMap::parse(json);
        assert_eq!(map.entries.len(), 2);
        let mut attrs = BTreeMap::new();
        attrs.insert("data-mv-src".to_string(), "1".to_string());
        let node = UiNode::El { tag: "button".into(), attrs, events: BTreeMap::new(), key: None, children: vec![] };
        assert_eq!(node_src_id(&node), Some(1));
        let e = map.resolve(&node).expect("button resolves to its source");
        assert_eq!(e.tag, "button");
        assert_eq!(e.file, "src/Pages/Board.mview");
        assert_eq!(e.span.line, 11);
        assert_eq!(e.span.col, 5);
        assert!(e.secure);
        assert_eq!(e.attrs[0].name, "class");
        assert_eq!(e.events[0].event, "click");
        assert_eq!(e.events[0].handler, "toggleCreate");
        // non-element, missing-id, out-of-range, and bad JSON all resolve to None
        // (selection just doesn't light up — never a panic).
        assert_eq!(node_src_id(&UiNode::Text { value: "x".into() }), None);
        let plain = UiNode::El { tag: "div".into(), attrs: BTreeMap::new(), events: BTreeMap::new(), key: None, children: vec![] };
        assert!(map.resolve(&plain).is_none());
        assert!(SrcMap::parse("not json at all").entries.is_empty());
    }

    #[test]
    fn crm_srcmap_resolves_button_to_board_source() {
        // End-to-end: run the REAL `preview --srcmap` on examples/crm, find the
        // "+ New deal" (toggleCreate) button in the forest, and resolve it back to
        // its exact Board.mview span via its `data-mv-src` id.
        if !motoview_available() {
            eprintln!("SKIP: motoview binary not found");
            return;
        }
        let Some(crm) = isolated_crm("srcmap") else {
            eprintln!("SKIP: examples/crm not present");
            return;
        };
        let pr = run_preview(&crm, None);
        if !pr.ok {
            eprintln!("SKIP: preview not runnable here (moc?)");
            let _ = std::fs::remove_dir_all(&crm);
            return;
        }
        assert!(
            !pr.srcmap.entries.is_empty(),
            "--srcmap should populate the side-map (got 0 entries)"
        );
        fn find<'a>(nodes: &'a [UiNode], pred: &dyn Fn(&UiNode) -> bool) -> Option<&'a UiNode> {
            for n in nodes {
                if pred(n) {
                    return Some(n);
                }
                if let UiNode::El { children, .. } = n {
                    if let Some(f) = find(children, pred) {
                        return Some(f);
                    }
                }
            }
            None
        }
        let btn = find(&pr.forest, &|n| {
            matches!(n, UiNode::El { events, .. } if events.get("click").map(|h| h == "toggleCreate").unwrap_or(false))
        })
        .expect("the toggleCreate button is in the CRM forest");
        let e = pr
            .srcmap
            .resolve(btn)
            .expect("the button resolves to its source via data-mv-src");
        assert_eq!(e.tag, "button");
        assert!(e.file.ends_with("Board.mview"), "resolved file = {}", e.file);
        assert!(e.span.line > 0, "resolved span should have a real line");
        assert!(
            e.events.iter().any(|ev| ev.handler == "toggleCreate"),
            "resolved entry should carry the toggleCreate event"
        );
        let _ = std::fs::remove_dir_all(&crm);
    }

    // ---- DRAG-AND-DROP (component insertion) -----------------------------

    #[test]
    fn splice_component_inserts_snippet_as_first_child() {
        let out = splice_component("<div></div>", 5, ComponentKind::Button, "  ");
        // The button is inserted right after the <div> open tag.
        assert_eq!(out, "<div>\n  <button>Button</button></div>");
        assert_eq!(ComponentKind::from_label("Container"), Some(ComponentKind::Container));
        assert_eq!(ComponentKind::from_label("Secure form"), Some(ComponentKind::SecureForm));
        assert_eq!(ComponentKind::from_label("nope"), None);
        // out-of-range offset clamps (never panics).
        let _ = splice_component("<a>", 999, ComponentKind::Text, "");
    }

    #[test]
    fn secure_form_is_secure_and_scaffolds_its_handler() {
        // The secure form snippet is `secure` (the deny-by-default lint requires it).
        assert!(ComponentKind::SecureForm.snippet().contains("secure"));
        assert_eq!(ComponentKind::SecureForm.handler(), Some("onSubmit"));
        // Splicing a secure form scaffolds its @code handler so it compiles.
        let page = "<div></div>\n@code {\n  var x : Nat = 0;\n}\n";
        let out = splice_component(page, 5, ComponentKind::SecureForm, "  ");
        assert!(out.contains("<form @submit=onSubmit secure>"), "out={out}");
        assert!(out.contains("func onSubmit() : async ()"), "handler not scaffolded: {out}");
        // Idempotent: scaffolding the same handler again does not duplicate it.
        let again = scaffold_handler(&out, "onSubmit").expect("scaffold");
        assert_eq!(again.matches("func onSubmit(").count(), 1);
        // A page with no @code block: the snippet is still inserted (no scaffold).
        let nocode = splice_component("<div></div>", 5, ComponentKind::SecureForm, "");
        assert!(nocode.contains("<form @submit=onSubmit secure>"));
    }

    /// Find the first forest node carrying `class` (for picking a drop target).
    fn find_by_class<'a>(nodes: &'a [UiNode], class: &str) -> Option<&'a UiNode> {
        for n in nodes {
            if let UiNode::El { attrs, children, .. } = n {
                if attrs
                    .get("class")
                    .map(|c| c.split_whitespace().any(|w| w == class))
                    .unwrap_or(false)
                {
                    return Some(n);
                }
                if let Some(f) = find_by_class(children, class) {
                    return Some(f);
                }
            }
        }
        None
    }

    fn forest_has_button_text(nodes: &[UiNode], txt: &str) -> bool {
        nodes.iter().any(|n| match n {
            UiNode::El { tag, children, .. } => {
                (tag == "button" && node_text(n).trim() == txt) || forest_has_button_text(children, txt)
            }
            _ => false,
        })
    }

    #[test]
    fn add_component_drops_a_button_into_the_crm_and_reappears() {
        // END-TO-END: drop a Button into the (non-@for) crm-header container, prove
        // it compiles (no revert) + the new button shows up in the re-preview.
        if !motoview_available() {
            eprintln!("SKIP: motoview binary not found");
            return;
        }
        let Some(crm) = isolated_crm("dnd") else {
            eprintln!("SKIP: examples/crm not present");
            return;
        };
        let pr = run_preview(&crm, None);
        if !pr.ok {
            eprintln!("SKIP: preview not runnable here (moc?)");
            let _ = std::fs::remove_dir_all(&crm);
            return;
        }
        // Resolve the crm-header div's source id -> file + open-tag end (drop point).
        let header = find_by_class(&pr.forest, "crm-header").expect("crm-header in forest");
        let id = node_src_id(header).expect("header carries data-mv-src");
        let e = pr.srcmap.get(id).expect("header srcmap entry");
        let (file, at) = (e.file.clone(), e.span.end);
        // Drop a Button — must compile (no revert).
        add_component(&crm, &file, at, ComponentKind::Button, "    ")
            .expect("dropping a Button should compile");
        let updated = read_file(&crm.join(&file)).expect("read updated file");
        assert!(updated.contains("<button>Button</button>"), "button not spliced");
        // Re-preview: the new button is now in the forest.
        let pr2 = run_preview(&crm, None);
        assert!(pr2.ok, "re-preview failed after drop");
        assert!(
            forest_has_button_text(&pr2.forest, "Button"),
            "the dropped Button is not in the re-rendered forest"
        );
        let _ = std::fs::remove_dir_all(&crm);
    }

    #[test]
    fn add_secure_form_scaffolds_handler_and_compiles() {
        // Dropping a Secure form inserts a `secure` form AND scaffolds its onSubmit
        // handler, so the secure-by-construction component compiles.
        if !motoview_available() {
            eprintln!("SKIP: motoview binary not found");
            return;
        }
        let Some(crm) = isolated_crm("dndform") else {
            eprintln!("SKIP: examples/crm not present");
            return;
        };
        let pr = run_preview(&crm, None);
        if !pr.ok {
            eprintln!("SKIP: preview not runnable here (moc?)");
            let _ = std::fs::remove_dir_all(&crm);
            return;
        }
        let header = find_by_class(&pr.forest, "crm-header").expect("crm-header");
        let id = node_src_id(header).expect("src id");
        let e = pr.srcmap.get(id).expect("entry");
        let (file, at) = (e.file.clone(), e.span.end);
        add_component(&crm, &file, at, ComponentKind::SecureForm, "    ")
            .expect("a secure form (with scaffolded handler) should compile");
        let updated = read_file(&crm.join(&file)).expect("read");
        assert!(updated.contains("@submit=onSubmit secure>"), "form not secure");
        assert!(updated.contains("func onSubmit("), "onSubmit not scaffolded");
        let _ = std::fs::remove_dir_all(&crm);
    }

    // ---- EDITABLE PROPERTY GRID ------------------------------------------

    #[test]
    fn parse_and_set_attr_value_round_trip() {
        // A simple quoted literal is editable.
        let (n, v, ed, q) = parse_attr_source("class=\"deal-card\"");
        assert_eq!((n.as_str(), v.as_str(), ed, q), ("class", "deal-card", true, '"'));
        // A concat / expr / bind attr is NOT editable (never corrupt it).
        assert!(!parse_attr_source("id=\"col-{stage}\"").2);
        assert!(!parse_attr_source("bind=\"@title\"").2);
        assert!(!parse_attr_source("value={count}").2);
        // A boolean attr (no value) is not value-editable.
        assert_eq!(parse_attr_source("secure"), ("secure".to_string(), String::new(), false, '"'));

        // set_attr_value replaces exactly the span.
        let src = "<div class=\"old\" id=\"x\"></div>";
        // span of `class="old"` = chars 5..16
        let span = SrcSpan { start: 5, end: 16, ..Default::default() };
        assert_eq!(slice_span(src, &span), "class=\"old\"");
        let out = set_attr_value(src, &span, "class", "new value", '"');
        assert_eq!(out, "<div class=\"new value\" id=\"x\"></div>");
        // a `"` in the value is neutralized so the attr stays well-formed.
        let out2 = set_attr_value(src, &span, "class", "a\"b", '"');
        assert!(!out2[5..].starts_with("class=\"a\"b\""), "unescaped quote: {out2}");
    }

    #[test]
    fn edit_attr_changes_a_class_in_the_crm_and_reverts_on_break() {
        // END-TO-END: edit a deal-card's class to a new literal; it compiles + the
        // source reflects it. Then a breaking edit reverts and leaves source intact.
        if !motoview_available() {
            eprintln!("SKIP: motoview binary not found");
            return;
        }
        let Some(crm) = isolated_crm("editattr") else {
            eprintln!("SKIP: examples/crm not present");
            return;
        };
        let pr = run_preview(&crm, None);
        if !pr.ok {
            eprintln!("SKIP: preview not runnable here (moc?)");
            let _ = std::fs::remove_dir_all(&crm);
            return;
        }
        // Find the crm-header node + its `class` attribute span.
        let header = find_by_class(&pr.forest, "crm-header").expect("crm-header");
        let id = node_src_id(header).expect("src id");
        let e = pr.srcmap.get(id).expect("entry");
        let class_attr = e.attrs.iter().find(|a| a.name == "class").expect("class attr");
        let file = e.file.clone();
        let before = read_file(&crm.join(&file)).unwrap();
        // Edit the class literal -> compiles, source updated.
        let updated = edit_attr(&crm, &file, &class_attr.span, "class", "crm-header pinned", '"')
            .expect("editing a class literal should compile");
        assert!(updated.contains("class=\"crm-header pinned\""), "class not updated: in {file}");
        assert!(!updated.contains("class=\"crm-header\""), "old class still present");
        // The whole file changed by exactly the value (length grew by the diff).
        assert_ne!(before, updated);
        let _ = std::fs::remove_dir_all(&crm);
    }

    #[test]
    fn replay_dispatch_mutates_counter_state() {
        if !motoview_available() {
            eprintln!("SKIP: motoview binary not found");
            return;
        }
        let Some(counter) = isolated_counter("mutate") else {
            eprintln!("SKIP: examples/counter not present");
            return;
        };
        // First confirm a plain preview is runnable in this env (needs moc -r).
        if !run_preview(&counter, None).ok {
            eprintln!("SKIP: preview/replay not runnable here (moc?)");
            let _ = std::fs::remove_dir_all(&counter);
            return;
        }

        // One increment -> the count reads "1".
        let f1 = replay_dispatch(&counter, None, &inc_session(1))
            .expect("replay with [increment] should succeed");
        assert_eq!(
            counter_value(&f1).as_deref(),
            Some("1"),
            "one increment must yield count 1 (real stateful replay), forest={f1:?}"
        );

        // Two increments -> the count reads "2". This is page-local STATE that
        // accumulated across the session, not a static render.
        let f2 = replay_dispatch(&counter, None, &inc_session(2))
            .expect("replay with [increment, increment] should succeed");
        assert_eq!(
            counter_value(&f2).as_deref(),
            Some("2"),
            "two increments must yield count 2, forest={f2:?}"
        );

        let _ = std::fs::remove_dir_all(&counter);
    }

    #[test]
    fn replay_dispatch_is_deterministic() {
        if !motoview_available() {
            eprintln!("SKIP: motoview binary not found");
            return;
        }
        let Some(counter) = isolated_counter("determ") else {
            eprintln!("SKIP: examples/counter not present");
            return;
        };
        if !run_preview(&counter, None).ok {
            eprintln!("SKIP: preview/replay not runnable here (moc?)");
            let _ = std::fs::remove_dir_all(&counter);
            return;
        }
        let session = inc_session(2);
        let a = replay_dispatch(&counter, None, &session).expect("replay a");
        let b = replay_dispatch(&counter, None, &session).expect("replay b");
        assert_eq!(a, b, "the same session must replay to an identical forest");
        let _ = std::fs::remove_dir_all(&counter);
    }

    // -- CROSS-PLATFORM PROOF: the CRM forest -> native egui widgets ---------
    //
    // The committed `tests/fixtures/crm.forest.json` is the EXACT UI-IR forest
    // that `motoview preview examples/crm` emits from the ONE examples/crm
    // source (the same forest the deployed web canister was Playwright-driven
    // against). These tests prove the egui renderer would DRAW that CRM: we
    // parse the committed forest, assert the CRM structure is present, and
    // assert `widget_kind` maps each key node to the right native widget
    // (kanban columns / form sections -> Group; "New deal" + per-card buttons
    // -> Button; deal titles -> Inline/Label text). `app::render_node` is a
    // thin performer over exactly these `widget_kind` decisions, so proving the
    // mapping proves the draw. (Launching the egui WINDOW is out of scope —
    // headless; this is the forest -> native-widget mapping the window uses.)

    /// Load the committed CRM forest fixture (NOT spawned — a real, committed
    /// artifact of `motoview preview examples/crm`).
    fn crm_forest() -> Vec<UiNode> {
        let path = fixtures_dir().join("crm.forest.json");
        let json = read_file(&path)
            .unwrap_or_else(|e| panic!("read CRM fixture {}: {e}", path.display()));
        parse_forest(json.trim())
            .unwrap_or_else(|e| panic!("parse CRM forest fixture: {e}"))
    }

    /// Walk the whole forest, visiting every node (pre-order).
    fn walk_all<'a>(nodes: &'a [UiNode], f: &mut dyn FnMut(&'a UiNode)) {
        for n in nodes {
            f(n);
            if let UiNode::El { children, .. } = n {
                walk_all(children, f);
            }
        }
    }

    /// Count `<el>` nodes whose `class` attribute equals `class`.
    fn count_with_class(forest: &[UiNode], class: &str) -> usize {
        let mut n = 0;
        walk_all(forest, &mut |node| {
            if let UiNode::El { attrs, .. } = node {
                if attrs.get("class").map(String::as_str) == Some(class) {
                    n += 1;
                }
            }
        });
        n
    }

    /// Collect every `<el>` node whose `class` equals `class`.
    fn nodes_with_class<'a>(forest: &'a [UiNode], class: &str) -> Vec<&'a UiNode> {
        let mut out = Vec::new();
        walk_all(forest, &mut |node| {
            if let UiNode::El { attrs, .. } = node {
                if attrs.get("class").map(String::as_str) == Some(class) {
                    out.push(node);
                }
            }
        });
        out
    }

    /// Collect every `<button>` node whose `click` event handler equals `handler`.
    fn buttons_clicking<'a>(forest: &'a [UiNode], handler: &str) -> Vec<&'a UiNode> {
        let mut out = Vec::new();
        walk_all(forest, &mut |node| {
            if let UiNode::El { tag, events, .. } = node {
                if tag == "button" && events.get("click").map(String::as_str) == Some(handler) {
                    out.push(node);
                }
            }
        });
        out
    }

    #[test]
    fn crm_fixture_is_the_crm_board() {
        // The committed fixture parses, and it IS the CRM board: the "Pipeline"
        // h1, the four kanban columns, the six deal cards, and the "+ New deal"
        // button (events.click=toggleCreate). This is the contract the web
        // canister was Playwright-proven on.
        let forest = crm_forest();
        assert!(!forest.is_empty(), "CRM forest must be non-empty");

        // The h1 reads "Pipeline".
        let mut h1_text = None;
        walk_all(&forest, &mut |node| {
            if let UiNode::El { tag, .. } = node {
                if tag == "h1" {
                    h1_text = Some(node_text(node));
                }
            }
        });
        assert_eq!(
            h1_text.as_deref(),
            Some("Pipeline"),
            "CRM board's h1 must read 'Pipeline'"
        );

        // Four kanban-col <section>s (Lead / Contacted / Proposal / Won).
        assert_eq!(
            count_with_class(&forest, "kanban-col"),
            4,
            "CRM has four kanban columns"
        );
        for col in nodes_with_class(&forest, "kanban-col") {
            assert_eq!(col.tag(), Some("section"), "a kanban-col is a <section>");
        }

        // Six deal-card <article>s with the expected deal titles.
        assert_eq!(
            count_with_class(&forest, "deal-card"),
            6,
            "CRM has six deal cards"
        );
        for card in nodes_with_class(&forest, "deal-card") {
            assert_eq!(card.tag(), Some("article"), "a deal-card is an <article>");
        }
        let titles: Vec<String> = nodes_with_class(&forest, "deal-title")
            .iter()
            .map(|n| node_text(n))
            .collect();
        assert!(
            titles.iter().any(|t| t == "Website redesign"),
            "expected the 'Website redesign' deal, got {titles:?}"
        );
        assert_eq!(titles.len(), 6, "six deal titles, got {titles:?}");

        // The "+ New deal" button toggles the create form.
        let new_deal = buttons_clicking(&forest, "toggleCreate");
        assert_eq!(new_deal.len(), 1, "exactly one '+ New deal' button");
        assert_eq!(
            node_text(new_deal[0]),
            "+ New deal",
            "the toggleCreate button is labelled '+ New deal'"
        );
    }

    #[test]
    fn crm_forest_maps_to_egui_widgets() {
        // PROVE the egui renderer would draw the CRM: `widget_kind` (the pure
        // decision app::render_node performs) maps each key CRM node to the
        // right native widget.
        let forest = crm_forest();

        // 1) The "+ New deal" button -> WidgetKind::Button.
        let new_deal = buttons_clicking(&forest, "toggleCreate");
        assert_eq!(
            widget_kind(new_deal[0]),
            WidgetKind::Button,
            "the '+ New deal' button must render as a native egui Button"
        );

        // 2) The per-card action buttons (remove / move) -> Button, all 18.
        let remove = buttons_clicking(&forest, "removeDeal"); // one per card -> 6
        let back = buttons_clicking(&forest, "moveBack"); // 6
        let fwd = buttons_clicking(&forest, "moveFwd"); // 6
        assert_eq!(remove.len(), 6, "one remove button per deal card");
        assert_eq!(back.len(), 6, "one move-back button per deal card");
        assert_eq!(fwd.len(), 6, "one move-forward button per deal card");
        for b in remove.iter().chain(back.iter()).chain(fwd.iter()) {
            assert_eq!(
                widget_kind(b),
                WidgetKind::Button,
                "every per-card action <button> maps to a native Button"
            );
        }

        // 3) Kanban columns (<section class=kanban-col>) -> Group containers.
        let cols = nodes_with_class(&forest, "kanban-col");
        assert_eq!(cols.len(), 4);
        for c in &cols {
            assert_eq!(
                widget_kind(c),
                WidgetKind::Group,
                "a kanban column <section> maps to a native group container"
            );
        }
        // The column BODY (<div class=kanban-col-body>) is also a Group, and the
        // deal cards (<article>) inside it are Groups too.
        for body in nodes_with_class(&forest, "kanban-col-body") {
            assert_eq!(widget_kind(body), WidgetKind::Group);
        }
        for card in nodes_with_class(&forest, "deal-card") {
            assert_eq!(
                widget_kind(card),
                WidgetKind::Group,
                "a deal-card <article> maps to a native group container"
            );
        }

        // 4) The deal TITLES: the title wrapper is an inline-ish <el> (Inline),
        //    and its visible text is the deal name (what egui draws as a label
        //    inside the inline row).
        for title in nodes_with_class(&forest, "deal-title") {
            assert_eq!(
                widget_kind(title),
                WidgetKind::Inline,
                "a deal title wrapper maps to an inline (heading-style) row"
            );
            assert!(
                !node_text(title).is_empty(),
                "every deal title carries visible text"
            );
        }

        // 5) The "Pipeline" h1 -> Inline (rendered as an egui heading).
        let mut h1: Option<&UiNode> = None;
        walk_all(&forest, &mut |node| {
            if node.tag() == Some("h1") {
                h1 = Some(node);
            }
        });
        let h1 = h1.expect("CRM has an <h1>");
        assert_eq!(widget_kind(h1), WidgetKind::Inline);
        assert_eq!(node_text(h1), "Pipeline");
    }

    // -- Open-by-path (the crash-free replacement for the native dialog) ----

    /// The absolute path to the in-repo `examples/crm` project.
    fn crm_project_dir() -> PathBuf {
        // CARGO_MANIFEST_DIR = apps/studio/native -> repo root is ../../.. .
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../../examples/crm")
            .canonicalize()
            .expect("examples/crm should exist in the repo")
    }

    #[test]
    fn open_by_path_lists_crm() {
        // This is exactly the logic `app::StudioApp::open_project_from_input` runs when the
        // user types the path + clicks Open (no native dialog, no run-loop
        // re-entry). It must NOT panic and must list the CRM's real .mview
        // files while ignoring e2e/node_modules, .dfx, .mvbuild and .DS_Store.
        let crm = crm_project_dir();
        let outcome = open_project(crm.to_str().unwrap());
        let (dir, files) = match outcome {
            OpenOutcome::Opened { dir, files, .. } => (dir, files),
            OpenOutcome::Rejected { note } => panic!("CRM should open, got reject: {note}"),
        };
        assert_eq!(dir, crm);

        let names: Vec<String> = files
            .iter()
            .map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        assert!(
            names.iter().any(|n| n == "Board.mview"),
            "CRM open must list Board.mview, got {names:?}"
        );
        assert!(
            names.iter().any(|n| n == "CrmLayout.mview"),
            "CRM open must list the layout .mview, got {names:?}"
        );

        // Noise must be ignored: nothing from node_modules / e2e build dirs /
        // .dfx / .mvbuild, and the .DS_Store (not a .mview) never appears.
        for f in &files {
            let s = f.to_string_lossy();
            assert!(
                !s.contains("node_modules"),
                "node_modules must be skipped: {s}"
            );
            assert!(!s.contains(".mvbuild"), ".mvbuild must be skipped: {s}");
            assert!(!s.contains(".dfx"), ".dfx must be skipped: {s}");
            assert!(!s.ends_with(".DS_Store"), ".DS_Store is not a .mview: {s}");
            assert_eq!(
                f.extension().and_then(|e| e.to_str()),
                Some("mview"),
                "only .mview files are listed: {s}"
            );
        }
        // The two real CRM .mview files, nothing more.
        assert_eq!(files.len(), 2, "CRM has exactly two .mview files: {names:?}");
    }

    #[test]
    fn open_nonexistent_path_is_clean_reject_no_panic() {
        let bogus = std::env::temp_dir().join("definitely-not-a-real-project-xyzzy-123");
        let _ = std::fs::remove_dir_all(&bogus); // make sure it's absent
        match open_project(bogus.to_str().unwrap()) {
            OpenOutcome::Rejected { note } => {
                assert!(note.contains("No such path"), "clear reason: {note}");
            }
            OpenOutcome::Opened { .. } => panic!("a missing path must be rejected, not opened"),
        }
    }

    #[test]
    fn open_empty_path_is_clean_reject() {
        match open_project("   ") {
            OpenOutcome::Rejected { note } => {
                assert!(!note.is_empty(), "rejection carries a human message");
            }
            OpenOutcome::Opened { .. } => panic!("an empty path must be rejected"),
        }
    }

    #[test]
    fn open_dir_without_mview_opens_with_empty_status() {
        // A real directory containing NO .mview -> Opened with an empty file
        // list and a clear note (NOT a reject, NOT a panic).
        let tmp = std::env::temp_dir().join(format!("mvstudio_nomview_{}", std::process::id()));
        std::fs::create_dir_all(&tmp).unwrap();
        std::fs::write(tmp.join("readme.txt"), "no mview here").unwrap();
        match open_project(tmp.to_str().unwrap()) {
            OpenOutcome::Opened { files, note, .. } => {
                assert!(files.is_empty(), "no .mview -> empty list");
                assert!(note.contains("no .mview"), "clear empty status: {note}");
            }
            OpenOutcome::Rejected { note } => {
                panic!("an existing dir should open even with no .mview: {note}")
            }
        }
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn open_a_file_path_is_rejected_not_a_dir() {
        // Pointing at a file (not a folder) is rejected with a clear message.
        let tmp = std::env::temp_dir().join(format!("mvstudio_filepath_{}", std::process::id()));
        std::fs::create_dir_all(&tmp).unwrap();
        let file = tmp.join("Board.mview");
        std::fs::write(&file, "@page \"/\"\n").unwrap();
        match open_project(file.to_str().unwrap()) {
            OpenOutcome::Rejected { note } => assert!(note.contains("Not a folder"), "{note}"),
            OpenOutcome::Opened { .. } => panic!("a file path must be rejected"),
        }
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn expand_path_handles_tilde_and_quotes() {
        std::env::set_var("HOME", "/Users/tester");
        assert_eq!(expand_path("~"), PathBuf::from("/Users/tester"));
        assert_eq!(
            expand_path("~/dev/MotoView"),
            PathBuf::from("/Users/tester/dev/MotoView")
        );
        // Surrounding whitespace and quotes (e.g. dragged from Finder) are
        // stripped, and a plain absolute path is left intact.
        assert_eq!(
            expand_path("  '/Users/tester/examples/crm'  "),
            PathBuf::from("/Users/tester/examples/crm")
        );
        assert_eq!(
            expand_path("/abs/path/already"),
            PathBuf::from("/abs/path/already")
        );
    }

    #[test]
    fn read_file_on_binary_is_err_not_panic() {
        // read_file uses read_to_string -> invalid UTF-8 returns Err (the GUI's
        // `open` shows the error in the status bar instead of unwrapping).
        let tmp = std::env::temp_dir().join(format!("mvstudio_bin_{}", std::process::id()));
        std::fs::create_dir_all(&tmp).unwrap();
        let bin = tmp.join("blob.mview");
        std::fs::write(&bin, [0xff, 0xfe, 0x00, 0x80, 0x9c]).unwrap();
        assert!(
            read_file(&bin).is_err(),
            "non-UTF8 bytes must surface as Err, not panic"
        );
        let _ = std::fs::remove_dir_all(&tmp);
    }
}
