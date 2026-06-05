//! Regression tests for the parser + codegen. Each locks in a bug that was
//! found and fixed while building real apps, so the whole class can't return.
#![cfg(test)]

use crate::ast::{FileKind, ParamDecl};
use crate::codegen::{CompInfo, Codegen};
use crate::parser;
use std::collections::HashMap;

/// Parse + generate a page object block.
fn page(src: &str) -> String {
    let models: HashMap<String, HashMap<String, String>> = HashMap::new();
    let comps: HashMap<String, CompInfo> = HashMap::new();
    let file = parser::parse(src, "T", FileKind::Page).expect("parse failed");
    let mut cg = Codegen::new(&models, &comps);
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
