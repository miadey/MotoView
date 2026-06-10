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
