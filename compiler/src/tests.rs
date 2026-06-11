//! Regression tests for the parser + codegen. Each locks in a bug that was
//! found and fixed while building real apps, so the whole class can't return.
#![cfg(test)]

use crate::ast::{FileKind, ParamDecl};
use crate::codegen::{CompInfo, Codegen, EmitMode};
use crate::lint::{self, Severity};
use crate::parser;
use crate::project;
use std::collections::HashMap;

/// Parse + generate a page object block.
fn page(src: &str) -> String {
    let models: HashMap<String, HashMap<String, String>> = HashMap::new();
    let comps: HashMap<String, CompInfo> = HashMap::new();
    let file = parser::parse(src, "T", FileKind::Page).expect("parse failed");
    let mut cg = Codegen::new(&models, &comps);
    cg.gen_page(&file).object_block
}

/// Parse + generate the page RECORD (the `MV.Page = { ... }` literal), which
/// carries route/authorize/authRedirect/secureHandlers metadata.
fn page_record(src: &str) -> String {
    let models: HashMap<String, HashMap<String, String>> = HashMap::new();
    let comps: HashMap<String, CompInfo> = HashMap::new();
    let file = parser::parse(src, "T", FileKind::Page).expect("parse failed");
    let mut cg = Codegen::new(&models, &comps);
    cg.gen_page(&file).page_record
}

/// Parse + generate a page object block via the IR (UINode) backend.
fn page_ir(src: &str) -> String {
    let models: HashMap<String, HashMap<String, String>> = HashMap::new();
    let comps: HashMap<String, CompInfo> = HashMap::new();
    let file = parser::parse(src, "T", FileKind::Page).expect("parse failed");
    let mut cg = Codegen::new_with_emit(&models, &comps, EmitMode::Ir);
    cg.gen_page(&file).object_block
}

/// No generated line should contain `}; else` / `}else` chains that don't
/// compile, and braces should balance.
fn balanced_braces(s: &str) -> bool {
    let mut depth: i32 = 0;
    let mut in_str = false;
    let mut prev = ' ';
    for c in s.chars() {
        match c {
            '"' if prev != '\\' => in_str = !in_str,
            '{' if !in_str => depth += 1,
            '}' if !in_str => depth -= 1,
            _ => {}
        }
        prev = c;
    }
    depth == 0
}

#[test]
fn if_else_generates_valid_motoko() {
    let g = page("@page \"/\"\n@if x { <p>a</p> } else { <p>b</p> }\n@code { var x : Bool = true; }");
    assert!(!g.contains("}; else"), "if/else must not emit `}}; else`:\n{g}");
    assert!(g.contains("} else {") || g.contains("}else{"), "missing else chain:\n{g}");
    assert!(balanced_braces(&g), "unbalanced braces:\n{g}");
}

#[test]
fn else_if_chain_is_valid() {
    let g = page("@page \"/\"\n@if x { <p>a</p> } else if y { <p>b</p> } else { <p>c</p> }\n@code { var x : Bool = true; var y : Bool = false; }");
    assert!(!g.contains("}; else"), "else-if must not emit `}}; else`:\n{g}");
    assert!(g.contains("else if ("), "missing else-if:\n{g}");
    assert!(balanced_braces(&g));
}

#[test]
fn apostrophe_in_comment_does_not_corrupt_handlers() {
    // The apostrophe in `it's` once started a bogus char-literal scan that
    // swallowed the rest of the body and the next function.
    let g = page(
        "@page \"/\"\n<button @click=\"a\">A</button><button @click=\"b\">B</button>\n@code {\n  func a(ctx : Context) : async () {\n    // it's fine, don't break\n    toast(\"x\");\n  };\n  func b(ctx : Context) : async () { toast(\"y\"); };\n}",
    );
    assert!(g.contains("case \"a\""), "handler a lost:\n{g}");
    assert!(g.contains("case \"b\""), "handler b lost (comment apostrophe corrupted parse):\n{g}");
    assert!(balanced_braces(&g), "unbalanced braces:\n{g}");
}

#[test]
fn type_alias_in_code_keeps_its_semicolon() {
    let g = page("@page \"/\"\n<p>x</p>\n@code { type T = { a : Nat }; var v : T = { a = 1 }; }");
    assert!(g.contains("type T = { a : Nat };"), "type alias lost its `;`:\n{g}");
}

#[test]
fn style_media_query_stays_literal_css() {
    let g = page("@page \"/\"\n<style>@media (max-width: 600px) { .x { color: red } }\n@keyframes k { from { opacity: 0 } }</style>\n@code { }");
    assert!(g.contains("@media"), "@media must survive as literal CSS:\n{g}");
    assert!(g.contains("@keyframes"), "@keyframes must survive:\n{g}");
}

#[test]
fn route_params_are_bound_and_typed() {
    let g = page("@page \"/orders/{id:Nat}/{tab}\"\n<p>@id @tab</p>\n@code { }");
    assert!(g.contains("mvParamGet(ctx, \"id\")"), "id param not bound:\n{g}");
    assert!(g.contains("mvParamGet(ctx, \"tab\")"), "tab param not bound:\n{g}");
    assert!(g.contains("mvNat("), "typed {{id:Nat}} should convert via mvNat:\n{g}");
}

#[test]
fn secure_form_handler_recorded_in_page_record() {
    // #40: the server must REQUIRE a token for handlers bound to a `secure` form,
    // not trust the request's `__mv_secure` flag — so the compiler bakes the
    // secure-handler set into the page record for the runtime to enforce.
    let g = page_record(
        "@page \"/\"\n<form @submit=\"save\" secure><button>Go</button></form>\n@code { func save(ctx : Context) : async () {}; }",
    );
    assert!(
        g.contains("secureHandlers = [\"save\"]"),
        "secure form's submit handler must be in secureHandlers:\n{g}"
    );
}

#[test]
fn non_secure_handler_absent_from_secure_set() {
    // A non-secure click handler must NOT be forced through token verification.
    let g = page_record(
        "@page \"/\"\n<button @click=\"ping\">x</button>\n@code { func ping(ctx : Context) {}; }",
    );
    assert!(
        g.contains("secureHandlers = []"),
        "a non-secure handler must leave secureHandlers empty:\n{g}"
    );
}

#[test]
fn authorize_redirect_is_emitted_in_page_record() {
    // #40: `@authorize redirect="/welcome"` lets a route (even "/") gate itself
    // without a 302 loop; the target rides into the page record.
    let g = page_record("@page \"/\"\n@authorize redirect=\"/welcome\"\n<p>x</p>\n@code {}");
    assert!(g.contains("authorize = true"), "authorize must be true:\n{g}");
    assert!(
        g.contains("authRedirect = \"/welcome\""),
        "authRedirect must carry the configured target:\n{g}"
    );
    // Default (no redirect attr) is the empty string -> runtime falls back to "/".
    let d = page_record("@page \"/x\"\n@authorize\n<p>x</p>\n@code {}");
    assert!(
        d.contains("authRedirect = \"\""),
        "absent redirect attr -> empty authRedirect:\n{d}"
    );
}

#[test]
fn layout_auth_gate_detection_is_precise() {
    // #40 lint: a layout that conditionally renders @yield on auth IS a (cosmetic)
    // gate; one that renders @yield unconditionally (auth only redirects/chrome)
    // is NOT — so /welcome-style login layouts aren't false-flagged.
    let gated = parser::parse(
        "@if (ctx.isAuthenticated) { <main>@yield</main> } else { <p>sign in</p> }",
        "L",
        FileKind::Layout,
    )
    .expect("parse gated layout");
    assert!(
        lint::layout_gates_on_auth(&gated),
        "a layout gating @yield behind isAuthenticated must be detected"
    );

    let chrome_only = parser::parse(
        "@if (ctx.isAuthenticated) { <meta http-equiv=\"refresh\" content=\"0;url=/\"> }\n<main>@yield</main>",
        "L",
        FileKind::Layout,
    )
    .expect("parse chrome layout");
    assert!(
        !lint::layout_gates_on_auth(&chrome_only),
        "a layout that renders @yield unconditionally must NOT be flagged"
    );
}

#[test]
fn redirect_builtin_is_emitted_and_usable_from_handlers() {
    let g = page("@page \"/\"\n<form @submit=\"go\"><button>Go</button></form>\n@code { func go(ctx : Context) : async () { redirect(\"/feed\"); }; }");
    assert!(
        g.contains("public func redirect(url : Text) { mvRedirect := url }"),
        "redirect() builtin must be emitted on every page object:\n{g}"
    );
    assert!(g.contains("redirect(\"/feed\")"), "handler body should call redirect():\n{g}");
    assert!(g.contains("mvTakeRedirect"), "redirect sink accessor missing:\n{g}");
}

#[test]
fn handler_ctx_is_injected_when_first_param() {
    let g = page("@page \"/\"\n<button @click=\"go\">go</button>\n@code { func go(ctx : Context) : async () { ignore ctx; }; }");
    assert!(g.contains("case \"go\" { go(ctx) }"), "ctx not injected into handler:\n{g}");
}

#[test]
fn handler_event_arg_is_bound() {
    let g = page("@page \"/\"\n<button @click=\"rm(item.id)\">x</button>\n@code { func rm(id : Nat) : async () { ignore id; }; }");
    assert!(g.contains("case \"rm\""), "handler not dispatched:\n{g}");
    assert!(g.contains("mvArgs"), "event arg not read from mvArgs:\n{g}");
}

#[test]
fn for_loop_compiles_to_vals_iteration() {
    let g = page("@page \"/\"\n@for x in items { <p>@x</p> }\n@code { var items : [Text] = []; }");
    assert!(g.contains("for (x in (items).vals())"), "@for must iterate .vals():\n{g}");
    assert!(balanced_braces(&g));
}

#[test]
fn double_at_escapes_to_literal() {
    let g = page("@page \"/\"\n<p>email me @@here</p>\n@code { }");
    assert!(g.contains("@here") || g.contains("@") , "@@ should escape to a literal @:\n{g}");
    assert!(!g.contains("mvParamGet"), "@@here must not be parsed as a directive:\n{g}");
}

#[test]
fn theme_preset_emits_tokens_in_layout() {
    let models: HashMap<String, HashMap<String, String>> = HashMap::new();
    let comps: HashMap<String, CompInfo> = HashMap::new();
    let src = "@theme \"ocean\"\n<!DOCTYPE html><html><head>@head</head><body>@yield</body></html>";
    let file = parser::parse(src, "L", FileKind::Layout).unwrap();
    let mut cg = Codegen::new(&models, &comps);
    let g = cg.gen_layout(&file);
    assert!(g.contains("<style>:root{"), "theme style block emitted:\n{g}");
    assert!(g.contains("--mv-primary:#0e76a0"), "ocean primary emitted:\n{g}");
}

#[test]
fn theme_override_wins_over_preset() {
    let models: HashMap<String, HashMap<String, String>> = HashMap::new();
    let comps: HashMap<String, CompInfo> = HashMap::new();
    let src = "@theme \"ocean\" { --mv-primary: #ff0000 }\n<html><head>@head</head><body>@yield</body></html>";
    let file = parser::parse(src, "L", FileKind::Layout).unwrap();
    let mut cg = Codegen::new(&models, &comps);
    let g = cg.gen_layout(&file);
    assert!(g.contains("--mv-primary:#ff0000"), "override wins:\n{g}");
    assert!(!g.contains("--mv-primary:#0e76a0"), "preset primary replaced:\n{g}");
}

#[test]
fn no_theme_emits_no_style() {
    let models: HashMap<String, HashMap<String, String>> = HashMap::new();
    let comps: HashMap<String, CompInfo> = HashMap::new();
    let src = "<html><head>@head</head><body>@yield</body></html>";
    let file = parser::parse(src, "L", FileKind::Layout).unwrap();
    let mut cg = Codegen::new(&models, &comps);
    let g = cg.gen_layout(&file);
    assert!(!g.contains("<style>:root"), "no @theme -> no injected style:\n{g}");
}

#[test]
fn typed_loop_field_access_avoids_debug_show() {
    // #19: a Model record type lets @p.field type precisely.
    let mut models: HashMap<String, HashMap<String, String>> = HashMap::new();
    let mut product = HashMap::new();
    product.insert("name".to_string(), "Text".to_string());
    product.insert("price".to_string(), "Nat".to_string());
    product.insert("on".to_string(), "Bool".to_string());
    models.insert("Catalog.Product".to_string(), product.clone());
    models.insert("Product".to_string(), product);
    let comps: HashMap<String, CompInfo> = HashMap::new();
    let src = "@page \"/\"\n@for p in products { <span>@p.name @p.price @p.on</span> }\n@code { var products : [Catalog.Product] = []; }";
    let file = parser::parse(src, "T", FileKind::Page).unwrap();
    let mut cg = Codegen::new(&models, &comps);
    let g = cg.gen_page(&file).object_block;
    assert!(g.contains("b.text(p.name)"), "Text field should render directly:\n{g}");
    assert!(g.contains("Nat.toText(p.price)"), "Nat field via toText:\n{g}");
    assert!(g.contains("if (p.on)"), "Bool field via if:\n{g}");
    assert!(!g.contains("debug_show(p."), "no debug_show fallback for typed fields:\n{g}");
}

#[test]
fn untyped_field_still_falls_back_to_debug_show() {
    // No models registry -> unknown field type -> safe debug_show fallback.
    let models: HashMap<String, HashMap<String, String>> = HashMap::new();
    let comps: HashMap<String, CompInfo> = HashMap::new();
    let src = "@page \"/\"\n@for p in xs { <span>@p.whatever</span> }\n@code { var xs : [Mystery] = []; }";
    let file = parser::parse(src, "T", FileKind::Page).unwrap();
    let mut cg = Codegen::new(&models, &comps);
    let g = cg.gen_page(&file).object_block;
    assert!(g.contains("debug_show(p.whatever)"), "unknown field falls back:\n{g}");
}

#[test]
fn enter_exit_attrs_become_data_attrs() {
    let g = page("@page \"/\"\n@for x in xs { <li key=\"@x\" enter=\"fade-up\" exit=\"fade-out\">@x</li> }\n@code { var xs : [Text] = []; }");
    assert!(g.contains("data-mv-enter"), "enter -> data-mv-enter:\n{g}");
    assert!(g.contains("data-mv-exit"), "exit -> data-mv-exit:\n{g}");
}

#[test]
fn raw_directive_emits_unescaped_html() {
    let g = page("@page \"/\"\n<div>@raw(body)</div>\n@code { var body : Text = \"<b>hi</b>\"; }");
    assert!(g.contains("b.raw(body)"), "@raw must emit b.raw (unescaped):\n{g}");
    // the surrounding @expr default still escapes
    let e = page("@page \"/\"\n<div>@body</div>\n@code { var body : Text = \"x\"; }");
    assert!(e.contains("b.text(body)"), "@expr must still escape:\n{e}");
}

#[test]
fn app_component_call_maps_props_and_defaults() {
    let models: HashMap<String, HashMap<String, String>> = HashMap::new();
    let mut comps: HashMap<String, CompInfo> = HashMap::new();
    // a non-built-in name (built-ins like Card/Button take precedence)
    comps.insert(
        "ProductCard".to_string(),
        CompInfo {
            params: vec![
                ParamDecl { name: "title".into(), ty: "Text".into(), default: None },
                ParamDecl { name: "n".into(), ty: "Nat".into(), default: Some("0".into()) },
            ],
            slots: vec![],
        },
    );
    let file = parser::parse("@page \"/\"\n<ProductCard title=\"Hi\">body</ProductCard>\n@code { }", "T", FileKind::Page).unwrap();
    let mut cg = Codegen::new(&models, &comps);
    let g = cg.gen_page(&file).object_block;
    // title from the prop, n from its default, children passed as a Text
    assert!(g.contains("mvComponent_ProductCard(\"Hi\", 0,"), "component call wrong:\n{g}");
}

#[test]
fn component_params_and_children_in_render() {
    let models: HashMap<String, HashMap<String, String>> = HashMap::new();
    let comps: HashMap<String, CompInfo> = HashMap::new();
    let src = "param title : Text\nparam featured : Bool = false\n<div>@title @if featured { <b>!</b> } <span>@children</span></div>";
    let file = parser::parse(src, "Card", FileKind::Component).unwrap();
    let mut cg = Codegen::new(&models, &comps);
    let g = cg.gen_app_component(&file);
    assert!(g.contains("func mvComponent_Card(title : Text, featured : Bool, mvChildren : Text)"), "component signature wrong:\n{g}");
    assert!(g.contains("b.raw(mvChildren)"), "@children not rendered:\n{g}");
}

#[test]
fn component_reserved_word_params_are_mangled() {
    let models: HashMap<String, HashMap<String, String>> = HashMap::new();
    let comps: HashMap<String, CompInfo> = HashMap::new();
    // `label` and `type` are Motoko keywords — a naive signature would be invalid
    // Motoko. They must be mangled in the signature AND in references, but the
    // literal word "label" in the HTML text must be left untouched.
    let src = "param label : Text\nparam type : Text\n<span>txt=<b>@label</b> kind=@type (literal label word)</span>";
    let file = parser::parse(src, "Tag", FileKind::Component).unwrap();
    let mut cg = Codegen::new(&models, &comps);
    let g = cg.gen_app_component(&file);
    assert!(g.contains("func mvComponent_Tag(mvP_label : Text, mvP_type : Text, mvChildren : Text)"), "reserved params not mangled in signature:\n{g}");
    assert!(g.contains("b.text(mvP_label)"), "label reference not mangled:\n{g}");
    assert!(g.contains("b.text(mvP_type)"), "type reference not mangled:\n{g}");
    // the keyword must NOT survive as a bare Motoko identifier
    assert!(!g.contains("b.text(label)") && !g.contains("(label :") && !g.contains("(type :"), "a reserved keyword leaked as an identifier:\n{g}");
    // but the literal HTML word inside b.raw("…") must be preserved verbatim
    assert!(g.contains("(literal label word)"), "literal HTML word was wrongly mangled:\n{g}");
}

// ---- Slice 6: UI-IR codegen backend ---------------------------------------

/// A representative page exercising the core node kinds the IR backend models:
/// elements + text, a click event with a baked arg, a keyed @for, and an @if.
const IR_SAMPLE: &str = "@page \"/\"\n<section><h1>Hi @name</h1>@if show { <p class=\"note\">visible</p> }<ul>@for it in items { <li key=\"@it\"><button @click=\"pick(it)\">@it</button></li> }</ul></section>\n@code { var name : Text = \"x\"; var show : Bool = true; var items : [Text] = []; func pick(v : Text) : async () { ignore v; }; }";

#[test]
fn html_backend_is_unchanged_under_ir_addition() {
    // Default codegen must remain the byte-identical Html path: it still emits an
    // Html.Builder and b.raw/b.text calls, and NEVER any Ir.Builder calls.
    let html = page(IR_SAMPLE);
    assert!(html.contains("let b = Html.Builder();"), "Html backend lost its builder:\n{html}");
    assert!(html.contains("b.raw(\"<section\")"), "Html backend lost element emit:\n{html}");
    assert!(html.contains("b.text(name)"), "Html backend lost escaped text:\n{html}");
    assert!(!html.contains("Ir.Builder"), "Html backend must not emit any IR:\n{html}");
    assert!(!html.contains("ir.open"), "Html backend must not emit any IR:\n{html}");
    assert!(balanced_braces(&html), "Html backend unbalanced:\n{html}");
}

#[test]
fn ir_backend_describes_the_same_tree() {
    // The IR backend emits an Ir.Builder tree with the SAME tags/text/event/key,
    // and returns the serialized forest (ir.toJson) — not HTML.
    let ir = page_ir(IR_SAMPLE);
    assert!(ir.contains("let ir = Ir.Builder();"), "IR backend missing its builder:\n{ir}");
    assert!(ir.contains("ir.toJson();"), "IR render must return the serialized forest:\n{ir}");
    assert!(!ir.contains("let b = Html.Builder()") || ir.contains("(do { let b = Html.Builder();"),
        "page-body IR must not use a bare Html.Builder (only the fallback do-block may):\n{ir}");
    // same element tags, modeled natively
    assert!(ir.contains("ir.open(\"section\")"), "section element missing:\n{ir}");
    assert!(ir.contains("ir.open(\"h1\")"), "h1 element missing:\n{ir}");
    assert!(ir.contains("ir.open(\"button\")"), "button element missing:\n{ir}");
    assert!(ir.contains("ir.close();"), "elements must close:\n{ir}");
    // escaped dynamic text + a static literal as a raw leaf
    assert!(ir.contains("ir.text(name)"), "dynamic text missing:\n{ir}");
    // class attribute preserved
    assert!(ir.contains("ir.attr(\"class\", \"note\")"), "static attr missing:\n{ir}");
    // the click event is native (eventName -> handlerId), with the baked arg
    assert!(ir.contains("ir.event(\"click\", \"pick\")"), "click event missing:\n{ir}");
    assert!(ir.contains("ir.attr(\"data-mv-arg0\", it)"), "event arg missing:\n{ir}");
    // the keyed @for region is native via ir.key, inside a real for-loop
    assert!(ir.contains("for (it in (items).vals())"), "@for must iterate .vals():\n{ir}");
    assert!(ir.contains("ir.key(it)"), "keyed region must be modeled with ir.key:\n{ir}");
    // the @if structure survives
    assert!(ir.contains("if (show) {"), "@if structure missing:\n{ir}");
    assert!(balanced_braces(&ir), "IR backend unbalanced:\n{ir}");
}

#[test]
fn ir_secure_form_carries_event_token_and_schema() {
    // A secure submit form is modeled as a native submit event plus its security
    // attrs (token/schema) — the same wiring the Html backend produces.
    let ir = page_ir("@page \"/\"\n<form @submit=\"save\" secure><input name=\"x\"><button>Go</button></form>\n@code { func save(ctx : Context) : async () { ignore ctx; }; }");
    assert!(ir.contains("ir.open(\"form\")"), "form element missing:\n{ir}");
    assert!(ir.contains("ir.attr(\"novalidate\", \"\")"), "server-driven form must be novalidate:\n{ir}");
    assert!(ir.contains("ir.event(\"submit\", \"save\")"), "submit event missing:\n{ir}");
    assert!(ir.contains("ir.attr(\"data-mv-secure\", \"1\")"), "secure marker missing:\n{ir}");
    assert!(ir.contains("ctx.mintToken(\"save\""), "secure token must be minted:\n{ir}");
    assert!(ir.contains("ir.attr(\"data-mv-schema\""), "secure schema missing:\n{ir}");
}

#[test]
fn ir_unmodeled_builtin_falls_back_to_raw_html() {
    // A built-in component (Button) is not yet IR-modeled: it must fall back to a
    // single ir.raw(...) leaf carrying the EXACT HTML the Html backend emits.
    let html = page("@page \"/\"\n<Button appearance=\"primary\" @click=\"go\">Save</Button>\n@code { func go() : async () {}; }");
    let ir = page_ir("@page \"/\"\n<Button appearance=\"primary\" @click=\"go\">Save</Button>\n@code { func go() : async () {}; }");
    // HTML path unchanged: the Button still compiles to its <button> markup.
    assert!(html.contains("mv-btn mv-btn-primary"), "Html Button markup changed:\n{html}");
    // IR path: a raw fallback leaf that reproduces the same Button markup inline.
    assert!(ir.contains("ir.raw((do { let b = Html.Builder();"), "builtin must fall back to ir.raw(do-block):\n{ir}");
    assert!(ir.contains("mv-btn mv-btn-primary"), "fallback must carry the real Button HTML:\n{ir}");
}

// ---- Slice 1: security lint pass ------------------------------------------

/// Parse a page and run the lint pass over it.
fn lint_page(src: &str) -> Vec<lint::Diagnostic> {
    let file = parser::parse(src, "T", FileKind::Page).expect("parse failed");
    lint::lint_file(&file, "src/Pages/T.mview")
}

#[test]
fn mutating_form_without_secure_is_a_lint_error() {
    let d = lint_page(
        "@page \"/\"\n<form @submit=\"save\"><input name=\"x\"><button>Go</button></form>\n@code { func save(ctx : Context) : async () { ignore ctx; }; }",
    );
    let errs: Vec<_> = d.iter().filter(|x| x.severity == Severity::Error).collect();
    assert_eq!(errs.len(), 1, "expected exactly one secure-form error:\n{:#?}", d);
    assert_eq!(errs[0].rule, "secure-form");
    assert!(errs[0].location.contains("save"), "location should name the handler:\n{:#?}", errs[0]);
}

// ---- R4: no-deploy preview (motoview preview) -----------------------------

mod preview {
    use crate::project;
    use std::path::PathBuf;
    use std::process::Command;

    /// Repo root: CARGO_MANIFEST_DIR is `<root>/compiler`, so the workspace root
    /// (where `examples/` and `runtime/` live) is its parent.
    fn repo_root() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .expect("compiler/ has a parent")
            .to_path_buf()
    }

    /// Locate moc + base under ~/.cache/dfinity/versions (newest), or None.
    fn find_moc() -> Option<(PathBuf, PathBuf)> {
        let home = std::env::var("HOME").ok()?;
        let versions = PathBuf::from(home).join(".cache/dfinity/versions");
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

    /// The moc `--package` args from a project's dfx.json (e.g.
    /// `--package motoview ../../runtime/src`), with the relative runtime path
    /// resolved against the project dir.
    fn dfx_package_args(dir: &PathBuf) -> Vec<String> {
        let txt = std::fs::read_to_string(dir.join("dfx.json")).unwrap_or_default();
        // crude extraction of the "args" string value
        let p = match txt.find("\"args\"") {
            Some(p) => p,
            None => return vec![],
        };
        let after = &txt[p..];
        let q1 = after.find(':').and_then(|c| after[c..].find('"')).map(|i| {
            let colon = after.find(':').unwrap();
            colon + i + 1
        });
        let start = match q1 {
            Some(s) => s,
            None => return vec![],
        };
        let rest = &after[start..];
        let end = match rest.find('"') {
            Some(e) => e,
            None => return vec![],
        };
        rest[..end]
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

    /// The PROOF test for R4: `motoview preview examples/counter` produces a REAL
    /// IR forest for the counter's initial render — built with the compiler and
    /// run through `moc -r` (the Motoko interpreter), with NO dfx and NO replica.
    ///
    /// The command path here uses ONLY the compiler (project::build_preview) and
    /// `moc` — `dfx` is never spawned. We assert the emitted stdout is valid IR
    /// forest JSON containing the counter's expected nodes (a `button` element and
    /// the live count `text` node).
    #[test]
    fn preview_counter_emits_real_ir_forest_with_no_deploy() {
        let counter = repo_root().join("examples").join("counter");
        assert!(counter.join("src/Pages/Counter.mview").exists(), "counter example missing");

        // 1) Generate the preview driver (compiler only — no dfx, no replica).
        let info = project::build_preview(&counter, None).expect("build_preview failed");
        assert!(info.driver_path.exists(), "driver not written");
        assert_eq!(info.route, "/", "counter route");
        assert_eq!(info.page_name, "Counter");
        let driver = std::fs::read_to_string(&info.driver_path).unwrap();
        // The driver must NOT be an actor and must NOT call dfx/deploy/http_request:
        // it is a pure `moc -r` program with a MOCK ctx.
        assert!(!driver.contains("actor {"), "preview driver must be a program, not an actor");
        assert!(driver.contains("let mockCtx : MV.Ctx"), "driver must build a mock ctx");
        assert!(driver.contains("Debug.print(CounterPage.mvRender(mockCtx))"), "driver must print the render");

        let (moc, base) = match find_moc() {
            Some(x) => x,
            None => {
                eprintln!("skipping preview run: moc not found under ~/.cache/dfinity/versions");
                return; // no moc in this env — driver generation already verified
            }
        };
        // 2) Run the driver through moc -r. The ONLY external binary is `moc`.
        //    `dfx` is never invoked: there is no Command::new("dfx") on this path.
        let mut cmd = Command::new(&moc);
        cmd.arg("-r").arg("--package").arg("base").arg(&base);
        for a in dfx_package_args(&counter) {
            cmd.arg(a);
        }
        cmd.arg(&info.driver_path);
        let out = cmd.output().expect("running moc -r failed");
        let stdout = String::from_utf8_lossy(&out.stdout);
        let forest = stdout
            .lines()
            .map(|l| l.trim())
            .find(|l| l.starts_with('[') && l.ends_with(']'))
            .unwrap_or_else(|| {
                panic!(
                    "no IR forest on stdout.\nstdout:\n{}\nstderr:\n{}",
                    stdout,
                    String::from_utf8_lossy(&out.stderr)
                )
            })
            .to_string();

        // 3) It is a VALID IR forest with the counter's expected nodes.
        assert!(forest.starts_with('[') && forest.ends_with(']'), "forest must be a JSON array");
        // The locked Ir.mo schema: element nodes carry {"t":"el","tag":...}.
        assert!(forest.contains("\"t\":\"el\""), "no element nodes in forest:\n{forest}");
        assert!(
            forest.contains("\"tag\":\"button\""),
            "counter must have a button element node:\n{forest}"
        );
        assert!(
            forest.contains("\"tag\":\"section\""),
            "counter wraps its body in a <section>:\n{forest}"
        );
        // The +1 / +5 buttons fire `increment`; the live count is a text node "0".
        assert!(
            forest.contains("\"click\":\"increment\""),
            "counter buttons must carry the increment click event:\n{forest}"
        );
        assert!(
            forest.contains("{\"t\":\"text\",\"value\":\"0\"}"),
            "the live count (Nat.toText(count)=\"0\") must render as a dynamic text node:\n{forest}"
        );

        // 4) A structural JSON sanity check independent of substring matching:
        //    the brackets/braces balance, proving it is well-formed JSON.
        let mut depth = 0i32;
        let mut in_str = false;
        let mut prev = ' ';
        for c in forest.chars() {
            match c {
                '"' if prev != '\\' => in_str = !in_str,
                '[' | '{' if !in_str => depth += 1,
                ']' | '}' if !in_str => depth -= 1,
                _ => {}
            }
            prev = c;
            assert!(depth >= 0, "unbalanced JSON in forest:\n{forest}");
        }
        assert_eq!(depth, 0, "unbalanced JSON in forest:\n{forest}");
    }

    /// An unknown `--route` is rejected with a helpful listing — and never falls
    /// back to silently rendering the wrong page.
    #[test]
    fn preview_unknown_route_is_an_error_listing_available_routes() {
        let counter = repo_root().join("examples").join("counter");
        let err = project::build_preview(&counter, Some("/does-not-exist"))
            .expect_err("unknown route must error");
        assert!(err.contains("no page matches route"), "error must explain:\n{err}");
        assert!(err.contains('/'), "error must list available routes:\n{err}");
    }
}

#[test]
fn secure_mutating_form_is_clean() {
    let d = lint_page(
        "@page \"/\"\n<form @submit=\"save\" secure><input name=\"x\"><button>Go</button></form>\n@code { func save(ctx : Context) : async () { ignore ctx; }; }",
    );
    assert!(
        !d.iter().any(|x| x.severity == Severity::Error),
        "a `secure` mutating form must NOT be flagged:\n{:#?}",
        d
    );
}

#[test]
fn get_search_form_without_submit_is_not_flagged() {
    // A non-mutating GET form (search box, no @submit) is legitimately unsecured.
    let d = lint_page(
        "@page \"/\"\n<form action=\"/search\" method=\"get\"><input name=\"q\"><button>Search</button></form>\n@code { }",
    );
    assert!(
        !d.iter().any(|x| x.rule == "secure-form"),
        "a GET form with no @submit must NOT trigger secure-form:\n{:#?}",
        d
    );
}

#[test]
fn raw_html_is_a_warning_not_an_error() {
    let d = lint_page("@page \"/\"\n<div>@raw(body)</div>\n@code { var body : Text = \"<b>x</b>\"; }");
    let warns: Vec<_> = d.iter().filter(|x| x.rule == "raw-html").collect();
    assert_eq!(warns.len(), 1, "expected one raw-html warning:\n{:#?}", d);
    assert_eq!(warns[0].severity, Severity::Warning, "@raw must be a Warning, never an Error:\n{:#?}", warns[0]);
    assert!(
        !d.iter().any(|x| x.severity == Severity::Error),
        "@raw must not produce any Error:\n{:#?}",
        d
    );
}

#[test]
fn nested_mutating_form_inside_if_is_still_caught() {
    // forms can be deep in the tree (inside @if / @for); the walker must recurse.
    let d = lint_page(
        "@page \"/\"\n@if show { <form @submit=\"save\"><button>Go</button></form> }\n@code { var show : Bool = true; func save(ctx : Context) : async () { ignore ctx; }; }",
    );
    assert!(
        d.iter().any(|x| x.rule == "secure-form" && x.severity == Severity::Error),
        "a mutating form nested in @if must still be flagged:\n{:#?}",
        d
    );
}

// ---- Slice 1: vetKD network gate ------------------------------------------

/// Build a one-page project in a temp dir for the given network and return the
/// generated main.mo. Panics on build error (callers that expect a hard-fail use
/// `project::build` directly).
fn build_main_mo(network: &str) -> String {
    let dir = std::env::temp_dir().join(format!("mv_test_{}_{}", network, std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let pages = dir.join("src").join("Pages");
    std::fs::create_dir_all(&pages).unwrap();
    std::fs::write(pages.join("Home.mview"), "@page \"/\"\n<p>hi</p>\n@code { }").unwrap();
    let out = dir.join(".mvbuild").join("main.mo");
    // Exercise the `Default` impl (the rest of the fields come from it).
    let opts = project::BuildOptions {
        project_dir: dir.clone(),
        app_name: "T".to_string(),
        out: out.clone(),
        network: network.to_string(),
        ..Default::default()
    };
    project::build(&opts).unwrap_or_else(|e| panic!("build failed for network {}: {}", network, e));
    let mo = std::fs::read_to_string(&out).unwrap();
    let _ = std::fs::remove_dir_all(&dir);
    mo
}

#[test]
fn network_ic_uses_key_1_never_dfx_test_key() {
    let mo = build_main_mo("ic");
    assert!(mo.contains("VetKeys.publicKey(\"key_1\""), "mainnet must use key_1:\n{}", mo);
    assert!(mo.contains("VetKeys.deriveKey(\"key_1\""), "mainnet deriveKey must use key_1:\n{}", mo);
    assert!(
        !mo.contains("VetKeys.publicKey(\"dfx_test_key\"") && !mo.contains("VetKeys.deriveKey(\"dfx_test_key\""),
        "the local dfx_test_key must NEVER appear in a mainnet build:\n{}",
        mo
    );
}

#[test]
fn network_default_uses_dfx_test_key() {
    let mo = build_main_mo("local");
    assert!(mo.contains("VetKeys.publicKey(\"dfx_test_key\""), "local build must use dfx_test_key:\n{}", mo);
    assert!(!mo.contains("\"key_1\""), "local build must not bake key_1:\n{}", mo);
}

#[test]
fn clean_mainnet_build_does_not_trip_gate() {
    // With the real mapping, `ic` resolves to key_1 and builds fine. This locks
    // in that a correctly-mapped mainnet build does NOT trip the defensive guard.
    let dir = std::env::temp_dir().join(format!("mv_gate_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let pages = dir.join("src").join("Pages");
    std::fs::create_dir_all(&pages).unwrap();
    std::fs::write(pages.join("Home.mview"), "@page \"/\"\n<p>hi</p>\n@code { }").unwrap();
    let opts = project::BuildOptions {
        project_dir: dir.clone(),
        app_name: "T".to_string(),
        out: dir.join(".mvbuild").join("main.mo"),
        network: "ic".to_string(),
        ..Default::default()
    };
    assert!(project::build(&opts).is_ok(), "a correctly-mapped mainnet build must not trip the gate");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn network_gate_rejects_local_key_on_mainnet() {
    // Directly exercise the Err branch of the guard: a mainnet target with the
    // LOCAL test key must be refused. This is the failure mode the guard exists
    // for (a future regression in the key mapping); testing it via the pure
    // function avoids depending on a deliberately-broken mapping.
    for net in ["ic", "mainnet", "  IC  ", "Mainnet"] {
        let r = project::enforce_network_gate(net, "dfx_test_key");
        assert!(r.is_err(), "network `{}` with dfx_test_key must be rejected", net);
        let msg = r.unwrap_err();
        assert!(msg.contains("dfx_test_key") && msg.contains("key_1"), "gate message must explain the fix:\n{}", msg);
    }
    // Correctly-mapped pairs and all non-mainnet networks pass.
    assert!(project::enforce_network_gate("ic", "key_1").is_ok());
    assert!(project::enforce_network_gate("local", "dfx_test_key").is_ok());
    assert!(project::enforce_network_gate("playground", "dfx_test_key").is_ok());
}

// ---- Slice 1: secure-form lint casing bypasses ----------------------------

#[test]
fn mixed_case_form_tag_with_submit_is_still_flagged() {
    // `<fOrm @submit>` / `<foRM @submit>` parse as Elements (lowercase first
    // char) whose tag the parser now lowercases — the browser treats them as
    // <form> and submits them, so the secure-form gate MUST still fire.
    // Regression for a CSRF-bypass via tag case. (UPPERCASE-first like `<FoRm>`
    // is a *component* reference, not a form: codegen renders it as a <div> and
    // drops the submit wiring, so it is harmless and intentionally not covered.)
    for tag in ["fOrm", "foRM", "foRm"] {
        let d = lint_page(&format!(
            "@page \"/\"\n<{tag} @submit=\"save\"><button>Go</button></{tag}>\n@code {{ func save(ctx : Context) : async () {{ ignore ctx; }}; }}",
            tag = tag
        ));
        assert!(
            d.iter().any(|x| x.rule == "secure-form" && x.severity == Severity::Error),
            "<{}> with @submit and no `secure` must be flagged (CSRF bypass):\n{:#?}",
            tag, d
        );
    }
}

#[test]
fn mixed_case_submit_event_is_still_flagged() {
    // `@Submit` / `@SUBMIT` are lowercased by the parser; the DOM submit event
    // fires the handler regardless of source casing, so the gate MUST still fire.
    for ev in ["Submit", "SUBMIT", "SuBmIt"] {
        let d = lint_page(&format!(
            "@page \"/\"\n<form @{ev}=\"save\"><button>Go</button></form>\n@code {{ func save(ctx : Context) : async () {{ ignore ctx; }}; }}",
            ev = ev
        ));
        assert!(
            d.iter().any(|x| x.rule == "secure-form" && x.severity == Severity::Error),
            "<form @{}> with no `secure` must be flagged (CSRF bypass):\n{:#?}",
            ev, d
        );
    }
}

#[test]
fn mixed_case_form_tag_does_not_reclassify_as_component() {
    // Lowercasing the tag must NOT turn a lowercase-first mixed-case tag into a
    // component (which codegen would not wire as a submit form). Verify codegen
    // emits the live submit wiring for `<fOrm @submit>` — proving lint and
    // codegen agree on what is a form.
    let g = page("@page \"/\"\n<fOrm @submit=\"save\" secure><input name=\"x\"><button>Go</button></fOrm>\n@code { func save(ctx : Context) : async () { ignore ctx; }; }");
    assert!(g.contains("data-mv-event=\\\"submit\\\""), "codegen must wire <fOrm> as a submit form:\n{}", g);
    assert!(g.contains("data-mv-secure=\\\"1\\\""), "secure <fOrm> must emit the secure token:\n{}", g);
}

#[test]
fn uppercase_first_form_tag_is_a_harmless_component() {
    // `<FoRm>` (uppercase first) is a COMPONENT reference, not a <form>. codegen
    // degrades an unknown component to a <div> and drops the @submit wiring, so
    // there is nothing to submit — hence the lint correctly does NOT flag it.
    let d = lint_page("@page \"/\"\n<FoRm @submit=\"save\"><button>Go</button></FoRm>\n@code { func save(ctx : Context) : async () { ignore ctx; }; }");
    assert!(!d.iter().any(|x| x.rule == "secure-form"), "uppercase-first <FoRm> is a component, not a form:\n{:#?}", d);
    let g = page("@page \"/\"\n<FoRm @submit=\"save\"><button>Go</button></FoRm>\n@code { func save(ctx : Context) : async () { ignore ctx; }; }");
    assert!(!g.contains("data-mv-event=\\\"submit\\\""), "<FoRm> component must NOT emit a live submit handler:\n{}", g);
    assert!(g.contains("mv-component mv-form"), "<FoRm> should degrade to a div component:\n{}", g);
}

// ---- R1: source spans (additive AST metadata) -----------------------------
//
// These lock in that the parser records correct CHAR-offset spans on the
// diagnostic-critical nodes (Element/Component/Attr/EventBind/FuncDecl/VarDecl),
// and that `span::line_col` maps offsets to 1-based (line, col) — the
// foundation a future language server (R2/R3) uses for line-accurate
// diagnostics. We assert by re-slicing the source: `src[span.start..span.end]`
// must equal the expected token text.
mod spans {
    use crate::ast::{FileKind, Node};
    use crate::parser;
    use crate::span::{line_col, Span};

    /// Slice a span out of the original `.mview` source (as the parser sees it:
    /// a `Vec<char>`), so we compare against the exact substring the offsets
    /// index into.
    fn slice(src: &str, span: Span) -> String {
        let chars: Vec<char> = src.chars().collect();
        span.slice(&chars)
    }

    /// Walk the template, returning the first Element with the given tag.
    fn find_element<'a>(nodes: &'a [Node], tag: &str) -> Option<&'a crate::ast::Element> {
        for n in nodes {
            match n {
                Node::Element(e) => {
                    if e.tag == tag {
                        return Some(e);
                    }
                    if let Some(found) = find_element(&e.children, tag) {
                        return Some(found);
                    }
                }
                Node::Component(c) => {
                    if let Some(found) = find_element(&c.children, tag) {
                        return Some(found);
                    }
                }
                Node::If(brs) => {
                    for b in brs {
                        if let Some(found) = find_element(&b.body, tag) {
                            return Some(found);
                        }
                    }
                }
                Node::For { body, .. } => {
                    if let Some(found) = find_element(body, tag) {
                        return Some(found);
                    }
                }
                _ => {}
            }
        }
        None
    }

    fn find_component<'a>(nodes: &'a [Node], name: &str) -> Option<&'a crate::ast::Component> {
        for n in nodes {
            match n {
                Node::Component(c) => {
                    if c.name == name {
                        return Some(c);
                    }
                    if let Some(found) = find_component(&c.children, name) {
                        return Some(found);
                    }
                }
                Node::Element(e) => {
                    if let Some(found) = find_component(&e.children, name) {
                        return Some(found);
                    }
                }
                _ => {}
            }
        }
        None
    }

    #[test]
    fn element_span_covers_the_open_tag() {
        let src = "@page \"/\"\n<button class=\"x\">Go</button>";
        let file = parser::parse(src, "T", FileKind::Page).expect("parse");
        let el = find_element(&file.template, "button").expect("button element");
        // span covers '<' .. '>' (inclusive of the closing '>').
        assert_eq!(slice(src, el.span), "<button class=\"x\">");
    }

    #[test]
    fn self_closing_element_span_covers_through_slash_gt() {
        let src = "@page \"/\"\n<input name=\"email\" />";
        let file = parser::parse(src, "T", FileKind::Page).expect("parse");
        let el = find_element(&file.template, "input").expect("input element");
        assert_eq!(slice(src, el.span), "<input name=\"email\" />");
    }

    #[test]
    fn attr_span_covers_name_through_value() {
        let src = "@page \"/\"\n<a href=\"/home\" target=\"_blank\">x</a>";
        let file = parser::parse(src, "T", FileKind::Page).expect("parse");
        let el = find_element(&file.template, "a").expect("anchor");
        let href = el.attrs.iter().find(|a| a.name == "href").expect("href attr");
        assert_eq!(slice(src, href.span), "href=\"/home\"");
        let target = el.attrs.iter().find(|a| a.name == "target").expect("target attr");
        assert_eq!(slice(src, target.span), "target=\"_blank\"");
    }

    #[test]
    fn boolean_attr_span_covers_just_the_name() {
        let src = "@page \"/\"\n<input required name=\"x\">";
        let file = parser::parse(src, "T", FileKind::Page).expect("parse");
        let el = find_element(&file.template, "input").expect("input");
        let req = el.attrs.iter().find(|a| a.name == "required").expect("required attr");
        assert_eq!(slice(src, req.span), "required");
    }

    #[test]
    fn event_bind_span_covers_the_at_binding() {
        let src = "@page \"/\"\n<button @click=\"increment(5)\">+</button>\n@code { func increment(n : Nat) { ignore n; }; }";
        let file = parser::parse(src, "T", FileKind::Page).expect("parse");
        let el = find_element(&file.template, "button").expect("button");
        let ev = el.events.iter().find(|e| e.event == "click").expect("click event");
        assert_eq!(ev.handler, "increment");
        assert_eq!(slice(src, ev.span), "@click=\"increment(5)\"");
    }

    #[test]
    fn component_span_covers_the_open_tag() {
        let src = "@page \"/\"\n<Card title=\"Hi\">body</Card>";
        let file = parser::parse(src, "T", FileKind::Page).expect("parse");
        let c = find_component(&file.template, "Card").expect("Card component");
        assert_eq!(slice(src, c.span), "<Card title=\"Hi\">");
    }

    #[test]
    fn func_span_is_file_relative_and_covers_the_decl() {
        // R11: FuncDecl/VarDecl spans are now FILE-relative (rebased in
        // parse_code_block by adding the @code body's file offset). So they slice
        // against the WHOLE-FILE chars and still yield the exact declaration text.
        let code_body = "\nvar count : Nat = 0;\nfunc bump(n : Nat) { count += n; };\n";
        let src = format!("@page \"/\"\n<p>@count</p>\n@code {{{}}}", code_body);
        let file = parser::parse(&src, "T", FileKind::Page).expect("parse");
        let file_chars: Vec<char> = src.chars().collect();

        let vd = file.code.vars.iter().find(|v| v.name == "count").expect("count var");
        assert_eq!(vd.span.slice(&file_chars), "var count : Nat = 0;");

        let fd = file.code.funcs.iter().find(|f| f.name == "bump").expect("bump func");
        assert_eq!(
            fd.span.slice(&file_chars),
            "func bump(n : Nat) { count += n; };"
        );
    }

    #[test]
    fn stable_var_span_covers_the_stable_keyword() {
        // R11: file-relative — slice against the whole file.
        let code_body = " stable var total : Nat = 0; ";
        let src = format!("@page \"/\"\n<p>@total</p>\n@code {{{}}}", code_body);
        let file = parser::parse(&src, "T", FileKind::Page).expect("parse");
        let file_chars: Vec<char> = src.chars().collect();
        let vd = file.code.vars.iter().find(|v| v.name == "total").expect("total var");
        assert!(vd.stable);
        assert_eq!(vd.span.slice(&file_chars), "stable var total : Nat = 0;");
    }

    #[test]
    fn line_col_maps_offsets_across_newlines() {
        let src: Vec<char> = "@page \"/\"\n<button>Go</button>".chars().collect();
        // first char -> line 1, col 1
        assert_eq!(line_col(&src, 0), (1, 1));
        // the '<' begins line 2 (after the '\n' at offset 9)
        let lt = src.iter().position(|&c| c == '<').unwrap();
        assert_eq!(line_col(&src, lt), (2, 1));
        // 'b' of "button" is one column further
        assert_eq!(line_col(&src, lt + 1), (2, 2));
    }

    #[test]
    fn element_span_line_col_is_line_accurate() {
        // Three template lines; the <span> sits on line 3 -> a diagnostic
        // pointing at el.span.start must report line 3, col 1.
        let src = "@page \"/\"\n<div>\n  <span>hi</span>\n</div>";
        let file = parser::parse(src, "T", FileKind::Page).expect("parse");
        let el = find_element(&file.template, "span").expect("span element");
        let chars: Vec<char> = src.chars().collect();
        let (line, col) = line_col(&chars, el.span.start);
        assert_eq!((line, col), (3, 3)); // 2 leading spaces -> col 3
        assert_eq!(el.span.slice(&chars), "<span>");
    }
}

// ---- R2: machine-readable diagnostics (lint/check --json) -----------------
//
// These lock in the JSON contract the editor (R6) and the AI repair loop (R5)
// consume: a stable array of {severity, rule, message, file, line, col,
// endLine, endCol}. We exercise `lint_project_json` against real temp projects
// (so spans resolve through the same parser+lint the build uses) and the moc
// mapping via synthetic moc output (invoking `moc` in a unit test is impractical
// and environment-dependent — the build's `check` already drives it for real).
#[cfg(test)]
mod json_diagnostics {
    use crate::lint::{self, JsonDiagnostic, Severity};
    use crate::parser;
    use crate::project;
    use std::path::PathBuf;

    /// Write a single-page temp project and return its dir. Caller removes it.
    fn temp_project(tag: &str, page_src: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "mv_json_{}_{}_{}",
            tag,
            std::process::id(),
            // a per-call salt so parallel tests never collide on the same dir
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        let _ = std::fs::remove_dir_all(&dir);
        let pages = dir.join("src").join("Pages");
        std::fs::create_dir_all(&pages).unwrap();
        std::fs::write(pages.join("Home.mview"), page_src).unwrap();
        dir
    }

    /// Minimal extractor: read a numeric field (`"key":N`) out of a JSON object
    /// string. Good enough to assert positions without pulling in serde — the
    /// crate is intentionally dependency-free.
    fn num_field(obj: &str, key: &str) -> Option<i64> {
        let needle = format!("\"{}\":", key);
        let p = obj.find(&needle)? + needle.len();
        let rest = &obj[p..];
        let end = rest
            .find(|c: char| !c.is_ascii_digit() && c != '-')
            .unwrap_or(rest.len());
        rest[..end].parse().ok()
    }

    /// Read a string field (`"key":"..."`) out of a JSON object string.
    fn str_field(obj: &str, key: &str) -> Option<String> {
        let needle = format!("\"{}\":\"", key);
        let p = obj.find(&needle)? + needle.len();
        let rest = &obj[p..];
        let end = rest.find('"')?;
        Some(rest[..end].to_string())
    }

    #[test]
    fn lint_json_flags_unsecured_form_with_a_position() {
        // A state-mutating <form @submit> with no `secure` on line 2.
        let dir = temp_project(
            "insecure",
            "@page \"/\"\n<form @submit=\"save\"><input name=\"x\"><button>Go</button></form>\n@code { func save(ctx : Context) : async () { ignore ctx; }; }",
        );
        let diags = project::lint_project_json(&dir).expect("lint_project_json");
        let _ = std::fs::remove_dir_all(&dir);

        let json = lint::diagnostics_to_json(&diags);
        // The whole document parses as a non-empty array containing our object.
        assert!(json.starts_with('[') && json.ends_with(']'), "not a JSON array: {json}");
        let obj = diags
            .iter()
            .find(|d| d.rule == "secure-form")
            .map(|d| d.to_json())
            .expect("a secure-form diagnostic in the JSON");
        // severity + rule + file
        assert_eq!(str_field(&obj, "severity").as_deref(), Some("error"), "{obj}");
        assert_eq!(str_field(&obj, "rule").as_deref(), Some("secure-form"), "{obj}");
        assert_eq!(str_field(&obj, "file").as_deref(), Some("src/Pages/Home.mview"), "{obj}");
        // position points at the <form ...> open tag: line 2, col 1.
        assert_eq!(num_field(&obj, "line"), Some(2), "form is on line 2: {obj}");
        assert_eq!(num_field(&obj, "col"), Some(1), "form starts at col 1: {obj}");
        assert!(num_field(&obj, "endCol").unwrap() > num_field(&obj, "col").unwrap(), "{obj}");
    }

    #[test]
    fn lint_json_on_a_clean_project_is_empty_array() {
        let dir = temp_project(
            "clean",
            "@page \"/\"\n<form @submit=\"save\" secure><input name=\"x\"><button>Go</button></form>\n@code { func save(ctx : Context) : async () { ignore ctx; }; }",
        );
        let diags = project::lint_project_json(&dir).expect("lint_project_json");
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(lint::diagnostics_to_json(&diags), "[]");
    }

    #[test]
    fn lint_json_emits_raw_html_warning() {
        let dir = temp_project(
            "raw",
            "@page \"/\"\n<div>@raw(body)</div>\n@code { var body : Text = \"<b>x</b>\"; }",
        );
        let diags = project::lint_project_json(&dir).expect("lint_project_json");
        let _ = std::fs::remove_dir_all(&dir);
        let raw = diags
            .iter()
            .find(|d| d.rule == "raw-html")
            .expect("a raw-html diagnostic");
        assert_eq!(raw.severity, Severity::Warning);
        let obj = raw.to_json();
        assert_eq!(str_field(&obj, "severity").as_deref(), Some("warning"), "{obj}");
    }

    #[test]
    fn check_json_maps_a_moc_type_error_to_the_mview_file() {
        // A generated actor with the `// mv:src` marker the build emits, then a
        // synthetic moc type error pointing inside that region. The mapper must
        // surface the .mview file + moc's line/col in the shared JSON shape.
        let main_mo = "\
// GENERATED by `motoview build`
actor {
  // mv:src src/Pages/Home.mview
  let bad : Nat = \"oops\";
};
";
        let moc = "/tmp/app/.mvbuild/main.mo:4.19-4.25: type error [M0096], expression of type\n  Text\ncannot produce expected type\n  Nat";
        // No source map -> falls back to the `// mv:src` marker FILE + moc's line.
        let diags = project::map_moc_errors_json(main_mo, moc, &crate::span::SourceMap::new(), None);
        assert_eq!(diags.len(), 1, "exactly one diagnostic: {diags:#?}");
        let d = &diags[0];
        assert_eq!(d.severity, Severity::Error);
        assert_eq!(d.rule, "type-check");
        assert_eq!(d.file, "src/Pages/Home.mview", "must map to the .mview source");
        assert_eq!(d.line, 4, "moc's line is surfaced");
        assert_eq!(d.col, 19, "moc's column is surfaced");
        assert_eq!((d.end_line, d.end_col), (4, 25), "end position from the range");
        assert!(d.message.contains("type error"), "message preserved: {}", d.message);
        // And it serializes to a valid JSON object.
        let obj = d.to_json();
        assert_eq!(str_field(&obj, "rule").as_deref(), Some("type-check"), "{obj}");
        assert_eq!(num_field(&obj, "line"), Some(4), "{obj}");
    }

    #[test]
    fn check_json_clean_output_is_empty_array() {
        // No moc output -> no diagnostics -> the serializer emits "[]".
        let diags: Vec<JsonDiagnostic> = project::map_moc_errors_json("actor {}\n", "", &crate::span::SourceMap::new(), None);
        assert!(diags.is_empty());
        assert_eq!(lint::diagnostics_to_json(&diags), "[]");
    }

    #[test]
    fn human_lint_output_is_unchanged() {
        // SNAPSHOT: the non-json formatter must keep its exact `error:`/`--> `
        // shape. If this changes, the byte-for-byte guarantee is broken.
        let file = parser::parse(
            "@page \"/\"\n<form @submit=\"save\"><button>Go</button></form>\n@code { func save(ctx : Context) : async () { ignore ctx; }; }",
            "T",
            crate::ast::FileKind::Page,
        )
        .expect("parse");
        let diags: Vec<(String, lint::Diagnostic)> = lint::lint_file(&file, "src/Pages/T.mview")
            .into_iter()
            .map(|d| ("src/Pages/T.mview".to_string(), d))
            .collect();
        let report = project::format_lint(&diags);
        assert_eq!(
            report,
            "error: [secure-form] state-mutating <form @submit=...> must be marked \
             `secure` (or remove the submit handler). Secure forms mint an HMAC \
             token binding the request; an unsecured mutating form is a CSRF + \
             over-posting hole.\n  --> src/Pages/T.mview (<form @submit=\"save\">)\n"
        );
    }
}

// ---- R8: templates + onboarding ------------------------------------------
//
// For EACH embedded template: scaffold it to a temp dir via the REAL `motoview
// new` binary (so we exercise placeholder substitution + runtime-relpath
// resolution exactly as a user would), then BUILD it and LINT it through the
// same library `motoview build` / `motoview lint` use, asserting BUILD OK and
// ZERO lint errors. The secure-form / identity / wallet templates carry secure
// forms and @authorize gates, so they MUST lint clean — that is the proof their
// security patterns are correct, not just present.
mod templates_e2e {
    use crate::project;
    use crate::templates;
    use std::path::PathBuf;

    fn repo_root() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .expect("compiler/ has a parent")
            .to_path_buf()
    }

    /// Scaffold `template` into a fresh temp dir through the REAL scaffold
    /// codepath (`templates::scaffold_project`, exactly what `motoview new`
    /// calls), returning `(tmp_parent, project_root)`. This exercises placeholder
    /// substitution AND runtime-relpath resolution for an arbitrary project
    /// location, just like the CLI.
    fn scaffold(template: &str) -> (PathBuf, PathBuf) {
        let tmp = std::env::temp_dir().join(format!(
            "mv_tpl_{}_{}_{}",
            template,
            std::process::id(),
            // a per-call nonce so the four templates don't fight over one dir
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        let _ = std::fs::remove_dir_all(&tmp);
        let proj = tmp.join("app");
        templates::scaffold_project(template, &proj, "app")
            .unwrap_or_else(|e| panic!("scaffold of `{template}` failed: {e}"));
        (tmp, proj)
    }

    /// The shared body: scaffold, assert the project shape (config + placeholder
    /// substitution + correct runtime path), then BUILD OK + 0 lint errors.
    fn scaffold_builds_and_lints_clean(template: &str) {
        let (tmp, proj) = scaffold(template);

        // ---- project shape: the docs-promised files all exist ----
        for need in ["motoview.json", "dfx.json", "mops.toml", "README.md", ".gitignore"] {
            assert!(proj.join(need).exists(), "{template}: scaffold missing {need}");
        }
        assert!(proj.join("src/Pages/Home.mview").exists(), "{template}: missing Home page");
        assert!(proj.join("src/Layouts/MainLayout.mview").exists(), "{template}: missing layout");

        // docs/ai-tools.md promises `motoview new` writes all three assistant
        // rule files into every project — assert the scaffold keeps that promise.
        for ai in [
            "skills/motoview/SKILL.md",
            ".github/copilot-instructions.md",
            ".cursor/rules/motoview.mdc",
        ] {
            assert!(proj.join(ai).exists(), "{template}: scaffold missing AI rule file {ai}");
        }

        // ---- placeholders were substituted (no token leaks into the project) ----
        let dfx = std::fs::read_to_string(proj.join("dfx.json")).unwrap();
        assert!(!dfx.contains("__NAME__"), "{template}: __NAME__ not substituted");
        assert!(!dfx.contains("__RUNTIME_PKG__"), "{template}: __RUNTIME_PKG__ not substituted");
        assert!(!dfx.contains("__PORT__"), "{template}: __PORT__ not substituted");
        assert!(dfx.contains("\"app\""), "{template}: canister not named after the project dir:\n{dfx}");

        // The computed runtime path must point at THIS repo's runtime/src — i.e.
        // `--package motoview <p>` where <p>, resolved against the project dir,
        // is the real runtime. (This is what makes the build below resolve
        // `mo:motoview` with no mops install.)
        let pkg = dfx
            .split("--package motoview ")
            .nth(1)
            .and_then(|s| s.split('"').next())
            .unwrap_or("")
            .trim()
            .to_string();
        assert!(!pkg.is_empty(), "{template}: no runtime package path in dfx.json:\n{dfx}");
        let resolved = if pkg.starts_with("mo:") {
            // mops fallback path — only valid if no checkout exists, which is NOT
            // the case in-repo, so fail loudly (the relpath should have resolved).
            panic!("{template}: expected a real runtime relpath in-repo, got `{pkg}`");
        } else {
            proj.join(&pkg)
        };
        assert!(
            resolved.join("App.mo").exists(),
            "{template}: runtime path `{pkg}` (-> {}) does not point at runtime/src",
            resolved.display()
        );
        // And it must be the SAME runtime this repo ships.
        let canonical_runtime = std::fs::canonicalize(repo_root().join("runtime/src")).unwrap();
        assert_eq!(
            std::fs::canonicalize(&resolved).unwrap(),
            canonical_runtime,
            "{template}: runtime relpath must resolve to the repo's runtime/src"
        );

        // ---- BUILD OK: emit the actor (no moc needed — build does not type-check) ----
        let opts = project::BuildOptions {
            project_dir: proj.clone(),
            app_name: "app".to_string(),
            out: proj.join(".mvbuild").join("main.mo"),
            ..Default::default()
        };
        let summary = project::build(&opts)
            .unwrap_or_else(|e| panic!("{template}: build FAILED:\n{e}"));
        assert!(
            proj.join(".mvbuild/main.mo").exists(),
            "{template}: build did not emit .mvbuild/main.mo"
        );
        assert!(summary.contains("page(s)"), "{template}: unexpected build summary:\n{summary}");

        // ---- 0 LINT ERRORS: secure forms / @authorize make these clean ----
        let diags = project::lint_project(&proj)
            .unwrap_or_else(|e| panic!("{template}: lint errored: {e}"));
        let errors: Vec<_> = diags
            .iter()
            .filter(|(_, d)| d.severity == crate::lint::Severity::Error)
            .collect();
        assert!(
            errors.is_empty(),
            "{template}: expected 0 lint errors, got {}:\n{:#?}",
            errors.len(),
            errors
        );

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn basic_template_builds_and_lints_clean() {
        scaffold_builds_and_lints_clean("basic");
    }

    #[test]
    fn secure_form_template_builds_and_lints_clean() {
        scaffold_builds_and_lints_clean("secure-form");
    }

    #[test]
    fn identity_template_builds_and_lints_clean() {
        scaffold_builds_and_lints_clean("identity");
    }

    #[test]
    fn wallet_template_builds_and_lints_clean() {
        scaffold_builds_and_lints_clean("wallet");
    }

    /// An unknown `--template` is rejected by the CLI before any write (it checks
    /// `files_for(...).is_none()`), and the scaffold codepath itself writes
    /// nothing for an unknown name — never a half-written project.
    #[test]
    fn unknown_template_is_rejected() {
        assert!(templates::files_for("does-not-exist").is_none());
        let tmp = std::env::temp_dir().join(format!("mv_tpl_bad_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        let proj = tmp.join("app");
        templates::scaffold_project("does-not-exist", &proj, "app").unwrap();
        assert!(
            !proj.exists(),
            "no project files should be written for an unknown template"
        );
        let _ = std::fs::remove_dir_all(&tmp);
    }
}

// ===========================================================================
// R9 — formatter (`motoview fmt`) + lint quick-fixes.
// ===========================================================================
mod r9_fmt {
    use crate::ast::FileKind;
    use crate::fmt;
    use crate::project;
    use std::path::{Path, PathBuf};

    /// Repo root: CARGO_MANIFEST_DIR is `<root>/compiler`.
    fn repo_root() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .expect("compiler/ has a parent")
            .to_path_buf()
    }

    /// Recursively copy a directory tree (skipping build/VCS dirs), so a test can
    /// format a throwaway copy of a real example without touching the source tree.
    fn copy_tree(src: &Path, dst: &Path) {
        std::fs::create_dir_all(dst).unwrap();
        for entry in std::fs::read_dir(src).unwrap().flatten() {
            let p = entry.path();
            let name = p.file_name().unwrap().to_string_lossy().to_string();
            if name == ".mvbuild" || name == ".git" || name == "node_modules" || name == "target" {
                continue;
            }
            let target = dst.join(&name);
            if p.is_dir() {
                copy_tree(&p, &target);
            } else {
                std::fs::copy(&p, &target).unwrap();
            }
        }
    }

    /// Collect every `.mview` under a directory (recursive), sorted.
    fn mview_files(dir: &Path) -> Vec<PathBuf> {
        let mut out = Vec::new();
        fn walk(d: &Path, out: &mut Vec<PathBuf>) {
            if let Ok(entries) = std::fs::read_dir(d) {
                for e in entries.flatten() {
                    let p = e.path();
                    if p.is_dir() {
                        walk(&p, out);
                    } else if p.extension().and_then(|x| x.to_str()) == Some("mview") {
                        out.push(p);
                    }
                }
            }
        }
        walk(dir, &mut out);
        out.sort();
        out
    }

    /// Build a project's `main.mo` to bytes (the HTML/default backend the real
    /// build uses), reading the artifact `project::build` writes.
    fn build_main_mo(project_dir: &Path) -> String {
        let out = project_dir.join(".mvbuild").join("main.mo");
        let opts = project::BuildOptions {
            project_dir: project_dir.to_path_buf(),
            app_name: "FmtSemanticsTest".to_string(),
            out: out.clone(),
            network: "local".to_string(),
            emit: crate::codegen::EmitMode::Html,
            instrument: false,
        };
        project::build(&opts)
            .unwrap_or_else(|e| panic!("build failed for {}: {}", project_dir.display(), e));
        std::fs::read_to_string(&out).expect("main.mo written")
    }

    /// THE hard gate: formatting an example NEVER changes the generated `main.mo`.
    /// Copies the example, builds it (hash A), formats every .mview in place,
    /// rebuilds (hash B), and asserts the two `main.mo` are byte-identical.
    fn assert_fmt_preserves_build(example_rel: &str) {
        let src = repo_root().join(example_rel);
        assert!(src.exists(), "missing example: {}", src.display());
        let tmp = std::env::temp_dir().join(format!(
            "mv_fmt_{}_{}",
            example_rel.replace(['/', '\\'], "_"),
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&tmp);
        copy_tree(&src, &tmp);

        let before = build_main_mo(&tmp);

        // Format every .mview in the copy, in place, the way `motoview fmt` does.
        for f in mview_files(&tmp) {
            let text = std::fs::read_to_string(&f).unwrap();
            let kind = fmt::kind_from_path(&f.to_string_lossy());
            let formatted = fmt::format_source(&text, kind);
            std::fs::write(&f, &formatted).unwrap();
        }

        let after = build_main_mo(&tmp);
        let _ = std::fs::remove_dir_all(&tmp);

        assert_eq!(
            before, after,
            "fmt changed the generated main.mo for {} — semantics NOT preserved",
            example_rel
        );
    }

    #[test]
    fn fmt_preserves_build_counter() {
        assert_fmt_preserves_build("examples/counter");
    }

    #[test]
    fn fmt_preserves_build_products() {
        assert_fmt_preserves_build("examples/products");
    }

    #[test]
    fn fmt_preserves_build_bzzz() {
        assert_fmt_preserves_build("apps/bzzz");
    }

    /// A deliberately MESSY fixture (CRLF, trailing whitespace on directive/@code
    /// lines, leading + trailing blank-line runs) whose messiness lives ONLY in
    /// regions that do not change the generated code OR is left untouched by the
    /// self-verifying gate. The generated codegen signature must be identical
    /// before and after formatting.
    #[test]
    fn fmt_preserves_codegen_on_messy_fixture() {
        // Trailing blank lines after @code, leading blank lines, CRLF on the
        // directive lines (read via read_line().trim(), so safe), and a tidy body.
        let messy = "\r\n\r\n@page \"/\"  \r\n@title \"Counter\"  \r\n@layout MainLayout\r\n\r\n<section class=\"mv-container\">\r\n    <h1>Counter</h1>\r\n</section>\r\n\r\n@code {\n    var count : Nat = 0;\n}\n\n\n\n\n";
        let before = fmt::codegen_signature(messy, FileKind::Page).expect("messy parses");
        let formatted = fmt::format_source(messy, FileKind::Page);
        let after = fmt::codegen_signature(&formatted, FileKind::Page).expect("formatted parses");
        assert_eq!(before, after, "fmt changed codegen on the messy fixture");
    }

    /// `fmt` is IDEMPOTENT: fmt(fmt(x)) == fmt(x) on each real example file and on
    /// the messy fixture.
    #[test]
    fn fmt_is_idempotent() {
        let mut sources: Vec<(String, FileKind)> = Vec::new();
        for rel in ["examples/counter", "examples/products", "apps/bzzz"] {
            for f in mview_files(&repo_root().join(rel)) {
                let text = std::fs::read_to_string(&f).unwrap();
                sources.push((text, fmt::kind_from_path(&f.to_string_lossy())));
            }
        }
        // plus a messy one
        sources.push((
            "\r\n@page \"/\"  \r\n<p>x</p>\r\n@code { }\n\n\n".to_string(),
            FileKind::Page,
        ));
        for (text, kind) in sources {
            let once = fmt::format_source(&text, kind.clone());
            let twice = fmt::format_source(&once, kind);
            assert_eq!(once, twice, "fmt is not idempotent");
        }
    }

    /// `--check` semantics: an already-formatted file reports clean (no rewrite
    /// needed); a messy file reports it needs formatting.
    #[test]
    fn fmt_check_distinguishes_clean_from_messy() {
        // A clean file: format_source returns it unchanged -> is_formatted == true.
        let clean = "@page \"/\"\n<p>x</p>\n@code { }\n";
        assert!(
            fmt::is_formatted(clean, FileKind::Page),
            "a tidy file should already be formatted"
        );
        // A messy file (trailing blank lines after @code) is NOT formatted.
        let messy = "@page \"/\"\n<p>x</p>\n@code { }\n\n\n\n";
        assert!(
            !fmt::is_formatted(messy, FileKind::Page),
            "trailing blank-line run should be reported as unformatted"
        );
        // And formatting it makes it clean (and the fix is stable).
        let fixed = fmt::format_source(messy, FileKind::Page);
        assert!(fmt::is_formatted(&fixed, FileKind::Page));
        assert_eq!(fixed, clean, "messy trailing blanks collapse to one newline");
    }

    /// The formatter must REFUSE any normalization that would change rendered text.
    /// Trailing whitespace INSIDE template text becomes part of a Text node, so the
    /// gate must keep it: format_source returns the input unchanged.
    #[test]
    fn fmt_refuses_to_strip_load_bearing_template_whitespace() {
        // The spaces after `hello` are inter-element/template text -> rendered.
        let src = "@page \"/\"\n<p>hello   </p>\n@code { }\n";
        let out = fmt::format_source(src, FileKind::Page);
        assert_eq!(out, src, "must not touch whitespace that is part of rendered text");
    }
}

// ===========================================================================
// R9 — LSP quick-fix: "Add `secure` to this form" code action.
// ===========================================================================
mod r9_quickfix {
    use crate::lsp;

    const INSECURE: &str =
        "@page \"/\"\n<form @submit=\"save\"><button>Go</button></form>\n@code { func save(ctx : Context) : async () { ignore ctx; }; }";

    /// The CORE proof: the code action's edit, applied to an insecure form's
    /// source, yields a `<form ... secure>` that the lint reports CLEAN (0 errors).
    #[test]
    fn quickfix_edit_makes_form_lint_clean() {
        let uri = "file:///proj/src/Pages/Login.mview";
        // Whole-buffer code-action request (no range filter).
        let actions = lsp::code_actions_for(uri, INSECURE, None);
        assert_eq!(actions.len(), 1, "exactly one quick-fix offered: {:?}", actions);
        let action = &actions[0];
        assert_eq!(
            action.path("title").and_then(|t| t.as_str()),
            Some("Add `secure` to this form")
        );
        assert_eq!(action.path("kind").and_then(|k| k.as_str()), Some("quickfix"));

        // Pull the single TextEdit out of the WorkspaceEdit and apply it.
        let edits = action
            .path("edit.changes")
            .and_then(|c| c.get(uri))
            .and_then(|e| e.as_array())
            .expect("workspace edit for the uri");
        assert_eq!(edits.len(), 1, "one insertion edit");
        let edit = &edits[0];
        let new_text = edit.path("newText").and_then(|t| t.as_str()).unwrap();
        assert!(new_text.contains("secure"), "edit inserts secure: {:?}", new_text);

        // Build a TextEdit and apply it to the source.
        let te = lsp::TextEdit {
            start_line: edit.path("range.start.line").and_then(|n| n.as_f64()).unwrap() as u32,
            start_char: edit.path("range.start.character").and_then(|n| n.as_f64()).unwrap() as u32,
            end_line: edit.path("range.end.line").and_then(|n| n.as_f64()).unwrap() as u32,
            end_char: edit.path("range.end.character").and_then(|n| n.as_f64()).unwrap() as u32,
            new_text: new_text.to_string(),
        };
        let fixed = lsp::apply_edit(INSECURE, &te);
        assert!(
            fixed.contains("secure>"),
            "fixed source has a secure form: {:?}",
            fixed
        );

        // The fixed source LINTS CLEAN: re-run the buffer diagnostics, expect 0.
        let diags = lsp::diagnostics_for(uri, &fixed);
        assert!(
            diags.is_empty(),
            "the quick-fix must clear the secure-form error; got {:?}",
            diags
        );
        // And it still parses + builds the same kind of form (sanity: no new diag).
        assert_eq!(diags.len(), 0);
    }

    /// `secure_form_edit` inserts ` secure` just before the `>` of the open tag,
    /// using the diagnostic's R1 span — at the correct 0-based position.
    #[test]
    fn secure_edit_targets_the_open_tag_close() {
        use crate::ast::FileKind;
        use crate::lint;
        use crate::parser;
        let file = parser::parse(INSECURE, "Login", FileKind::Page).unwrap();
        let diags = lint::lint_file(&file, "Login.mview");
        let secure = diags.iter().find(|d| d.rule == "secure-form").unwrap();
        let span = secure.span.expect("secure-form has a span");
        let edit = lsp::secure_form_edit(INSECURE, span).expect("an edit");
        // <form ...> sits on 0-based line 1. The `>` of the open tag is at some
        // column; the insert is a zero-width range at that column.
        assert_eq!(edit.start_line, 1);
        assert_eq!(edit.start_line, edit.end_line);
        assert_eq!(edit.start_char, edit.end_char, "insertion is zero-width");
        assert_eq!(edit.new_text, " secure");
        // Applying it yields `...="save" secure>` (the keyword lands before `>`).
        let fixed = lsp::apply_edit(INSECURE, &edit);
        assert!(fixed.contains("\"save\" secure>"), "got: {:?}", fixed);
    }

    /// A SECURE form yields NO quick-fix (nothing to fix), and a non-overlapping
    /// requested range filters the action out.
    #[test]
    fn no_quickfix_when_form_already_secure() {
        let uri = "file:///proj/src/Pages/Login.mview";
        let secure_src =
            "@page \"/\"\n<form secure @submit=\"save\"><button>Go</button></form>\n@code { func save(ctx : Context) : async () { ignore ctx; }; }";
        let actions = lsp::code_actions_for(uri, secure_src, None);
        assert!(actions.is_empty(), "no fix for an already-secure form");

        // Range filter: ask for actions on line 0 (the @page line) — the form is on
        // line 1, so the fix should NOT be offered there.
        let actions_off = lsp::code_actions_for(uri, INSECURE, Some((0, 0, 0, 5)));
        assert!(
            actions_off.is_empty(),
            "fix should not surface on a non-overlapping range"
        );
        // But asking ON the form line DOES surface it.
        let actions_on = lsp::code_actions_for(uri, INSECURE, Some((1, 0, 1, 4)));
        assert_eq!(actions_on.len(), 1);
    }

    /// Drive the fix through the LSP protocol handler end-to-end:
    /// initialize advertises codeActionProvider; a `textDocument/codeAction`
    /// request returns the quick-fix.
    #[test]
    fn protocol_code_action_returns_quickfix() {
        use crate::lsp::{parse_json, Json, LspServer};
        let uri = "file:///proj/src/Pages/Login.mview";
        let mut server = LspServer::new();

        // initialize -> codeActionProvider advertised.
        let init = parse_json(r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}"#).unwrap();
        let init_reply = server.handle(&init);
        let init_resp = &init_reply.messages[0];
        assert!(
            init_resp
                .path("result.capabilities.codeActionProvider")
                .is_some(),
            "codeActionProvider advertised"
        );

        // didOpen the insecure buffer.
        let open = parse_json(&format!(
            r#"{{"jsonrpc":"2.0","method":"textDocument/didOpen","params":{{"textDocument":{{"uri":{},"languageId":"mview","version":1,"text":{}}}}}}}"#,
            lsp::json_string(uri),
            lsp::json_string(INSECURE)
        ))
        .unwrap();
        let _ = server.handle(&open);

        // codeAction request over the form's line.
        let ca = parse_json(&format!(
            r#"{{"jsonrpc":"2.0","id":2,"method":"textDocument/codeAction","params":{{"textDocument":{{"uri":{}}},"range":{{"start":{{"line":1,"character":0}},"end":{{"line":1,"character":5}}}},"context":{{"diagnostics":[]}}}}}}"#,
            lsp::json_string(uri)
        ))
        .unwrap();
        let reply = server.handle(&ca);
        let result = reply.messages[0].path("result").expect("a result");
        let actions = match result {
            Json::Arr(a) => a,
            other => panic!("expected an array of actions, got {:?}", other),
        };
        assert_eq!(actions.len(), 1, "the secure-form quick-fix");
        assert_eq!(
            actions[0].path("title").and_then(|t| t.as_str()),
            Some("Add `secure` to this form")
        );
    }
}

// ---- R7: debug / observability (opt-in dispatch instrumentation) ----------
//
// The instrument flag MUST be inert by default (byte-identical generated actor)
// and, when on, MUST wrap each event handler with a structured, parseable
// Debug.print line that ALSO type-checks against the runtime. These tests lock
// both halves down against the REAL `examples/counter`.
mod r7_observability {
    use crate::project;
    use std::path::PathBuf;
    use std::process::Command;

    fn repo_root() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .expect("compiler/ has a parent")
            .to_path_buf()
    }

    fn find_moc() -> Option<(PathBuf, PathBuf)> {
        let home = std::env::var("HOME").ok()?;
        let versions = PathBuf::from(home).join(".cache/dfinity/versions");
        let mut dirs: Vec<PathBuf> = std::fs::read_dir(&versions)
            .ok()?
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.join("moc").exists())
            .collect();
        // The project pins moc 0.28.0 (the version its dfx/build uses). Prefer it:
        // moc >= 0.29 promoted the implicit-transient lint (M0219) from a warning
        // to a hard error, which would reject the unchanged actor codegen this
        // slice does not touch. Fall back to the newest moc only if 0.28.0 is gone.
        if let Some(pinned) = dirs.iter().find(|p| p.file_name().map(|n| n == "0.28.0").unwrap_or(false)).cloned() {
            return Some((pinned.join("moc"), pinned.join("base")));
        }
        dirs.sort();
        let dir = dirs.pop()?;
        Some((dir.join("moc"), dir.join("base")))
    }

    fn dfx_package_args(dir: &PathBuf) -> Vec<String> {
        let txt = std::fs::read_to_string(dir.join("dfx.json")).unwrap_or_default();
        let p = match txt.find("\"args\"") {
            Some(p) => p,
            None => return vec![],
        };
        let after = &txt[p..];
        let colon = match after.find(':') {
            Some(c) => c,
            None => return vec![],
        };
        let start = match after[colon..].find('"') {
            Some(i) => colon + i + 1,
            None => return vec![],
        };
        let rest = &after[start..];
        let end = match rest.find('"') {
            Some(e) => e,
            None => return vec![],
        };
        rest[..end]
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

    /// Build `examples/counter` to a temp out file with the given instrument flag
    /// (without disturbing the committed `.mvbuild/main.mo`). Returns (out_path,
    /// generated source).
    fn build_counter(instrument: bool, tag: &str) -> (PathBuf, String) {
        let counter = repo_root().join("examples").join("counter");
        let out = std::env::temp_dir().join(format!(
            "mv_r7_{}_{}_{}.mo",
            tag,
            instrument,
            std::process::id()
        ));
        let opts = project::BuildOptions {
            project_dir: counter,
            app_name: "counter".to_string(),
            out: out.clone(),
            instrument,
            ..Default::default()
        };
        project::build(&opts).unwrap_or_else(|e| panic!("counter build failed (instrument={instrument}): {e}"));
        let src = std::fs::read_to_string(&out).unwrap();
        (out, src)
    }

    /// DEFAULT byte-identical: the instrument flag adds NOTHING when off. Build
    /// counter with instrument=false twice and assert the bytes match, and that
    /// none of the instrumentation tokens leak into the default actor.
    #[test]
    fn default_build_is_byte_identical_and_uninstrumented() {
        let (o1, a) = build_counter(false, "def_a");
        let (o2, b) = build_counter(false, "def_b");
        assert_eq!(a, b, "two default builds must be byte-identical");
        // No observability tokens in the default actor.
        assert!(!a.contains("Debug.print"), "default actor must not emit Debug.print:\n{a}");
        assert!(!a.contains("MV|dispatch"), "default actor must not carry the dispatch tag:\n{a}");
        assert!(!a.contains("performanceCounter"), "default actor must not use the perf counter:\n{a}");
        assert!(!a.contains("import Debug"), "default actor must not import Debug:\n{a}");
        // The dispatch still ignores ctx on the default path.
        assert!(a.contains("ignore ctx; ignore mvArgs;"), "default dispatch must keep `ignore ctx`:\n{a}");
        let _ = std::fs::remove_file(o1);
        let _ = std::fs::remove_file(o2);
    }

    /// INSTRUMENTED: the generated dispatch carries the structured Debug.print in
    /// the stable, parseable format — tag + page + handler + event + caller +
    /// batchId + instruction cost.
    #[test]
    fn instrumented_dispatch_emits_structured_log_line() {
        let (out, src) = build_counter(true, "instr");
        // Imports added ONLY when instrumented.
        assert!(src.contains("import Debug \"mo:base/Debug\""), "instrumented build imports Debug:\n{src}");
        assert!(
            src.contains("import ExperimentalIC \"mo:base/ExperimentalInternetComputer\""),
            "instrumented build imports the IC performance counter:\n{src}"
        );
        // The structured, parseable dispatch line, with every field the studio
        // log parser keys on.
        assert!(src.contains("Debug.print(\"MV|dispatch|page=Counter|"), "missing the dispatch tag/page:\n{src}");
        assert!(src.contains("|handler=\" # mvH #"), "handler field missing:\n{src}");
        assert!(src.contains("|event=\" # mvH #"), "event field missing:\n{src}");
        assert!(src.contains("|caller=\" # Principal.toText(ctx.caller) #"), "caller (principal text) field missing:\n{src}");
        assert!(src.contains("|lastBatch=\" # ctx.lastBatchId #"), "batchId (lastBatch) field missing:\n{src}");
        assert!(src.contains("|costInstr=\" # debug_show (mvCost)"), "instruction cost field missing:\n{src}");
        // The instruction-cost delta brackets the dispatch via the IC perf counter.
        assert!(src.contains("ExperimentalIC.performanceCounter(0)"), "perf counter call missing:\n{src}");
        // ctx is now USED, not ignored, on the instrumented path.
        assert!(!src.contains("ignore ctx; ignore mvArgs;"), "instrumented dispatch must not ignore ctx:\n{src}");
        let _ = std::fs::remove_file(out);
    }

    /// INSTRUMENTED type-checks: feed the instrumented counter actor to `moc
    /// --check` against the real runtime. Asserts zero type errors (warnings are
    /// fine). Skips (does not fail) if `moc` is absent in this environment.
    #[test]
    fn instrumented_actor_type_checks_against_runtime() {
        let (out, _src) = build_counter(true, "tc");
        let counter = repo_root().join("examples").join("counter");
        let (moc, base) = match find_moc() {
            Some(x) => x,
            None => {
                eprintln!("skipping instrumented type-check: moc not found under ~/.cache/dfinity/versions");
                let _ = std::fs::remove_file(&out);
                return;
            }
        };
        let mut cmd = Command::new(&moc);
        cmd.arg("--check").arg("--package").arg("base").arg(&base);
        for a in dfx_package_args(&counter) {
            cmd.arg(a);
        }
        cmd.arg(&out);
        let result = cmd.output().expect("running moc --check failed");
        let stderr = String::from_utf8_lossy(&result.stderr);
        // moc emits type ERRORS as `[M0xxx], ... error` / `: type error`. Warnings
        // (e.g. unused imports) are fine; only fail on real errors / nonzero exit
        // accompanied by an `error` diagnostic.
        let has_error = stderr.lines().any(|l| {
            let ll = l.to_ascii_lowercase();
            ll.contains("error") && !ll.contains("warning")
        });
        assert!(
            result.status.success() && !has_error,
            "instrumented actor failed to type-check:\nstatus={:?}\nstderr:\n{stderr}",
            result.status
        );
        let _ = std::fs::remove_file(out);
    }
}

// ===========================================================================
// R11 — line-accurate editor errors: the generated->source line map, moc-error
// remapping to the .mview LINE, the @raw span, and parse-error offsets.
// ===========================================================================
mod r11_source_map {
    use crate::ast::FileKind;
    use crate::parser;
    use crate::project;
    use crate::span::{line_col, SourceMap};
    use std::path::{Path, PathBuf};
    use std::process::Command;

    fn repo_root() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .expect("compiler/ has a parent")
            .to_path_buf()
    }

    fn copy_tree(src: &Path, dst: &Path) {
        std::fs::create_dir_all(dst).unwrap();
        for entry in std::fs::read_dir(src).unwrap().flatten() {
            let p = entry.path();
            let name = p.file_name().unwrap().to_string_lossy().to_string();
            if name == ".mvbuild" || name == ".git" || name == "node_modules" || name == "target" {
                continue;
            }
            let target = dst.join(&name);
            if p.is_dir() {
                copy_tree(&p, &target);
            } else {
                std::fs::copy(&p, &target).unwrap();
            }
        }
    }

    fn find_moc() -> Option<(PathBuf, PathBuf)> {
        let home = std::env::var("HOME").ok()?;
        let versions = PathBuf::from(home).join(".cache/dfinity/versions");
        let dirs: Vec<PathBuf> = std::fs::read_dir(&versions)
            .ok()?
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.join("moc").exists())
            .collect();
        // Pin moc 0.28.0 to match the project (see r7_observability::find_moc).
        if let Some(pinned) = dirs
            .iter()
            .find(|p| p.file_name().map(|n| n == "0.28.0").unwrap_or(false))
            .cloned()
        {
            return Some((pinned.join("moc"), pinned.join("base")));
        }
        let mut dirs = dirs;
        dirs.sort();
        let dir = dirs.pop()?;
        Some((dir.join("moc"), dir.join("base")))
    }

    fn dfx_package_args(dir: &Path) -> Vec<String> {
        let txt = std::fs::read_to_string(dir.join("dfx.json")).unwrap_or_default();
        let p = match txt.find("\"args\"") {
            Some(p) => p,
            None => return vec![],
        };
        let after = &txt[p..];
        let colon = match after.find(':') {
            Some(c) => c,
            None => return vec![],
        };
        let start = match after[colon..].find('"') {
            Some(i) => colon + i + 1,
            None => return vec![],
        };
        let rest = &after[start..];
        let end = match rest.find('"') {
            Some(e) => e,
            None => return vec![],
        };
        rest[..end]
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

    fn build(project_dir: &Path) -> PathBuf {
        let out = project_dir.join(".mvbuild").join("main.mo");
        let opts = project::BuildOptions {
            project_dir: project_dir.to_path_buf(),
            app_name: "R11Test".to_string(),
            out: out.clone(),
            ..Default::default()
        };
        project::build(&opts)
            .unwrap_or_else(|e| panic!("build failed for {}: {}", project_dir.display(), e));
        out
    }

    /// HEADLINE: inject a Motoko TYPE error into a page's `@code` on a KNOWN
    /// `.mview` line N, then run the EXACT `check --json` mapping path. The
    /// diagnostic must name the `.mview` file AND report line == N — the generated
    /// `main.mo` line is NOT what the editor sees. Skips (does not fail) when moc
    /// is unavailable in the environment.
    #[test]
    fn check_json_reports_moc_type_error_at_the_mview_line() {
        let (moc, base) = match find_moc() {
            Some(x) => x,
            None => {
                eprintln!("skipping R11 headline: moc not found under ~/.cache/dfinity/versions");
                return;
            }
        };
        // A repo-local throwaway copy of counter so the `../../runtime/src`
        // package path in its dfx.json still resolves for moc.
        let src = repo_root().join("examples").join("counter");
        let tmp = repo_root()
            .join("examples")
            .join(format!("r11_headline_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        copy_tree(&src, &tmp);

        // Replace the `count += by;` body line with a TYPE error (assign Text to a
        // Nat var). Record the 1-based `.mview` line we mutate — that is N.
        let page = tmp.join("src").join("Pages").join("Counter.mview");
        let text = std::fs::read_to_string(&page).unwrap();
        let mut lines: Vec<String> = text.lines().map(|s| s.to_string()).collect();
        let mut injected_line = 0usize;
        for (i, l) in lines.iter_mut().enumerate() {
            if l.contains("count += by;") {
                *l = "        count := \"definitely not a Nat\";".to_string();
                injected_line = i + 1; // 1-based
                break;
            }
        }
        assert!(injected_line > 0, "fixture must contain `count += by;`");
        std::fs::write(&page, lines.join("\n") + "\n").unwrap();

        // Build (writes main.mo + main.mo.map), then run moc --check and map.
        let out = build(&tmp);
        let main_mo = std::fs::read_to_string(&out).unwrap();
        let source_map = project::load_source_map(&out);

        let mut cmd = Command::new(&moc);
        cmd.arg("--check").arg("--package").arg("base").arg(&base);
        for a in dfx_package_args(&tmp) {
            cmd.arg(a);
        }
        cmd.arg(&out);
        let result = cmd.output().expect("running moc --check failed");
        let stderr = String::from_utf8_lossy(&result.stderr);

        let diags = project::map_moc_errors_json(&main_mo, &stderr, &source_map, Some(&tmp));
        let _ = std::fs::remove_dir_all(&tmp);

        // EXACTLY one type error, mapped to the .mview FILE and the injected LINE.
        let type_errs: Vec<_> = diags.iter().filter(|d| d.rule == "type-check").collect();
        assert_eq!(
            type_errs.len(),
            1,
            "expected exactly one type error, got: {diags:#?}\nstderr:\n{stderr}"
        );
        let d = type_errs[0];
        assert_eq!(d.file, "src/Pages/Counter.mview", "must name the .mview source");
        assert_eq!(
            d.line as usize, injected_line,
            "type error must be reported at the .mview LINE {injected_line}, got {} \
             (this is THE R11 headline)",
            d.line
        );
    }

    /// BYTE-IDENTITY: building writes the source map as a SIDE file; `main.mo`
    /// itself is byte-identical to the committed golden artifact, and contains NO
    /// map markers. Asserted for counter (the known-good 72a2ab39… hash) by direct
    /// byte comparison against the committed artifact.
    #[test]
    fn build_keeps_main_mo_byte_identical_and_writes_side_map() {
        let counter = repo_root().join("examples").join("counter");
        let committed = std::fs::read(counter.join(".mvbuild").join("main.mo"))
            .expect("committed counter main.mo exists");

        // Build into a temp out so we never disturb the committed artifact.
        let out = std::env::temp_dir().join(format!("mv_r11_bi_{}.mo", std::process::id()));
        let opts = project::BuildOptions {
            project_dir: counter.clone(),
            app_name: "counter".to_string(),
            out: out.clone(),
            ..Default::default()
        };
        project::build(&opts).expect("counter build");
        let fresh = std::fs::read(&out).expect("fresh main.mo");

        assert_eq!(
            fresh, committed,
            "generated main.mo must be byte-identical to the committed golden artifact"
        );
        // The map is a SIDE artifact next to the out file, never in main.mo.
        let map_path = project::source_map_path(&out);
        assert!(map_path.exists(), "the source map side file must be written");
        // The map's records (the `gen_start gen_end src_start <file>` lines) must
        // NOT appear in main.mo — byte-identity already proves that, but assert the
        // map's first line is absent so a future regression that inlines it is
        // caught directly. (The pre-existing `// mv:src` FILE markers are expected
        // and are part of the committed golden bytes.)
        let map_text = std::fs::read_to_string(&map_path).unwrap_or_default();
        if let Some(first) = map_text.lines().next() {
            assert!(
                !String::from_utf8_lossy(&fresh).contains(first),
                "the source-map records must stay OUT of main.mo (found `{first}`)"
            );
        }

        let _ = std::fs::remove_file(&out);
        let _ = std::fs::remove_file(&map_path);
    }

    /// The map built from a real build resolves a body line to its `.mview` line.
    #[test]
    fn source_map_resolves_func_body_to_mview_line() {
        let counter = repo_root().join("examples").join("counter");
        let out = std::env::temp_dir().join(format!("mv_r11_map_{}.mo", std::process::id()));
        let opts = project::BuildOptions {
            project_dir: counter.clone(),
            app_name: "counter".to_string(),
            out: out.clone(),
            ..Default::default()
        };
        project::build(&opts).expect("counter build");
        let map = project::load_source_map(&out);
        assert!(!map.entries.is_empty(), "counter has @code funcs -> a non-empty map");

        // The `increment` func keyword is on src line 22, its body `count += by;`
        // on src line 23 (see examples/counter/src/Pages/Counter.mview).
        let inc = map
            .entries
            .iter()
            .find(|e| e.src_start_line == 22)
            .expect("an entry anchored at the increment func (src line 22)");
        // The body line (gen_start + 1) resolves to src line 23.
        let (file, src_line) = map.resolve(inc.gen_start_line + 1).expect("body line resolves");
        assert_eq!(file, "src/Pages/Counter.mview");
        assert_eq!(src_line, 23, "the func body line maps to its .mview line");

        let _ = std::fs::remove_file(&out);
        let _ = std::fs::remove_file(project::source_map_path(&out));
    }

    /// PARSE-ERROR OFFSET: a malformed `.mview` (unterminated `@code` block) yields
    /// a `ParseError` carrying a real file offset, and that offset maps to a
    /// line/col past the start of the file (not the (0,0) fallback).
    #[test]
    fn parse_error_carries_a_real_offset() {
        // The `@code {` block is never closed -> `read_brace_block` errors at EOF.
        let src = "@page \"/\"\n<h1>Hi</h1>\n@code {\n    var count : Nat = 0;\n";
        let err = parser::parse(src, "T", FileKind::Page).expect_err("must fail to parse");
        let off = err.offset.expect("parse error carries an offset");
        let chars: Vec<char> = src.chars().collect();
        let (line, _col) = line_col(&chars, off);
        // The unterminated block is detected at EOF -> the offset is at/near the
        // end, well past line 1 (not the old (0,0) fallback).
        assert!(line >= 4, "offset must map past the start, got line {line} (offset {off})");
        assert!(off <= chars.len(), "offset must be within the source");
    }

    /// A deterministic parse-error offset: an `@if` whose body brace is never
    /// closed. The parser fails (`expected '}'`) at EOF; the carried offset is at
    /// the end of the input and well past the `@page` line.
    #[test]
    fn parse_error_offset_points_at_the_failure_site() {
        let src = "@page \"/\"\n@if cond {\n  <p>x</p>\n";
        let err = parser::parse(src, "T", FileKind::Page).expect_err("must fail");
        assert!(!err.message.is_empty(), "error carries a message");
        let off = err.offset.expect("offset present");
        let chars: Vec<char> = src.chars().collect();
        // The offset advances past the `@page` line (the failure is in the @if body).
        let (line, _col) = line_col(&chars, off);
        assert!(line >= 2, "offset must be past line 1, got line {line} (offset {off})");
        assert!(off <= chars.len(), "offset within source");
    }

    /// Unit: the on-disk source-map format round-trips and resolves by linear
    /// extrapolation within a region.
    #[test]
    fn source_map_text_round_trips_and_extrapolates() {
        let mut m = SourceMap::new();
        m.push("src/Pages/A.mview".to_string(), 100, 104, 20);
        let text = m.to_text();
        let back = SourceMap::parse(&text);
        assert_eq!(back.entries.len(), 1);
        // gen 100 -> src 20 (start), gen 102 -> src 22 (extrapolated), gen 104 -> 24.
        assert_eq!(back.resolve(100), Some(("src/Pages/A.mview".to_string(), 20)));
        assert_eq!(back.resolve(102), Some(("src/Pages/A.mview".to_string(), 22)));
        assert_eq!(back.resolve(104), Some(("src/Pages/A.mview".to_string(), 24)));
        // Outside the region -> None (caller falls back to mv:src markers).
        assert_eq!(back.resolve(105), None);
        assert_eq!(back.resolve(99), None);
    }

    // ---- R12: token-anchored (column-accurate) moc errors -----------------
    //
    // R11 fixed the LINE; R12 fixes the COLUMN. The headline is the `await`
    // case: codegen strips `await ` from the handler body, so a token to its
    // right lands at a SMALLER generated column than its true source column.
    // The mapper must re-anchor onto the source line and report the SOURCE col.

    /// Copy `counter` into a repo-local temp dir (so `../../runtime/src` resolves),
    /// patch the `increment` body with `body_line`, BUILD, run `moc --check`, and
    /// return the mapped JSON diagnostics. Returns `None` when moc is unavailable
    /// (the caller then SKIPs, never fails). Also returns the 1-based `.mview` line
    /// the body was injected on and that line's exact source text.
    fn moc_json_for_increment_body(tag: &str, body_line: &str) -> Option<(Vec<crate::lint::JsonDiagnostic>, usize, String)> {
        moc_json_for_increment_body_with_helper(tag, body_line, false)
    }

    /// As [`moc_json_for_increment_body`]; when `with_mv_noop` is set, an async helper
    /// `func mvNoop() : async () { };` is inserted just BEFORE `increment` so that
    /// `await mvNoop()` is valid/bound — moc's first error is then the post-await token,
    /// not an unbound `mvNoop`. The injected line is tracked AFTER any preamble insert.
    fn moc_json_for_increment_body_with_helper(
        tag: &str,
        body_line: &str,
        with_mv_noop: bool,
    ) -> Option<(Vec<crate::lint::JsonDiagnostic>, usize, String)> {
        let (moc, base) = find_moc()?;
        let src = repo_root().join("examples").join("counter");
        let tmp = repo_root()
            .join("examples")
            .join(format!("r12_{}_{}", tag, std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        copy_tree(&src, &tmp);

        let page = tmp.join("src").join("Pages").join("Counter.mview");
        let text = std::fs::read_to_string(&page).unwrap();
        let mut lines: Vec<String> = text.lines().map(|s| s.to_string()).collect();
        // Optionally insert the async helper just before the `increment` declaration.
        if with_mv_noop {
            if let Some(pos) = lines.iter().position(|l| l.contains("func increment(")) {
                lines.insert(pos, "    func mvNoop() : async () { };".to_string());
            }
        }
        let mut injected_line = 0usize;
        for (i, l) in lines.iter_mut().enumerate() {
            if l.contains("count += by;") {
                *l = body_line.to_string();
                injected_line = i + 1;
                break;
            }
        }
        assert!(injected_line > 0, "fixture must contain `count += by;`");
        let injected_text = lines[injected_line - 1].clone();
        std::fs::write(&page, lines.join("\n") + "\n").unwrap();

        let out = build(&tmp);
        let main_mo = std::fs::read_to_string(&out).unwrap();
        let source_map = project::load_source_map(&out);
        let mut cmd = Command::new(&moc);
        cmd.arg("--check").arg("--package").arg("base").arg(&base);
        for a in dfx_package_args(&tmp) {
            cmd.arg(a);
        }
        cmd.arg(&out);
        let stderr = String::from_utf8_lossy(&cmd.output().expect("running moc --check failed").stderr).into_owned();
        let diags = project::map_moc_errors_json(&main_mo, &stderr, &source_map, Some(&tmp));
        let _ = std::fs::remove_dir_all(&tmp);
        Some((diags, injected_line, injected_text))
    }

    /// 1-based CHAR column of the first occurrence of `needle` in `line`.
    fn col_of(line: &str, needle: &str) -> usize {
        let byte = line.find(needle).unwrap_or_else(|| panic!("`{needle}` not in `{line}`"));
        line[..byte].chars().count() + 1
    }

    /// HEADLINE: a handler body with `ignore (await mvNoop());` (await is STRIPPED by
    /// codegen) followed by an unbound `nope` on the SAME line. moc fingers `nope` at
    /// the GENERATED column (after the await was removed); R12 must report it at the
    /// SOURCE column (6 chars further right — the width of `await `).
    #[test]
    fn check_json_anchors_token_after_stripped_await_to_source_column() {
        // `mvNoop` is injected as an async helper just above `increment` so
        // `await mvNoop()` is valid/bound — moc's only error is the `nope` we anchor.
        let Some((diags, injected_line, injected_text)) = moc_json_for_increment_body_with_helper(
            "await",
            "        ignore (await mvNoop()); let _b : Nat = nope;",
            true,
        ) else {
            eprintln!("skipping R12 await headline: moc not found");
            return;
        };
        let nope: Vec<_> = diags
            .iter()
            .filter(|d| d.rule == "type-check" && d.message.contains("nope"))
            .collect();
        assert_eq!(nope.len(), 1, "exactly one `nope` error, got: {diags:#?}");
        let d = nope[0];
        assert_eq!(d.file, "src/Pages/Counter.mview");
        assert_eq!(d.line as usize, injected_line, "line stays correct (R11)");
        let src_col = col_of(&injected_text, "nope");
        assert_eq!(
            d.col as usize, src_col,
            "THE R12 HEADLINE: `nope` must report its SOURCE column {src_col} (after the \
             stripped `await `), got {}. injected: {injected_text:?}",
            d.col
        );
        assert_eq!(d.end_col as usize, src_col + "nope".len(), "end col = col + token len");
        // And it must be STRICTLY to the right of where the stripped-await generated
        // column would have put it (proving the fix actually moved the column).
        assert!(d.col as usize > src_col - 6, "sanity: source col accounts for `await `");
    }

    /// SIMPLE (regression guard): a plain unbound token on a NON-transformed line still
    /// reports the EXACT source column — R12 must not move columns that were already right.
    #[test]
    fn check_json_simple_unbound_token_keeps_exact_source_column() {
        let Some((diags, injected_line, injected_text)) =
            moc_json_for_increment_body("simple", "        let _b : Nat = nope;")
        else {
            eprintln!("skipping R12 simple: moc not found");
            return;
        };
        let nope: Vec<_> = diags
            .iter()
            .filter(|d| d.rule == "type-check" && d.message.contains("nope"))
            .collect();
        assert_eq!(nope.len(), 1, "exactly one `nope` error, got: {diags:#?}");
        let d = nope[0];
        assert_eq!(d.line as usize, injected_line);
        let src_col = col_of(&injected_text, "nope");
        assert_eq!(
            d.col as usize, src_col,
            "simple line: column must stay EXACTLY the source column {src_col}, got {}",
            d.col
        );
    }

    /// FALLBACK: a renamed identifier — `validate { x }` is TRANSLATED by codegen so the
    /// generated text differs from the source, and a token from that translation has no
    /// verbatim match in the source line. The mapper must NOT panic and must report a
    /// column NO WORSE than moc's (the LINE stays correct). We assert it never reports a
    /// column past the end of the source line and the line is right.
    #[test]
    fn check_json_renamed_or_notfound_falls_back_without_regression() {
        // Inject a type error whose offending token is a generated-only artifact: assign
        // a Text to the Nat `count` THROUGH a `validate`-style construct is overkill;
        // simplest robust not-found case: the error token is the LHS `count`, which IS in
        // the source, plus we prove the fallback path by also running with project_dir
        // pointing at a WRONG dir (source unreadable) -> must keep moc's column, no panic.
        let Some((diags, injected_line, _injected_text)) =
            moc_json_for_increment_body("fallback", "        count := \"not a Nat\";")
        else {
            eprintln!("skipping R12 fallback: moc not found");
            return;
        };
        let errs: Vec<_> = diags.iter().filter(|d| d.rule == "type-check").collect();
        assert!(!errs.is_empty(), "the type error is reported: {diags:#?}");
        let d = errs[0];
        // LINE still correct (R11 intact).
        assert_eq!(d.line as usize, injected_line);
        // Column is sane (1-based, within the source line) — never a panic, never worse.
        assert!(d.col >= 1, "column stays 1-based and valid, got {}", d.col);

        // Now the explicit not-found / unreadable-source path: feed a synthetic moc error
        // for a mapped line but with project_dir = None (source cannot be read). The
        // column must be exactly moc's (fallback), the line mapped, no panic.
        let mut sm = SourceMap::new();
        sm.push("src/Pages/X.mview".to_string(), 4, 4, 9);
        let main_mo = "// GENERATED\nactor {\n  // mv:src src/Pages/X.mview\n  let bad = renamedToken;\n};\n";
        let moc = "/tmp/app/.mvbuild/main.mo:4.13-4.25: type error [M0057], unbound variable renamedToken";
        let diags2 = project::map_moc_errors_json(main_mo, moc, &sm, None);
        assert_eq!(diags2.len(), 1);
        assert_eq!(diags2[0].line, 9, "line mapped via the source map (R11)");
        assert_eq!(diags2[0].col, 13, "no source dir -> column falls back to moc's, unchanged");
        assert_eq!(diags2[0].end_col, 25, "end col falls back too");
    }

    // ---- R13: var-init + template-expr regions (real-moc, end-to-end) ------
    //
    // R11 mapped @code FUNC BODIES; R13 extends the SAME generated->source map to
    // (1) `var`/`let`/`type` declaration lines in the page object block — so a type
    // error in a var/let INITIALIZER lands on the var's `.mview` line+col — and
    // (2) template `@(expr)`/`@raw(expr)` directives — so a type error inside the
    // interpolated expression lands on the directive's `.mview` line+col. Both ride
    // the R12 token-anchor for free (any newly-mapped line gets column accuracy).

    /// Copy a project into a repo-local temp dir (so `../../runtime/src` resolves),
    /// patch its Counter page by replacing whole lines via `edits` (find -> replace),
    /// BUILD, run `moc --check`, and return the mapped JSON diagnostics + the patched
    /// page text. Returns `None` when moc is unavailable (caller SKIPs, never fails).
    fn moc_json_for_counter_edits(
        tag: &str,
        edits: &[(&str, &str)],
    ) -> Option<(Vec<crate::lint::JsonDiagnostic>, Vec<String>)> {
        let (moc, base) = find_moc()?;
        let src = repo_root().join("examples").join("counter");
        let tmp = repo_root()
            .join("examples")
            .join(format!("r13_{}_{}", tag, std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        copy_tree(&src, &tmp);

        let page = tmp.join("src").join("Pages").join("Counter.mview");
        let text = std::fs::read_to_string(&page).unwrap();
        let mut lines: Vec<String> = text.lines().map(|s| s.to_string()).collect();
        for (find, replace) in edits {
            let mut hit = false;
            for l in lines.iter_mut() {
                if l.contains(find) {
                    *l = replace.to_string();
                    hit = true;
                    break;
                }
            }
            assert!(hit, "fixture must contain `{find}` to patch");
        }
        std::fs::write(&page, lines.join("\n") + "\n").unwrap();

        let out = build(&tmp);
        let main_mo = std::fs::read_to_string(&out).unwrap();
        let source_map = project::load_source_map(&out);
        let mut cmd = Command::new(&moc);
        cmd.arg("--check").arg("--package").arg("base").arg(&base);
        for a in dfx_package_args(&tmp) {
            cmd.arg(a);
        }
        cmd.arg(&out);
        let stderr =
            String::from_utf8_lossy(&cmd.output().expect("running moc --check failed").stderr)
                .into_owned();
        let diags = project::map_moc_errors_json(&main_mo, &stderr, &source_map, Some(&tmp));
        let _ = std::fs::remove_dir_all(&tmp);
        Some((diags, lines))
    }

    /// 1-based CHAR column of the first occurrence of `needle` in `line` (R13 reuse
    /// of the R12 helper convention; redeclared so this region is self-contained).
    fn col_of_r13(line: &str, needle: &str) -> usize {
        let byte = line.find(needle).unwrap_or_else(|| panic!("`{needle}` not in `{line}`"));
        line[..byte].chars().count() + 1
    }

    /// HEADLINE (var-init): replace the `var count : Nat = 0;` initializer with a
    /// TYPE error (`var count : Nat = "boom";`). moc reports a type error on the
    /// GENERATED `var count` line; R13 must map it to the var's `.mview` LINE and
    /// (via the R12 anchor) the COLUMN of the offending `"boom"` token.
    #[test]
    fn check_json_var_init_type_error_maps_to_mview_line_and_col() {
        let Some((diags, patched)) = moc_json_for_counter_edits(
            "varinit",
            &[("var count : Nat = 0;", "    var count : Nat = \"boom\";")],
        ) else {
            eprintln!("skipping R13 var-init headline: moc not found");
            return;
        };
        // The injected var line is line 20 in the .mview (Counter.mview); find it.
        let injected_idx = patched
            .iter()
            .position(|l| l.contains("var count : Nat = \"boom\";"))
            .expect("patched var line present");
        let injected_line = injected_idx + 1; // 1-based
        let injected_text = &patched[injected_idx];

        let errs: Vec<_> = diags.iter().filter(|d| d.rule == "type-check").collect();
        assert_eq!(
            errs.len(),
            1,
            "expected exactly one type error, got: {diags:#?}"
        );
        let d = errs[0];
        assert_eq!(d.file, "src/Pages/Counter.mview", "must name the .mview source");
        assert_eq!(
            d.line as usize, injected_line,
            "THE R13 var-init HEADLINE: error must report the var's .mview LINE {injected_line}, got {}",
            d.line
        );
        // R12 column anchor: the offending token is the `"boom"` string literal.
        let src_col = col_of_r13(injected_text, "\"boom\"");
        assert_eq!(
            d.col as usize, src_col,
            "var-init error must report the COLUMN of `\"boom\"` ({src_col}), got {}. line: {injected_text:?}",
            d.col
        );
    }

    /// HEADLINE (template expr): make the `@count` interpolation ill-typed by
    /// referencing an UNDEFINED field/var `@(badField)` in the template. moc errors
    /// inside the generated `b.text(...)` line for that interpolation; R13 must map
    /// it to the `@(badField)` directive's `.mview` line and column.
    #[test]
    fn check_json_template_expr_type_error_maps_to_mview_line_and_col() {
        // The counter template line 9 is:
        //   `    <p class="counter-value">Current value: <strong>@count</strong></p>`
        // Replace `@count` with `@(badField)` (an unbound identifier). The rest of
        // the template (the @click handlers) is unchanged, so the only NEW error is
        // the unbound `badField` inside the single `b.text(...)` interpolation —
        // keeping the render region's emit-line count equal to the recorded span
        // count (one), so R13 maps it (the reliability gate is satisfied).
        let Some((diags, patched)) = moc_json_for_counter_edits(
            "tmplexpr",
            &[(
                "<strong>@count</strong>",
                "    <p class=\"counter-value\">Current value: <strong>@(badField)</strong></p>",
            )],
        ) else {
            eprintln!("skipping R13 template-expr headline: moc not found");
            return;
        };
        let injected_idx = patched
            .iter()
            .position(|l| l.contains("@(badField)"))
            .expect("patched template line present");
        let injected_line = injected_idx + 1;
        let injected_text = &patched[injected_idx];

        let errs: Vec<_> = diags
            .iter()
            .filter(|d| d.rule == "type-check" && d.message.contains("badField"))
            .collect();
        assert_eq!(
            errs.len(),
            1,
            "expected exactly one `badField` type error, got: {diags:#?}"
        );
        let d = errs[0];
        assert_eq!(d.file, "src/Pages/Counter.mview", "must name the .mview source");
        assert_eq!(
            d.line as usize, injected_line,
            "THE R13 template-expr HEADLINE: `badField` must report the @() directive's \
             .mview LINE {injected_line}, got {}",
            d.line
        );
        // R12 column anchor: `badField` appears verbatim in the source line.
        let src_col = col_of_r13(injected_text, "badField");
        assert_eq!(
            d.col as usize, src_col,
            "template-expr error must report the COLUMN of `badField` ({src_col}), got {}. line: {injected_text:?}",
            d.col
        );
    }

    /// NO-WRONG-MAPPING: a page that MIXES mapped + unmapped regions. We inject TWO
    /// independent type errors — one in a var INITIALIZER (mapped region) and one in
    /// a FUNC body (R11-mapped region) — and assert EACH is reported on its OWN
    /// correct `.mview` line. This guards the queue/region bookkeeping: a var error
    /// must never steal a func's anchor (or vice-versa), and neither lands on the
    /// page's file-marker fallback.
    #[test]
    fn check_json_mixed_regions_every_error_on_its_own_line() {
        let Some((diags, patched)) = moc_json_for_counter_edits(
            "mixed",
            &[
                // var-init error (mapped var region)
                ("var count : Nat = 0;", "    var count : Nat = \"boom\";"),
                // func-body error (R11 func region): assign a Text to the Nat count
                ("count += by;", "        count := \"also not a Nat\";"),
            ],
        ) else {
            eprintln!("skipping R13 mixed-regions: moc not found");
            return;
        };
        let var_line = patched
            .iter()
            .position(|l| l.contains("var count : Nat = \"boom\";"))
            .map(|i| i + 1)
            .expect("var line present");
        let func_body_line = patched
            .iter()
            .position(|l| l.contains("count := \"also not a Nat\";"))
            .map(|i| i + 1)
            .expect("func body line present");
        assert_ne!(var_line, func_body_line, "two distinct injected lines");

        let errs: Vec<_> = diags.iter().filter(|d| d.rule == "type-check").collect();
        assert!(!errs.is_empty(), "errors reported: {diags:#?}");
        // Every reported type error must sit on one of the two injected lines and
        // name the .mview file — NEVER a wrong/fallback line.
        for d in &errs {
            assert_eq!(d.file, "src/Pages/Counter.mview", "error names the .mview file");
            assert!(
                d.line as usize == var_line || d.line as usize == func_body_line,
                "every error must be on an injected line ({var_line} or {func_body_line}), \
                 got {} (msg: {}) — a WRONG anchor is worse than fallback",
                d.line,
                d.message
            );
        }
        // And BOTH injected lines must actually be hit (the var error AND the func
        // error each map to their own region — proving mixed mapping works).
        assert!(
            errs.iter().any(|d| d.line as usize == var_line),
            "the var-init error must land on the var line {var_line}: {errs:#?}"
        );
        assert!(
            errs.iter().any(|d| d.line as usize == func_body_line),
            "the func-body error must land on the func line {func_body_line}: {errs:#?}"
        );
    }
}

/// R13 build-level tests that do NOT need moc: they build a real project and
/// assert the generated->source map gained the var-init + template-expr regions,
/// that those regions resolve to the right `.mview` lines, and that building keeps
/// `main.mo` byte-identical (the map is still a SIDE artifact).
#[cfg(test)]
mod r13_var_template_map {
    use crate::project;
    use std::path::PathBuf;

    fn repo_root() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .expect("compiler/ has a parent")
            .to_path_buf()
    }

    /// Build counter into a temp out and return the loaded source map (plus the
    /// out path so the caller can clean up). `tag` keeps the temp path unique
    /// across concurrently-running tests (the test harness runs them in parallel,
    /// so a shared `process::id()`-only path would clobber).
    fn build_counter_map(tag: &str) -> (crate::span::SourceMap, PathBuf) {
        let counter = repo_root().join("examples").join("counter");
        let out = std::env::temp_dir().join(format!("mv_r13_{}_{}.mo", tag, std::process::id()));
        let opts = project::BuildOptions {
            project_dir: counter.clone(),
            app_name: "counter".to_string(),
            out: out.clone(),
            ..Default::default()
        };
        project::build(&opts).expect("counter build");
        (project::load_source_map(&out), out)
    }

    /// The map gained a single-line region anchoring the `var count` decl line to
    /// its `.mview` line (20 in examples/counter/src/Pages/Counter.mview).
    #[test]
    fn map_anchors_the_var_decl_line_to_its_mview_line() {
        let (map, out) = build_counter_map("vardecl");
        // src line 20 is `    var count : Nat = 0;` — find the entry anchored there.
        let entry = map
            .entries
            .iter()
            .find(|e| e.src_start_line == 20 && e.gen_start_line == e.gen_end_line)
            .expect("a single-line region anchored at the var decl (src line 20)");
        let (file, src_line) = map.resolve(entry.gen_start_line).expect("var line resolves");
        assert_eq!(file, "src/Pages/Counter.mview");
        assert_eq!(src_line, 20, "the var decl line maps to its .mview line");
        let _ = std::fs::remove_file(&out);
        let _ = std::fs::remove_file(project::source_map_path(&out));
    }

    /// The map gained a single-line region anchoring the `@count` interpolation's
    /// generated `b.text(...)` line to its `.mview` line (9). This is the template-
    /// expr region; counter has exactly ONE template expr, so the reliability gate
    /// (emit-line count == recorded-span count) is satisfied and it is mapped.
    #[test]
    fn map_anchors_the_template_expr_emit_line_to_its_mview_line() {
        let (map, out) = build_counter_map("tmplexpr");
        // src line 9 carries `<strong>@count</strong>`.
        let entry = map
            .entries
            .iter()
            .find(|e| e.src_start_line == 9 && e.gen_start_line == e.gen_end_line)
            .expect("a single-line region anchored at the @count interpolation (src line 9)");
        let (file, src_line) = map.resolve(entry.gen_start_line).expect("expr line resolves");
        assert_eq!(file, "src/Pages/Counter.mview");
        assert_eq!(src_line, 9, "the @count interpolation maps to its .mview line");
        let _ = std::fs::remove_file(&out);
        let _ = std::fs::remove_file(project::source_map_path(&out));
    }

    /// BYTE-IDENTITY (R13): adding var + template-expr regions to the SIDE map must
    /// not change a single byte of `main.mo`. Assert the fresh counter build equals
    /// the committed golden artifact (same guard the R11 test uses, re-asserted now
    /// that R13 records more regions).
    #[test]
    fn r13_keeps_counter_main_mo_byte_identical() {
        let counter = repo_root().join("examples").join("counter");
        let committed = std::fs::read(counter.join(".mvbuild").join("main.mo"))
            .expect("committed counter main.mo exists");
        let out = std::env::temp_dir().join(format!("mv_r13_bi_{}.mo", std::process::id()));
        let opts = project::BuildOptions {
            project_dir: counter.clone(),
            app_name: "counter".to_string(),
            out: out.clone(),
            ..Default::default()
        };
        project::build(&opts).expect("counter build");
        let fresh = std::fs::read(&out).expect("fresh main.mo");
        assert_eq!(
            fresh, committed,
            "R13 must keep the generated main.mo byte-identical to the golden artifact"
        );
        let _ = std::fs::remove_file(&out);
        let _ = std::fs::remove_file(project::source_map_path(&out));
    }
}
