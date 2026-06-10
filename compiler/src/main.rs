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
mod lint;
mod parser;
mod project;
#[cfg(test)]
mod tests;

use std::collections::HashMap;
use std::path::PathBuf;
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
        "compile" => cmd_compile(&args[1..]),
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
         \x20 motoview build [dir]           Compile .mview files into Motoko (.mvbuild/main.mo)\n\
         \x20                                  --network <local|ic>  vetKD key gate (default local)\n\
         \x20 motoview check [dir]           Build, then type-check; errors point at your .mview\n\
         \x20                                  --network <local|ic>  vetKD key gate (default local)\n\
         \x20 motoview lint [dir]            Run the security lint pass; print diagnostics\n\
         \x20 motoview compile <file.mview>  Compile a single file and print the Motoko\n\
         \x20 motoview dev [dir]             Build, then `dfx deploy` to the local replica\n\
         \x20 motoview shell --url <url>     Scaffold desktop (Tauri) + mobile (Capacitor) shells\n\
         \x20 motoview version               Print the version\n\n\
         Rendering is a query. Events are updates. Write Motoko, ship to ICP.\n"
    );
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
const VALUE_FLAGS: &[&str] = &["--name", "--out", "--network", "--url", "--id"];

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
    let opts = project::BuildOptions {
        project_dir: dir,
        app_name: name,
        out,
        network,
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

/// `motoview lint [dir]` — run the security lint pass over every .mview and
/// print all diagnostics (`error:` / `warning:`). Exits 1 if any Error, else 0.
fn cmd_lint(args: &[String]) -> i32 {
    let dir = PathBuf::from(positional(args).unwrap_or("."));
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

/// Build, then type-check the generated actor with `moc`, rewriting any errors
/// to name the originating .mview source instead of the generated main.mo.
fn cmd_check(args: &[String]) -> i32 {
    let dir = PathBuf::from(positional(args).unwrap_or("."));
    let name = app_name_for(&dir, opt(args, "--name"));
    let out = dir.join(".mvbuild").join("main.mo");
    let network = opt(args, "--network").unwrap_or("local").to_string();
    let opts = project::BuildOptions {
        project_dir: dir.clone(),
        app_name: name,
        out: out.clone(),
        network,
    };
    match project::build(&opts) {
        Ok(summary) => print!("{}", summary),
        Err(e) => {
            eprintln!("build error: {}", e);
            return 1;
        }
    }
    let (moc, base) = match find_moc() {
        Some(x) => x,
        None => {
            eprintln!("\nnote: `moc` not found under ~/.cache/dfinity/versions — run `dfx deploy`\nto type-check. Errors map to .mvbuild/main.mo; the `// mv:src <file>`\nmarkers above each region tell you which .mview it came from.");
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
    let (mapped, had_err) = project::map_moc_errors(&main_mo, &stderr);
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

fn cmd_new(args: &[String]) -> i32 {
    let name = match positional(args) {
        Some(n) => n,
        None => {
            eprintln!("usage: motoview new <name>");
            return 1;
        }
    };
    let root = PathBuf::from(name);
    let mk = |rel: &str, contents: &str| -> std::io::Result<()> {
        let p = root.join(rel);
        if let Some(parent) = p.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(p, contents)
    };
    let res = (|| -> std::io::Result<()> {
        mk("src/Pages/Home.mview", scaffold::HOME)?;
        mk("src/Layouts/MainLayout.mview", scaffold::LAYOUT)?;
        mk("dfx.json", &scaffold::dfx_json(name))?;
        mk("motoview.json", &scaffold::motoview_json(name))?;
        mk("README.md", &format!("# {}\n\nA MotoView app. Run `motoview dev`.\n", name))?;
        Ok(())
    })();
    match res {
        Ok(()) => {
            println!("created MotoView project '{}'.\n\nNext:\n  cd {}\n  motoview dev\n", name, name);
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

mod scaffold {
    pub const HOME: &str = r#"@page "/"
@layout MainLayout
@title "Home"

<section class="mv-container">
    <h1>Hello from MotoView</h1>
    <p>You clicked <strong>@count</strong> times.</p>
    <button class="mv-btn mv-btn-primary" @click="bump">Click me</button>
</section>

@code {
    var count : Nat = 0;

    func bump() : async () {
        count += 1;
    };
}
"#;

    pub const LAYOUT: &str = r#"<!DOCTYPE html>
<html lang="en">
<head>
    <meta charset="utf-8">
    <meta name="viewport" content="width=device-width, initial-scale=1">
    <title>@View.title</title>
    @head
</head>
<body>
    <main>
        @yield
    </main>
</body>
</html>
"#;

    pub fn dfx_json(name: &str) -> String {
        // Bind a dedicated local port (not dfx's default 4943) so `dfx start
        // --clean` here never wipes other projects' replica state.
        format!(
            "{{\n  \"version\": 1,\n  \"canisters\": {{\n    \"{}\": {{\n      \"type\": \"motoko\",\n      \"main\": \".mvbuild/main.mo\",\n      \"args\": \"--package motoview ../../runtime/src\"\n    }}\n  }},\n  \"networks\": {{\n    \"local\": {{\n      \"bind\": \"127.0.0.1:4955\",\n      \"type\": \"ephemeral\"\n    }}\n  }}\n}}\n",
            name
        )
    }

    pub fn motoview_json(name: &str) -> String {
        format!(
            "{{\n  \"name\": \"{}\",\n  \"pages\": \"src/Pages\",\n  \"components\": \"src/Components\",\n  \"layouts\": \"src/Layouts\",\n  \"output\": \".mvbuild/main.mo\",\n  \"seo\": true\n}}\n",
            name
        )
    }

    // ---- native shell scaffolds (motoview shell) --------------------------
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
    use super::{opt, positional};

    fn argv(a: &[&str]) -> Vec<String> {
        a.iter().map(|s| s.to_string()).collect()
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
