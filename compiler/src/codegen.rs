//! Motoko code generation from a parsed `.mview` AST.
//!
//! A page compiles to a Motoko `object` holding its state + handlers + a
//! `render` that builds HTML via `Html.Builder`. The project orchestrator
//! wires all pages/layouts into one actor exposing http_request[_update].

use crate::ast::*;
use std::collections::HashMap;

pub struct PageGen {
    pub name: String,
    pub object_block: String,
    pub page_record: String,
    pub route: String,
}

pub struct Codegen<'a> {
    types: HashMap<String, String>, // name -> Motoko type (vars, params, func returns)
    is_layout: bool,
    _models: &'a HashMap<String, HashMap<String, String>>,
}

impl<'a> Codegen<'a> {
    pub fn new(models: &'a HashMap<String, HashMap<String, String>>) -> Self {
        Codegen {
            types: HashMap::new(),
            is_layout: false,
            _models: models,
        }
    }

    fn build_type_env(&mut self, code: &CodeBlock) {
        self.types.clear();
        for v in &code.vars {
            if let Some(t) = &v.ty {
                self.types.insert(v.name.clone(), t.clone());
            }
        }
        for p in &code.params {
            self.types.insert(p.name.clone(), p.ty.clone());
        }
        for f in &code.funcs {
            if let Some(r) = &f.ret {
                if r != "()" && !r.is_empty() {
                    self.types.insert(f.name.clone(), r.clone());
                }
            }
        }
    }

    // ---- page generation --------------------------------------------------
    pub fn gen_page(&mut self, file: &MviewFile) -> PageGen {
        self.is_layout = false;
        self.build_type_env(&file.code);
        let obj = format!("{}Page", file.name);

        let mut s = String::new();
        s.push_str(&format!("  // ===== Page: {} ({}) =====\n", file.name, file.route.clone().unwrap_or_default()));
        s.push_str(&format!("  let {} = object {{\n", obj));

        // state
        for v in &file.code.vars {
            s.push_str(&format!("    {}\n", v.raw));
        }
        // extras (let bindings, helper types)
        for e in &file.code.extra {
            s.push_str(&format!("    {}\n", e));
        }
        // error + redirect sinks
        s.push_str("    let mvErrors = Buffer.Buffer<(Text, Text)>(0);\n");
        s.push_str("    var mvRedirect : Text = \"\";\n");
        // user functions (async stripped) with validate translated
        for f in &file.code.funcs {
            let params = f
                .params
                .iter()
                .map(|(n, t)| format!("{} : {}", n, t))
                .collect::<Vec<_>>()
                .join(", ");
            let body = translate_validate(&f.body);
            s.push_str(&format!("    func {}({}) {{{}}};\n", f.name, params, body));
        }

        // render
        s.push_str("    public func mvRender(ctx : MV.Ctx) : Text {\n");
        s.push_str("      let b = Html.Builder();\n");
        s.push_str("      ignore ctx;\n");
        let mut body = String::new();
        self.gen_nodes(&file.template, &mut body, "      ");
        s.push_str(&body);
        s.push_str("      b.build();\n    };\n");

        // title / description / head
        let title_expr = file.title.clone().unwrap_or_else(|| "\"\"".into());
        s.push_str(&format!(
            "    public func mvTitle(ctx : MV.Ctx) : Text {{ ignore ctx; {} }};\n",
            self.as_text(&title_expr)
        ));
        let desc_expr = file.description.clone().unwrap_or_else(|| "\"\"".into());
        let canon_expr = file.canonical.clone().unwrap_or_else(|| "\"\"".into());
        // head extra (from @section "head")
        let head_extra = self.gen_head_extra(file);
        s.push_str(&format!(
            "    public func mvHead(ctx : MV.Ctx) : MV.Head {{ ignore ctx; {{ title = {}; description = {}; canonical = {}; extra = {} }} }};\n",
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
                s.push_str("    public func mvOnLoad(ctx : MV.Ctx) { onLoad(ctx) };\n");
            } else {
                s.push_str("    public func mvOnLoad(ctx : MV.Ctx) { ignore ctx; onLoad() };\n");
            }
        } else {
            s.push_str("    public func mvOnLoad(ctx : MV.Ctx) { ignore ctx };\n");
        }

        // dispatch
        s.push_str("    public func mvDispatch(ctx : MV.Ctx, mvH : Text, mvArgs : [Text]) {\n");
        s.push_str("      ignore ctx; ignore mvArgs;\n");
        s.push_str("      mvErrors.clear(); // each interaction starts with a clean slate\n");
        // two-way binding: populate bound vars from the submitted form
        let binds = collect_simple_binds(&file.template);
        for (lvalue, name) in &binds {
            s.push_str(&format!(
                "      switch (mvFormGet(ctx, \"{}\")) {{ case (?mvV) {{ {} := mvV }}; case null {{}} }};\n",
                name, lvalue
            ));
        }
        s.push_str("      switch mvH {\n");
        for f in &file.code.funcs {
            if f.name == "onLoad" {
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
        s.push_str("  };\n");

        // Page record
        let route = file.route.clone().unwrap_or_else(|| "/".into());
        let layout = file.layout.clone().unwrap_or_default();
        let (authorize, role) = match &file.authorize {
            Some(a) => ("true", a.role.clone().unwrap_or_default()),
            None => ("false", String::new()),
        };
        let rec = format!(
            "  let {n}Def : MV.Page = {{\n    route = {route:?};\n    layout = {layout:?};\n    authorize = {auth};\n    role = {role:?};\n    onLoad = {o}.mvOnLoad;\n    render = {o}.mvRender;\n    title = {o}.mvTitle;\n    head = {o}.mvHead;\n    dispatch = {o}.mvDispatch;\n    takeErrors = {o}.mvTakeErrors;\n    takeRedirect = {o}.mvTakeRedirect;\n  }};\n",
            n = file.name,
            route = route,
            layout = layout,
            auth = authorize,
            role = role,
            o = obj,
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
        for (idx, (_n, t)) in f.params.iter().enumerate() {
            // first ctx param? if a func takes ctx, we still pass args; but most
            // handlers take typed value args bound to mvArgs[idx].
            let access = format!("(if (mvArgs.size() > {i}) mvArgs[{i}] else \"\")", i = idx);
            args.push(convert_from_text(t, &access));
        }
        format!("{}({})", f.name, args.join(", "))
    }

    // ---- layout generation ------------------------------------------------
    pub fn gen_layout(&mut self, file: &MviewFile) -> String {
        self.is_layout = true;
        self.build_type_env(&file.code);
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
                out.push_str(&format!("{}b.text({});\n", indent, self.as_text(e)));
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
            }
            Node::SectionRef(_) => {
                // sections other than the default body are not wired in the MVP layout
            }
            Node::Slot(_) => {}
            Node::Effect { .. } => {
                // effects in templates are emitted as data markers picked up by the client.
                // (MVP: effects are primarily delivered via the batch; template effects are no-ops here.)
            }
            Node::If(branches) => {
                for (k, br) in branches.iter().enumerate() {
                    match &br.cond {
                        Some(c) => {
                            let kw = if k == 0 { "if" } else { "else if" };
                            out.push_str(&format!("{}{} ({}) {{\n", indent, kw, c));
                        }
                        None => {
                            out.push_str(&format!("{}else {{\n", indent));
                        }
                    }
                    self.gen_nodes(&br.body, out, &format!("{}  ", indent));
                    out.push_str(&format!("{}}};\n", indent));
                }
            }
            Node::For { var, iter, body } => {
                out.push_str(&format!("{}for ({} in ({}).vals()) {{\n", indent, var, iter));
                self.gen_nodes(body, out, &format!("{}  ", indent));
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
                    let schema = collect_form_schema(el);
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
        match &a.value {
            AttrValue::Bool => {
                out.push_str(&format!("{}b.raw(\" {}\");\n", indent, a.name));
            }
            AttrValue::Literal(v) => {
                out.push_str(&format!("{}b.raw({});\n", indent, mo_str(&format!(" {}=\"{}\"", a.name, v))));
            }
            AttrValue::Expr(e) => {
                out.push_str(&format!("{}b.attr({:?}, {});\n", indent, a.name, self.as_text(e)));
            }
            AttrValue::Concat(parts) => {
                out.push_str(&format!("{}b.raw(\" {}=\\\"\");\n", indent, a.name));
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
        // Built-in semantic components compile to plain HTML.
        if let Some(()) = self.gen_builtin(c, out, indent) {
            return;
        }
        // Unknown / app components: render default content inside a tagged div
        // (full app-component compilation is supported by compiling Components/*.mview;
        //  here we degrade gracefully so unknown tags still render their children).
        out.push_str(&format!("{}b.raw(\"<div class=\\\"mv-component mv-{}\\\">\");\n", indent, c.name.to_lowercase()));
        self.gen_nodes(&c.children, out, indent);
        out.push_str(&format!("{}b.raw(\"</div>\");\n", indent));
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
                let kind = lit("kind").unwrap_or_else(|| "secondary".into());
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
        if let Some((_n, nodes)) = file.sections.iter().find(|(n, _)| n == "head") {
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
        }
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
                    format!("debug_show({})", e)
                }
            }
        }
    }

    fn infer_type(&self, e: &str) -> Option<String> {
        if let Some(t) = self.types.get(e) {
            return Some(t.clone());
        }
        // function call f(...)
        if let Some(open) = e.find('(') {
            let f = &e[..open];
            if let Some(t) = self.types.get(f) {
                return Some(t.clone());
            }
        }
        None
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
