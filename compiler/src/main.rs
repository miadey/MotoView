//! `motoview` — the MotoView compiler CLI.
//!
//!   motoview new <name>           scaffold a new project
//!   motoview build [dir]          compile .mview files -> .mvbuild/main.mo
//!   motoview compile <file.mview> compile one file and print the Motoko (debug)
//!   motoview dev [dir]            build, then `dfx deploy` (local)
//!   motoview version

mod ast;
mod codegen;
mod color;
mod color_native;
mod fmt;
mod lint;
mod lsp;
mod parser;
mod project;
mod services;
mod span;
#[cfg(test)]
mod tests;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;

const VERSION: &str = env!("CARGO_PKG_VERSION");

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let code = run(&args);
    std::process::exit(code);
}

fn run(args: &[String]) -> i32 {
    let cmd = args.first().map(|s| s.as_str()).unwrap_or("help");
    match cmd {
        "build" => cmd_build(&args[1..]),
        "check" => cmd_check(&args[1..]),
        "lint" => cmd_lint(&args[1..]),
        "fmt" => cmd_fmt(&args[1..]),
        "lsp" => lsp::run_stdio(),
        "compile" => cmd_compile(&args[1..]),
        "preview" => cmd_preview(&args[1..]),
        "new" => cmd_new(&args[1..]),
        "dev" => cmd_dev(&args[1..]),
        "shell" => cmd_shell(&args[1..]),
        "version" | "--version" | "-v" => {
            println!("motoview {}", VERSION);
            0
        }
        "help" | "--help" | "-h" => {
            print_help();
            0
        }
        other => {
            eprintln!("unknown command: {}\n", other);
            print_help();
            1
        }
    }
}

fn print_help() {
    println!(
        "MotoView {VERSION} — a Motoko-native, server-driven UI framework for ICP.\n\n\
         USAGE:\n\
         \x20 motoview new <name>            Scaffold a new MotoView project\n\
         \x20                                  --template <t>  basic|secure-form|identity|wallet (default basic)\n\
         \x20                                  --dir <parent>  create at <parent>/<name>\n\
         \x20 motoview build [dir]           Compile .mview files into Motoko (.mvbuild/main.mo)\n\
         \x20                                  --network <local|ic>  vetKD key gate (default local)\n\
         \x20                                  --instrument          opt-in observability (structured Debug.print + cost per event)\n\
         \x20 motoview check [dir]           Build, then type-check; errors point at your .mview\n\
         \x20                                  --network <local|ic>  vetKD key gate (default local)\n\
         \x20                                  --instrument          type-check the instrumented actor\n\
         \x20                                  --json                machine-readable diagnostics\n\
         \x20 motoview lint [dir]            Run the security lint pass; print diagnostics\n\
         \x20                                  --json                machine-readable diagnostics\n\
         \x20 motoview fmt [dir|file]        Format .mview files (conservative, semantics-preserving)\n\
         \x20                                  --check               exit nonzero if any file is unformatted (CI)\n\
         \x20 motoview lsp                   Run the .mview language server (LSP over stdio)\n\
         \x20 motoview compile <file.mview>  Compile a single file and print the Motoko\n\
         \x20 motoview preview [dir]         Render a page's IR forest with NO deploy (moc -r)\n\
         \x20                                  --route <path>        which page (default: first)\n\
         \x20                                  --watch               re-emit on .mview change\n\
         \x20                                  --serve [--port N]    SSE + 3-up (web/iOS/Android) panel\n\
         \x20                                  --replay <session>    deterministic record/replay of events\n\
         \x20 motoview dev [dir]             Build, then `dfx deploy` to the local replica\n\
         \x20 motoview shell --url <url>     Scaffold desktop (Tauri) + mobile (Capacitor) shells\n\
         \x20 motoview version               Print the version\n\n\
         Rendering is a query. Events are updates. Write Motoko, ship to ICP.\n"
    );
}

/// Whether a boolean flag (`--json`) is present anywhere in `args`. Unlike `opt`,
/// it consumes no value token, so `positional` is unaffected by it.
fn flag(args: &[String], name: &str) -> bool {
    args.iter().any(|a| a == name)
}

fn opt<'a>(args: &'a [String], name: &str) -> Option<&'a str> {
    let mut it = args.iter();
    while let Some(a) = it.next() {
        if a == name {
            return it.next().map(|s| s.as_str());
        }
        if let Some(v) = a.strip_prefix(&format!("{}=", name)) {
            return Some(v);
        }
    }
    None
}

/// Flags that take a following value (`--flag value`). `positional` must skip
/// the value token after any of these, or e.g. `build --network ic` would pick
/// `ic` as the project directory (the documented space form was broken before).
const VALUE_FLAGS: &[&str] = &[
    "--name", "--out", "--network", "--url", "--id", "--emit", "--route", "--port",
    "--template", "--dir",
];

fn positional(args: &[String]) -> Option<&str> {
    let mut i = 0;
    while i < args.len() {
        let a = &args[i];
        if a.starts_with('-') {
            // `--flag=value` carries its own value; only `--flag value` (no `=`)
            // consumes the following token, so skip it for known value flags.
            if !a.contains('=') && VALUE_FLAGS.contains(&a.as_str()) {
                i += 1; // skip the value token belonging to this flag
            }
            i += 1;
            continue;
        }
        return Some(a.as_str());
    }
    None
}

fn app_name_for(dir: &PathBuf, override_name: Option<&str>) -> String {
    if let Some(n) = override_name {
        return n.to_string();
    }
    // try dfx.json first canister name
    if let Ok(txt) = std::fs::read_to_string(dir.join("dfx.json")) {
        if let Some(name) = first_json_key_under(&txt, "canisters") {
            return name;
        }
    }
    dir.file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("MotoViewApp")
        .to_string()
}

/// Extremely small helper: find the first object key under `"canisters": {`.
fn first_json_key_under(txt: &str, parent: &str) -> Option<String> {
    let needle = format!("\"{}\"", parent);
    let p = txt.find(&needle)?;
    let after = &txt[p + needle.len()..];
    let brace = after.find('{')?;
    let rest = &after[brace + 1..];
    let q1 = rest.find('"')?;
    let rest2 = &rest[q1 + 1..];
    let q2 = rest2.find('"')?;
    Some(rest2[..q2].to_string())
}

fn cmd_build(args: &[String]) -> i32 {
    let dir = PathBuf::from(positional(args).unwrap_or("."));
    let name = app_name_for(&dir, opt(args, "--name"));
    // The generated actor is a BUILD ARTIFACT (like Blazor's obj/) — written to
    // .mvbuild/ (gitignored), not committed. You edit .mview; dfx + `motoview
    // check` read it from here; errors map back to your .mview.
    let out = opt(args, "--out")
        .map(PathBuf::from)
        .unwrap_or_else(|| dir.join(".mvbuild").join("main.mo"));
    let network = opt(args, "--network").unwrap_or("local").to_string();
    // `--emit ir` flips page render to the portable UINode forest (the preview
    // backend). DEFAULT is `html` — byte-identical to the legacy path.
    let emit = match opt(args, "--emit").unwrap_or("html").trim().to_ascii_lowercase().as_str() {
        "ir" => codegen::EmitMode::Ir,
        _ => codegen::EmitMode::Html,
    };
    // `--instrument` (R7 observability): wrap each event handler in the generated
    // dispatch with a structured Debug.print + instruction-cost line. Off by
    // default — when off the generated main.mo is byte-identical to the legacy
    // path. The studio log parser consumes the `MV|dispatch|...` lines.
    let instrument = flag(args, "--instrument");
    let opts = project::BuildOptions {
        project_dir: dir,
        app_name: name,
        out,
        network,
        emit,
        instrument,
    };
    match project::build(&opts) {
        Ok(summary) => {
            print!("{}", summary);
            0
        }
        Err(e) => {
            eprintln!("motoview build error: {}", e);
            1
        }
    }
}

fn cmd_compile(args: &[String]) -> i32 {
    let file = match positional(args) {
        Some(f) => f,
        None => {
            eprintln!("usage: motoview compile <file.mview>");
            return 1;
        }
    };
    let source = match std::fs::read_to_string(file) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("cannot read {}: {}", file, e);
            return 1;
        }
    };
    let name = PathBuf::from(file)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("Page")
        .to_string();
    // Decide kind from the directory name.
    let kind = if file.contains("Layouts") {
        ast::FileKind::Layout
    } else if file.contains("Components") {
        ast::FileKind::Component
    } else {
        ast::FileKind::Page
    };
    match parser::parse(&source, &name, kind.clone()) {
        Ok(f) => {
            let models: HashMap<String, HashMap<String, String>> = HashMap::new();
            let components: HashMap<String, codegen::CompInfo> = HashMap::new();
            let mut cg = codegen::Codegen::new(&models, &components);
            match kind {
                ast::FileKind::Component => println!("{}", cg.gen_app_component(&f)),
                ast::FileKind::Layout => println!("{}", cg.gen_layout(&f)),
                _ => {
                    let pg = cg.gen_page(&f);
                    println!("{}", pg.object_block);
                    println!("{}", pg.page_record);
                }
            }
            0
        }
        Err(e) => {
            eprintln!("parse error in {}: {}", file, e);
            1
        }
    }
}

/// `motoview preview [dir] [--route <path>] [--watch] [--serve [--port N]]` —
/// the NO-DEPLOY inner loop. Compiles the target page in IR mode into a tiny
/// preview DRIVER (`.mvbuild/preview.mo`), runs it through the Motoko interpreter
/// (`moc -r`) with a MOCK request context, and prints the page's INITIAL-render
/// IR forest (UINode JSON) to stdout. NO `dfx deploy`, NO replica.
///
///   --route <path>    pick the page by route (default: the first page)
///   --watch           re-emit on any .mview change (Ctrl-C to stop)
///   --serve [--port]  serve an SSE stream + a browser preview harness that
///                     renders the forest to the DOM (a tiny JS mirror of the
///                     NativeView element/text/raw mapping)
fn cmd_preview(args: &[String]) -> i32 {
    let dir = PathBuf::from(positional(args).unwrap_or("."));
    let route = opt(args, "--route");
    let serve = flag(args, "--serve");
    let watch = flag(args, "--watch");

    // Resolve moc + the project's package args once (shared by every render).
    let (moc, base) = match find_moc() {
        Some(x) => x,
        None => {
            eprintln!(
                "motoview preview: `moc` not found under ~/.cache/dfinity/versions.\n\
                 Install the DFINITY SDK (dfx) — preview runs the page through moc -r (no replica)."
            );
            return 1;
        }
    };
    let pkg_args = dfx_package_args(&dir);

    // --replay <session.json>: deterministic record/replay (R10). Load the recorded
    // event session, re-run it through the page's dispatch, and print the resulting
    // forest. The same session always yields the same forest (determinism).
    if let Some(session_path) = opt(args, "--replay") {
        return cmd_preview_replay(&dir, route, &moc, &base, &pkg_args, session_path);
    }

    if serve {
        return preview_serve(&dir, route, &moc, &base, &pkg_args, args);
    }
    if watch {
        return preview_watch(&dir, route, &moc, &base, &pkg_args, None);
    }

    match render_preview(&dir, route, &moc, &base, &pkg_args) {
        Ok((forest, info)) => {
            eprintln!(
                "preview: {} ({}) — IR forest via moc -r, no deploy",
                info.page_name, info.route
            );
            println!("{}", forest);
            0
        }
        Err(e) => {
            eprintln!("motoview preview error: {}", e);
            1
        }
    }
}

/// Compile the target page in IR mode and run the driver through `moc -r`,
/// returning the emitted IR forest JSON (trimmed) plus which page it was.
fn render_preview(
    dir: &PathBuf,
    route: Option<&str>,
    moc: &PathBuf,
    base: &PathBuf,
    pkg_args: &[String],
) -> Result<(String, project::PreviewBuild), String> {
    render_preview_with_events(dir, route, moc, base, pkg_args, &[])
}

/// Like [`render_preview`], but applies a recorded event session through the page's
/// dispatch BEFORE rendering (R10 deterministic replay). With no events this is the
/// initial render; with events the forest reflects the page state after replaying
/// them, in order. Running the SAME events twice yields a byte-identical forest.
fn render_preview_with_events(
    dir: &PathBuf,
    route: Option<&str>,
    moc: &PathBuf,
    base: &PathBuf,
    pkg_args: &[String],
    events: &[project::ReplayEvent],
) -> Result<(String, project::PreviewBuild), String> {
    let info = project::build_preview_with_events(dir, route, events)?;
    let mut cmd = Command::new(moc);
    cmd.arg("-r").arg("--package").arg("base").arg(base);
    for a in pkg_args {
        cmd.arg(a);
    }
    cmd.arg(&info.driver_path);
    let output = cmd
        .output()
        .map_err(|e| format!("could not run moc -r: {}", e))?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    // moc -r prints Debug.print output on stdout; the forest is the last non-empty
    // line that looks like a JSON array (warnings go to stderr, not stdout).
    let forest = stdout
        .lines()
        .map(|l| l.trim())
        .filter(|l| l.starts_with('[') && l.ends_with(']'))
        .last()
        .map(|s| s.to_string());
    match forest {
        Some(f) if !f.is_empty() => Ok((f, info)),
        _ => {
            // No forest on stdout — surface moc's diagnostics (type errors etc.).
            let mut msg = String::from("preview produced no IR forest.");
            if !stderr.trim().is_empty() {
                msg.push_str("\nmoc -r said:\n");
                msg.push_str(stderr.trim());
            }
            if !stdout.trim().is_empty() {
                msg.push_str("\nstdout:\n");
                msg.push_str(stdout.trim());
            }
            Err(msg)
        }
    }
}

/// `motoview preview --replay <session.json>` — DETERMINISTIC record/replay (R10).
///
/// Loads a recorded session (an ordered list of `{handler, args, caller}` events),
/// re-runs that exact sequence through the page's `mvDispatch`, and prints the
/// resulting IR forest. Because the IC's dispatch+render is deterministic, replaying
/// the same session always produces a byte-identical forest — this is the property
/// that makes time-travel debugging near-free. The forest goes to stdout (so it can
/// be diffed/asserted); a one-line summary goes to stderr.
fn cmd_preview_replay(
    dir: &PathBuf,
    route: Option<&str>,
    moc: &PathBuf,
    base: &PathBuf,
    pkg_args: &[String],
    session_path: &str,
) -> i32 {
    let raw = match std::fs::read_to_string(session_path) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("motoview preview --replay: cannot read {}: {}", session_path, e);
            return 1;
        }
    };
    let events = match parse_session(&raw) {
        Ok(e) => e,
        Err(e) => {
            eprintln!("motoview preview --replay: bad session {}: {}", session_path, e);
            return 1;
        }
    };
    match render_preview_with_events(dir, route, moc, base, pkg_args, &events) {
        Ok((forest, info)) => {
            eprintln!(
                "replay: {} ({}) — {} event(s) re-dispatched, then rendered (deterministic, no deploy)",
                info.page_name,
                info.route,
                events.len()
            );
            println!("{}", forest);
            0
        }
        Err(e) => {
            eprintln!("motoview preview --replay error: {}", e);
            1
        }
    }
}

/// Parse a recorded replay session. The format is a JSON object with an `events`
/// array (or a bare array) of `{ "handler": "...", "args": ["..."], "caller": "..." }`.
/// `args` and `caller` are optional (default: no args / anonymous principal). We
/// reuse the LSP's dependency-free JSON parser so the compiler stays serde-free.
fn parse_session(raw: &str) -> Result<Vec<project::ReplayEvent>, String> {
    let v = lsp::parse_json(raw)?;
    // Accept either { "events": [...] } or a bare [...] of events.
    let arr = match v.get("events").and_then(|e| e.as_array()) {
        Some(a) => a,
        None => v
            .as_array()
            .ok_or("session must be an array of events or an object with an `events` array")?,
    };
    let mut out = Vec::with_capacity(arr.len());
    for (i, item) in arr.iter().enumerate() {
        let handler = item
            .get("handler")
            .and_then(|h| h.as_str())
            .ok_or_else(|| format!("event[{}] missing string `handler`", i))?
            .to_string();
        let args = item
            .get("args")
            .and_then(|a| a.as_array())
            .map(|a| {
                a.iter()
                    .map(|x| x.as_str().unwrap_or("").to_string())
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        let caller = item
            .get("caller")
            .and_then(|c| c.as_str())
            .unwrap_or(project::ReplayEvent::ANON)
            .to_string();
        out.push(project::ReplayEvent {
            handler,
            args,
            caller,
        });
    }
    Ok(out)
}

/// `--watch`: re-emit the forest whenever any `.mview` under `<dir>/src` changes
/// (polled by mtime). If `on_forest` is given, each new forest is handed to it
/// (used by `--serve` to push over SSE); otherwise the forest is printed.
fn preview_watch(
    dir: &PathBuf,
    route: Option<&str>,
    moc: &PathBuf,
    base: &PathBuf,
    pkg_args: &[String],
    on_forest: Option<&dyn Fn(&str)>,
) -> i32 {
    let src = dir.join("src");
    eprintln!("preview --watch: watching {} for .mview changes (Ctrl-C to stop)", src.display());
    let mut last = String::new();
    loop {
        match render_preview(dir, route, moc, base, pkg_args) {
            Ok((forest, info)) => {
                if forest != last {
                    last = forest.clone();
                    match on_forest {
                        Some(cb) => cb(&forest),
                        None => {
                            eprintln!("preview: {} ({}) re-rendered", info.page_name, info.route);
                            println!("{}", forest);
                        }
                    }
                }
            }
            Err(e) => eprintln!("preview error: {}", e),
        }
        std::thread::sleep(std::time::Duration::from_millis(700));
    }
}

/// `--serve`: a tiny HTTP server (std::net only, no deps) exposing:
///   GET /            -> the browser preview harness (HTML+JS) that renders the
///                       forest to the DOM (element->tag, text->text, raw->innerHTML)
///   GET /forest      -> the current IR forest JSON (one-shot fetch)
///   GET /events      -> an SSE stream that pushes a new forest on each .mview change
fn preview_serve(
    dir: &PathBuf,
    route: Option<&str>,
    moc: &PathBuf,
    base: &PathBuf,
    pkg_args: &[String],
    args: &[String],
) -> i32 {
    use std::io::{BufRead, BufReader, Write};
    use std::net::TcpListener;
    use std::sync::{Arc, Mutex};

    let port: u16 = opt(args, "--port")
        .and_then(|p| p.parse().ok())
        .unwrap_or(4956);
    // Shared current forest; the watch thread updates it, request handlers read it.
    let shared = Arc::new(Mutex::new(String::from("[]")));
    // A monotonically increasing version so SSE clients only get pushed on change.
    let version = Arc::new(Mutex::new(0u64));

    // Initial render so the very first page load shows something.
    match render_preview(dir, route, moc, base, pkg_args) {
        Ok((forest, info)) => {
            *shared.lock().unwrap() = forest;
            *version.lock().unwrap() = 1;
            eprintln!("preview --serve: {} ({})", info.page_name, info.route);
        }
        Err(e) => eprintln!("preview --serve: initial render failed: {}", e),
    }

    // Background watcher thread: re-render on change, bump version + store forest.
    {
        let dir = dir.clone();
        let route = route.map(|s| s.to_string());
        let moc = moc.clone();
        let base = base.clone();
        let pkg_args: Vec<String> = pkg_args.to_vec();
        let shared = Arc::clone(&shared);
        let version = Arc::clone(&version);
        std::thread::spawn(move || {
            let mut last = shared.lock().unwrap().clone();
            loop {
                if let Ok((forest, _)) =
                    render_preview(&dir, route.as_deref(), &moc, &base, &pkg_args)
                {
                    if forest != last {
                        last = forest.clone();
                        *shared.lock().unwrap() = forest;
                        *version.lock().unwrap() += 1;
                    }
                }
                std::thread::sleep(std::time::Duration::from_millis(700));
            }
        });
    }

    let listener = match TcpListener::bind(("127.0.0.1", port)) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("preview --serve: cannot bind 127.0.0.1:{} ({})", port, e);
            return 1;
        }
    };
    eprintln!(
        "preview --serve: open http://127.0.0.1:{}/  (3-up canvas: web + iOS + Android from one IR forest; live, no deploy)",
        port
    );

    for stream in listener.incoming() {
        let mut stream = match stream {
            Ok(s) => s,
            Err(_) => continue,
        };
        let shared = Arc::clone(&shared);
        let version = Arc::clone(&version);
        std::thread::spawn(move || {
            let mut reader = BufReader::new(match stream.try_clone() {
                Ok(s) => s,
                Err(_) => return,
            });
            let mut request_line = String::new();
            if reader.read_line(&mut request_line).is_err() {
                return;
            }
            // Drain the rest of the headers.
            let mut header = String::new();
            while reader.read_line(&mut header).map(|n| n > 0).unwrap_or(false) {
                if header == "\r\n" || header == "\n" {
                    break;
                }
                header.clear();
            }
            let path = request_line.split_whitespace().nth(1).unwrap_or("/");
            if path.starts_with("/events") {
                // SSE: push the forest whenever the version changes.
                let head = "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\n\
                            Cache-Control: no-cache\r\nConnection: keep-alive\r\n\
                            Access-Control-Allow-Origin: *\r\n\r\n";
                if stream.write_all(head.as_bytes()).is_err() {
                    return;
                }
                let mut seen = 0u64;
                loop {
                    let (v, f) = {
                        (*version.lock().unwrap(), shared.lock().unwrap().clone())
                    };
                    if v != seen {
                        seen = v;
                        let msg = format!("data: {}\n\n", f.replace('\n', "\ndata: "));
                        if stream.write_all(msg.as_bytes()).is_err() {
                            return;
                        }
                        let _ = stream.flush();
                    }
                    std::thread::sleep(std::time::Duration::from_millis(400));
                }
            } else if path.starts_with("/forest") {
                let body = shared.lock().unwrap().clone();
                let resp = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\n\
                     Access-Control-Allow-Origin: *\r\nContent-Length: {}\r\n\r\n{}",
                    body.len(),
                    body
                );
                let _ = stream.write_all(resp.as_bytes());
            } else {
                let body = PREVIEW_HARNESS_HTML;
                let resp = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\n\
                     Content-Length: {}\r\n\r\n{}",
                    body.len(),
                    body
                );
                let _ = stream.write_all(resp.as_bytes());
            }
        });
    }
    0
}

/// The browser preview harness: the 3-UP CANVAS (R10). It fetches the IR forest and
/// renders it across THREE columns from the ONE forest — web (live DOM), iOS
/// (SwiftUI NativeView, forest-faithful preview), Android (Compose NativeView,
/// forest-faithful preview) — then subscribes to `/events` (SSE) to live-update all
/// three on every `.mview` change.
///
/// The web column mirrors the NativeView IR DOM mapping (`el`->element,
/// `text`->text, `raw`->innerHTML). The iOS/Android columns consume the SAME native
/// descriptor produced by `fanout()` (inlined below between the MV-FANOUT markers,
/// a byte-faithful copy of tools/studio/fanout.js, parity-tested by fanout-test.js),
/// rendered as a forest-faithful preview styled per platform. True native panes
/// need Xcode/Android Studio simulator processes — flagged in the column header.
const PREVIEW_HARNESS_HTML: &str = r##"<!doctype html>
<html lang="en"><head>
<meta charset="utf-8"><meta name="viewport" content="width=device-width, initial-scale=1">
<title>MotoView 3-up preview</title>
<style>
  body{margin:0;font:14px/1.5 system-ui,sans-serif;background:#0b0b10;color:#e7e7ee}
  header{display:flex;align-items:center;gap:.6rem;padding:.6rem 1rem;background:#15151c;border-bottom:1px solid #262632}
  header b{font-weight:600}
  .dot{width:.55rem;height:.55rem;border-radius:50%;background:#3fb950}
  small{color:#9a9aab}
  .canvas{display:grid;grid-template-columns:repeat(3,1fr);gap:1rem;padding:1rem;max-width:1400px;margin:0 auto}
  .pane{background:#15151c;border:1px solid #262632;border-radius:.6rem;overflow:hidden;display:flex;flex-direction:column}
  .pane-head{display:flex;align-items:center;gap:.5rem;padding:.5rem .75rem;background:#1b1b24;border-bottom:1px solid #262632;font-weight:600}
  .pane-head .badge{font-size:.7rem;font-weight:500;color:#9a9aab}
  .pane-body{padding:1rem;min-height:240px}
  .mv-container{padding:.25rem}
  .mv-btn{padding:.45rem .8rem;border-radius:.4rem;border:1px solid #3a3a48;background:#1d1d27;color:#e7e7ee;cursor:pointer;margin:.15rem 0}
  .mv-btn-primary{background:#6d28d9;border-color:#6d28d9;color:#fff}
  /* native pane primitives (forest-faithful preview) */
  .nv-container{border-left:2px solid #2a2a38;padding-left:.6rem;margin:.25rem 0;display:flex;flex-direction:column;gap:.2rem}
  .nv-text{margin:.1rem 0}
  .nv-text.bold{font-weight:700}
  .nv-button{display:inline-block;padding:.4rem .7rem;border-radius:.5rem;border:1px solid #3a3a48;background:#22222e;margin:.15rem 0;font-weight:600}
  .nv-ios .nv-button{background:#0a84ff22;border-color:#0a84ff}
  .nv-android .nv-button{background:#3ddc8422;border-color:#3ddc84}
  .nv-raw{color:#d29922;font-style:italic;font-size:.8rem}
  .handler{color:#9a9aab;font-size:.72rem;margin-left:.35rem}
</style></head>
<body>
<header><span class="dot" id="dot"></span><b>MotoView 3-up canvas</b>
  <small>one IR forest → web + iOS + Android</small>
  <small id="status" style="margin-left:auto">connecting…</small></header>
<div class="canvas">
  <section class="pane"><div class="pane-head">Web <span class="badge">DOM (live)</span></div>
    <div class="pane-body" id="web"><small>loading IR forest…</small></div></section>
  <section class="pane nv-ios"><div class="pane-head">iOS <span class="badge">SwiftUI · forest-faithful preview</span></div>
    <div class="pane-body" id="ios"></div></section>
  <section class="pane nv-android"><div class="pane-head">Android <span class="badge">Compose · forest-faithful preview</span></div>
    <div class="pane-body" id="android"></div></section>
</div>
<script>
// ===== Web column: the live DOM mapping (el->element, text->text, raw->innerHTML).
function renderNode(n){
  if(n.t==="text"){return document.createTextNode(n.value||"");}
  if(n.t==="raw"){const w=document.createElement("span");w.innerHTML=n.html||"";return w;}
  if(n.t==="el"){
    const el=document.createElement(n.tag||"div");
    const a=n.attrs||{};for(const k in a){el.setAttribute(k,a[k]);}
    const ev=n.events||{};for(const k in ev){el.setAttribute("data-mv-"+k,ev[k]);}
    (n.children||[]).forEach(c=>el.appendChild(renderNode(c)));
    return el;
  }
  return document.createComment("unknown node");
}
// MV-FANOUT-BEGIN  (byte-faithful copy of tools/studio/fanout.js core; parity-tested)
const BLOCK_TAGS=new Set(["div","section","main","nav","header","footer","article","aside","ul","ol","li","form","fieldset","figure"]);
const TEXT_TAGS=new Set(["span","p","h1","h2","h3","h4","h5","h6","a","label","strong","em","b","i","small","code","pre","blockquote"]);
const BOLD_TAGS=new Set(["h1","h2","h3","h4","h5","h6","strong","b"]);
function stripTags(html){let out="";let inTag=false;for(const c of html||""){if(c==="<")inTag=true;else if(c===">")inTag=false;else if(!inTag)out+=c;}return out;}
function flattenText(nodes){let out="";for(const n of nodes||[]){if(n.t==="text")out+=n.value||"";else if(n.t==="raw")out+=stripTags(n.html||"");else if(n.t==="el")out+=flattenText(n.children||[]);}return out;}
function buttonEmit(node){const ev=node.events||{};const handler=ev.click||Object.values(ev)[0]||"";const args={};const attrs=node.attrs||{};for(const k in attrs)if(k.startsWith("data-mv-arg"))args[k]=attrs[k];return{event:"click",handler:handler,args:args};}
function nativeNode(node){
  if(node.t==="text")return{kind:"text-leaf",value:node.value||""};
  if(node.t==="raw"){if(!stripTags(node.html).trim())return null;return{kind:"raw",html:node.html||"",native:false};}
  if(node.t==="el"){
    const tag=(node.tag||"").toLowerCase();
    if(tag==="button")return{kind:"button",tag:tag,label:flattenText(node.children),emit:buttonEmit(node),native:true};
    if(TEXT_TAGS.has(tag))return{kind:"text",tag:tag,text:flattenText(node.children),bold:BOLD_TAGS.has(tag),native:true};
    const children=(node.children||[]).map(nativeNode).filter(Boolean);
    return{kind:"container",tag:tag,children:children,native:true};
  }
  return null;
}
function nativePane(forest){return(Array.isArray(forest)?forest:[forest]).map(nativeNode).filter(Boolean);}
function webNode(node){
  if(node.t==="text")return{kind:"text",value:node.value||""};
  if(node.t==="raw")return{kind:"raw",html:node.html||""};
  if(node.t==="el")return{kind:"el",tag:node.tag||"div",attrs:node.attrs||{},events:node.events||{},children:(node.children||[]).map(webNode)};
  return{kind:"unknown"};
}
function webPane(forest){return(Array.isArray(forest)?forest:[forest]).map(webNode);}
function fanout(forest){
  const web=webPane(forest);const native=nativePane(forest);
  return{source:"one IR forest",
    web:{platform:"web",renderer:"DOM (live)",nodes:web},
    ios:{platform:"ios",renderer:"SwiftUI NativeView (forest-faithful preview)",nodes:native},
    android:{platform:"android",renderer:"Compose NativeView (forest-faithful preview)",nodes:native}};
}
function paneButtons(paneNodes){const out=[];(function walk(ns){for(const n of ns){if(n.kind==="button")out.push({label:n.label,handler:n.emit.handler});if(n.children)walk(n.children);}})(paneNodes);return out;}
// MV-FANOUT-END
// Render a native descriptor node into the iOS/Android column (forest-faithful).
function renderNative(n){
  if(n.kind==="text-leaf"){return document.createTextNode(n.value||"");}
  if(n.kind==="raw"){const d=document.createElement("div");d.className="nv-raw";d.textContent="⟨raw HTML leaf — WebView fallback⟩";return d;}
  if(n.kind==="text"){const d=document.createElement("div");d.className="nv-text"+(n.bold?" bold":"");d.textContent=n.text;return d;}
  if(n.kind==="button"){
    const b=document.createElement("div");b.className="nv-button";b.textContent=n.label;
    const h=document.createElement("span");h.className="handler";h.textContent="→ "+(n.emit&&n.emit.handler||"");b.appendChild(h);
    return b;
  }
  if(n.kind==="container"){const d=document.createElement("div");d.className="nv-container";(n.children||[]).forEach(c=>d.appendChild(renderNative(c)));return d;}
  return document.createComment("?");
}
function renderAll(forest){
  let nodes;try{nodes=JSON.parse(forest);}catch(e){document.getElementById("web").textContent="bad forest JSON: "+e;return;}
  const fan=fanout(nodes);
  const web=document.getElementById("web");web.textContent="";
  fan.web.nodes&&(Array.isArray(nodes)?nodes:[nodes]).forEach(n=>web.appendChild(renderNode(n)));
  for(const plat of["ios","android"]){
    const col=document.getElementById(plat);col.textContent="";
    fan[plat].nodes.forEach(n=>col.appendChild(renderNative(n)));
  }
}
function setStatus(s,ok){document.getElementById("status").textContent=s;
  document.getElementById("dot").style.background=ok?"#3fb950":"#d29922";}
fetch("/forest").then(r=>r.text()).then(t=>{renderAll(t);setStatus("live (SSE)",true);}).catch(e=>setStatus("error: "+e,false));
try{
  const es=new EventSource("/events");
  es.onmessage=e=>{renderAll(e.data);setStatus("updated "+new Date().toLocaleTimeString(),true);};
  es.onerror=()=>setStatus("reconnecting…",false);
}catch(e){setStatus("no SSE: "+e,false);}
</script>
</body></html>
"##;

/// `motoview lint [dir] [--json]` — run the security lint pass over every .mview.
/// Default: print all diagnostics (`error:` / `warning:`). `--json`: print a JSON
/// array of `{severity, rule, message, file, line, col, endLine, endCol}` to
/// stdout instead. Exit code is identical in both modes (1 if any Error, else 0).
fn cmd_lint(args: &[String]) -> i32 {
    let dir = PathBuf::from(positional(args).unwrap_or("."));
    if flag(args, "--json") {
        return cmd_lint_json(&dir);
    }
    let diags = match project::lint_project(&dir) {
        Ok(d) => d,
        Err(e) => {
            eprintln!("motoview lint error: {}", e);
            return 1;
        }
    };
    if diags.is_empty() {
        println!("\u{2713} no lint findings");
        return 0;
    }
    let report = project::format_lint(&diags);
    print!("{}", report);
    let errors = diags
        .iter()
        .filter(|(_, d)| d.severity == lint::Severity::Error)
        .count();
    let warnings = diags.len() - errors;
    eprintln!("\n{} error(s), {} warning(s)", errors, warnings);
    if errors > 0 {
        1
    } else {
        0
    }
}

/// `motoview lint --json` — emit findings as a JSON array on stdout (machine
/// readable, with line/col positions). Exit code matches the human path: 1 if
/// any Error, else 0. On a project error (e.g. parse failure) the error goes to
/// stderr and we exit 1 — stdout stays valid-but-empty so a consumer parsing
/// stdout never chokes on a half-written array.
fn cmd_lint_json(dir: &PathBuf) -> i32 {
    let diags = match project::lint_project_json(dir) {
        Ok(d) => d,
        Err(e) => {
            eprintln!("motoview lint error: {}", e);
            return 1;
        }
    };
    println!("{}", lint::diagnostics_to_json(&diags));
    if diags.iter().any(|d| d.severity == lint::Severity::Error) {
        1
    } else {
        0
    }
}

/// `motoview fmt <dir|file> [--check]` — the CONSERVATIVE, semantics-preserving
/// `.mview` formatter. Walks a directory for `.mview` files (or formats a single
/// file), normalizing whitespace ONLY where it provably cannot change the
/// generated Motoko (the formatter self-verifies each file via `fmt::format_source`
/// — see fmt.rs). Template text and `@code` Motoko are left untouched.
///
///   (no flag)   rewrite each file in place that is not already formatted.
///   --check     rewrite NOTHING; exit 1 if any file would be reformatted (CI gate),
///               printing the offending paths. Exit 0 when every file is clean.
fn cmd_fmt(args: &[String]) -> i32 {
    let target = PathBuf::from(positional(args).unwrap_or("."));
    let check = flag(args, "--check");

    let files = collect_mview_files(&target);
    if files.is_empty() {
        eprintln!("motoview fmt: no .mview files found under {}", target.display());
        // Nothing to do is not an error (an empty project formats cleanly).
        return 0;
    }

    let mut unformatted: Vec<PathBuf> = Vec::new();
    let mut rewritten = 0usize;
    let mut errors = 0usize;
    for path in &files {
        let source = match std::fs::read_to_string(path) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("cannot read {}: {}", path.display(), e);
                errors += 1;
                continue;
            }
        };
        let kind = fmt::kind_from_path(&path.to_string_lossy());
        let formatted = fmt::format_source(&source, kind);
        if formatted == source {
            continue; // already formatted
        }
        if check {
            unformatted.push(path.clone());
        } else {
            match std::fs::write(path, &formatted) {
                Ok(()) => {
                    println!("formatted {}", path.display());
                    rewritten += 1;
                }
                Err(e) => {
                    eprintln!("cannot write {}: {}", path.display(), e);
                    errors += 1;
                }
            }
        }
    }

    if check {
        if unformatted.is_empty() {
            println!("\u{2713} {} file(s) already formatted", files.len());
            return if errors > 0 { 1 } else { 0 };
        }
        eprintln!("{} file(s) need formatting (run `motoview fmt`):", unformatted.len());
        for p in &unformatted {
            eprintln!("  {}", p.display());
        }
        return 1; // CI: unformatted files fail the gate
    }

    if rewritten == 0 {
        println!("\u{2713} {} file(s) already formatted", files.len());
    } else {
        println!("formatted {} of {} file(s)", rewritten, files.len());
    }
    if errors > 0 {
        1
    } else {
        0
    }
}

/// Collect `.mview` files for `fmt`. A single `.mview` file path yields just that
/// file; a directory is walked recursively (so `src/Pages`, `src/Layouts`,
/// `src/Components` are all covered in one pass). The order is sorted by path so
/// output is deterministic.
fn collect_mview_files(target: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    if target.is_file() {
        if target.extension().and_then(|e| e.to_str()) == Some("mview") {
            out.push(target.to_path_buf());
        }
        return out;
    }
    fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
        let entries = match std::fs::read_dir(dir) {
            Ok(e) => e,
            Err(_) => return,
        };
        let mut paths: Vec<PathBuf> = entries.flatten().map(|e| e.path()).collect();
        paths.sort();
        for p in paths {
            if p.is_dir() {
                // Skip build/output and VCS dirs — never any source .mview there.
                let name = p.file_name().and_then(|n| n.to_str()).unwrap_or("");
                if name == ".mvbuild" || name == ".git" || name == "node_modules" || name == "target" {
                    continue;
                }
                walk(&p, out);
            } else if p.extension().and_then(|e| e.to_str()) == Some("mview") {
                out.push(p);
            }
        }
    }
    walk(target, &mut out);
    out.sort();
    out
}

/// Build, then type-check the generated actor with `moc`, rewriting any errors
/// to name the originating .mview source instead of the generated main.mo.
///
/// With `--json`, the SAME type-check runs but the mapped diagnostics are emitted
/// as a JSON array (the lint `--json` schema) on stdout; all human-facing chatter
/// (build summary, "no type errors", the moc-missing note) goes to stderr so
/// stdout stays a single valid JSON document. Exit code is identical to the human
/// path in every branch.
fn cmd_check(args: &[String]) -> i32 {
    let dir = PathBuf::from(positional(args).unwrap_or("."));
    let json = flag(args, "--json");
    let name = app_name_for(&dir, opt(args, "--name"));
    let out = dir.join(".mvbuild").join("main.mo");
    let network = opt(args, "--network").unwrap_or("local").to_string();
    // `motoview check --instrument` type-checks the INSTRUMENTED actor (the same
    // code `--instrument` builds), so the structured Debug.print path is verified
    // against the runtime, not just the default path.
    let instrument = flag(args, "--instrument");
    let opts = project::BuildOptions {
        project_dir: dir.clone(),
        app_name: name,
        out: out.clone(),
        network,
        emit: codegen::EmitMode::Html,
        instrument,
    };
    match project::build(&opts) {
        // In JSON mode the build summary is informational, not part of the
        // diagnostic stream — send it to stderr so stdout is pure JSON.
        Ok(summary) => {
            if json {
                eprint!("{}", summary);
            } else {
                print!("{}", summary);
            }
        }
        Err(e) => {
            eprintln!("build error: {}", e);
            return 1;
        }
    }
    let (moc, base) = match find_moc() {
        Some(x) => x,
        None => {
            eprintln!("\nnote: `moc` not found under ~/.cache/dfinity/versions — run `dfx deploy`\nto type-check. Errors map to .mvbuild/main.mo; the `// mv:src <file>`\nmarkers above each region tell you which .mview it came from.");
            // Nothing was type-checked: stdout must still be valid JSON.
            if json {
                println!("[]");
            }
            return 0;
        }
    };
    let main_mo = std::fs::read_to_string(&out).unwrap_or_default();
    let mut cmd = Command::new(&moc);
    cmd.arg("--check").arg("--package").arg("base").arg(&base);
    for a in dfx_package_args(&dir) {
        cmd.arg(a);
    }
    cmd.arg(&out);
    let output = match cmd.output() {
        Ok(o) => o,
        Err(e) => {
            eprintln!("could not run moc: {}", e);
            return 1;
        }
    };
    let stderr = String::from_utf8_lossy(&output.stderr);
    // R11: load the generated->source line map so moc errors map to the .mview LINE.
    let source_map = project::load_source_map(&out);
    if json {
        let diags = project::map_moc_errors_json(&main_mo, &stderr, &source_map);
        println!("{}", lint::diagnostics_to_json(&diags));
        return if diags.iter().any(|d| d.severity == lint::Severity::Error) {
            1
        } else {
            0
        };
    }
    let (mapped, had_err) = project::map_moc_errors(&main_mo, &stderr, &source_map);
    if mapped.trim().is_empty() {
        println!("\n\u{2713} no type errors");
        0
    } else {
        println!();
        print!("{}", mapped);
        if had_err {
            1
        } else {
            0
        }
    }
}

/// Locate the `moc` + base library that the active dfx uses (so `check` matches
/// what `dfx deploy` will accept), falling back to the newest cached version.
fn find_moc() -> Option<(PathBuf, PathBuf)> {
    let home = std::env::var("HOME").ok()?;
    let versions = PathBuf::from(home).join(".cache/dfinity/versions");
    // Prefer the moc bundled with the active dfx version.
    if let Ok(out) = Command::new("dfx").arg("--version").output() {
        let v = String::from_utf8_lossy(&out.stdout);
        if let Some(ver) = v.split_whitespace().nth(1) {
            let dir = versions.join(ver.trim());
            if dir.join("moc").exists() {
                return Some((dir.join("moc"), dir.join("base")));
            }
        }
    }
    let mut dirs: Vec<PathBuf> = std::fs::read_dir(&versions)
        .ok()?
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.join("moc").exists())
        .collect();
    dirs.sort();
    let dir = dirs.pop()?;
    Some((dir.join("moc"), dir.join("base")))
}

/// Extract the moc `--package` args from dfx.json, resolving relative package
/// paths against the project dir so `moc` finds them.
fn dfx_package_args(dir: &PathBuf) -> Vec<String> {
    let txt = match std::fs::read_to_string(dir.join("dfx.json")) {
        Ok(t) => t,
        Err(_) => return vec![],
    };
    let args_str = match extract_json_string(&txt, "args") {
        Some(s) => s,
        None => return vec![],
    };
    args_str
        .split_whitespace()
        .map(|tok| {
            if tok.contains('/') && !tok.starts_with('-') {
                dir.join(tok).to_string_lossy().to_string()
            } else {
                tok.to_string()
            }
        })
        .collect()
}

fn extract_json_string(txt: &str, key: &str) -> Option<String> {
    let needle = format!("\"{}\"", key);
    let p = txt.find(&needle)?;
    let after = &txt[p + needle.len()..];
    let colon = after.find(':')?;
    let rest = &after[colon + 1..];
    let q1 = rest.find('"')?;
    let rest2 = &rest[q1 + 1..];
    let q2 = rest2.find('"')?;
    Some(rest2[..q2].to_string())
}

fn cmd_dev(args: &[String]) -> i32 {
    let rc = cmd_build(args);
    if rc != 0 {
        return rc;
    }
    let dir = PathBuf::from(positional(args).unwrap_or("."));
    println!("\n==> dfx deploy");
    match Command::new("dfx").arg("deploy").current_dir(&dir).status() {
        Ok(s) if s.success() => 0,
        Ok(s) => s.code().unwrap_or(1),
        Err(e) => {
            eprintln!("could not run dfx (is the DFINITY SDK installed?): {}", e);
            1
        }
    }
}

/// `motoview new <name> [--template <t>] [--dir <parent>]` — scaffold a complete,
/// build-and-lint-clean project from an EMBEDDED template (the files are baked
/// into the binary via `include_str!`, so `new` is self-contained — no template
/// dir needs to ship alongside the compiler).
///
///   <name>           project name; also the destination dir (created) unless --dir
///   --template <t>    one of: basic (default), secure-form, identity, wallet
///   --dir <parent>    create the project at <parent>/<name> instead of ./<name>
///   --name <app>      override the canister/app name (defaults to the dir name)
///
/// The chosen template's `dfx.json` carries `--package motoview <relpath>`; the
/// relpath is computed here so it is correct from the NEW project's location back
/// to the runtime (`runtime/src`). When the runtime can't be located on disk (an
/// installed binary, no checkout) we still write a correct `mops.toml` declaring
/// the `motoview` package and drop the `--package` arg, so an `mops install`
/// build resolves `mo:motoview` instead.
fn cmd_new(args: &[String]) -> i32 {
    let raw_name = match positional(args) {
        Some(n) => n,
        None => {
            eprintln!(
                "usage: motoview new <name> [--template <t>] [--dir <parent>]\n  \
                 templates: {}",
                templates::NAMES.join(", ")
            );
            return 1;
        }
    };
    let template = opt(args, "--template").unwrap_or("basic");
    if templates::files_for(template).is_none() {
        eprintln!(
            "unknown template `{}`. Available: {}",
            template,
            templates::NAMES.join(", ")
        );
        return 1;
    }

    // Destination: --dir <parent>/<name>, else the bare name as a path. The app
    // (canister) name is the final path component, or an explicit --name.
    let root = match opt(args, "--dir") {
        Some(parent) => PathBuf::from(parent).join(raw_name),
        None => PathBuf::from(raw_name),
    };
    let app_name = opt(args, "--name")
        .map(|s| s.to_string())
        .unwrap_or_else(|| {
            root.file_name()
                .and_then(|s| s.to_str())
                .unwrap_or("motoview_app")
                .to_string()
        });

    if root.exists() && std::fs::read_dir(&root).map(|mut d| d.next().is_some()).unwrap_or(false) {
        eprintln!("refusing to scaffold into a non-empty directory: {}", root.display());
        return 1;
    }

    let res = templates::scaffold_project(template, &root, &app_name);

    match res {
        Ok(()) => {
            let disp = root.display();
            println!(
                "created MotoView project '{}' from template '{}'.\n",
                app_name, template
            );
            println!("Next:");
            println!("  cd {}", disp);
            println!("  motoview build      # compile .mview -> .mvbuild/main.mo");
            println!("  motoview lint       # security lint (secure forms, @authorize)");
            println!("  motoview dev        # build, then `dfx deploy` to a local replica");
            if template != "basic" {
                println!(
                    "\nThis template demonstrates the `{}` pattern — see the comments in src/Pages/Home.mview.",
                    template
                );
            }
            0
        }
        Err(e) => {
            eprintln!("could not scaffold project: {}", e);
            1
        }
    }
}

/// `motoview shell [dir] --url <canister-url>` — scaffold native desktop (Tauri)
/// and mobile (Capacitor) shells that load the app from its canister URL. This
/// emits CONFIGS; building the actual binaries needs the native toolchains
/// (Tauri CLI, Xcode, Android SDK) the developer supplies.
fn cmd_shell(args: &[String]) -> i32 {
    let dir = PathBuf::from(positional(args).unwrap_or("."));
    let url = match opt(args, "--url") {
        Some(u) => u.trim_end_matches('/').to_string(),
        None => {
            eprintln!("usage: motoview shell [dir] --url <canister-url> [--name <AppName>] [--id <bundle.id>]\n  e.g. motoview shell --url https://<canister-id>.icp0.io");
            return 1;
        }
    };
    let name = opt(args, "--name").map(|s| s.to_string()).unwrap_or_else(|| {
        let n = app_name_for(&dir, None);
        let mut c = n.chars();
        match c.next() {
            Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
            None => n,
        }
    });
    let id = opt(args, "--id")
        .map(|s| s.to_string())
        .unwrap_or_else(|| format!("app.motoview.{}", name.to_lowercase()));
    let root = dir.join("clients");
    let mk = |rel: &str, contents: &str| -> std::io::Result<()> {
        let p = root.join(rel);
        if let Some(parent) = p.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(p, contents)
    };
    let res = (|| -> std::io::Result<()> {
        mk("desktop-tauri/src-tauri/tauri.conf.json", &scaffold::tauri_conf(&name, &id, &url))?;
        mk("desktop-tauri/src-tauri/Cargo.toml", &scaffold::tauri_cargo(&name))?;
        mk("desktop-tauri/src-tauri/build.rs", "fn main() {\n    tauri_build::build()\n}\n")?;
        mk("desktop-tauri/src-tauri/src/main.rs", scaffold::TAURI_MAIN_RS)?;
        mk("desktop-tauri/src-tauri/capabilities/default.json", scaffold::TAURI_CAPABILITIES)?;
        mk("desktop-tauri/dist/index.html", &scaffold::shell_redirect_html(&url))?;
        mk("mobile-capacitor/capacitor.config.json", &scaffold::capacitor_conf(&name, &id, &url))?;
        mk("mobile-capacitor/package.json", &scaffold::capacitor_package(&id))?;
        mk("mobile-capacitor/www/index.html", &scaffold::shell_redirect_html(&url))?;
        mk("README.md", &scaffold::shell_readme(&name, &url))?;
        Ok(())
    })();
    match res {
        Ok(()) => {
            println!(
                "scaffolded native shells under {}/clients  (url: {})\n\n\
                 Desktop (Tauri):   cd clients/desktop-tauri  &&  cargo tauri build   # needs the Tauri CLI\n\
                 Mobile (Capacitor): cd clients/mobile-capacitor && npm i && npx cap add ios|android && npx cap open ios|android   # needs Xcode / Android SDK\n\n\
                 These wrap your live canister — the app itself stays on-chain; the shells are just native windows.\n",
                dir.display(),
                url
            );
            0
        }
        Err(e) => {
            eprintln!("could not scaffold shells: {}", e);
            1
        }
    }
}

/// Embedded standalone project templates. Each template is a COMPLETE project
/// (motoview.json + dfx.json + mops.toml + README + .gitignore + src/...), baked
/// into the binary with `include_str!` so `motoview new` ships them itself —
/// nothing extra needs to be installed alongside the compiler.
///
/// Files carry three placeholders the scaffolder substitutes:
///   __NAME__         the app / canister name
///   __RUNTIME_PKG__  the `--package motoview <path>` runtime path for the new
///                    project's location (see `runtime_pkg_arg`)
///   __PORT__         the local replica bind port (distinct per template so two
///                    templates scaffolded side by side don't clash)
mod templates {
    use std::path::Path;

    /// The template names available via `--template`, in menu order.
    pub const NAMES: &[&str] = &["basic", "secure-form", "identity", "wallet"];

    /// One embedded file: its path relative to the project root, and its contents.
    type File = (&'static str, &'static str);

    // Shared config files differ only by placeholder, so each template embeds its
    // OWN copy (kept byte-identical on disk); embedding per-template keeps the
    // include paths simple and lets a template diverge later without surprises.
    macro_rules! tpl {
        ($dir:literal) => {
            &[
                ("dfx.json", include_str!(concat!("../templates/", $dir, "/dfx.json"))),
                ("motoview.json", include_str!(concat!("../templates/", $dir, "/motoview.json"))),
                ("mops.toml", include_str!(concat!("../templates/", $dir, "/mops.toml"))),
                ("README.md", include_str!(concat!("../templates/", $dir, "/README.md"))),
                (".gitignore", include_str!(concat!("../templates/", $dir, "/.gitignore"))),
                (
                    "src/Layouts/MainLayout.mview",
                    include_str!(concat!("../templates/", $dir, "/src/Layouts/MainLayout.mview")),
                ),
                (
                    "src/Pages/Home.mview",
                    include_str!(concat!("../templates/", $dir, "/src/Pages/Home.mview")),
                ),
            ]
        };
    }

    const BASIC: &[File] = tpl!("basic");
    const SECURE_FORM: &[File] = tpl!("secure-form");
    const IDENTITY: &[File] = tpl!("identity");
    const WALLET: &[File] = tpl!("wallet");

    /// The assistant rule files `docs/ai-tools.md` promises `motoview new` writes
    /// into EVERY project — one per assistant, all from the SAME canonical source
    /// (the repo's own rule files), so a scaffold can never drift from the docs or
    /// from the framework's real shape. Embedded once here (single source of
    /// truth), written into each project by `scaffold_project`.
    const AI_RULES: &[File] = &[
        // Claude / Claude Code skill.
        (
            "skills/motoview/SKILL.md",
            include_str!("../../skills/motoview/SKILL.md"),
        ),
        // GitHub Copilot workspace instructions.
        (
            ".github/copilot-instructions.md",
            include_str!("../../.github/copilot-instructions.md"),
        ),
        // Cursor project rule.
        (
            ".cursor/rules/motoview.mdc",
            include_str!("../../.cursor/rules/motoview.mdc"),
        ),
    ];

    /// The embedded files for a template name, or None if the name is unknown.
    pub fn files_for(name: &str) -> Option<&'static [File]> {
        match name {
            "basic" => Some(BASIC),
            "secure-form" => Some(SECURE_FORM),
            "identity" => Some(IDENTITY),
            "wallet" => Some(WALLET),
            _ => None,
        }
    }

    /// Write the chosen `template`'s files into `root`, substituting the three
    /// placeholders (`__NAME__`, `__RUNTIME_PKG__`, `__PORT__`). The runtime path
    /// is resolved for `root`'s location so the emitted `dfx.json` builds with no
    /// extra install in a checkout. Caller validates the template name first; an
    /// unknown name here is a no-op `Ok(())` (never reached on the CLI path). This
    /// is the single scaffold codepath exercised by both `motoview new` and the
    /// per-template build/lint tests.
    pub fn scaffold_project(template: &str, root: &Path, app_name: &str) -> std::io::Result<()> {
        let files = match files_for(template) {
            Some(f) => f,
            None => return Ok(()),
        };
        // Prefer a real relative path to a checkout's runtime/src (so `motoview
        // build` finds it with no mops install); fall back to `mo:motoview`.
        let runtime_pkg = runtime_pkg_arg(root);
        let port = default_port(template);
        // The template's own files, then the shared AI rule files (docs/ai-tools.md
        // promises every scaffold writes all three). AI rules carry no placeholders,
        // so the substitutions below are harmless no-ops for them.
        for (rel, contents) in files.iter().chain(AI_RULES.iter()) {
            let p = root.join(rel);
            if let Some(parent) = p.parent() {
                std::fs::create_dir_all(parent)?;
            }
            let body = contents
                .replace("__NAME__", app_name)
                .replace("__RUNTIME_PKG__", &runtime_pkg)
                .replace("__PORT__", &port.to_string());
            std::fs::write(p, body)?;
        }
        Ok(())
    }

    /// A distinct default local-replica bind port per template, so two templates
    /// scaffolded side by side never fight over the same port (and none collide
    /// with dfx's default 4943 or the repo examples' 4955/4962 range).
    fn default_port(template: &str) -> u16 {
        match template {
            "secure-form" => 4971,
            "identity" => 4972,
            "wallet" => 4973,
            _ => 4970, // basic
        }
    }

    /// Resolve the `--package motoview <path>` runtime path for a project being
    /// scaffolded at `root`. Prefers a REAL relative path from the project to a
    /// local checkout's `runtime/src` (so `motoview build` resolves `mo:motoview`
    /// with zero install). Falls back to the published mops package path
    /// `mo:motoview` — written here as the literal `mo:motoview`, which dfx
    /// accepts when the package is provided by mops (see the template's
    /// `mops.toml`). `dfx_package_args` only rewrites tokens containing `/`, so a
    /// bare `mo:motoview` is passed through untouched.
    fn runtime_pkg_arg(root: &Path) -> String {
        match locate_runtime_src() {
            Some(runtime) => rel_path(root, &runtime),
            // No checkout found: rely on mops (`mops install` fetches `motoview`).
            None => "mo:motoview".to_string(),
        }
    }

    /// Find the runtime's `src/` directory on disk. Tries, in order:
    ///   1. `$MOTOVIEW_RUNTIME` (an explicit override),
    ///   2. the runtime that sat next to this binary's source tree at BUILD time
    ///      (`CARGO_MANIFEST_DIR/../runtime/src` — true for an in-repo build),
    /// returning the first that exists.
    fn locate_runtime_src() -> Option<std::path::PathBuf> {
        if let Ok(p) = std::env::var("MOTOVIEW_RUNTIME") {
            let pb = std::path::PathBuf::from(p);
            if pb.join("App.mo").exists() || pb.exists() {
                return Some(pb);
            }
        }
        // CARGO_MANIFEST_DIR is `<repo>/compiler`; the runtime is its sibling.
        let built = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .map(|r| r.join("runtime").join("src"));
        match built {
            Some(p) if p.join("App.mo").exists() => Some(p),
            _ => None,
        }
    }

    /// Relative path FROM `from` (the project root) TO `to` (runtime/src), as a
    /// forward-slash string. Both are made absolute first (against the CWD) so a
    /// project created with a relative `root` still resolves correctly, and any
    /// existing ancestor is CANONICALIZED so symlinks on the path (e.g. macOS's
    /// `/var` -> `/private/var`, where temp dirs live) don't add phantom `..`
    /// levels that break the `..`-traversal at build time. Falls back to the
    /// absolute `to` if a relative path can't be formed (different roots, e.g. on
    /// Windows across drives).
    fn rel_path(from: &Path, to: &Path) -> String {
        let cwd = std::env::current_dir().unwrap_or_default();
        let abs = |p: &Path| -> std::path::PathBuf {
            if p.is_absolute() { p.to_path_buf() } else { cwd.join(p) }
        };
        let from = canonicalize_existing(&abs(from));
        let to = canonicalize_existing(&abs(to));
        let f: Vec<_> = from.components().collect();
        let t: Vec<_> = to.components().collect();
        // Require a shared prefix (same root/drive) to form a relative path.
        if f.first() != t.first() {
            return to.to_string_lossy().replace('\\', "/");
        }
        let common = f.iter().zip(t.iter()).take_while(|(a, b)| a == b).count();
        let mut parts: Vec<String> =
            std::iter::repeat("..".to_string()).take(f.len() - common).collect();
        for c in &t[common..] {
            parts.push(c.as_os_str().to_string_lossy().to_string());
        }
        if parts.is_empty() {
            ".".to_string()
        } else {
            parts.join("/")
        }
    }

    /// Canonicalize the longest EXISTING ancestor of `p` (resolving symlinks like
    /// macOS's `/var` -> `/private/var`), then re-append the not-yet-created tail,
    /// lexically normalized. The project dir doesn't exist at scaffold time, so we
    /// can't canonicalize it whole; canonicalizing its real parent is enough for
    /// the relative path to traverse correctly at build time.
    fn canonicalize_existing(p: &Path) -> std::path::PathBuf {
        let norm = normalize(p);
        // Walk up to the first ancestor that exists, canonicalize it, then push
        // the remaining components back on.
        let mut tail: Vec<std::ffi::OsString> = Vec::new();
        let mut cur = norm.clone();
        loop {
            if let Ok(real) = std::fs::canonicalize(&cur) {
                let mut out = real;
                for part in tail.iter().rev() {
                    out.push(part);
                }
                return out;
            }
            match cur.file_name() {
                Some(name) => {
                    tail.push(name.to_os_string());
                    if !cur.pop() {
                        return norm;
                    }
                }
                None => return norm, // reached the root with nothing existing
            }
        }
    }

    /// Lexically normalize a path (resolve `.` and `..` without touching the FS),
    /// so the project root and runtime path share a comparable component prefix.
    fn normalize(p: &Path) -> std::path::PathBuf {
        use std::path::Component;
        let mut out: Vec<Component> = Vec::new();
        for c in p.components() {
            match c {
                Component::ParentDir => {
                    if matches!(out.last(), Some(Component::Normal(_))) {
                        out.pop();
                    } else {
                        out.push(c);
                    }
                }
                Component::CurDir => {}
                other => out.push(other),
            }
        }
        out.iter().collect()
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use std::path::PathBuf;

        #[test]
        fn every_named_template_has_complete_files() {
            for &n in NAMES {
                let files = files_for(n).unwrap_or_else(|| panic!("no files for {n}"));
                let paths: Vec<&str> = files.iter().map(|(p, _)| *p).collect();
                for need in [
                    "dfx.json",
                    "motoview.json",
                    "mops.toml",
                    "src/Layouts/MainLayout.mview",
                    "src/Pages/Home.mview",
                ] {
                    assert!(paths.contains(&need), "{n} missing {need}");
                }
                // Config placeholders must be present for substitution to bite.
                let dfx = files.iter().find(|(p, _)| *p == "dfx.json").unwrap().1;
                assert!(dfx.contains("__NAME__"), "{n} dfx.json missing __NAME__");
                assert!(dfx.contains("__RUNTIME_PKG__"), "{n} dfx.json missing __RUNTIME_PKG__");
                assert!(dfx.contains("__PORT__"), "{n} dfx.json missing __PORT__");
            }
        }

        #[test]
        fn unknown_template_is_none() {
            assert!(files_for("does-not-exist").is_none());
        }

        #[test]
        fn ports_are_distinct_per_template() {
            let mut seen = std::collections::HashSet::new();
            for &n in NAMES {
                assert!(seen.insert(default_port(n)), "duplicate port for {n}");
            }
        }

        #[test]
        fn rel_path_is_relative_and_round_trips() {
            // A project two levels under a root resolves runtime/src as ../../runtime/src.
            let root = PathBuf::from("/x/apps/myapp");
            let runtime = PathBuf::from("/x/runtime/src");
            assert_eq!(rel_path(&root, &runtime), "../../runtime/src");
            // Sibling dirs.
            assert_eq!(
                rel_path(&PathBuf::from("/x/a"), &PathBuf::from("/x/runtime/src")),
                "../runtime/src"
            );
        }

        #[test]
        fn normalize_resolves_dot_and_dotdot() {
            assert_eq!(normalize(&PathBuf::from("/a/b/../c/./d")), PathBuf::from("/a/c/d"));
        }
    }
}

mod scaffold {
    // ---- native shell scaffolds (motoview shell) --------------------------
    // (Project scaffolding for `motoview new` lives in the `templates` module
    //  above — complete, embedded, build-and-lint-clean templates.)
    pub fn tauri_conf(name: &str, id: &str, url: &str) -> String {
        format!(
            r#"{{
  "$schema": "https://schema.tauri.app/config/2",
  "productName": "{name}",
  "version": "0.1.0",
  "identifier": "{id}.desktop",
  "build": {{ "frontendDist": "../dist" }},
  "app": {{
    "windows": [
      {{ "title": "{name}", "label": "main", "width": 1200, "height": 820,
        "minWidth": 380, "minHeight": 600, "resizable": true, "url": "{url}/" }}
    ],
    "security": {{ "csp": null }}
  }},
  "bundle": {{ "active": true, "targets": "all" }}
}}
"#
        )
    }

    pub fn tauri_cargo(name: &str) -> String {
        let c = name.to_lowercase();
        format!(
            r#"[package]
name = "{c}-desktop"
version = "0.1.0"
edition = "2021"

[build-dependencies]
tauri-build = {{ version = "2", features = [] }}

[dependencies]
tauri = {{ version = "2", features = [] }}

[[bin]]
name = "{c}-desktop"
path = "src/main.rs"
"#
        )
    }

    pub const TAURI_MAIN_RS: &str = r#"// The window loads the live canister URL (see tauri.conf.json). The app's logic
// stays on-chain; this is only a native window. No application code here.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    tauri::Builder::default()
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
"#;

    pub const TAURI_CAPABILITIES: &str = r#"{
  "$schema": "../gen/schemas/desktop-schema.json",
  "identifier": "default",
  "description": "Default capability for the main window",
  "windows": ["main"],
  "permissions": ["core:default"]
}
"#;

    pub fn capacitor_conf(name: &str, id: &str, url: &str) -> String {
        format!(
            r##"{{
  "appId": "{id}",
  "appName": "{name}",
  "webDir": "www",
  "server": {{ "url": "{url}", "cleartext": false, "androidScheme": "https", "iosScheme": "https" }},
  "backgroundColor": "#6d28d9ff",
  "ios": {{ "contentInset": "always" }},
  "android": {{ "allowMixedContent": false }}
}}
"##
        )
    }

    pub fn capacitor_package(id: &str) -> String {
        format!(
            r#"{{
  "name": "{id}",
  "version": "0.1.0",
  "private": true,
  "dependencies": {{ "@capacitor/core": "^6", "@capacitor/ios": "^6", "@capacitor/android": "^6" }},
  "devDependencies": {{ "@capacitor/cli": "^6" }}
}}
"#
        )
    }

    pub fn shell_redirect_html(url: &str) -> String {
        // A declarative meta-refresh fallback (no JavaScript) for the bundled
        // frontendDist/www; the real app loads from the canister URL.
        format!(
            r#"<!doctype html>
<html><head><meta charset="utf-8">
<meta http-equiv="refresh" content="0; url={url}/">
<title>Loading…</title></head>
<body><p>Loading the app… <a href="{url}/">{url}</a></p></body></html>
"#
        )
    }

    pub fn shell_readme(name: &str, url: &str) -> String {
        format!(
            r#"# {name} — native shells

Thin native wrappers around your MotoView app, served from its canister at
`{url}`. The app's logic stays on-chain; these shells just open a native window
pointed at it. **No application code lives here.**

## Simplest "native app": install the PWA (no build)

Your MotoView app is already an installable PWA (web manifest + offline service
worker). Desktop Chrome/Edge: the install icon in the address bar. Android:
"Add to Home screen". iOS Safari: Share → Add to Home Screen. You get a
standalone, offline-capable app with zero build steps.

## Desktop (Tauri) — needs the Tauri CLI

```
cd desktop-tauri
cargo tauri build      # -> .app / .dmg / .exe / .AppImage
```

The window loads `{url}` (see `src-tauri/tauri.conf.json`). Install the CLI with
`cargo install tauri-cli`; add icons under `src-tauri/icons/`.

## Mobile (Capacitor) — needs Xcode (iOS) / Android SDK

```
cd mobile-capacitor
npm install
npx cap add ios            # or: android
npx cap open ios           # build & run in Xcode / Android Studio
```

`capacitor.config.json` points `server.url` at `{url}`.

> `motoview shell` generates these configs; building the native binaries needs
> the platform toolchains (Tauri CLI, Xcode, Android SDK) on your machine.
"#
        )
    }
}

#[cfg(test)]
mod cli_tests {
    use super::{opt, parse_session, positional};

    fn argv(a: &[&str]) -> Vec<String> {
        a.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn parse_session_reads_events_object_and_bare_array() {
        // Object form with an `events` array; args + caller defaults applied.
        let obj = r#"{ "events": [
            { "handler": "increment", "args": ["1"] },
            { "handler": "increment", "args": ["1"], "caller": "aaaaa-aa" },
            { "handler": "reset" }
        ] }"#;
        let evs = parse_session(obj).expect("session");
        assert_eq!(evs.len(), 3);
        assert_eq!(evs[0].handler, "increment");
        assert_eq!(evs[0].args, vec!["1".to_string()]);
        // default caller is the anonymous principal.
        assert_eq!(evs[0].caller, crate::project::ReplayEvent::ANON);
        // explicit caller is honoured.
        assert_eq!(evs[1].caller, "aaaaa-aa");
        // missing args -> empty.
        assert!(evs[2].args.is_empty());
        assert_eq!(evs[2].handler, "reset");

        // Bare-array form parses identically.
        let bare = r#"[ { "handler": "decrement" } ]"#;
        let evs2 = parse_session(bare).expect("bare session");
        assert_eq!(evs2.len(), 1);
        assert_eq!(evs2[0].handler, "decrement");

        // A malformed event (no handler) is an error, not a silent drop.
        assert!(parse_session(r#"[ { "args": ["x"] } ]"#).is_err());
    }

    #[test]
    fn space_form_network_flag_does_not_eat_the_positional_dir() {
        // REGRESSION: `build --network ic` must NOT pick `ic` as the project dir.
        let a = argv(&["--network", "ic"]);
        assert_eq!(positional(&a), None, "no explicit dir -> default `.`");
        assert_eq!(opt(&a, "--network"), Some("ic"));

        // With an explicit dir before the flag, the dir is still found.
        let a = argv(&["myproj", "--network", "ic"]);
        assert_eq!(positional(&a), Some("myproj"));
        assert_eq!(opt(&a, "--network"), Some("ic"));

        // And after the flag.
        let a = argv(&["--network", "ic", "myproj"]);
        assert_eq!(positional(&a), Some("myproj"));
        assert_eq!(opt(&a, "--network"), Some("ic"));
    }

    #[test]
    fn equals_form_network_flag_is_unaffected() {
        let a = argv(&["--network=ic", "myproj"]);
        assert_eq!(positional(&a), Some("myproj"));
        assert_eq!(opt(&a, "--network"), Some("ic"));
        // `--network=ic` alone -> no positional, value still read.
        let a = argv(&["--network=ic"]);
        assert_eq!(positional(&a), None);
        assert_eq!(opt(&a, "--network"), Some("ic"));
    }

    #[test]
    fn other_value_flags_also_skip_their_value() {
        let a = argv(&["--name", "Cool", "--out", "out/main.mo", "dir"]);
        assert_eq!(positional(&a), Some("dir"));
        assert_eq!(opt(&a, "--name"), Some("Cool"));
        assert_eq!(opt(&a, "--out"), Some("out/main.mo"));
    }
}

#[cfg(test)]
mod shell_tests {
    use super::scaffold;
    #[test]
    fn shell_configs_embed_the_canister_url() {
        let url = "https://abcde-cai.icp0.io";
        let t = scaffold::tauri_conf("MyApp", "app.x", url);
        assert!(t.contains(&format!("\"url\": \"{}/\"", url)), "tauri window url missing:\n{t}");
        assert!(t.contains("schema.tauri.app"), "not a tauri v2 config:\n{t}");
        let c = scaffold::capacitor_conf("MyApp", "app.x", url);
        assert!(c.contains(&format!("\"url\": \"{}\"", url)), "capacitor server url missing:\n{c}");
        assert!(c.contains("\"appName\": \"MyApp\""), "capacitor appName missing:\n{c}");
        let r = scaffold::shell_readme("MyApp", url);
        assert!(r.contains("Xcode") && r.contains("Tauri"), "readme must be honest about native toolchains:\n{r}");
    }
}
