//! Motoko code generation from a parsed `.mview` AST.
//!
//! A page compiles to a Motoko `object` holding its state + handlers + a
//! `render` that builds HTML via `Html.Builder`. The project orchestrator
//! wires all pages/layouts into one actor exposing http_request[_update].

use crate::ast::*;
use std::collections::{HashMap, HashSet};

pub struct PageGen {
    pub name: String,
    pub object_block: String,
    pub page_record: String,
    pub route: String,
}

/// What the codegen needs to know to call an app component: its ordered params
/// and the names of the named slots it declares (`@slot "x"`).
pub struct CompInfo {
    pub params: Vec<ParamDecl>,
    pub slots: Vec<String>,
}

/// Collect the named slots (`@slot "name"`) declared in a component template.
pub fn collect_slot_names(nodes: &[Node]) -> Vec<String> {
    let mut out = Vec::new();
    fn walk(nodes: &[Node], out: &mut Vec<String>) {
        for n in nodes {
            match n {
                Node::Slot(name) => {
                    if !out.contains(name) {
                        out.push(name.clone());
                    }
                }
                Node::Element(e) => walk(&e.children, out),
                Node::Component(c) => walk(&c.children, out),
                Node::If(brs) => {
                    for b in brs {
                        walk(&b.body, out);
                    }
                }
                Node::For { body, .. } => walk(body, out),
                Node::Switch { cases, .. } => {
                    for c in cases {
                        walk(&c.body, out);
                    }
                }
                _ => {}
            }
        }
    }
    walk(nodes, &mut out);
    out
}

pub struct Codegen<'a> {
    // name -> Motoko type (vars, params, func returns, and scoped @for loop
    // vars). RefCell so loop-var types can be pushed/popped during the otherwise
    // `&self` render walk.
    types: std::cell::RefCell<HashMap<String, String>>,
    is_layout: bool,
    is_component: bool,
    // The current layout's @theme `<style>…</style>`, emitted at its `@head`.
    layout_theme: Option<String>,
    // Scanned record types: type name -> (field -> field type). Lets `@item.name`
    // infer its type instead of falling back to debug_show.
    models: &'a HashMap<String, HashMap<String, String>>,
    // app components: name -> params + slot names. Used to compile `<MyCard .../>`
    // to a call of the generated `mvComponent_MyCard(...)`.
    components: &'a HashMap<String, CompInfo>,
}

impl<'a> Codegen<'a> {
    pub fn new(
        models: &'a HashMap<String, HashMap<String, String>>,
        components: &'a HashMap<String, CompInfo>,
    ) -> Self {
        Codegen {
            types: std::cell::RefCell::new(HashMap::new()),
            is_layout: false,
            is_component: false,
            layout_theme: None,
            models,
            components,
        }
    }

    /// Compile a `src/Components/*.mview` into a render function:
    /// `func mvComponent_<Name>(<params>, mvChildren : Text) : Text { ... }`.
    /// `@children` inside the template emits the passed default-slot content.
    pub fn gen_app_component(&mut self, file: &MviewFile) -> String {
        self.is_component = true;
        self.is_layout = false;
        self.build_type_env(&file.code);
        let slots = collect_slot_names(&file.template);
        let mut sig: Vec<String> = file
            .code
            .params
            .iter()
            .map(|p| format!("{} : {}", p.name, p.ty))
            .collect();
        sig.push("mvChildren : Text".to_string());
        for sl in &slots {
            sig.push(format!("mvSlot_{} : Text", sl));
        }
        let mut s = String::new();
        s.push_str(&format!("  // ===== Component: {} =====\n", file.name));
        s.push_str(&format!("  func mvComponent_{}({}) : Text {{\n", file.name, sig.join(", ")));
        s.push_str("    let b = Html.Builder();\n");
        s.push_str("    ignore mvChildren;\n");
        for sl in &slots {
            s.push_str(&format!("    ignore mvSlot_{};\n", sl));
        }
        let mut body = String::new();
        self.gen_nodes(&file.template, &mut body, "    ");
        s.push_str(&body);
        s.push_str("    b.build();\n  };\n");
        self.is_component = false;
        s
    }

    fn build_type_env(&mut self, code: &CodeBlock) {
        let mut t = self.types.borrow_mut();
        t.clear();
        for v in &code.vars {
            if let Some(ty) = &v.ty {
                t.insert(v.name.clone(), ty.clone());
            }
        }
        for p in &code.params {
            t.insert(p.name.clone(), p.ty.clone());
        }
        for f in &code.funcs {
            if let Some(r) = &f.ret {
                if r != "()" && !r.is_empty() {
                    t.insert(f.name.clone(), r.clone());
                }
            }
        }
    }

    // ---- page generation --------------------------------------------------
    pub fn gen_page(&mut self, file: &MviewFile) -> PageGen {
        self.is_layout = false;
        self.build_type_env(&file.code);
        let obj = format!("{}Page", file.name);

        // Route params (e.g. `/orders/{id:Nat}` -> ("id","Nat")) are made
        // available by name everywhere in the page: emitted as object fields and
        // refreshed from ctx.params at the start of every ctx-entry method.
        let route_params = parse_route_params(file.route.as_deref().unwrap_or(""));
        let declared: HashSet<String> = file.code.vars.iter().map(|v| v.name.clone()).collect();
        // register their types so @id renders correctly
        for (n, t) in &route_params {
            self.types.borrow_mut().insert(n.clone(), t.clone());
        }
        let set_params = {
            let mut sp = String::new();
            for (n, t) in &route_params {
                let conv = convert_from_text(t, &format!("mvParamGet(ctx, \"{}\")", n));
                sp.push_str(&format!("      {} := {};\n", n, conv));
            }
            sp
        };
        let set_params_inline = {
            let mut sp = String::new();
            for (n, t) in &route_params {
                let conv = convert_from_text(t, &format!("mvParamGet(ctx, \"{}\")", n));
                sp.push_str(&format!("{} := {}; ", n, conv));
            }
            sp
        };

        let mut s = String::new();
        s.push_str(&format!("  // ===== Page: {} ({}) =====\n", file.name, file.route.clone().unwrap_or_default()));
        s.push_str(&format!("  let {} = object {{\n", obj));

        // route-param fields (skip any the user already declared as state)
        for (n, t) in &route_params {
            if !declared.contains(n) {
                let default = match t.as_str() {
                    "Nat" | "Int" | "Nat8" | "Nat16" | "Nat32" | "Nat64" => "0".to_string(),
                    "Text" => "\"\"".to_string(),
                    _ => "\"\"".to_string(),
                };
                s.push_str(&format!("    var {} : {} = {};\n", n, t, default));
            }
        }

        // state
        for v in &file.code.vars {
            s.push_str(&format!("    {}\n", v.raw));
        }
        // extras (let bindings, helper types)
        for e in &file.code.extra {
            s.push_str(&format!("    {}\n", e));
        }
        // error + redirect + effect sinks (and declarative effect helpers usable
        // from @code: toast(...), animate("#sel","pulse"), focusOn(...), scrollTo(...))
        s.push_str("    let mvErrors = Buffer.Buffer<(Text, Text)>(0);\n");
        s.push_str("    var mvRedirect : Text = \"\";\n");
        s.push_str("    let mvEffects = Buffer.Buffer<MV.Effect>(0);\n");
        s.push_str("    public func toast(m : Text) { mvEffects.add({ kind = \"toast\"; target = m; value = \"\" }) };\n");
        s.push_str("    public func animate(sel : Text, name : Text) { mvEffects.add({ kind = \"animate\"; target = sel; value = name }) };\n");
        s.push_str("    public func focusOn(sel : Text) { mvEffects.add({ kind = \"focus\"; target = sel; value = \"\" }) };\n");
        s.push_str("    public func scrollTo(sel : Text) { mvEffects.add({ kind = \"scrollTo\"; target = sel; value = \"\" }) };\n");
        // user functions (async stripped) with validate translated
        for f in &file.code.funcs {
            let params = f
                .params
                .iter()
                .map(|(n, t)| format!("{} : {}", n, t))
                .collect::<Vec<_>>()
                .join(", ");
            // Emit the (async-stripped) return type so non-unit helpers type-check.
            let ret = match &f.ret {
                Some(r) if !r.is_empty() => format!(" : {}", r),
                _ => String::new(),
            };
            let body = translate_validate(&f.body);
            s.push_str(&format!("    func {}({}){} {{{}}};\n", f.name, params, ret, body));
        }

        // render
        s.push_str("    public func mvRender(ctx : MV.Ctx) : Text {\n");
        s.push_str("      let b = Html.Builder();\n");
        s.push_str("      ignore ctx;\n");
        s.push_str(&set_params);
        let mut body = String::new();
        self.gen_nodes(&file.template, &mut body, "      ");
        s.push_str(&body);
        s.push_str("      b.build();\n    };\n");

        // title / description / head
        let title_expr = file.title.clone().unwrap_or_else(|| "\"\"".into());
        s.push_str(&format!(
            "    public func mvTitle(ctx : MV.Ctx) : Text {{ ignore ctx; {}{} }};\n",
            set_params_inline,
            self.as_text(&title_expr)
        ));
        let desc_expr = file.description.clone().unwrap_or_else(|| "\"\"".into());
        let canon_expr = file.canonical.clone().unwrap_or_else(|| "\"\"".into());
        // head extra (from @section "head")
        let head_extra = self.gen_head_extra(file);
        s.push_str(&format!(
            "    public func mvHead(ctx : MV.Ctx) : MV.Head {{ ignore ctx; {}{{ title = {}; description = {}; canonical = {}; extra = {} }} }};\n",
            set_params_inline,
            self.as_text(&title_expr),
            self.as_text(&desc_expr),
            self.as_text(&canon_expr),
            head_extra
        ));

        // onLoad
        let has_onload = file.code.funcs.iter().any(|f| f.name == "onLoad");
        if has_onload {
            let onload_takes_ctx = file
                .code
                .funcs
                .iter()
                .find(|f| f.name == "onLoad")
                .map(|f| !f.params.is_empty())
                .unwrap_or(false);
            if onload_takes_ctx {
                s.push_str(&format!("    public func mvOnLoad(ctx : MV.Ctx) {{ ignore ctx; {}onLoad(ctx) }};\n", set_params_inline));
            } else {
                s.push_str(&format!("    public func mvOnLoad(ctx : MV.Ctx) {{ ignore ctx; {}onLoad() }};\n", set_params_inline));
            }
        } else {
            s.push_str(&format!("    public func mvOnLoad(ctx : MV.Ctx) {{ ignore ctx; {} }};\n", set_params_inline));
        }

        // dispatch
        s.push_str("    public func mvDispatch(ctx : MV.Ctx, mvH : Text, mvArgs : [Text]) {\n");
        s.push_str("      ignore ctx; ignore mvArgs;\n");
        s.push_str(&set_params);
        s.push_str("      mvErrors.clear(); // each interaction starts with a clean slate\n");
        s.push_str("      mvEffects.clear();\n");
        // two-way binding: populate bound vars from the submitted form,
        // converting to the bound var's declared type.
        let binds = collect_simple_binds(&file.template);
        for (lvalue, name) in &binds {
            let ty = self.types.borrow().get(lvalue).cloned().unwrap_or_default();
            let conv = convert_from_text(&ty, "mvV");
            s.push_str(&format!(
                "      switch (mvFormGet(ctx, \"{}\")) {{ case (?mvV) {{ {} := {} }}; case null {{}} }};\n",
                name, lvalue, conv
            ));
        }
        // Only funcs referenced as event handlers (@click/@submit/@input or a
        // data-mv-drop drop target) are dispatchable. Helper funcs used from the
        // template (e.g. dealsIn, money) are not.
        let handlers = collect_handlers(&file.template);
        s.push_str("      switch mvH {\n");
        for f in &file.code.funcs {
            if f.name == "onLoad" || !handlers.contains(&f.name) {
                continue;
            }
            let call = self.gen_dispatch_call(f);
            s.push_str(&format!("        case \"{}\" {{ {} }};\n", f.name, call));
        }
        s.push_str("        case _ {};\n");
        s.push_str("      };\n    };\n");

        // Errors persist across render polls (so they don't flash away); they are
        // cleared at the start of the next dispatch.
        s.push_str("    public func mvTakeErrors() : [(Text, Text)] { Buffer.toArray(mvErrors) };\n");
        s.push_str("    public func mvTakeRedirect() : Text { let r = mvRedirect; mvRedirect := \"\"; r };\n");
        // Effects are one-shot: returned (and cleared) for the event response only.
        s.push_str("    public func mvTakeEffects() : [MV.Effect] { let e = Buffer.toArray(mvEffects); mvEffects.clear(); e };\n");
        s.push_str("  };\n");

        // Page record
        let route = file.route.clone().unwrap_or_else(|| "/".into());
        let layout = file.layout.clone().unwrap_or_default();
        let (authorize, role) = match &file.authorize {
            Some(a) => ("true", a.role.clone().unwrap_or_default()),
            None => ("false", String::new()),
        };
        let cacheable = if file.cacheable { "true" } else { "false" };
        let rec = format!(
            "  let {n}Def : MV.Page = {{\n    route = {route:?};\n    layout = {layout:?};\n    authorize = {auth};\n    role = {role:?};\n    cacheable = {cacheable};\n    onLoad = {o}.mvOnLoad;\n    render = {o}.mvRender;\n    title = {o}.mvTitle;\n    head = {o}.mvHead;\n    dispatch = {o}.mvDispatch;\n    takeErrors = {o}.mvTakeErrors;\n    takeRedirect = {o}.mvTakeRedirect;\n  }};\n",
            n = file.name,
            route = route,
            layout = layout,
            auth = authorize,
            role = role,
            cacheable = cacheable,
            o = obj,
        );
        let rec = rec.replace(
            "    takeRedirect = ",
            &format!("    takeEffects = {o}.mvTakeEffects;\n    takeRedirect = ", o = obj),
        );

        PageGen {
            name: file.name.clone(),
            object_block: s,
            page_record: rec,
            route,
        }
    }

    fn gen_dispatch_call(&self, f: &FuncDecl) -> String {
        let mut args = Vec::new();
        // A handler may take the request context as its first parameter
        // (`ctx : Context` / `ctx : MV.Ctx`, or simply a first param named `ctx`).
        // We pass the live `ctx` for it and bind the remaining params to mvArgs.
        // This is how a handler reads `ctx.caller`, `ctx.form`, route params, etc.
        let mut arg_idx = 0usize;
        for (pos, (n, t)) in f.params.iter().enumerate() {
            let tt = t.trim();
            if pos == 0 && (n == "ctx" || tt == "Context" || tt == "MV.Ctx") {
                args.push("ctx".to_string());
            } else {
                let access = format!("(if (mvArgs.size() > {i}) mvArgs[{i}] else \"\")", i = arg_idx);
                args.push(convert_from_text(t, &access));
                arg_idx += 1;
            }
        }
        format!("{}({})", f.name, args.join(", "))
    }

    // ---- layout generation ------------------------------------------------
    pub fn gen_layout(&mut self, file: &MviewFile) -> String {
        self.is_layout = true;
        self.build_type_env(&file.code);
        // A layout-level @theme is emitted at the layout's `@head`.
        self.layout_theme = self.theme_style_css(file);
        let mut s = String::new();
        s.push_str(&format!("  // ===== Layout: {} =====\n", file.name));
        s.push_str(&format!(
            "  func mvLayout_{}(ctx : MV.Ctx, mvHead : MV.Head, mvBody : Text) : Text {{\n",
            file.name
        ));
        s.push_str("    ignore ctx;\n");
        s.push_str("    let b = Html.Builder();\n");
        let mut body = String::new();
        self.gen_nodes(&file.template, &mut body, "    ");
        s.push_str(&body);
        s.push_str("    b.build();\n  };\n");
        s
    }

    // ---- node rendering ---------------------------------------------------
    fn gen_nodes(&self, nodes: &[Node], out: &mut String, indent: &str) {
        for n in nodes {
            self.gen_node(n, out, indent);
        }
    }

    fn gen_node(&self, node: &Node, out: &mut String, indent: &str) {
        match node {
            Node::Text(t) => {
                if !t.is_empty() {
                    out.push_str(&format!("{}b.raw({});\n", indent, mo_str(t)));
                }
            }
            Node::Expr(e) => {
                if self.is_component && e.trim() == "children" {
                    out.push_str(&format!("{}b.raw(mvChildren);\n", indent));
                } else {
                    out.push_str(&format!("{}b.text({});\n", indent, self.as_text(e)));
                }
            }
            // `@raw(expr)` — trusted HTML, emitted without escaping. The
            // expression is emitted verbatim and must be `Text` (else moc errors);
            // we deliberately do NOT route through as_text (no debug_show wrap).
            Node::Raw(e) => {
                let e = e.trim();
                // keep the layout `View.x` -> head-field convenience
                let expr = if self.is_layout {
                    e.strip_prefix("View.").map(|r| format!("mvHead.{}", r)).unwrap_or_else(|| e.to_string())
                } else {
                    e.to_string()
                };
                out.push_str(&format!("{}b.raw({});\n", indent, expr));
            }
            Node::Yield => {
                out.push_str(&format!("{}b.raw(mvBody);\n", indent));
            }
            Node::Head => {
                // emit the accumulated head extra (description/canonical/og) + nothing else;
                // title is handled by the layout's <title>@View.title</title>.
                out.push_str(&format!("{}b.raw(mvHead.extra);\n", indent));
                out.push_str(&format!(
                    "{}if (mvHead.description != \"\") {{ b.raw(\"<meta name=\\\"description\\\" content=\\\"\"); b.text(mvHead.description); b.raw(\"\\\">\") }};\n",
                    indent
                ));
                out.push_str(&format!(
                    "{}if (mvHead.canonical != \"\") {{ b.raw(\"<link rel=\\\"canonical\\\" href=\\\"\"); b.text(mvHead.canonical); b.raw(\"\\\">\") }};\n",
                    indent
                ));
                // A layout-level @theme injects its tokens here, after the base
                // CSS link, so they override the defaults.
                if let Some(style) = &self.layout_theme {
                    out.push_str(&format!("{}b.raw({});\n", indent, mo_str(style)));
                }
            }
            Node::SectionRef(_) => {
                // sections other than the default body are not wired in the MVP layout
            }
            Node::Slot(name) => {
                if self.is_component {
                    out.push_str(&format!("{}b.raw(mvSlot_{});\n", indent, name));
                }
            }
            Node::Effect { .. } => {
                // effects in templates are emitted as data markers picked up by the client.
                // (MVP: effects are primarily delivered via the batch; template effects are no-ops here.)
            }
            Node::If(branches) => {
                // Emit `if (c) { } else if (c) { } else { };` as ONE statement:
                // the `else`/`else if` must follow the preceding `}` with no
                // semicolon; only the final branch terminates with `};`.
                let n = branches.len();
                for (k, br) in branches.iter().enumerate() {
                    let opener = match &br.cond {
                        Some(c) => {
                            if k == 0 {
                                format!("if ({}) {{\n", c)
                            } else {
                                format!("else if ({}) {{\n", c)
                            }
                        }
                        None => "else {\n".to_string(),
                    };
                    if k == 0 {
                        out.push_str(&format!("{}{}", indent, opener));
                    } else {
                        // continue on the same line as the previous close brace
                        out.push_str(&opener);
                    }
                    self.gen_nodes(&br.body, out, &format!("{}  ", indent));
                    if k + 1 < n {
                        // brace + space; the next branch's `else` follows directly
                        out.push_str(&format!("{}}} ", indent));
                    } else {
                        out.push_str(&format!("{}}};\n", indent));
                    }
                }
            }
            Node::For { var, iter, body } => {
                out.push_str(&format!("{}for ({} in ({}).vals()) {{\n", indent, var, iter));
                // Scope the loop var's element type so `@var.field` resolves.
                let prev = self.types.borrow().get(var).cloned();
                if let Some(elem) = self.element_type(iter) {
                    self.types.borrow_mut().insert(var.clone(), elem);
                }
                self.gen_nodes(body, out, &format!("{}  ", indent));
                {
                    let mut t = self.types.borrow_mut();
                    match prev {
                        Some(p) => { t.insert(var.clone(), p); }
                        None => { t.remove(var); }
                    }
                }
                out.push_str(&format!("{}}};\n", indent));
            }
            Node::Switch { subject, cases } => {
                out.push_str(&format!("{}switch ({}) {{\n", indent, subject));
                for c in cases {
                    let pat = if c.pattern.starts_with('(') {
                        c.pattern.clone()
                    } else {
                        format!("({})", c.pattern)
                    };
                    out.push_str(&format!("{}  case {} {{\n", indent, pat));
                    self.gen_nodes(&c.body, out, &format!("{}    ", indent));
                    out.push_str(&format!("{}  }};\n", indent));
                }
                out.push_str(&format!("{}}};\n", indent));
            }
            Node::Element(el) => self.gen_element(el, out, indent),
            Node::Component(c) => self.gen_component(c, out, indent),
        }
    }

    fn gen_element(&self, el: &Element, out: &mut String, indent: &str) {
        out.push_str(&format!("{}b.raw(\"<{}\");\n", indent, el.tag));
        // Server-driven forms must bypass native constraint validation so the
        // submit reaches MotoView (server-side validation is the source of truth).
        if el.tag == "form" && el.events.iter().any(|e| e.event == "submit") {
            out.push_str(&format!("{}b.raw(\" novalidate\");\n", indent));
        }
        // static + expr attributes
        for a in &el.attrs {
            self.gen_attr(a, out, indent);
        }
        // bind -> name + data-mv-key + value
        if let Some(lv) = &el.bind {
            let key = lv.clone();
            let name = lv.split('.').last().unwrap_or(lv).to_string();
            out.push_str(&format!("{}b.raw(\" name=\\\"{}\\\" data-mv-key=\\\"{}\\\"\");\n", indent, name, key));
            out.push_str(&format!("{}b.attr(\"value\", {});\n", indent, self.as_text(lv)));
        }
        // events
        for ev in &el.events {
            if ev.event == "submit" {
                // forms: handled at the <form> level
                out.push_str(&format!("{}b.raw(\" data-mv-handler=\\\"{}\\\" data-mv-event=\\\"submit\\\"\");\n", indent, ev.handler));
                if el.secure {
                    // schema = sorted form field names; escaped defensively even
                    // though field names are identifiers.
                    let schema = escape_mo_inner(&collect_form_schema(el));
                    out.push_str(&format!("{}b.raw(\" data-mv-secure=\\\"1\\\"\");\n", indent));
                    out.push_str(&format!("{}b.attr(\"data-mv-token\", ctx.mintToken(\"{}\", \"{}\"));\n", indent, ev.handler, schema));
                    out.push_str(&format!("{}b.raw(\" data-mv-schema=\\\"{}\\\"\");\n", indent, schema));
                }
            } else {
                out.push_str(&format!("{}b.raw(\" data-mv-handler=\\\"{}\\\" data-mv-event=\\\"{}\\\"\");\n", indent, ev.handler, ev.event));
                for (i, arg) in ev.args.iter().enumerate() {
                    if is_simple_literal(arg) {
                        out.push_str(&format!("{}b.raw(\" data-mv-arg{}=\\\"{}\\\"\");\n", indent, i, arg.trim_matches('"')));
                    } else {
                        out.push_str(&format!("{}b.attr(\"data-mv-arg{}\", {});\n", indent, i, self.as_text(arg)));
                    }
                }
            }
        }
        if el.self_closing {
            out.push_str(&format!("{}b.raw(\">\");\n", indent));
            return;
        }
        out.push_str(&format!("{}b.raw(\">\");\n", indent));
        self.gen_nodes(&el.children, out, indent);
        out.push_str(&format!("{}b.raw(\"</{}>\");\n", indent, el.tag));
    }

    fn gen_attr(&self, a: &Attr, out: &mut String, indent: &str) {
        // Framework attributes compile to their `data-mv-*` markers: `key` for
        // keyed reconciliation, `enter`/`exit` for the animation the client plays
        // when the element is inserted/removed in a keyed list.
        let name = match a.name.as_str() {
            "key" => "data-mv-key",
            "enter" => "data-mv-enter",
            "exit" => "data-mv-exit",
            other => other,
        };
        match &a.value {
            AttrValue::Bool => {
                out.push_str(&format!("{}b.raw(\" {}\");\n", indent, name));
            }
            AttrValue::Literal(v) => {
                out.push_str(&format!("{}b.raw({});\n", indent, mo_str(&format!(" {}=\"{}\"", name, v))));
            }
            AttrValue::Expr(e) => {
                out.push_str(&format!("{}b.attr({:?}, {});\n", indent, name, self.as_text(e)));
            }
            AttrValue::Concat(parts) => {
                out.push_str(&format!("{}b.raw(\" {}=\\\"\");\n", indent, name));
                for p in parts {
                    match p {
                        AttrPart::Lit(l) => out.push_str(&format!("{}b.raw({});\n", indent, mo_str(l))),
                        AttrPart::Expr(e) => out.push_str(&format!("{}b.text({});\n", indent, self.as_text(e))),
                    }
                }
                out.push_str(&format!("{}b.raw(\"\\\"\");\n", indent));
            }
        }
    }

    fn gen_component(&self, c: &Component, out: &mut String, indent: &str) {
        // Built-in semantic components compile to plain HTML (reserved names).
        if let Some(()) = self.gen_builtin(c, out, indent) {
            return;
        }
        // App component (src/Components/<Name>.mview): call its generated render.
        if let Some(info) = self.components.get(&c.name) {
            let mut args: Vec<String> = info
                .params
                .iter()
                .map(|p| self.prop_to_arg(c.props.iter().find(|a| a.name == p.name), p))
                .collect();
            args.push(self.build_children_text(&c.children));
            // named slot content: the parent's @section "name" for this component
            for sl in &info.slots {
                let content = c
                    .slots
                    .iter()
                    .find(|(n, _)| n == sl)
                    .map(|(_, nodes)| self.build_children_text(nodes))
                    .unwrap_or_else(|| "\"\"".to_string());
                args.push(content);
            }
            out.push_str(&format!("{}b.raw(mvComponent_{}({}));\n", indent, c.name, args.join(", ")));
            return;
        }
        // Unknown component: degrade gracefully so its children still render.
        out.push_str(&format!("{}b.raw(\"<div class=\\\"mv-component mv-{}\\\">\");\n", indent, c.name.to_lowercase()));
        self.gen_nodes(&c.children, out, indent);
        out.push_str(&format!("{}b.raw(\"</div>\");\n", indent));
    }

    /// Map a component prop (or its absence) to a Motoko argument of the param's
    /// declared type.
    fn prop_to_arg(&self, prop: Option<&Attr>, p: &ParamDecl) -> String {
        match prop {
            None => p.default.clone().unwrap_or_else(|| default_for_type(&p.ty)),
            Some(a) => match &a.value {
                AttrValue::Bool => "true".to_string(),
                AttrValue::Literal(v) => literal_to_typed(v, &p.ty),
                AttrValue::Expr(e) => e.clone(),
                AttrValue::Concat(parts) => {
                    let mut pieces = Vec::new();
                    for part in parts {
                        match part {
                            AttrPart::Lit(l) => pieces.push(mo_str(l)),
                            AttrPart::Expr(e) => pieces.push(self.as_text(e)),
                        }
                    }
                    format!("({})", pieces.join(" # "))
                }
            },
        }
    }

    /// Build the default-slot children into a `Text` expression (a do-block with
    /// its own builder), passed as `mvChildren` to the component.
    fn build_children_text(&self, children: &[Node]) -> String {
        if children.is_empty() {
            return "\"\"".to_string();
        }
        let mut tmp = String::new();
        self.gen_nodes(children, &mut tmp, "");
        let mut s = String::from("(do { let b = Html.Builder(); ");
        for line in tmp.lines() {
            s.push_str(line.trim());
            s.push(' ');
        }
        s.push_str("b.build() })");
        s
    }

    fn gen_builtin(&self, c: &Component, out: &mut String, indent: &str) -> Option<()> {
        let prop = |name: &str| -> Option<&Attr> { c.props.iter().find(|a| a.name == name) };
        let lit = |name: &str| -> Option<String> {
            prop(name).and_then(|a| match &a.value {
                AttrValue::Literal(v) => Some(v.clone()),
                _ => None,
            })
        };
        match c.name.as_str() {
            "Button" => {
                // Fluent uses `appearance`; `kind` kept as an alias.
                let kind = lit("appearance").or_else(|| lit("kind")).unwrap_or_else(|| "secondary".into());
                let size = lit("size");
                let mut cls = format!("mv-btn mv-btn-{}", kind);
                if let Some(sz) = size {
                    cls.push_str(&format!(" mv-btn-{}", sz));
                }
                let ty = lit("type").unwrap_or_else(|| "button".into());
                out.push_str(&format!("{}b.raw(\"<button type=\\\"{}\\\" class=\\\"{}\\\"\");\n", indent, ty, cls));
                for ev in &c.events {
                    out.push_str(&format!("{}b.raw(\" data-mv-handler=\\\"{}\\\" data-mv-event=\\\"{}\\\"\");\n", indent, ev.handler, ev.event));
                    for (i, arg) in ev.args.iter().enumerate() {
                        if is_simple_literal(arg) {
                            out.push_str(&format!("{}b.raw(\" data-mv-arg{}=\\\"{}\\\"\");\n", indent, i, arg.trim_matches('"')));
                        } else {
                            out.push_str(&format!("{}b.attr(\"data-mv-arg{}\", {});\n", indent, i, self.as_text(arg)));
                        }
                    }
                }
                out.push_str(&format!("{}b.raw(\">\");\n", indent));
                self.gen_nodes(&c.children, out, indent);
                out.push_str(&format!("{}b.raw(\"</button>\");\n", indent));
                Some(())
            }
            "Card" => {
                out.push_str(&format!("{}b.raw(\"<div class=\\\"mv-card\\\">\");\n", indent));
                if let Some(t) = prop("title") {
                    out.push_str(&format!("{}b.raw(\"<h2>\");\n", indent));
                    self.gen_attr_text(t, out, indent);
                    out.push_str(&format!("{}b.raw(\"</h2>\");\n", indent));
                }
                self.gen_nodes(&c.children, out, indent);
                out.push_str(&format!("{}b.raw(\"</div>\");\n", indent));
                Some(())
            }
            "Alert" => {
                let ty = lit("type").unwrap_or_else(|| "info".into());
                out.push_str(&format!("{}b.raw(\"<div class=\\\"mv-alert mv-alert-{}\\\">\");\n", indent, ty));
                self.gen_nodes(&c.children, out, indent);
                out.push_str(&format!("{}b.raw(\"</div>\");\n", indent));
                Some(())
            }
            "Badge" => {
                let ty = lit("type").unwrap_or_else(|| "".into());
                let cls = if ty.is_empty() { "mv-badge".to_string() } else { format!("mv-badge mv-badge-{}", ty) };
                out.push_str(&format!("{}b.raw(\"<span class=\\\"{}\\\">\");\n", indent, cls));
                self.gen_nodes(&c.children, out, indent);
                out.push_str(&format!("{}b.raw(\"</span>\");\n", indent));
                Some(())
            }
            "PageHeader" => {
                out.push_str(&format!("{}b.raw(\"<div class=\\\"mv-page-header\\\"><h1>\");\n", indent));
                if let Some(t) = prop("title") {
                    self.gen_attr_text(t, out, indent);
                }
                out.push_str(&format!("{}b.raw(\"</h1><div>\");\n", indent));
                self.gen_nodes(&c.children, out, indent);
                out.push_str(&format!("{}b.raw(\"</div></div>\");\n", indent));
                Some(())
            }
            "Table" => {
                out.push_str(&format!("{}b.raw(\"<table class=\\\"mv-table\\\">\");\n", indent));
                self.gen_nodes(&c.children, out, indent);
                out.push_str(&format!("{}b.raw(\"</table>\");\n", indent));
                Some(())
            }
            "Grid" => {
                let cols = lit("columns").unwrap_or_else(|| "3".into());
                out.push_str(&format!("{}b.raw(\"<div class=\\\"mv-grid\\\" style=\\\"--mv-cols:{}\\\">\");\n", indent, cols));
                self.gen_nodes(&c.children, out, indent);
                out.push_str(&format!("{}b.raw(\"</div>\");\n", indent));
                Some(())
            }
            "ValidationSummary" => {
                out.push_str(&format!("{}if (mvErrors.size() > 0) {{\n", indent));
                out.push_str(&format!("{}  b.raw(\"<div class=\\\"mv-validation\\\"><strong>Please fix the following:</strong><ul>\");\n", indent));
                out.push_str(&format!("{}  for ((mvF, mvM) in mvErrors.vals()) {{ b.raw(\"<li>\"); b.text(mvM); b.raw(\"</li>\") }};\n", indent));
                out.push_str(&format!("{}  b.raw(\"</ul></div>\");\n", indent));
                out.push_str(&format!("{}}};\n", indent));
                Some(())
            }
            "InputText" | "InputEmail" | "InputNumber" | "TextArea" => {
                let input_type = match c.name.as_str() {
                    "InputEmail" => "email",
                    "InputNumber" => "number",
                    _ => "text",
                };
                let label = lit("label");
                let bind = c.props.iter().find(|a| a.name == "bind").and_then(|a| match &a.value {
                    AttrValue::Expr(e) => Some(e.clone()),
                    AttrValue::Literal(v) => Some(v.trim_start_matches('@').to_string()),
                    _ => None,
                });
                let name = lit("name").or_else(|| bind.as_ref().map(|b| b.split('.').last().unwrap_or(b).to_string())).unwrap_or_default();
                let required = prop("required").is_some();
                out.push_str(&format!("{}b.raw(\"<div class=\\\"mv-field\\\">\");\n", indent));
                if let Some(l) = label {
                    out.push_str(&format!("{}b.raw(\"<label>{}{}</label>\");\n", indent, mo_attr_text(&l), if required { " *" } else { "" }));
                }
                if c.name == "TextArea" {
                    out.push_str(&format!("{}b.raw(\"<textarea class=\\\"mv-textarea\\\" name=\\\"{}\\\" data-mv-key=\\\"{}\\\"\");\n", indent, name, bind.clone().unwrap_or(name.clone())));
                    if required { out.push_str(&format!("{}b.raw(\" required\");\n", indent)); }
                    out.push_str(&format!("{}b.raw(\">\");\n", indent));
                    if let Some(bv) = &bind { out.push_str(&format!("{}b.text({});\n", indent, self.as_text(bv))); }
                    out.push_str(&format!("{}b.raw(\"</textarea>\");\n", indent));
                } else {
                    out.push_str(&format!("{}b.raw(\"<input type=\\\"{}\\\" class=\\\"mv-input\\\" name=\\\"{}\\\" data-mv-key=\\\"{}\\\"\");\n", indent, input_type, name, bind.clone().unwrap_or(name.clone())));
                    if required { out.push_str(&format!("{}b.raw(\" required\");\n", indent)); }
                    if let Some(bv) = &bind { out.push_str(&format!("{}b.attr(\"value\", {});\n", indent, self.as_text(bv))); }
                    out.push_str(&format!("{}b.raw(\">\");\n", indent));
                }
                // inline field error
                out.push_str(&format!("{}for ((mvF, mvM) in mvErrors.vals()) {{ if (mvF == \"{}\") {{ b.raw(\"<div class=\\\"mv-error\\\">\"); b.text(mvM); b.raw(\"</div>\") }} }};\n", indent, name));
                out.push_str(&format!("{}b.raw(\"</div>\");\n", indent));
                Some(())
            }
            // ---- Fluent: people ----
            "Avatar" => {
                let size = lit("size").unwrap_or_else(|| "32".into());
                let mut cls = format!("mv-avatar mv-avatar-{}", size);
                if lit("shape").as_deref() == Some("square") { cls.push_str(" mv-avatar-square"); }
                if let Some(p) = lit("presence") { cls.push_str(&format!(" mv-avatar-{}", p)); }
                out.push_str(&format!("{}b.raw(\"<span class=\\\"{}\\\">\");\n", indent, cls));
                if let Some(t) = prop("text") { self.gen_attr_text(t, out, indent); }
                self.gen_nodes(&c.children, out, indent);
                out.push_str(&format!("{}b.raw(\"</span>\");\n", indent));
                Some(())
            }
            "Persona" => {
                out.push_str(&format!("{}b.raw(\"<div class=\\\"mv-persona\\\">\");\n", indent));
                let asize = lit("size").unwrap_or_else(|| "40".into());
                let mut acls = format!("mv-avatar mv-avatar-{}", asize);
                if let Some(p) = lit("presence") { acls.push_str(&format!(" mv-avatar-{}", p)); }
                out.push_str(&format!("{}b.raw(\"<span class=\\\"{}\\\">\");\n", indent, acls));
                if let Some(a) = prop("avatar") { self.gen_attr_text(a, out, indent); }
                out.push_str(&format!("{}b.raw(\"</span><div class=\\\"mv-persona-text\\\"><span class=\\\"mv-persona-name\\\">\");\n", indent));
                if let Some(n) = prop("name") { self.gen_attr_text(n, out, indent); }
                out.push_str(&format!("{}b.raw(\"</span>\");\n", indent));
                if let Some(s) = prop("secondary") {
                    out.push_str(&format!("{}b.raw(\"<span class=\\\"mv-persona-secondary\\\">\");\n", indent));
                    self.gen_attr_text(s, out, indent);
                    out.push_str(&format!("{}b.raw(\"</span>\");\n", indent));
                }
                out.push_str(&format!("{}b.raw(\"</div></div>\");\n", indent));
                Some(())
            }
            // ---- Fluent: navigation (containers) ----
            "Nav" => {
                out.push_str(&format!("{}b.raw(\"<nav class=\\\"mv-nav\\\">\");\n", indent));
                self.gen_nodes(&c.children, out, indent);
                out.push_str(&format!("{}b.raw(\"</nav>\");\n", indent));
                Some(())
            }
            "AppBar" => {
                out.push_str(&format!("{}b.raw(\"<header class=\\\"mv-appbar\\\">\");\n", indent));
                self.gen_nodes(&c.children, out, indent);
                out.push_str(&format!("{}b.raw(\"</header>\");\n", indent));
                Some(())
            }
            "TabList" => {
                out.push_str(&format!("{}b.raw(\"<div class=\\\"mv-tablist\\\">\");\n", indent));
                self.gen_nodes(&c.children, out, indent);
                out.push_str(&format!("{}b.raw(\"</div>\");\n", indent));
                Some(())
            }
            // NavItem (side rail) + Tab (tablist): an <a> with optional active state.
            "NavItem" | "Tab" => {
                let base = if c.name == "Tab" { "mv-tab" } else { "mv-nav-item" };
                out.push_str(&format!("{}b.raw(\"<a class=\\\"{}\");\n", indent, base));
                // `match="/feed"` marks active when ctx.path matches (prefix; exact
                // for "/"). `active="@expr"` / `active` are also supported.
                if let Some(m) = lit("match") {
                    let cond = if m == "/" {
                        "ctx.path == \"/\"".to_string()
                    } else {
                        format!("Text.startsWith(ctx.path, #text \"{}\")", m)
                    };
                    out.push_str(&format!("{}if ({}) {{ b.raw(\" is-active\") }};\n", indent, cond));
                } else if let Some(a) = prop("active") {
                    match &a.value {
                        AttrValue::Expr(e) => out.push_str(&format!("{}if ({}) {{ b.raw(\" is-active\") }};\n", indent, e)),
                        _ => out.push_str(&format!("{}b.raw(\" is-active\");\n", indent)),
                    }
                }
                out.push_str(&format!("{}b.raw(\"\\\" href=\\\"\");\n", indent));
                if let Some(h) = prop("href") { self.gen_attr_text(h, out, indent); }
                out.push_str(&format!("{}b.raw(\"\\\">\");\n", indent));
                if let Some(ic) = prop("icon") {
                    out.push_str(&format!("{}b.raw(\"<span class=\\\"mv-nav-ico\\\">\");\n", indent));
                    self.gen_attr_text(ic, out, indent);
                    out.push_str(&format!("{}b.raw(\"</span>\");\n", indent));
                }
                self.gen_nodes(&c.children, out, indent);
                out.push_str(&format!("{}b.raw(\"</a>\");\n", indent));
                Some(())
            }
            // Menu: a pure-CSS details/summary popover.
            "Menu" => {
                out.push_str(&format!("{}b.raw(\"<details class=\\\"mv-menu\\\"><summary class=\\\"mv-menu-trigger\\\">\");\n", indent));
                if let Some(l) = prop("label") { self.gen_attr_text(l, out, indent); }
                out.push_str(&format!("{}b.raw(\"</summary><div class=\\\"mv-menu-list\\\">\");\n", indent));
                self.gen_nodes(&c.children, out, indent);
                out.push_str(&format!("{}b.raw(\"</div></details>\");\n", indent));
                Some(())
            }
            "MenuItem" => {
                out.push_str(&format!("{}b.raw(\"<a class=\\\"mv-menu-item\\\" href=\\\"\");\n", indent));
                if let Some(h) = prop("href") { self.gen_attr_text(h, out, indent); } else { out.push_str(&format!("{}b.raw(\"#\");\n", indent)); }
                out.push_str(&format!("{}b.raw(\"\\\">\");\n", indent));
                self.gen_nodes(&c.children, out, indent);
                out.push_str(&format!("{}b.raw(\"</a>\");\n", indent));
                Some(())
            }
            // ---- Fluent: feedback + text ----
            "Switch" => {
                let name = lit("name").unwrap_or_default();
                out.push_str(&format!("{}b.raw(\"<label class=\\\"mv-switch\\\"><input type=\\\"checkbox\\\" name=\\\"{}\\\"\");\n", indent, name));
                if prop("checked").is_some() { out.push_str(&format!("{}b.raw(\" checked\");\n", indent)); }
                out.push_str(&format!("{}b.raw(\"><span class=\\\"mv-switch-track\\\"></span>\");\n", indent));
                if let Some(l) = prop("label") { self.gen_attr_text(l, out, indent); }
                self.gen_nodes(&c.children, out, indent);
                out.push_str(&format!("{}b.raw(\"</label>\");\n", indent));
                Some(())
            }
            "Divider" => {
                let mut cls = "mv-divider".to_string();
                if lit("appearance").as_deref() == Some("strong") { cls.push_str(" mv-divider-strong"); }
                if prop("vertical").is_some() { cls.push_str(" mv-divider-vertical"); }
                out.push_str(&format!("{}b.raw(\"<div class=\\\"{}\\\">\");\n", indent, cls));
                self.gen_nodes(&c.children, out, indent);
                out.push_str(&format!("{}b.raw(\"</div>\");\n", indent));
                Some(())
            }
            "Spinner" => {
                let size = lit("size").map(|s| format!(" mv-spinner-{}", s)).unwrap_or_default();
                out.push_str(&format!("{}b.raw(\"<span class=\\\"mv-spinner{}\\\"></span>\");\n", indent, size));
                Some(())
            }
            "MessageBar" => {
                let ty = lit("intent").or_else(|| lit("type")).unwrap_or_else(|| "info".into());
                out.push_str(&format!("{}b.raw(\"<div class=\\\"mv-alert mv-alert-{}\\\">\");\n", indent, ty));
                self.gen_nodes(&c.children, out, indent);
                out.push_str(&format!("{}b.raw(\"</div>\");\n", indent));
                Some(())
            }
            "Text" => {
                let v = lit("variant").unwrap_or_else(|| "body1".into());
                out.push_str(&format!("{}b.raw(\"<span class=\\\"fui-text-{}\\\">\");\n", indent, v));
                self.gen_nodes(&c.children, out, indent);
                out.push_str(&format!("{}b.raw(\"</span>\");\n", indent));
                Some(())
            }
            "Link" => {
                let mut cls = "mv-link".to_string();
                if prop("subtle").is_some() { cls.push_str(" mv-link-subtle"); }
                out.push_str(&format!("{}b.raw(\"<a class=\\\"{}\\\" href=\\\"\");\n", indent, cls));
                if let Some(h) = prop("href") { self.gen_attr_text(h, out, indent); }
                out.push_str(&format!("{}b.raw(\"\\\">\");\n", indent));
                self.gen_nodes(&c.children, out, indent);
                out.push_str(&format!("{}b.raw(\"</a>\");\n", indent));
                Some(())
            }
            _ => None,
        }
    }

    fn gen_attr_text(&self, a: &Attr, out: &mut String, indent: &str) {
        match &a.value {
            AttrValue::Literal(v) => out.push_str(&format!("{}b.text({});\n", indent, mo_str(v))),
            AttrValue::Expr(e) => out.push_str(&format!("{}b.text({});\n", indent, self.as_text(e))),
            AttrValue::Concat(parts) => {
                for p in parts {
                    match p {
                        AttrPart::Lit(l) => out.push_str(&format!("{}b.text({});\n", indent, mo_str(l))),
                        AttrPart::Expr(e) => out.push_str(&format!("{}b.text({});\n", indent, self.as_text(e))),
                    }
                }
            }
            AttrValue::Bool => {}
        }
    }

    fn gen_head_extra(&self, file: &MviewFile) -> String {
        // Render @section "head" nodes (if any) into a Text literal expression.
        let base = if let Some((_n, nodes)) = file.sections.iter().find(|(n, _)| n == "head") {
            let mut body = String::new();
            body.push_str("(do { let b = Html.Builder(); ");
            let mut tmp = String::new();
            self.gen_nodes(nodes, &mut tmp, "");
            // inline (strip newlines/indent)
            for line in tmp.lines() {
                body.push_str(line.trim());
                body.push(' ');
            }
            body.push_str("b.build() })");
            body
        } else {
            "\"\"".to_string()
        };
        // A page-level @theme rides into <head> as a <style> via head.extra.
        match self.theme_style_css(file) {
            Some(style) => format!("({} # {})", mo_str(&style), base),
            None => base,
        }
    }

    /// Build the `<style>:root{…}</style>` for a file's `@theme` (a preset and/or
    /// token overrides), or None when the file declares no theme. Overrides win.
    fn theme_style_css(&self, file: &MviewFile) -> Option<String> {
        let mut out = String::new();
        // @theme brand="#hex" generates the full Fluent brand ramp + light/dark aliases.
        if let Some(brand) = &file.theme_brand {
            if let Some(css) = crate::color::brand_theme_css(brand) {
                out.push_str(&css);
            }
        }
        // @theme "preset" and/or { token overrides }.
        if file.theme_preset.is_some() || !file.theme.is_empty() {
            let mut tokens: Vec<(String, String)> = Vec::new();
            if let Some(p) = &file.theme_preset {
                for (k, v) in theme_preset(p) {
                    tokens.push((k.to_string(), v.to_string()));
                }
            }
            for (k, v) in &file.theme {
                if let Some(slot) = tokens.iter_mut().find(|(tk, _)| tk == k) {
                    slot.1 = v.clone();
                } else {
                    tokens.push((k.clone(), v.clone()));
                }
            }
            if !tokens.is_empty() {
                let body: String = tokens.iter().map(|(k, v)| format!("{}:{};", k, v)).collect();
                out.push_str(&format!("<style>:root{{{}}}</style>", body));
            }
        }
        if out.is_empty() { None } else { Some(out) }
    }

    // ---- expression -> Text -----------------------------------------------
    /// Wrap a Motoko expression so it produces escaped display `Text`.
    fn as_text(&self, expr: &str) -> String {
        let e = expr.trim();
        // string literal -> already Text
        if e.starts_with('"') {
            return e.to_string();
        }
        // View.* in layouts maps to mvHead fields
        if self.is_layout {
            if let Some(rest) = e.strip_prefix("View.") {
                return format!("mvHead.{}", rest);
            }
        }
        match self.infer_type(e).as_deref() {
            Some("Text") => e.to_string(),
            Some("Nat") => format!("Nat.toText({})", e),
            Some("Int") => format!("Int.toText({})", e),
            Some("Float") => format!("Float.toText({})", e),
            Some("Bool") => format!("(if ({}) \"true\" else \"false\")", e),
            _ => {
                // numeric literal?
                if e.chars().all(|c| c.is_ascii_digit()) && !e.is_empty() {
                    e.to_string()
                } else {
                    // Type unknown (e.g. a record field on a loop variable):
                    // debug_show then strip wrapping quotes so Text displays cleanly.
                    format!("Html.unquote(debug_show({}))", e)
                }
            }
        }
    }

    fn infer_type(&self, e: &str) -> Option<String> {
        let e = e.trim();
        if let Some(t) = self.types.borrow().get(e) {
            return Some(t.clone());
        }
        // member access `base.field[.field...]` — resolve through scanned types.
        if let Some(dot) = e.find('.') {
            let base = &e[..dot];
            let path = &e[dot + 1..];
            // base must be a typed value (var / param / loop var), not a module.
            if let Some(bt) = self.types.borrow().get(base).cloned() {
                return self.field_type(&bt, path);
            }
        }
        // function call f(...)
        if let Some(open) = e.find('(') {
            let f = &e[..open];
            if let Some(t) = self.types.borrow().get(f) {
                return Some(t.clone());
            }
        }
        None
    }

    /// The element type of an iterable expression: `[T]` -> `T`. Used to type a
    /// `@for x in expr` loop variable so `@x.field` resolves precisely.
    fn element_type(&self, iter: &str) -> Option<String> {
        let t = self.infer_type(iter.trim())?;
        let t = t.trim();
        if t.starts_with('[') && t.ends_with(']') {
            Some(t[1..t.len() - 1].trim().to_string())
        } else {
            None
        }
    }

    /// Resolve `path` (e.g. "name" or "author.handle") against record type `ty`.
    fn field_type(&self, ty: &str, path: &str) -> Option<String> {
        let (field, rest) = match path.find('.') {
            Some(d) => (&path[..d], Some(&path[d + 1..])),
            None => (path, None),
        };
        let ft = self.lookup_type(ty)?.get(field)?.clone();
        match rest {
            Some(r) => self.field_type(&ft, r),
            None => Some(ft),
        }
    }

    /// The scanned field map for a record type, tolerating an `?` optional and a
    /// `Module.Type` qualifier (falls back to the bare type name).
    fn lookup_type(&self, ty: &str) -> Option<&'a HashMap<String, String>> {
        let ty = ty.trim().trim_start_matches('?').trim();
        if let Some(m) = self.models.get(ty) {
            return Some(m);
        }
        ty.rfind('.').and_then(|d| self.models.get(&ty[d + 1..]))
    }
}

// ---- free helpers ---------------------------------------------------------

/// Escape text into a Motoko string literal (including the surrounding quotes).
pub fn mo_str(s: &str) -> String {
    let mut out = String::from("\"");
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => {}
            '\t' => out.push_str("\\t"),
            _ => out.push(c),
        }
    }
    out.push('"');
    out
}

/// Escape text for embedding inside a larger Motoko-string-literal context
/// (no surrounding quotes; backslash-escape quotes/backslashes).
fn mo_attr_text(s: &str) -> String {
    let mut out = String::new();
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => {}
            _ => out.push(c),
        }
    }
    out
}

/// Escape a value for embedding inside a Motoko/HTML double-quoted context
/// (no surrounding quotes added).
fn escape_mo_inner(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"")
}

/// Parse route params from a path: `/orders/{id:Nat}/{tab}` ->
/// [("id","Nat"), ("tab","Text")]. Untyped params default to Text.
fn parse_route_params(route: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let mut rest = route;
    while let Some(open) = rest.find('{') {
        rest = &rest[open + 1..];
        if let Some(close) = rest.find('}') {
            let inner = &rest[..close];
            let (name, ty) = match inner.split_once(':') {
                Some((n, t)) => (n.trim().to_string(), t.trim().to_string()),
                None => (inner.trim().to_string(), "Text".to_string()),
            };
            if !name.is_empty() {
                out.push((name, ty));
            }
            rest = &rest[close + 1..];
        } else {
            break;
        }
    }
    out
}

/// Default Motoko value for a component param type when no prop is given and the
/// param declared no default.
fn default_for_type(ty: &str) -> String {
    match ty.trim() {
        "Text" => "\"\"".to_string(),
        "Bool" => "false".to_string(),
        "Nat" | "Nat8" | "Nat16" | "Nat32" | "Nat64" | "Int" => "0".to_string(),
        "Float" => "0.0".to_string(),
        _ => "\"\"".to_string(),
    }
}

/// Convert a literal prop value to a Motoko expression of the param's type.
fn literal_to_typed(v: &str, ty: &str) -> String {
    match ty.trim() {
        "Bool" => {
            if v == "true" { "true".to_string() } else { "false".to_string() }
        }
        "Nat" | "Nat8" | "Nat16" | "Nat32" | "Nat64" | "Int" | "Float" => v.to_string(),
        _ => mo_str(v),
    }
}

fn is_simple_literal(s: &str) -> bool {
    let s = s.trim();
    (!s.is_empty() && s.chars().all(|c| c.is_ascii_digit())) || (s.starts_with('"') && s.ends_with('"'))
}

fn convert_from_text(ty: &str, access: &str) -> String {
    match ty.trim() {
        "Text" => access.to_string(),
        "Nat" | "Nat8" | "Nat16" | "Nat32" | "Nat64" => format!("mvNat({})", access),
        "Int" => format!("mvInt({})", access),
        "Bool" => format!("({} == \"true\")", access),
        _ => access.to_string(),
    }
}

/// Collect the names of functions referenced as event handlers in the template
/// (events `@click`/`@submit`/`@input`/`@change` and `data-mv-drop` targets).
fn collect_handlers(nodes: &[Node]) -> HashSet<String> {
    let mut out = HashSet::new();
    walk_handlers(nodes, &mut out);
    out
}

fn walk_handlers(nodes: &[Node], out: &mut HashSet<String>) {
    for n in nodes {
        match n {
            Node::Element(e) => {
                for ev in &e.events {
                    out.insert(ev.handler.clone());
                }
                for a in &e.attrs {
                    if a.name == "data-mv-drop" {
                        if let AttrValue::Literal(v) = &a.value {
                            out.insert(v.clone());
                        }
                    }
                }
                walk_handlers(&e.children, out);
            }
            Node::Component(c) => {
                for ev in &c.events {
                    out.insert(ev.handler.clone());
                }
                walk_handlers(&c.children, out);
                for (_n, body) in &c.slots {
                    walk_handlers(body, out);
                }
            }
            Node::If(branches) => {
                for br in branches {
                    walk_handlers(&br.body, out);
                }
            }
            Node::For { body, .. } => walk_handlers(body, out),
            Node::Switch { cases, .. } => {
                for c in cases {
                    walk_handlers(&c.body, out);
                }
            }
            _ => {}
        }
    }
}

/// Collect `bind="@var"` targets that are simple (dotless) lvalues, paired with
/// their submitted field name. Used to wire two-way binding in dispatch.
fn collect_simple_binds(nodes: &[Node]) -> Vec<(String, String)> {
    let mut out = Vec::new();
    walk_binds(nodes, &mut out);
    // de-dup
    out.sort();
    out.dedup();
    out
}

fn walk_binds(nodes: &[Node], out: &mut Vec<(String, String)>) {
    for n in nodes {
        match n {
            Node::Element(e) => {
                if let Some(lv) = &e.bind {
                    if !lv.contains('.') {
                        out.push((lv.clone(), lv.clone()));
                    }
                }
                walk_binds(&e.children, out);
            }
            Node::Component(c) => {
                let bind = c.props.iter().find(|a| a.name == "bind").and_then(|a| match &a.value {
                    AttrValue::Expr(e) => Some(e.clone()),
                    AttrValue::Literal(v) => Some(v.trim_start_matches('@').to_string()),
                    _ => None,
                });
                if let Some(lv) = bind {
                    if !lv.contains('.') {
                        let name = c
                            .props
                            .iter()
                            .find(|a| a.name == "name")
                            .and_then(|a| match &a.value {
                                AttrValue::Literal(v) => Some(v.clone()),
                                _ => None,
                            })
                            .unwrap_or_else(|| lv.clone());
                        out.push((lv, name));
                    }
                }
                walk_binds(&c.children, out);
            }
            Node::If(branches) => {
                for br in branches {
                    walk_binds(&br.body, out);
                }
            }
            Node::For { body, .. } => walk_binds(body, out),
            Node::Switch { cases, .. } => {
                for c in cases {
                    walk_binds(&c.body, out);
                }
            }
            _ => {}
        }
    }
}

fn collect_form_schema(el: &Element) -> String {
    let mut names = Vec::new();
    collect_input_names(&el.children, &mut names);
    names.sort();
    names.join(",")
}

fn collect_input_names(nodes: &[Node], names: &mut Vec<String>) {
    for n in nodes {
        match n {
            Node::Element(e) => {
                if let Some(b) = &e.bind {
                    names.push(b.split('.').last().unwrap_or(b).to_string());
                } else if let Some(a) = e.attrs.iter().find(|a| a.name == "name") {
                    if let AttrValue::Literal(v) = &a.value {
                        names.push(v.clone());
                    }
                }
                collect_input_names(&e.children, names);
            }
            Node::Component(c) => {
                // built-in inputs declare name= or bind=
                let bind = c.props.iter().find(|a| a.name == "bind").and_then(|a| match &a.value {
                    AttrValue::Expr(e) => Some(e.split('.').last().unwrap_or(e).to_string()),
                    AttrValue::Literal(v) => Some(v.trim_start_matches('@').split('.').last().unwrap_or(v).to_string()),
                    _ => None,
                });
                let name = c.props.iter().find(|a| a.name == "name").and_then(|a| match &a.value {
                    AttrValue::Literal(v) => Some(v.clone()),
                    _ => None,
                });
                if let Some(nm) = name.or(bind) {
                    names.push(nm);
                }
                collect_input_names(&c.children, names);
            }
            Node::If(branches) => {
                for br in branches {
                    collect_input_names(&br.body, names);
                }
            }
            Node::For { body, .. } => collect_input_names(body, names),
            _ => {}
        }
    }
}

/// Translate `validate TARGET { rules }` inside a function body into Motoko.
fn translate_validate(body: &str) -> String {
    let chars: Vec<char> = body.chars().collect();
    let mut out = String::new();
    let mut i = 0;
    let n = chars.len();
    while i < n {
        // detect the word "validate" at a boundary
        if chars[i..].iter().collect::<String>().starts_with("validate")
            && (i == 0 || !chars[i - 1].is_alphanumeric())
        {
            let mut j = i + "validate".len();
            // whitespace
            while j < n && chars[j].is_whitespace() {
                j += 1;
            }
            // target ident
            let mut target = String::new();
            while j < n && (chars[j].is_alphanumeric() || chars[j] == '_' || chars[j] == '.') {
                target.push(chars[j]);
                j += 1;
            }
            while j < n && chars[j].is_whitespace() {
                j += 1;
            }
            if j < n && chars[j] == '{' {
                // capture block
                j += 1;
                let mut depth = 1;
                let start = j;
                while j < n && depth > 0 {
                    if chars[j] == '{' {
                        depth += 1;
                    } else if chars[j] == '}' {
                        depth -= 1;
                        if depth == 0 {
                            break;
                        }
                    }
                    j += 1;
                }
                let block: String = chars[start..j].iter().collect();
                j += 1; // closing }
                // skip optional ';'
                while j < n && chars[j].is_whitespace() {
                    j += 1;
                }
                if j < n && chars[j] == ';' {
                    j += 1;
                }
                out.push_str(&gen_validate_block(&target, &block));
                i = j;
                continue;
            }
        }
        out.push(chars[i]);
        i += 1;
    }
    out
}

fn gen_validate_block(target: &str, block: &str) -> String {
    let mut s = String::from(" mvErrors.clear(); ");
    for rule in block.split(';') {
        let rule = rule.trim();
        if rule.is_empty() {
            continue;
        }
        // FIELD CHECK [ARG] ["message"]
        let (head, msg) = match rule.find('"') {
            Some(q) => (rule[..q].trim().to_string(), rule[q..].trim().to_string()),
            None => (rule.to_string(), String::new()),
        };
        let toks: Vec<&str> = head.split_whitespace().collect();
        if toks.len() < 2 {
            continue;
        }
        let field = toks[0];
        let check = toks[1];
        let msg_expr = if msg.is_empty() {
            format!("\"{} is invalid\"", field)
        } else {
            msg
        };
        let path = if target.is_empty() {
            field.to_string()
        } else {
            format!("{}.{}", target, field)
        };
        let cond_fail = match check {
            "required" => format!("({} == \"\")", path),
            "email" => format!("(not mvIsEmail({}))", path),
            "minLength" => {
                let narg = toks.get(2).copied().unwrap_or("1");
                format!("({}.size() < {})", path, narg)
            }
            "min" => {
                let nargs = toks.get(2).copied().unwrap_or("1");
                format!("({} < {})", path, nargs)
            }
            "max" => {
                let nargs = toks.get(2).copied().unwrap_or("0");
                format!("({} > {})", path, nargs)
            }
            _ => "false".to_string(),
        };
        s.push_str(&format!("if {} {{ mvErrors.add((\"{}\", {})) }}; ", cond_fail, field, msg_expr));
    }
    s.push_str("if (mvErrors.size() > 0) { return }; ");
    s
}

/// Built-in theme presets — the shareable "theme packages" applied with
/// `@theme "name"`. Each returns token overrides on top of the base CSS tokens.
/// Palettes are WCAG-AA contrast-checked. Unknown name -> no tokens.
pub fn theme_preset(name: &str) -> Vec<(&'static str, &'static str)> {
    match name {
        "midnight" => vec![
            ("--mv-primary", "#8b7cf6"), ("--mv-primary-600", "#6d5ce0"), ("--mv-primary-fg", "#0b0a1a"),
            ("--mv-bg", "#0b0b12"), ("--mv-surface", "#13131d"), ("--mv-muted", "#1c1c2b"),
            ("--mv-border", "#2a2a3d"), ("--mv-text", "#e9e9f2"), ("--mv-text-soft", "#9a9ab0"),
            ("--mv-success", "#34d399"), ("--mv-danger", "#f87171"), ("--mv-warning", "#fbbf24"),
            ("--mv-shadow", "0 1px 2px rgba(0,0,0,.45), 0 8px 24px rgba(0,0,0,.55)"),
        ],
        "ocean" => vec![
            ("--mv-primary", "#0e76a0"), ("--mv-primary-600", "#0a5876"), ("--mv-primary-fg", "#ffffff"),
            ("--mv-bg", "#f4f7fa"), ("--mv-surface", "#ffffff"), ("--mv-muted", "#eef3f8"),
            ("--mv-border", "#d4e0eb"), ("--mv-text", "#0f2433"), ("--mv-text-soft", "#48637a"),
            ("--mv-success", "#0a7d44"), ("--mv-danger", "#cf2f2f"), ("--mv-warning", "#9a5d00"),
            ("--mv-shadow", "0 1px 2px rgba(13,42,66,.06), 0 8px 24px rgba(13,42,66,.08)"),
        ],
        "forest" => vec![
            ("--mv-primary", "#2f7d4f"), ("--mv-primary-600", "#236340"), ("--mv-primary-fg", "#ffffff"),
            ("--mv-bg", "#f7f5ef"), ("--mv-surface", "#fffefb"), ("--mv-muted", "#eeeae0"),
            ("--mv-border", "#ddd6c6"), ("--mv-text", "#23271f"), ("--mv-text-soft", "#5c6253"),
            ("--mv-success", "#2f7d4f"), ("--mv-danger", "#b3261e"), ("--mv-warning", "#9c5708"),
            ("--mv-shadow", "0 1px 2px rgba(43,40,25,.06), 0 8px 24px rgba(43,40,25,.07)"),
        ],
        "sunset" => vec![
            ("--mv-primary", "#c93f15"), ("--mv-primary-600", "#a82f0c"), ("--mv-primary-fg", "#ffffff"),
            ("--mv-bg", "#fffaf5"), ("--mv-surface", "#ffffff"), ("--mv-muted", "#fdeede"),
            ("--mv-border", "#f3ddc7"), ("--mv-text", "#2b1810"), ("--mv-text-soft", "#80614f"),
            ("--mv-success", "#15803d"), ("--mv-danger", "#c2300f"), ("--mv-warning", "#b45309"),
            ("--mv-shadow", "0 1px 2px rgba(120,53,15,.06), 0 8px 24px rgba(120,53,15,.10)"),
        ],
        "slate" => vec![
            ("--mv-primary", "#4f6080"), ("--mv-primary-600", "#3a4861"), ("--mv-primary-fg", "#ffffff"),
            ("--mv-bg", "#fbfbfc"), ("--mv-surface", "#ffffff"), ("--mv-muted", "#f1f3f6"),
            ("--mv-border", "#e2e5ec"), ("--mv-text", "#1a1d24"), ("--mv-text-soft", "#5b6373"),
            ("--mv-success", "#2f7a4f"), ("--mv-danger", "#c23b3b"), ("--mv-warning", "#9c5d12"),
            ("--mv-shadow", "0 1px 2px rgba(20,24,38,.05), 0 8px 24px rgba(20,24,38,.05)"),
        ],
        _ => vec![],
    }
}
