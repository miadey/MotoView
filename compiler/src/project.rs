//! Project orchestration: discover `.mview` files, compile them, and assemble
//! a single Motoko actor (`src/main.mo`) that wires every page/layout into the
//! MotoView runtime.

use crate::ast::FileKind;
use crate::codegen::Codegen;
use crate::parser;
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

pub struct BuildOptions {
    pub project_dir: PathBuf,
    pub app_name: String,
    pub out: PathBuf,
}

pub fn build(opts: &BuildOptions) -> Result<String, String> {
    let src = opts.project_dir.join("src");
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
            extra_imports.push_str(&format!("import {n}__mod \"Services/{n}\";\n", n = n));
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
            extra_imports.push_str(&format!("import {} \"Services/{}\";\n", n, n));
        }
    }
    // Generate the stable backing + upgrade hooks. Each persistent service keeps
    // its live state in non-stable collections; on `--mode upgrade` we snapshot
    // it to a `stable var` Blob (preupgrade) and restore it (postupgrade), so
    // state survives upgrades. See is_stateful_service for the service convention.
    let mut persistence = String::new();
    if !persistent_services.is_empty() {
        persistence.push_str("  // ---- upgrade-stable persistence ----\n");
        for n in &persistent_services {
            persistence.push_str(&format!("  stable var {n}__state : Blob = \"\" : Blob;\n", n = n));
        }
        persistence.push_str("  system func preupgrade() {\n");
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
    }
    for f in list_mo(&src.join("Models")) {
        let n = file_stem(&f);
        extra_imports.push_str(&format!("import {} \"Models/{}\";\n", n, n));
    }

    if page_files.is_empty() {
        return Err(format!(
            "no .mview pages found in {}",
            pages_dir.display()
        ));
    }

    // Model type scanning is a roadmap feature; pass an empty registry for now.
    let models: HashMap<String, HashMap<String, String>> = HashMap::new();

    // App components: parse src/Components/*.mview, register their params, and
    // generate a render function per component. Pages then compile a `<Card .../>`
    // tag to a call of `mvComponent_Card(...)`.
    let mut components: HashMap<String, crate::codegen::CompInfo> = HashMap::new();
    let mut parsed_components = Vec::new();
    for cf in &component_files {
        let name = file_stem(cf);
        let source = fs::read_to_string(cf).map_err(|e| format!("{}: {}", cf.display(), e))?;
        let file = parser::parse(&source, &name, FileKind::Component)?;
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

    for pf in &page_files {
        let name = file_stem(pf);
        let source = fs::read_to_string(pf).map_err(|e| format!("{}: {}", pf.display(), e))?;
        let file = parser::parse(&source, &name, FileKind::Page)?;
        let mut cg = Codegen::new(&models, &components);
        let pg = cg.gen_page(&file);
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
        let mut cg = Codegen::new(&models, &components);
        layout_funcs.push_str(&format!("  // mv:src {}\n", rel_src(&opts.project_dir, lf)));
        layout_funcs.push_str(&cg.gen_layout(&file));
        layout_funcs.push('\n');
        layout_entries.push((name.clone(), format!("mvLayout_{}", name)));
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
    );
    // Collapse the per-token `b.raw("…")` storm into one call per contiguous
    // static run, so the generated actor reads as readable HTML chunks.
    let main = coalesce_raw(&main);

    if let Some(parent) = opts.out.parent() {
        let _ = fs::create_dir_all(parent);
    }
    fs::write(&opts.out, &main).map_err(|e| format!("writing {}: {}", opts.out.display(), e))?;

    let mut summary = format!(
        "compiled {} page(s), {} layout(s) -> {}\n",
        page_files.len(),
        layout_files.len(),
        opts.out.display()
    );
    summary.push_str("routes:\n");
    for (r, n) in &routes {
        summary.push_str(&format!("  {:<24} {}\n", r, n));
    }
    Ok(summary)
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
  if (mvSecret.size() == 32) {{ mvApp.setSecret(mvSecret) }};
  mvApp.setEpochs(mvEpochs);
  mvApp.setRoles(mvRoles);

  public shared query (msg) func http_request(req : MV.HttpRequest) : async MV.HttpResponse {{
    mvApp.httpRequest(req, msg.caller);
  }};

  public shared (msg) func http_request_update(req : MV.HttpRequest) : async MV.HttpResponse {{
    if (mvApp.needsSecret()) {{ mvSecret := await Random.blob(); mvApp.setSecret(mvSecret) }};
    let mvResp = mvApp.httpRequestUpdate(req, msg.caller);
    mvEpochs := mvApp.dumpEpochs(); // persist any logout-bump
    mvRoles := mvApp.dumpRoles(); // persist any role grant/revoke
    mvResp;
  }};

  // Internet Identity login bridge: an authenticated update call whose caller
  // the IC has verified. Records the principal under the client's nonce so a
  // following GET /mv-session can mint a session token bound to it.
  public shared (msg) func mvEstablish(nonce : Text) : async () {{
    if (mvApp.needsSecret()) {{ mvSecret := await Random.blob(); mvApp.setSecret(mvSecret) }};
    mvApp.establish(nonce, msg.caller);
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
    )
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

/// Path of a source file relative to the project dir (for `// mv:src` markers).
fn rel_src(project_dir: &Path, file: &Path) -> String {
    file.strip_prefix(project_dir)
        .unwrap_or(file)
        .to_string_lossy()
        .replace('\\', "/")
}

/// Rewrite `moc` errors that point at the generated `main.mo` so they name the
/// originating `.mview`/source region instead. Uses the `// mv:src <path>`
/// markers emitted per page/component/layout. Returns the mapped report and
/// whether any errors were found.
pub fn map_moc_errors(main_mo: &str, moc_output: &str) -> (String, bool) {
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
            match src_for(gen_line) {
                Some(src) => out.push_str(&format!("{}  ({}, generated main.mo:{})\n", msg, src, gen_line)),
                None => out.push_str(&format!("{}  (generated main.mo:{})\n", msg, gen_line)),
            }
        } else if !line.trim().is_empty() {
            out.push_str(line);
            out.push('\n');
        }
    }
    (out, had_error)
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
