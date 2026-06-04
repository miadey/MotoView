//! `motoview` — the MotoView compiler CLI.
//!
//!   motoview new <name>           scaffold a new project
//!   motoview build [dir]          compile .mview files -> src/main.mo
//!   motoview compile <file.mview> compile one file and print the Motoko (debug)
//!   motoview dev [dir]            build, then `dfx deploy` (local)
//!   motoview version

mod ast;
mod codegen;
mod parser;
mod project;

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
        "compile" => cmd_compile(&args[1..]),
        "new" => cmd_new(&args[1..]),
        "dev" => cmd_dev(&args[1..]),
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
         \x20 motoview build [dir]           Compile .mview files into Motoko (src/main.mo)\n\
         \x20 motoview compile <file.mview>  Compile a single file and print the Motoko\n\
         \x20 motoview dev [dir]             Build, then `dfx deploy` to the local replica\n\
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

fn positional(args: &[String]) -> Option<&str> {
    args.iter().find(|a| !a.starts_with('-')).map(|s| s.as_str())
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
    let out = opt(args, "--out")
        .map(PathBuf::from)
        .unwrap_or_else(|| dir.join("src").join("main.mo"));
    let opts = project::BuildOptions {
        project_dir: dir,
        app_name: name,
        out,
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
            "{{\n  \"version\": 1,\n  \"canisters\": {{\n    \"{}\": {{\n      \"type\": \"motoko\",\n      \"main\": \"src/main.mo\",\n      \"args\": \"--package motoview ../../runtime/src\"\n    }}\n  }},\n  \"networks\": {{\n    \"local\": {{\n      \"bind\": \"127.0.0.1:4955\",\n      \"type\": \"ephemeral\"\n    }}\n  }}\n}}\n",
            name
        )
    }

    pub fn motoview_json(name: &str) -> String {
        format!(
            "{{\n  \"name\": \"{}\",\n  \"pages\": \"src/Pages\",\n  \"components\": \"src/Components\",\n  \"layouts\": \"src/Layouts\",\n  \"output\": \"src/main.mo\",\n  \"seo\": true\n}}\n",
            name
        )
    }
}
