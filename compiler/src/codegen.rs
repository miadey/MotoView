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

/// Motoko reserved keywords — a component param named one of these would generate
/// invalid Motoko (`func f(label : Text)`), so we auto-mangle it (see
/// [`mangle_param`]). Kept in sync with the moc lexer's keyword set.
const MOTOKO_KEYWORDS: &[&str] = &[
    "actor", "and", "async", "assert", "await", "break", "case", "catch", "class", "continue",
    "debug", "debug_show", "do", "else", "false", "finally", "flexible", "for", "func", "if",
    "ignore", "import", "in", "module", "not", "null", "object", "or", "label", "let", "loop",
    "private", "public", "query", "return", "shared", "stable", "switch", "system", "throw",
    "to_candid", "from_candid", "true", "try", "type", "var", "while", "with", "composite", "prim",
];

pub fn is_motoko_keyword(name: &str) -> bool {
    MOTOKO_KEYWORDS.contains(&name)
}

/// The safe Motoko identifier to use for a component param: the name itself, or
/// `mvP_<name>` if it collides with a Motoko keyword. The `mvP_` prefix is
/// guaranteed non-reserved and is namespaced to avoid clashing with author code.
pub fn mangle_param(name: &str) -> String {
    if is_motoko_keyword(name) { format!("mvP_{}", name) } else { name.to_string() }
}

/// Replace whole-identifier occurrences of `from` with `to` in generated Motoko,
/// but ONLY in real code positions: skips string literals, char literals, and
/// line/block comments (so HTML text inside `b.raw("…")` is never touched), and
/// skips field accesses (`obj.from` — the `.from` is a field name, not the param).
/// Used to rewrite reserved-word param references after they've been emitted.
pub fn replace_ident_in_code(src: &str, from: &str, to: &str) -> String {
    let chars: Vec<char> = src.chars().collect();
    let n = chars.len();
    let mut out = String::with_capacity(src.len() + 8);
    let mut i = 0usize;
    let mut block_depth = 0usize; // Motoko block comments nest
    while i < n {
        let c = chars[i];
        if block_depth > 0 {
            if c == '*' && i + 1 < n && chars[i + 1] == '/' { block_depth -= 1; out.push('*'); out.push('/'); i += 2; continue; }
            if c == '/' && i + 1 < n && chars[i + 1] == '*' { block_depth += 1; out.push('/'); out.push('*'); i += 2; continue; }
            out.push(c); i += 1; continue;
        }
        if c == '/' && i + 1 < n && chars[i + 1] == '*' { block_depth += 1; out.push('/'); out.push('*'); i += 2; continue; }
        if c == '/' && i + 1 < n && chars[i + 1] == '/' {
            while i < n && chars[i] != '\n' { out.push(chars[i]); i += 1; }
            continue;
        }
        if c == '"' || c == '\'' {
            let quote = c;
            out.push(c); i += 1;
            while i < n {
                let d = chars[i]; out.push(d); i += 1;
                if d == '\\' && i < n { out.push(chars[i]); i += 1; continue; }
                if d == quote { break; }
            }
            continue;
        }
        if c.is_alphabetic() || c == '_' {
            let start = i;
            while i < n && (chars[i].is_alphanumeric() || chars[i] == '_') { i += 1; }
            let word: String = chars[start..i].iter().collect();
            let prev = if start == 0 { ' ' } else { chars[start - 1] };
            if word == from && prev != '.' { out.push_str(to); } else { out.push_str(&word); }
            continue;
        }
        out.push(c); i += 1;
    }
    out
}

/// Which backend the node walker emits. `Html` is the default and is BYTE-
/// IDENTICAL to the original single-path codegen; `Ir` emits a portable
/// `Ir.Builder` UINode tree (see runtime/src/Ir.mo) for native renderers.
///
/// In `Ir` mode the generated render uses an `ir` builder instead of `b`, and
/// builtins/charts that are not yet IR-modeled fall back to an `ir.raw(html)`
/// leaf carrying the EXACT HTML the `Html` backend would emit — so output is
/// never wrong, only not-yet-native.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum EmitMode {
    Html,
    Ir,
}

impl Default for EmitMode {
    fn default() -> Self {
        EmitMode::Html
    }
}

pub struct Codegen<'a> {
    // name -> Motoko type (vars, params, func returns, and scoped @for loop
    // vars). RefCell so loop-var types can be pushed/popped during the otherwise
    // `&self` render walk.
    types: std::cell::RefCell<HashMap<String, String>>,
    // Active emit backend. Defaults to Html (byte-identical to the original).
    emit: EmitMode,
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
    // Opt-in observability (R7). When true, `gen_page` wraps each event handler
    // in the generated `mvDispatch` with a structured `Debug.print` line (a
    // stable, parseable `MV|dispatch|...` record) plus an instruction-cost
    // delta from `ExperimentalInternetComputer.performanceCounter`. DEFAULT is
    // false, in which case the generated dispatch is BYTE-IDENTICAL to the
    // legacy path (no Debug.print, no perf counter, `ignore ctx`). The studio
    // log parser (tools/studio/log-parser.js) consumes the emitted format.
    instrument: bool,
}

impl<'a> Codegen<'a> {
    pub fn new(
        models: &'a HashMap<String, HashMap<String, String>>,
        components: &'a HashMap<String, CompInfo>,
    ) -> Self {
        Codegen {
            types: std::cell::RefCell::new(HashMap::new()),
            emit: EmitMode::Html,
            is_layout: false,
            is_component: false,
            layout_theme: None,
            models,
            components,
            instrument: false,
        }
    }

    /// Like [`new`] but selects the emit backend. `EmitMode::Html` reproduces
    /// the default (byte-identical) path; `EmitMode::Ir` emits the portable
    /// UINode tree. Builder-style so call sites read clearly.
    pub fn new_with_emit(
        models: &'a HashMap<String, HashMap<String, String>>,
        components: &'a HashMap<String, CompInfo>,
        emit: EmitMode,
    ) -> Self {
        let mut cg = Codegen::new(models, components);
        cg.emit = emit;
        cg
    }

    /// Enable opt-in dispatch instrumentation (R7 debug/observability). When set,
    /// `gen_page` wraps each handler in `mvDispatch` with a structured
    /// `Debug.print` log line + an instruction-cost delta. Default (unset) keeps
    /// the generated dispatch byte-identical to the legacy path. Builder-style.
    pub fn with_instrument(mut self, instrument: bool) -> Self {
        self.instrument = instrument;
        self
    }

    /// The active emit backend.
    pub fn emit_mode(&self) -> EmitMode {
        self.emit
    }

    /// The render-function preamble that constructs the builder. In Html mode
    /// this is byte-identical to the original (`let b = Html.Builder();`); in Ir
    /// mode it constructs an `Ir.Builder` as `ir`.
    fn builder_decl(&self) -> &'static str {
        match self.emit {
            EmitMode::Html => "let b = Html.Builder();\n",
            EmitMode::Ir => "let ir = Ir.Builder();\n",
        }
    }

    /// The build/return expression. Html returns the joined HTML; Ir returns the
    /// serialized UINode forest as JSON Text (so the `: Text` signature holds in
    /// both modes — the IR rides as a Text payload).
    fn builder_build(&self) -> &'static str {
        match self.emit {
            EmitMode::Html => "b.build();\n",
            EmitMode::Ir => "ir.toJson();\n",
        }
    }

    /// Compile a `src/Components/*.mview` into a render function:
    /// `func mvComponent_<Name>(<params>, mvChildren : Text) : Text { ... }`.
    /// `@children` inside the template emits the passed default-slot content.
    pub fn gen_app_component(&mut self, file: &MviewFile) -> String {
        // App components compile to a `… : Text` HTML fragment that pages embed
        // via `b.raw(mvComponent_X(...))`; they are always HTML-emitted. In Ir
        // mode a page's component invocation becomes an `ir.raw(...)` fallback.
        let saved_emit = self.emit;
        self.emit = EmitMode::Html;
        self.is_component = true;
        self.is_layout = false;
        self.build_type_env(&file.code);
        let slots = collect_slot_names(&file.template);
        let mut sig: Vec<String> = file
            .code
            .params
            .iter()
            .map(|p| format!("{} : {}", mangle_param(&p.name), p.ty))
            .collect();
        sig.push("mvChildren : Text".to_string());
        for sl in &slots {
            sig.push(format!("mvSlot_{} : Text", sl));
        }
        // Params whose name is a Motoko keyword were mangled in the signature;
        // rewrite their references in the body to match (Motoko-aware, so HTML
        // text and field accesses are left untouched).
        let mangled: Vec<(String, String)> = file
            .code
            .params
            .iter()
            .filter(|p| is_motoko_keyword(&p.name))
            .map(|p| (p.name.clone(), mangle_param(&p.name)))
            .collect();
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
        for (from, to) in &mangled {
            body = replace_ident_in_code(&body, from, to);
        }
        s.push_str(&body);
        s.push_str("    b.build();\n  };\n");
        self.is_component = false;
        self.emit = saved_emit;
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
        s.push_str(&format!("      {}", self.builder_decl()));
        s.push_str("      ignore ctx;\n");
        s.push_str(&set_params);
        let mut body = String::new();
        self.gen_nodes(&file.template, &mut body, "      ");
        s.push_str(&body);
        s.push_str(&format!("      {}    }};\n", self.builder_build()));

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
        if self.instrument {
            // R7 observability: `ctx` is now USED (caller + lastBatchId feed the
            // structured log), so it is no longer ignored. `mvArgs` may still be
            // unused if no handler takes args, so keep ignoring it.
            s.push_str("      ignore mvArgs;\n");
            s.push_str("      let mvT0 = ExperimentalIC.performanceCounter(0);\n");
        } else {
            s.push_str("      ignore ctx; ignore mvArgs;\n");
        }
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
        s.push_str("      };\n");
        if self.instrument {
            // Structured, parseable observability record — ONE line per event,
            // stable field order so tools/studio/log-parser.js can split it.
            // Format (pipe-delimited key=value, see docs/observability.md):
            //   MV|dispatch|page=<page>|handler=<h>|event=<h>|caller=<principal>|lastBatch=<id>|costInstr=<delta>
            // `event` mirrors `handler` (the dispatched event IS the handler id
            // on the server). `costInstr` is the instruction delta across the
            // handler body via the IC performance counter.
            s.push_str("      let mvCost = ExperimentalIC.performanceCounter(0) - mvT0;\n");
            s.push_str(&format!(
                "      Debug.print(\"MV|dispatch|page={}|handler=\" # mvH # \"|event=\" # mvH # \"|caller=\" # Principal.toText(ctx.caller) # \"|lastBatch=\" # ctx.lastBatchId # \"|costInstr=\" # debug_show (mvCost));\n",
                file.name
            ));
        }
        s.push_str("    };\n");

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
        // Layouts are the HTML document shell (they wrap `mvBody : Text`, which
        // is HTML) — they are always emitted on the HTML path, even when this
        // Codegen is otherwise in Ir mode (the IR backend targets page bodies).
        let saved_emit = self.emit;
        self.emit = EmitMode::Html;
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
        self.emit = saved_emit;
        s
    }

    // ---- node rendering ---------------------------------------------------
    fn gen_nodes(&self, nodes: &[Node], out: &mut String, indent: &str) {
        for n in nodes {
            self.gen_node(n, out, indent);
        }
    }

    fn gen_node(&self, node: &Node, out: &mut String, indent: &str) {
        // The Ir backend has a parallel walker; the Html walker below is left
        // byte-identical to the original single-path codegen.
        if self.emit == EmitMode::Ir {
            self.gen_node_ir(node, out, indent);
            return;
        }
        match node {
            Node::Text(t) => {
                if !t.is_empty() {
                    out.push_str(&format!("{}b.raw({});\n", indent, mo_str(t)));
                }
            }
            Node::Expr(e, _) => {
                if self.is_component && e.trim() == "children" {
                    out.push_str(&format!("{}b.raw(mvChildren);\n", indent));
                } else {
                    out.push_str(&format!("{}b.text({});\n", indent, self.as_text(e)));
                }
            }
            // `@raw(expr)` — trusted HTML, emitted without escaping. The
            // expression is emitted verbatim and must be `Text` (else moc errors);
            // we deliberately do NOT route through as_text (no debug_show wrap).
            Node::Raw(e, _) => {
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

    // ====================================================================
    // IR backend (EmitMode::Ir) — a parallel node walker that builds an
    // `Ir.Builder` UINode tree. Core node kinds (elements, attrs, events,
    // keyed regions, escaped/dynamic text, @if/@for/@switch) are native IR.
    // Builtins/charts/components/@raw that are not yet IR-modeled fall back to
    // an `ir.raw(<html>)` leaf carrying the EXACT HTML the Html backend emits,
    // so output is never wrong — only not-yet-native.
    // ====================================================================

    /// Capture the HTML the Html backend would emit for `nodes` as a Motoko
    /// `Text` expression: `(do { let b = Html.Builder(); …; b.build() })`. Used
    /// to wrap not-yet-IR-modeled nodes as an `ir.raw(...)` fallback leaf.
    fn html_fallback_expr(&self, nodes: &[Node]) -> String {
        // Re-enter the Html walker via a throwaway HTML-mode clone so the
        // captured chunk is byte-for-byte the real HTML output.
        let html_cg = Codegen {
            types: std::cell::RefCell::new(self.types.borrow().clone()),
            emit: EmitMode::Html,
            is_layout: self.is_layout,
            is_component: self.is_component,
            layout_theme: self.layout_theme.clone(),
            models: self.models,
            components: self.components,
            instrument: self.instrument,
        };
        let mut tmp = String::new();
        html_cg.gen_nodes(nodes, &mut tmp, "");
        let mut s = String::from("(do { let b = Html.Builder(); ");
        for line in tmp.lines() {
            s.push_str(line.trim());
            s.push(' ');
        }
        s.push_str("b.build() })");
        s
    }

    fn gen_nodes_ir(&self, nodes: &[Node], out: &mut String, indent: &str) {
        for n in nodes {
            self.gen_node_ir(n, out, indent);
        }
    }

    fn gen_node_ir(&self, node: &Node, out: &mut String, indent: &str) {
        match node {
            Node::Text(t) => {
                if !t.is_empty() {
                    // Static literal template text -> a #raw leaf (it is already
                    // exact HTML/markup the author wrote).
                    out.push_str(&format!("{}ir.raw({});\n", indent, mo_str(t)));
                }
            }
            Node::Expr(e, _) => {
                if self.is_component && e.trim() == "children" {
                    out.push_str(&format!("{}ir.raw(mvChildren);\n", indent));
                } else {
                    out.push_str(&format!("{}ir.text({});\n", indent, self.as_text(e)));
                }
            }
            // @raw(expr): trusted literal HTML -> a #raw leaf (no native model).
            Node::Raw(e, _) => {
                let e = e.trim();
                let expr = if self.is_layout {
                    e.strip_prefix("View.").map(|r| format!("mvHead.{}", r)).unwrap_or_else(|| e.to_string())
                } else {
                    e.to_string()
                };
                out.push_str(&format!("{}ir.raw({});\n", indent, expr));
            }
            Node::Yield => {
                out.push_str(&format!("{}ir.raw(mvBody);\n", indent));
            }
            Node::Head => {
                // Document <head> is an HTML-shell concern; emit the same HTML the
                // Html backend would, wrapped as a #raw leaf.
                out.push_str(&format!("{}ir.raw({});\n", indent, self.html_fallback_expr(&[node.clone()])));
            }
            Node::SectionRef(_) => {}
            Node::Slot(name) => {
                if self.is_component {
                    out.push_str(&format!("{}ir.raw(mvSlot_{});\n", indent, name));
                }
            }
            Node::Effect { .. } => {}
            Node::If(branches) => {
                let n = branches.len();
                for (k, br) in branches.iter().enumerate() {
                    let opener = match &br.cond {
                        Some(c) => {
                            if k == 0 { format!("if ({}) {{\n", c) } else { format!("else if ({}) {{\n", c) }
                        }
                        None => "else {\n".to_string(),
                    };
                    if k == 0 {
                        out.push_str(&format!("{}{}", indent, opener));
                    } else {
                        out.push_str(&opener);
                    }
                    self.gen_nodes_ir(&br.body, out, &format!("{}  ", indent));
                    if k + 1 < n {
                        out.push_str(&format!("{}}} ", indent));
                    } else {
                        out.push_str(&format!("{}}};\n", indent));
                    }
                }
            }
            Node::For { var, iter, body } => {
                out.push_str(&format!("{}for ({} in ({}).vals()) {{\n", indent, var, iter));
                let prev = self.types.borrow().get(var).cloned();
                if let Some(elem) = self.element_type(iter) {
                    self.types.borrow_mut().insert(var.clone(), elem);
                }
                self.gen_nodes_ir(body, out, &format!("{}  ", indent));
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
                    let pat = if c.pattern.starts_with('(') { c.pattern.clone() } else { format!("({})", c.pattern) };
                    out.push_str(&format!("{}  case {} {{\n", indent, pat));
                    self.gen_nodes_ir(&c.body, out, &format!("{}    ", indent));
                    out.push_str(&format!("{}  }};\n", indent));
                }
                out.push_str(&format!("{}}};\n", indent));
            }
            Node::Element(el) => self.gen_element_ir(el, out, indent),
            Node::Component(c) => self.gen_component_ir(c, out, indent),
        }
    }

    fn gen_element_ir(&self, el: &Element, out: &mut String, indent: &str) {
        out.push_str(&format!("{}ir.open({});\n", indent, mo_str(&el.tag)));
        // Server-driven forms get `novalidate` so the submit reaches MotoView.
        if el.tag.eq_ignore_ascii_case("form") && el.events.iter().any(|e| e.event.eq_ignore_ascii_case("submit")) {
            out.push_str(&format!("{}ir.attr(\"novalidate\", \"\");\n", indent));
        }
        for a in &el.attrs {
            self.gen_attr_ir(a, out, indent);
        }
        // two-way bind -> name + keyed marker + value
        if let Some(lv) = &el.bind {
            let key = lv.clone();
            let name = lv.split('.').last().unwrap_or(lv).to_string();
            out.push_str(&format!("{}ir.attr(\"name\", {});\n", indent, mo_str(&name)));
            out.push_str(&format!("{}ir.key({});\n", indent, mo_str(&key)));
            out.push_str(&format!("{}ir.attr(\"value\", {});\n", indent, self.as_text(lv)));
        }
        // events
        for ev in &el.events {
            if ev.event.eq_ignore_ascii_case("submit") {
                out.push_str(&format!("{}ir.event(\"submit\", {});\n", indent, mo_str(&ev.handler)));
                if el.secure {
                    let schema = escape_mo_inner(&collect_form_schema(el));
                    out.push_str(&format!("{}ir.attr(\"data-mv-secure\", \"1\");\n", indent));
                    out.push_str(&format!("{}ir.attr(\"data-mv-token\", ctx.mintToken(\"{}\", \"{}\"));\n", indent, ev.handler, schema));
                    out.push_str(&format!("{}ir.attr(\"data-mv-schema\", \"{}\");\n", indent, schema));
                }
            } else {
                out.push_str(&format!("{}ir.event({}, {});\n", indent, mo_str(&ev.event), mo_str(&ev.handler)));
                for (i, arg) in ev.args.iter().enumerate() {
                    if is_simple_literal(arg) {
                        out.push_str(&format!("{}ir.attr(\"data-mv-arg{}\", {});\n", indent, i, mo_str(arg.trim_matches('"'))));
                    } else {
                        out.push_str(&format!("{}ir.attr(\"data-mv-arg{}\", {});\n", indent, i, self.as_text(arg)));
                    }
                }
            }
        }
        if el.self_closing {
            out.push_str(&format!("{}ir.close();\n", indent));
            return;
        }
        self.gen_nodes_ir(&el.children, out, indent);
        out.push_str(&format!("{}ir.close();\n", indent));
    }

    fn gen_attr_ir(&self, a: &Attr, out: &mut String, indent: &str) {
        let name = match a.name.as_str() {
            "key" => "data-mv-key",
            "enter" => "data-mv-enter",
            "exit" => "data-mv-exit",
            other => other,
        };
        // `key="…"` is the keyed-region marker; model it natively as ir.key so a
        // native renderer can reconcile lists. Other data attrs are plain attrs.
        match &a.value {
            AttrValue::Bool => {
                out.push_str(&format!("{}ir.attr({:?}, \"\");\n", indent, name));
            }
            AttrValue::Literal(v) => {
                if name == "data-mv-key" {
                    out.push_str(&format!("{}ir.key({});\n", indent, mo_str(v)));
                } else {
                    out.push_str(&format!("{}ir.attr({:?}, {});\n", indent, name, mo_str(v)));
                }
            }
            AttrValue::Expr(e) => {
                if name == "data-mv-key" {
                    out.push_str(&format!("{}ir.key({});\n", indent, self.as_text(e)));
                } else {
                    out.push_str(&format!("{}ir.attr({:?}, {});\n", indent, name, self.as_text(e)));
                }
            }
            AttrValue::Concat(parts) => {
                let mut pieces = Vec::new();
                for p in parts {
                    match p {
                        AttrPart::Lit(l) => pieces.push(mo_str(l)),
                        AttrPart::Expr(e) => pieces.push(self.as_text(e)),
                    }
                }
                let joined = format!("({})", pieces.join(" # "));
                if name == "data-mv-key" {
                    out.push_str(&format!("{}ir.key({});\n", indent, joined));
                } else {
                    out.push_str(&format!("{}ir.attr({:?}, {});\n", indent, name, joined));
                }
            }
        }
    }

    fn gen_component_ir(&self, c: &Component, out: &mut String, indent: &str) {
        // Builtins/charts and app components are not yet IR-modeled: emit the
        // EXACT HTML the Html backend produces, wrapped as an `ir.raw(...)`
        // fallback leaf. (Honest coverage: see irCoverage report.)
        out.push_str(&format!("{}ir.raw({});\n", indent, self.html_fallback_expr(&[Node::Component(c.clone())])));
    }

    fn gen_element(&self, el: &Element, out: &mut String, indent: &str) {
        out.push_str(&format!("{}b.raw(\"<{}\");\n", indent, el.tag));
        // Server-driven forms must bypass native constraint validation so the
        // submit reaches MotoView (server-side validation is the source of truth).
        if el.tag.eq_ignore_ascii_case("form") && el.events.iter().any(|e| e.event.eq_ignore_ascii_case("submit")) {
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
            if ev.event.eq_ignore_ascii_case("submit") {
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
        // Resolve a string prop to a Motoko Text EXPRESSION (literal -> "…",
        // @(expr) -> the expr). Mirrors gen_attr / prop_to_arg. Default "" if
        // absent. Used by the chart arms to accept both literal data and a
        // dynamic @(expr) interpolation.
        let str_expr = |name: &str| -> String {
            match c.props.iter().find(|a| a.name == name).map(|a| &a.value) {
                Some(AttrValue::Literal(v)) => mo_str(v),
                Some(AttrValue::Expr(e)) => self.as_text(e),
                Some(AttrValue::Concat(parts)) => {
                    let mut pieces = Vec::new();
                    for p in parts {
                        match p {
                            AttrPart::Lit(l) => pieces.push(mo_str(l)),
                            AttrPart::Expr(e) => pieces.push(self.as_text(e)),
                        }
                    }
                    format!("({})", pieces.join(" # "))
                }
                Some(AttrValue::Bool) | None => "\"\"".to_string(),
            }
        };
        // Build the chart options record-update on `Charts.def`, including only
        // the props the author actually set (so defaults hold otherwise).
        // `extra` lets a chart inject options it derives itself (e.g. yMax).
        let chart_opts = |extra: &[String]| -> String {
            let mut o: Vec<String> = Vec::new();
            if c.props.iter().any(|a| a.name == "title") {
                o.push(format!("title = {}", str_expr("title")));
            }
            if let Some(w) = lit("width") {
                o.push(format!("width = {}", w));
            }
            if let Some(h) = lit("height") {
                o.push(format!("height = {}", h));
            }
            if let Some(u) = lit("unit") {
                o.push(format!("unit = {}", mo_str(&u)));
            }
            if let Some(m) = lit("yMin") {
                o.push(format!("yMin = ?({} : Float)", m));
            }
            if let Some(m) = lit("yMax") {
                o.push(format!("yMax = ?({} : Float)", m));
            }
            if prop("hideAxes").is_some() {
                o.push("showAxes = false".into());
            }
            if prop("hideGrid").is_some() {
                o.push("showGrid = false".into());
            }
            if prop("hideLegend").is_some() {
                o.push("showLegend = false".into());
            }
            for e in extra {
                o.push(e.clone());
            }
            if o.is_empty() {
                "Charts.def".to_string()
            } else {
                format!("{{ Charts.def with {} }}", o.join("; "))
            }
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
                // shape: rounded (default) | circular | square
                match lit("shape").as_deref() {
                    Some("circular") => cls.push_str(" mv-btn-circular"),
                    Some("square") => cls.push_str(" mv-btn-square"),
                    Some("rounded") => cls.push_str(" mv-btn-rounded"),
                    _ => {}
                }
                if prop("danger").is_some() || lit("color").as_deref() == Some("danger") { cls.push_str(" mv-btn-danger"); }
                if prop("iconOnly").is_some() { cls.push_str(" mv-btn-icon"); }
                let disabled = prop("disabled").is_some();
                if disabled { cls.push_str(" mv-btn-disabled"); }
                let ty = lit("type").unwrap_or_else(|| "button".into());
                out.push_str(&format!("{}b.raw(\"<button type=\\\"{}\\\" class=\\\"{}\\\"\");\n", indent, ty, cls));
                if disabled { out.push_str(&format!("{}b.raw(\" disabled\");\n", indent)); }
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
                // icon: a literal emoji/text rendered before (default) or after the label.
                let icon = lit("icon");
                let icon_after = lit("iconPosition").as_deref() == Some("after");
                if let (Some(ic), false) = (&icon, icon_after) {
                    out.push_str(&format!("{}b.raw(\"<span class=\\\"mv-btn-ico\\\">{}</span>\");\n", indent, esc_lit(ic)));
                }
                self.gen_nodes(&c.children, out, indent);
                if let (Some(ic), true) = (&icon, icon_after) {
                    out.push_str(&format!("{}b.raw(\"<span class=\\\"mv-btn-ico\\\">{}</span>\");\n", indent, esc_lit(ic)));
                }
                out.push_str(&format!("{}b.raw(\"</button>\");\n", indent));
                Some(())
            }
            // CompoundButton: a button with a bold primary line + a smaller
            // secondary description line. Children = primary label; `secondary`
            // prop (or a nested secondary) = description.
            "CompoundButton" => {
                let kind = lit("appearance").or_else(|| lit("kind")).unwrap_or_else(|| "secondary".into());
                let mut cls = format!("mv-btn mv-compound-btn mv-btn-{}", kind);
                match lit("shape").as_deref() { Some("circular")=>cls.push_str(" mv-btn-circular"), Some("square")=>cls.push_str(" mv-btn-square"), Some("rounded")=>cls.push_str(" mv-btn-rounded"), _=>{} }
                if prop("danger").is_some() { cls.push_str(" mv-btn-danger"); }
                if prop("iconOnly").is_some() { cls.push_str(" mv-btn-icon"); }
                if let Some(sz) = lit("size") { cls.push_str(&format!(" mv-btn-{}", sz)); }
                let disabled = prop("disabled").is_some();
                if disabled { cls.push_str(" mv-btn-disabled"); }
                let ty = lit("type").unwrap_or_else(|| "button".into());
                out.push_str(&format!("{}b.raw(\"<button type=\\\"{}\\\" class=\\\"{}\\\"\");\n", indent, ty, cls));
                if disabled { out.push_str(&format!("{}b.raw(\" disabled\");\n", indent)); }
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
                if let Some(ic) = lit("icon") {
                    out.push_str(&format!("{}b.raw(\"<span class=\\\"mv-btn-ico\\\">{}</span>\");\n", indent, esc_lit(&ic)));
                }
                out.push_str(&format!("{}b.raw(\"<span class=\\\"mv-compound-content\\\"><span class=\\\"mv-compound-primary\\\">\");\n", indent));
                self.gen_nodes(&c.children, out, indent);
                out.push_str(&format!("{}b.raw(\"</span>\");\n", indent));
                if let Some(s) = prop("secondary") {
                    out.push_str(&format!("{}b.raw(\"<span class=\\\"mv-compound-secondary\\\">\");\n", indent));
                    self.gen_attr_text(s, out, indent);
                    out.push_str(&format!("{}b.raw(\"</span>\");\n", indent));
                }
                out.push_str(&format!("{}b.raw(\"</span></button>\");\n", indent));
                Some(())
            }
            // ToggleButton: a pressable button with a pressed/checked state.
            // CSS-only via checkbox-hack: hidden checkbox + <label> styled as the
            // button; :checked paints the pressed state.
            "ToggleButton" => {
                let kind = lit("appearance").or_else(|| lit("kind")).unwrap_or_else(|| "secondary".into());
                let mut cls = format!("mv-btn mv-toggle-btn mv-btn-{}", kind);
                match lit("shape").as_deref() { Some("circular")=>cls.push_str(" mv-btn-circular"), Some("square")=>cls.push_str(" mv-btn-square"), Some("rounded")=>cls.push_str(" mv-btn-rounded"), _=>{} }
                if prop("danger").is_some() { cls.push_str(" mv-btn-danger"); }
                if prop("iconOnly").is_some() { cls.push_str(" mv-btn-icon"); }
                if let Some(sz) = lit("size") { cls.push_str(&format!(" mv-btn-{}", sz)); }
                match lit("shape").as_deref() {
                    Some("circular") => cls.push_str(" mv-btn-circular"),
                    Some("square") => cls.push_str(" mv-btn-square"),
                    _ => {}
                }
                let disabled = prop("disabled").is_some();
                if disabled { cls.push_str(" mv-btn-disabled"); }
                let name = esc_lit(&lit("name").unwrap_or_default());
                out.push_str(&format!("{}b.raw(\"<label class=\\\"{}\\\"><input type=\\\"checkbox\\\" class=\\\"mv-toggle-input\\\" name=\\\"{}\\\"\");\n", indent, cls, name));
                if prop("checked").is_some() { out.push_str(&format!("{}b.raw(\" checked\");\n", indent)); }
                if disabled { out.push_str(&format!("{}b.raw(\" disabled\");\n", indent)); }
                out.push_str(&format!("{}b.raw(\">\");\n", indent));
                if let Some(ic) = lit("icon") {
                    out.push_str(&format!("{}b.raw(\"<span class=\\\"mv-btn-ico\\\">{}</span>\");\n", indent, esc_lit(&ic)));
                }
                self.gen_nodes(&c.children, out, indent);
                out.push_str(&format!("{}b.raw(\"</label>\");\n", indent));
                Some(())
            }
            // MenuButton: a button showing a chevron that opens a menu.
            // Pure-CSS details/summary popover; children are MenuItem nodes.
            "MenuButton" => {
                let kind = lit("appearance").or_else(|| lit("kind")).unwrap_or_else(|| "secondary".into());
                let mut scls = format!("mv-menu-trigger mv-btn mv-menubtn-trigger mv-btn-{}", kind);
                if let Some(sz) = lit("size") { scls.push_str(&format!(" mv-btn-{}", sz)); }
                out.push_str(&format!("{}b.raw(\"<details class=\\\"mv-menu mv-menubtn\\\"><summary class=\\\"{}\\\">\");\n", indent, scls));
                if let Some(ic) = lit("icon") {
                    out.push_str(&format!("{}b.raw(\"<span class=\\\"mv-btn-ico\\\">{}</span>\");\n", indent, esc_lit(&ic)));
                }
                if let Some(l) = prop("label") { self.gen_attr_text(l, out, indent); }
                out.push_str(&format!("{}b.raw(\"<span class=\\\"mv-menubtn-chev\\\">\\u{{25be}}</span></summary><div class=\\\"mv-menu-list\\\">\");\n", indent));
                self.gen_nodes(&c.children, out, indent);
                out.push_str(&format!("{}b.raw(\"</div></details>\");\n", indent));
                Some(())
            }
            // SplitButton: a primary action button joined to a small chevron menu
            // trigger. The chevron half is a CSS-only details/summary popover; the
            // primary half is a real <button> carrying any click handler.
            "SplitButton" => {
                let kind = lit("appearance").or_else(|| lit("kind")).unwrap_or_else(|| "primary".into());
                let szc = lit("size").map(|s| format!(" mv-btn-{}", s)).unwrap_or_default();
                let mut cls = format!("mv-split mv-split-{}", kind);
                if let Some(sh) = lit("shape") { cls.push_str(&format!(" mv-split-{}", sh)); }
                if prop("disabled").is_some() { cls.push_str(" mv-split-disabled"); }
                let btncls = format!("mv-btn mv-split-action mv-btn-{}{}", kind, szc);
                let trigcls = format!("mv-menu-trigger mv-btn mv-split-trigger mv-btn-{}{}", kind, szc);
                out.push_str(&format!("{}b.raw(\"<span class=\\\"{}\\\"><button type=\\\"button\\\" class=\\\"{}\\\"\");\n", indent, cls, btncls));
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
                if let Some(ic) = lit("icon") {
                    out.push_str(&format!("{}b.raw(\"<span class=\\\"mv-btn-ico\\\">{}</span>\");\n", indent, esc_lit(&ic)));
                }
                if let Some(l) = prop("label") { self.gen_attr_text(l, out, indent); }
                out.push_str(&format!("{}b.raw(\"</button><details class=\\\"mv-menu mv-split-menu\\\"><summary class=\\\"{}\\\"><span class=\\\"mv-menubtn-chev\\\">\\u{{25be}}</span></summary><div class=\\\"mv-menu-list\\\">\");\n", indent, trigcls));
                self.gen_nodes(&c.children, out, indent);
                out.push_str(&format!("{}b.raw(\"</div></details></span>\");\n", indent));
                Some(())
            }
            "Card" => {
                let mut ccls = "mv-card".to_string();
                if let Some(ap) = lit("appearance") { ccls.push_str(&format!(" mv-card-{}", ap)); }
                if let Some(sz) = lit("size") { ccls.push_str(&format!(" mv-card-{}", sz)); }
                if prop("interactive").is_some() { ccls.push_str(" mv-card-interactive"); }
                if prop("disabled").is_some() { ccls.push_str(" mv-card-disabled"); }
                if prop("horizontal").is_some() { ccls.push_str(" mv-card-horizontal"); }
                if prop("selected").is_some() { ccls.push_str(" mv-card-selected"); }
                out.push_str(&format!("{}b.raw(\"<div class=\\\"{}\\\">\");\n", indent, ccls));
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
                let mut cls = "mv-badge".to_string();
                if prop("disabled").is_some() { cls.push_str(" mv-badge-disabled"); }
                if prop("selected").is_some() { cls.push_str(" mv-badge-selected"); }
                if prop("clickable").is_some() { cls.push_str(" mv-badge-clickable"); }
                // appearance: filled (default) | ghost | outline | tint
                if let Some(ap) = lit("appearance") {
                    if ap != "filled" { cls.push_str(&format!(" mv-badge-{}", ap)); }
                }
                // type = filled intent (brand/success/warning/danger/severe/informative/subtle)
                if let Some(ty) = lit("type") {
                    if !ty.is_empty() { cls.push_str(&format!(" mv-badge-{}", ty)); }
                }
                // color = colorful palette (neutral/brand/red/green/blue/...). Same set as CounterBadge.
                if let Some(color) = lit("color") {
                    cls.push_str(&format!(" mv-badge-{}", color));
                }
                // shape: rounded (default) | circular | square
                if let Some(shape) = lit("shape") {
                    cls.push_str(&format!(" mv-badge-{}", shape));
                }
                // size: tiny | extra-small | small | medium (default) | large | extra-large
                if let Some(size) = lit("size") {
                    cls.push_str(&format!(" mv-badge-{}", size));
                }
                out.push_str(&format!("{}b.raw(\"<span class=\\\"{}\\\">\");\n", indent, cls));
                if let Some(t) = prop("title") { self.gen_attr_text(t, out, indent); }
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
                if let Some(sh) = lit("shape") { cls.push_str(&format!(" mv-avatar-{}", sh)); }
                if let Some(aa) = lit("activeAppearance") { cls.push_str(&format!(" mv-avatar-{}", aa)); }
                // presence: online/busy/away — ring + dot (existing).
                if let Some(p) = lit("presence") { cls.push_str(&format!(" mv-avatar-{}", p)); }
                // color: named/colorful palette (neutral/brand/red/green/blue/...) → tinted bg + ring.
                if let Some(color) = lit("color") { cls.push_str(&format!(" mv-avatar-{}", color)); }
                // active: active (brand ring + emphasis) | inactive (dimmed/grayscale).
                if let Some(act) = lit("active") {
                    let act = if act.is_empty() { "active".to_string() } else { act };
                    cls.push_str(&format!(" mv-avatar-{}", act));
                } else if prop("active").is_some() {
                    cls.push_str(" mv-avatar-active");
                }
                // badge: a standalone presence dot (online/busy/dnd/away/offline) without the ring.
                if let Some(b) = lit("badge") {
                    cls.push_str(" mv-avatar-badge");
                    if !b.is_empty() { cls.push_str(&format!(" mv-avatar-badge-{}", b)); }
                }
                out.push_str(&format!("{}b.raw(\"<span class=\\\"{}\\\">\");\n", indent, cls));
                if let Some(t) = prop("text") { self.gen_attr_text(t, out, indent); }
                self.gen_nodes(&c.children, out, indent);
                out.push_str(&format!("{}b.raw(\"</span>\");\n", indent));
                Some(())
            }
            "Persona" => {
                let mut pcls = "mv-persona".to_string();
                if let Some(sz) = lit("size") { pcls.push_str(&format!(" mv-persona-{}", sz)); }
                if let Some(ly) = lit("layout") { pcls.push_str(&format!(" mv-persona-{}", ly)); }
                out.push_str(&format!("{}b.raw(\"<div class=\\\"{}\\\">\");\n", indent, pcls));
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
                let mut tlcls = "mv-tablist".to_string();
                if lit("appearance").as_deref() == Some("subtle") { tlcls.push_str(" mv-tablist-subtle"); }
                if lit("orientation").as_deref() == Some("vertical") { tlcls.push_str(" mv-tablist-vertical"); }
                out.push_str(&format!("{}b.raw(\"<div class=\\\"{}\\\">\");\n", indent, tlcls));
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
                let mut scls = "mv-switch".to_string();
                if let Some(sz) = lit("size") { scls.push_str(&format!(" mv-switch-{}", sz)); }
                if let Some(lp) = lit("labelPosition") { scls.push_str(&format!(" mv-switch-{}", lp)); }
                out.push_str(&format!("{}b.raw(\"<label class=\\\"{}\\\"><input type=\\\"checkbox\\\" name=\\\"{}\\\"\");\n", indent, scls, name));
                if prop("checked").is_some() { out.push_str(&format!("{}b.raw(\" checked\");\n", indent)); }
                if prop("disabled").is_some() { out.push_str(&format!("{}b.raw(\" disabled\");\n", indent)); }
                out.push_str(&format!("{}b.raw(\"><span class=\\\"mv-switch-track\\\"></span>\");\n", indent));
                if let Some(l) = prop("label") { self.gen_attr_text(l, out, indent); }
                self.gen_nodes(&c.children, out, indent);
                out.push_str(&format!("{}b.raw(\"</label>\");\n", indent));
                Some(())
            }
            "Divider" => {
                let mut cls = "mv-divider".to_string();
                if let Some(ap) = lit("appearance") { cls.push_str(&format!(" mv-divider-{}", ap)); }
                if prop("vertical").is_some() { cls.push_str(" mv-divider-vertical"); }
                if let Some(al) = lit("align") { cls.push_str(&format!(" mv-divider-{}", al)); }
                if prop("inset").is_some() { cls.push_str(" mv-divider-inset"); }
                out.push_str(&format!("{}b.raw(\"<div class=\\\"{}\\\">\");\n", indent, cls));
                self.gen_nodes(&c.children, out, indent);
                out.push_str(&format!("{}b.raw(\"</div>\");\n", indent));
                Some(())
            }
            "Spinner" => {
                let size = format!("{}{}", lit("size").map(|s| format!(" mv-spinner-{}", s)).unwrap_or_default(), lit("appearance").map(|a| format!(" mv-spinner-{}", a)).unwrap_or_default());
                // label: optional caption. labelPosition: after (default, inline) | below (stacked).
                if let Some(label) = prop("label") {
                    let pos = lit("labelPosition").unwrap_or_else(|| "after".into());
                    let _ = &pos; let pos_cls = format!(" mv-spinner-{}", pos);
                    out.push_str(&format!("{}b.raw(\"<span class=\\\"mv-spinner-wrap{}{}\\\"><span class=\\\"mv-spinner{}\\\"></span><span class=\\\"mv-spinner-label\\\">\");\n", indent, pos_cls, size, size));
                    self.gen_attr_text(label, out, indent);
                    out.push_str(&format!("{}b.raw(\"</span></span>\");\n", indent));
                } else {
                    out.push_str(&format!("{}b.raw(\"<span class=\\\"mv-spinner{}\\\"></span>\");\n", indent, size));
                }
                Some(())
            }
            // Light/dark theme switch. The button carries [data-mv-theme-toggle];
            // the glue flips <html data-theme> + persists the mv_theme cookie, and
            // the sun/moon icon is pure CSS driven by [data-theme]. No app JS.
            "ThemeToggle" => {
                out.push_str(&format!("{}b.raw(\"<button type=\\\"button\\\" class=\\\"mv-theme-toggle\\\" data-mv-theme-toggle aria-label=\\\"Toggle light or dark theme\\\" title=\\\"Toggle theme\\\"><span class=\\\"mv-theme-ico\\\"></span></button>\");\n", indent));
                Some(())
            }
            // Fluent theme picker — a CSS-only dropdown of the named themes. The
            // glue ([data-mv-theme-set]) applies + persists the choice; the active
            // option + label are painted on load. No app JS.
            "ThemePicker" => {
                let html = r##"<details class="mv-theme-picker"><summary>🎨 <span class="mv-theme-picker-label">Theme</span> <span class="mv-theme-picker-chev">▾</span></summary><div class="mv-theme-picker-menu"><button type="button" class="mv-theme-opt" data-mv-theme-set="web-light"><span class="mv-theme-opt-check"></span><span class="mv-theme-opt-sw" style="background:#0f6cbd"></span><span class="mv-theme-opt-label">Web Light</span></button><button type="button" class="mv-theme-opt" data-mv-theme-set="web-dark"><span class="mv-theme-opt-check"></span><span class="mv-theme-opt-sw" style="background:#242424"></span><span class="mv-theme-opt-label">Web Dark</span></button><button type="button" class="mv-theme-opt" data-mv-theme-set="teams-light"><span class="mv-theme-opt-check"></span><span class="mv-theme-opt-sw" style="background:#5b5fc7"></span><span class="mv-theme-opt-label">Teams Light</span></button><button type="button" class="mv-theme-opt" data-mv-theme-set="teams-dark"><span class="mv-theme-opt-check"></span><span class="mv-theme-opt-sw" style="background:#444791"></span><span class="mv-theme-opt-label">Teams Dark</span></button><button type="button" class="mv-theme-opt" data-mv-theme-set="hc"><span class="mv-theme-opt-check"></span><span class="mv-theme-opt-sw" style="background:#ffff00"></span><span class="mv-theme-opt-label">High Contrast</span></button></div></details>"##;
                out.push_str(&format!("{}b.raw({});\n", indent, mo_str(html)));
                Some(())
            }
            "MessageBar" => {
                let ty = lit("intent").or_else(|| lit("type")).unwrap_or_else(|| "info".into());
                let mut mbcls = format!("mv-alert mv-alert-{}", ty);
                if let Some(l) = lit("layout") { mbcls.push_str(&format!(" mv-alert-{}", l)); }
                if let Some(ap) = lit("appearance") { mbcls.push_str(&format!(" mv-alert-{}", ap)); }
                if let Some(sh) = lit("shape") { mbcls.push_str(&format!(" mv-alert-{}", sh)); }
                out.push_str(&format!("{}b.raw(\"<div class=\\\"{}\\\">\");\n", indent, mbcls));
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
            // ---- Fluent: form controls ----
            "Checkbox" | "Radio" => {
                let (cls, kind, dot) = if c.name == "Radio" {
                    ("mv-radio", "radio", "mv-radio-dot")
                } else {
                    ("mv-checkbox", "checkbox", "mv-checkbox-box")
                };
                let name = lit("name").unwrap_or_default();
                let mut lcls = cls.to_string();
                if let Some(sz) = lit("size") { lcls.push_str(&format!(" {}-{}", cls, sz)); }
                if lit("shape").as_deref() == Some("circular") { lcls.push_str(&format!(" {}-circular", cls)); }
                if let Some(lp) = lit("labelPosition") { lcls.push_str(&format!(" {}-{}", cls, lp)); }
                if prop("mixed").is_some() { lcls.push_str(&format!(" {}-mixed", cls)); }
                out.push_str(&format!("{}b.raw(\"<label class=\\\"{}\\\"><input type=\\\"{}\\\" name=\\\"{}\\\"\");\n", indent, lcls, kind, name));
                if let Some(v) = lit("value") { out.push_str(&format!("{}b.raw(\" value=\\\"{}\\\"\");\n", indent, esc_lit(&v))); }
                if prop("checked").is_some() { out.push_str(&format!("{}b.raw(\" checked\");\n", indent)); }
                if prop("disabled").is_some() { out.push_str(&format!("{}b.raw(\" disabled\");\n", indent)); }
                out.push_str(&format!("{}b.raw(\"><span class=\\\"{}\\\"></span>\");\n", indent, dot));
                if let Some(l) = prop("label") { self.gen_attr_text(l, out, indent); }
                self.gen_nodes(&c.children, out, indent);
                out.push_str(&format!("{}b.raw(\"</label>\");\n", indent));
                Some(())
            }
            "Select" => {
                let name = lit("name").unwrap_or_default();
                let has_label = prop("label").is_some();
                if has_label {
                    out.push_str(&format!("{}b.raw(\"<div class=\\\"mv-field\\\"><label>\");\n", indent));
                    self.gen_attr_text(prop("label").unwrap(), out, indent);
                    out.push_str(&format!("{}b.raw(\"</label>\");\n", indent));
                }
                let mut selcls = "mv-select".to_string();
                if let Some(ap) = lit("appearance") { selcls.push_str(&format!(" mv-select-{}", ap)); }
                if let Some(sz) = lit("size") { selcls.push_str(&format!(" mv-select-{}", sz)); }
                let seldis = if prop("disabled").is_some() { " disabled" } else { "" };
                out.push_str(&format!("{}b.raw(\"<select class=\\\"{}\\\" name=\\\"{}\\\"{}>\");\n", indent, selcls, name, seldis));
                if let Some(opts) = lit("options") {
                    for o in opts.split(',') {
                        out.push_str(&format!("{}b.raw(\"<option>{}</option>\");\n", indent, esc_lit(o.trim())));
                    }
                }
                self.gen_nodes(&c.children, out, indent);
                out.push_str(&format!("{}b.raw(\"</select>\");\n", indent));
                if has_label { out.push_str(&format!("{}b.raw(\"</div>\");\n", indent)); }
                Some(())
            }
            "Searchbox" => {
                let name = lit("name").unwrap_or_default();
                let ph = esc_lit(&lit("placeholder").unwrap_or_else(|| "Search".into()));
                let mut sbcls = "mv-input mv-search".to_string();
                if let Some(ap) = lit("appearance") { sbcls.push_str(&format!(" mv-search-{}", ap)); }
                if let Some(sz) = lit("size") { sbcls.push_str(&format!(" mv-search-{}", sz)); }
                let sbdis = if prop("disabled").is_some() { " disabled" } else { "" };
                out.push_str(&format!("{}b.raw(\"<input type=\\\"search\\\" class=\\\"{}\\\" name=\\\"{}\\\" placeholder=\\\"{}\\\"{}>\");\n", indent, sbcls, name, ph, sbdis));
                Some(())
            }
            "Combobox" => {
                let name = lit("name").unwrap_or_default();
                let listid = format!("{}-list", name);
                out.push_str(&format!("{}b.raw(\"<div class=\\\"mv-field\\\">\");\n", indent));
                if let Some(l) = prop("label") {
                    out.push_str(&format!("{}b.raw(\"<label>\");\n", indent));
                    self.gen_attr_text(l, out, indent);
                    out.push_str(&format!("{}b.raw(\"</label>\");\n", indent));
                }
                let ph = esc_lit(&lit("placeholder").unwrap_or_default());
                let mut cbcls = "mv-input mv-combobox".to_string();
                if let Some(ap) = lit("appearance") { cbcls.push_str(&format!(" mv-combobox-{}", ap)); }
                if let Some(sz) = lit("size") { cbcls.push_str(&format!(" mv-combobox-{}", sz)); }
                let cbdis = if prop("disabled").is_some() { " disabled" } else { "" };
                out.push_str(&format!("{}b.raw(\"<input class=\\\"{}\\\" name=\\\"{}\\\" list=\\\"{}\\\" placeholder=\\\"{}\\\"{}><datalist id=\\\"{}\\\">\");\n", indent, cbcls, name, listid, ph, cbdis, listid));
                if let Some(opts) = lit("options") {
                    for o in opts.split(',') {
                        out.push_str(&format!("{}b.raw(\"<option value=\\\"{}\\\"></option>\");\n", indent, esc_lit(o.trim())));
                    }
                }
                out.push_str(&format!("{}b.raw(\"</datalist></div>\");\n", indent));
                Some(())
            }
            // ---- Fluent: data display ----
            "Tag" => {
                let mut cls = "mv-tag".to_string();
                if prop("disabled").is_some() { cls.push_str(" mv-tag-disabled"); }
                if prop("selected").is_some() { cls.push_str(" mv-tag-selected"); }
                if prop("interactive").is_some() { cls.push_str(" mv-tag-interactive"); }
                // appearance: outline (default) | filled | brand
                if let Some(ap) = lit("appearance") { cls.push_str(&format!(" mv-tag-{}", ap)); }
                // size: small | medium (default) | extra-large
                if let Some(size) = lit("size") { cls.push_str(&format!(" mv-tag-{}", size)); }
                // shape: rounded (default) | circular
                if let Some(shape) = lit("shape") { cls.push_str(&format!(" mv-tag-{}", shape)); }
                out.push_str(&format!("{}b.raw(\"<span class=\\\"{}\\\">\");\n", indent, cls));
                // `value` prop is an optional text label; children also render as content.
                if let Some(val) = prop("value") { self.gen_attr_text(val, out, indent); }
                self.gen_nodes(&c.children, out, indent);
                if prop("dismissible").is_some() {
                    out.push_str(&format!("{}b.raw(\"<button type=\\\"button\\\" class=\\\"mv-tag-dismiss\\\"></button>\");\n", indent));
                }
                out.push_str(&format!("{}b.raw(\"</span>\");\n", indent));
                Some(())
            }
            "AvatarGroup" => {
                let mut agcls = "mv-avatar-group".to_string();
                if let Some(ly) = lit("layout") { agcls.push_str(&format!(" mv-avatar-group-{}", ly)); }
                if let Some(sz) = lit("size") { agcls.push_str(&format!(" mv-avatar-group-{}", sz)); }
                out.push_str(&format!("{}b.raw(\"<div class=\\\"{}\\\">\");\n", indent, agcls));
                self.gen_nodes(&c.children, out, indent);
                out.push_str(&format!("{}b.raw(\"</div>\");\n", indent));
                Some(())
            }
            "ProgressBar" => {
                let v: f64 = lit("value").and_then(|s| s.parse().ok()).unwrap_or(0.0);
                let m: f64 = lit("max").and_then(|s| s.parse().ok()).unwrap_or(100.0);
                let pct = if m > 0.0 { (v / m * 100.0).clamp(0.0, 100.0) } else { 0.0 };
                let mut cls = "mv-progress".to_string();
                if lit("value").is_none() { cls.push_str(" mv-progress-indeterminate"); }
                // color / validationState: brand (default) | success | warning | error
                if let Some(color) = lit("color").or_else(|| lit("validationState")) {
                    cls.push_str(&format!(" mv-progress-{}", color));
                }
                // thickness: medium (default) | large
                if let Some(thick) = lit("thickness") {
                    if thick != "medium" { cls.push_str(&format!(" mv-progress-{}", thick)); }
                }
                // shape: rounded | square
                if let Some(shape) = lit("shape") { cls.push_str(&format!(" mv-progress-{}", shape)); }
                out.push_str(&format!("{}b.raw(\"<div class=\\\"{}\\\"><div class=\\\"mv-progress-bar\\\" style=\\\"width:{:.1}%\\\"></div></div>\");\n", indent, cls, pct));
                Some(())
            }
            "Skeleton" => {
                let shape = lit("shape").map(|s| format!(" mv-skeleton-{}", s)).unwrap_or_default();
                out.push_str(&format!("{}b.raw(\"<span class=\\\"mv-skeleton{}\\\"></span>\");\n", indent, shape));
                Some(())
            }
            "Rating" => {
                let v: usize = lit("value").and_then(|s| s.parse().ok()).unwrap_or(0);
                let m: usize = lit("max").and_then(|s| s.parse().ok()).unwrap_or(5);
                let size = lit("size").map(|s| format!(" mv-rating-{}", s)).unwrap_or_default();
                out.push_str(&format!("{}b.raw(\"<span class=\\\"mv-rating{}\\\" role=\\\"img\\\">\");\n", indent, size));
                for i in 0..m {
                    let on = if i < v { "mv-star-on" } else { "mv-star-off" };
                    out.push_str(&format!("{}b.raw(\"<span class=\\\"mv-star {}\\\">&#9733;</span>\");\n", indent, on));
                }
                out.push_str(&format!("{}b.raw(\"</span>\");\n", indent));
                Some(())
            }
            "Image" => {
                let mut cls = "mv-image".to_string();
                if let Some(s) = lit("shape") { cls.push_str(&format!(" mv-image-{}", s)); }
                if let Some(f) = lit("fit") { cls.push_str(&format!(" mv-image-{}", f)); }
                if prop("bordered").is_some() { cls.push_str(" mv-image-bordered"); }
                if prop("shadow").is_some() { cls.push_str(" mv-image-shadow"); }
                if prop("fluid").is_some() { cls.push_str(" mv-image-fluid"); }
                out.push_str(&format!("{}b.raw(\"<img class=\\\"{}\\\" src=\\\"\");\n", indent, cls));
                if let Some(s) = prop("src") { self.gen_attr_text(s, out, indent); }
                out.push_str(&format!("{}b.raw(\"\\\" alt=\\\"\");\n", indent));
                if let Some(a) = prop("alt") { self.gen_attr_text(a, out, indent); }
                out.push_str(&format!("{}b.raw(\"\\\">\");\n", indent));
                Some(())
            }
            "Breadcrumb" => {
                let mut bccls = "mv-breadcrumb".to_string();
                if let Some(sz) = lit("size") { bccls.push_str(&format!(" mv-breadcrumb-{}", sz)); }
                out.push_str(&format!("{}b.raw(\"<nav class=\\\"{}\\\">\");\n", indent, bccls));
                self.gen_nodes(&c.children, out, indent);
                out.push_str(&format!("{}b.raw(\"</nav>\");\n", indent));
                Some(())
            }
            "BreadcrumbItem" => {
                let cur = if prop("current").is_some() { " is-current" } else { "" };
                out.push_str(&format!("{}b.raw(\"<a class=\\\"mv-breadcrumb-item{}\\\" href=\\\"\");\n", indent, cur));
                if let Some(h) = prop("href") { self.gen_attr_text(h, out, indent); } else { out.push_str(&format!("{}b.raw(\"#\");\n", indent)); }
                out.push_str(&format!("{}b.raw(\"\\\">\");\n", indent));
                self.gen_nodes(&c.children, out, indent);
                out.push_str(&format!("{}b.raw(\"</a>\");\n", indent));
                Some(())
            }
            // ---- Fluent: disclosure + overlays (CSS-only) ----
            "Accordion" => {
                let mut accls = "mv-accordion".to_string();
                if let Some(sz) = lit("size") { accls.push_str(&format!(" mv-accordion-{}", sz)); }
                if let Some(lk) = lit("look") { accls.push_str(&format!(" mv-accordion-{}", lk)); }
                out.push_str(&format!("{}b.raw(\"<div class=\\\"{}\\\">\");\n", indent, accls));
                self.gen_nodes(&c.children, out, indent);
                out.push_str(&format!("{}b.raw(\"</div>\");\n", indent));
                Some(())
            }
            "AccordionItem" => {
                let open = if prop("open").is_some() { " open" } else { "" };
                out.push_str(&format!("{}b.raw(\"<details class=\\\"mv-accordion-item\\\"{}><summary class=\\\"mv-accordion-header\\\">\");\n", indent, open));
                if let Some(h) = prop("header") { self.gen_attr_text(h, out, indent); }
                out.push_str(&format!("{}b.raw(\"</summary><div class=\\\"mv-accordion-panel\\\">\");\n", indent));
                self.gen_nodes(&c.children, out, indent);
                out.push_str(&format!("{}b.raw(\"</div></details>\");\n", indent));
                Some(())
            }
            "Popover" => {
                let mut pvcls = "mv-popover".to_string();
                if let Some(pl) = lit("placement").or_else(|| lit("position")) { pvcls.push_str(&format!(" mv-popover-{}", pl)); }
                if let Some(sz) = lit("size") { pvcls.push_str(&format!(" mv-popover-{}", sz)); }
                if let Some(ap) = lit("appearance") { pvcls.push_str(&format!(" mv-popover-{}", ap)); }
                out.push_str(&format!("{}b.raw(\"<details class=\\\"{}\\\"><summary class=\\\"mv-popover-trigger\\\">\");\n", indent, pvcls));
                if let Some(l) = prop("label") { self.gen_attr_text(l, out, indent); }
                out.push_str(&format!("{}b.raw(\"</summary><div class=\\\"mv-popover-surface\\\">\");\n", indent));
                self.gen_nodes(&c.children, out, indent);
                out.push_str(&format!("{}b.raw(\"</div></details>\");\n", indent));
                Some(())
            }
            "Tooltip" => {
                let mut ttcls = "mv-tooltip".to_string();
                if let Some(pl) = lit("placement").or_else(|| lit("position")) { ttcls.push_str(&format!(" mv-tooltip-{}", pl)); }
                out.push_str(&format!("{}b.raw(\"<span class=\\\"{}\\\" tabindex=\\\"0\\\">\");\n", indent, ttcls));
                self.gen_nodes(&c.children, out, indent);
                out.push_str(&format!("{}b.raw(\"<span class=\\\"mv-tooltip-text\\\" role=\\\"tooltip\\\">\");\n", indent));
                if let Some(t) = prop("text") { self.gen_attr_text(t, out, indent); }
                out.push_str(&format!("{}b.raw(\"</span></span>\");\n", indent));
                Some(())
            }
            "Dialog" | "Drawer" => {
                let is_drawer = c.name == "Drawer";
                let id = esc_lit(&lit("id").unwrap_or_else(|| if is_drawer { "mvdrawer".into() } else { "mvdlg".into() }));
                let trigger = esc_lit(&lit("trigger").unwrap_or_else(|| "Open".into()));
                let dlgmods = {
                    let p = if is_drawer { "mv-drawer" } else { "mv-dialog" };
                    let mut m = String::new();
                    if let Some(sz) = lit("size") { m.push_str(&format!(" {}-{}", p, sz)); }
                    if let Some(ty2) = lit("type") { m.push_str(&format!(" {}-{}", p, ty2)); }
                    if let Some(po) = lit("position") { m.push_str(&format!(" {}-{}", p, po)); }
                    m
                };
                if is_drawer {
                    let side = if lit("side").as_deref() == Some("end") { "mv-drawer-end" } else { "mv-drawer-start" };
                    out.push_str(&format!("{}b.raw(\"<span class=\\\"mv-drawer-root{}\\\"><input type=\\\"checkbox\\\" id=\\\"{}\\\" class=\\\"mv-drawer-toggle\\\" hidden><label for=\\\"{}\\\" class=\\\"mv-drawer-trigger\\\">{}</label><div class=\\\"mv-drawer-overlay\\\"><label for=\\\"{}\\\" class=\\\"mv-drawer-backdrop\\\"></label><aside class=\\\"mv-drawer-surface {}\\\">\");\n", indent, dlgmods, id, id, trigger, id, side));
                } else {
                    out.push_str(&format!("{}b.raw(\"<span class=\\\"mv-dialog-root{}\\\"><input type=\\\"checkbox\\\" id=\\\"{}\\\" class=\\\"mv-dialog-toggle\\\" hidden><label for=\\\"{}\\\" class=\\\"mv-dialog-trigger mv-btn mv-btn-primary mv-btn-small\\\">{}</label><div class=\\\"mv-dialog-overlay\\\"><label for=\\\"{}\\\" class=\\\"mv-dialog-backdrop\\\"></label><div class=\\\"mv-dialog-surface\\\" role=\\\"dialog\\\">\");\n", indent, dlgmods, id, id, trigger, id));
                }
                let pfx = if is_drawer { "mv-drawer" } else { "mv-dialog" };
                if let Some(t) = prop("title") {
                    out.push_str(&format!("{}b.raw(\"<div class=\\\"{}-title\\\">\");\n", indent, pfx));
                    self.gen_attr_text(t, out, indent);
                    out.push_str(&format!("{}b.raw(\"</div>\");\n", indent));
                }
                out.push_str(&format!("{}b.raw(\"<div class=\\\"{}-body\\\">\");\n", indent, pfx));
                self.gen_nodes(&c.children, out, indent);
                let close = if is_drawer { "</div></aside></div></span>" } else { "</div></div></div></span>" };
                out.push_str(&format!("{}b.raw(\"{}\");\n", indent, close));
                Some(())
            }
"Field" => {
    // Fluent Field wrapper: optional label (+ red * when required), the control
    // as children, and an optional hint or validation message below.
    out.push_str(&format!("{}b.raw(\"<div class=\\\"mv-field\\\">\");\n", indent));
    if let Some(l) = prop("label") {
        out.push_str(&format!("{}b.raw(\"<label class=\\\"mv-field-label\\\">\");\n", indent));
        self.gen_attr_text(l, out, indent);
        if prop("required").is_some() {
            out.push_str(&format!("{}b.raw(\"<span class=\\\"mv-required\\\" aria-hidden=\\\"true\\\"> *</span>\");\n", indent));
        }
        out.push_str(&format!("{}b.raw(\"</label>\");\n", indent));
    }
    // the control(s)
    self.gen_nodes(&c.children, out, indent);
    // message: validationMessage (colored by validationState) takes precedence over hint.
    if let Some(msg) = prop("validationMessage").or_else(|| prop("hint")) {
        let state = lit("validationState").unwrap_or_default();
        if prop("validationMessage").is_some() && !state.is_empty() {
            out.push_str(&format!("{}b.raw(\"<span class=\\\"mv-field-validation mv-field-validation-{}\\\">\");\n", indent, esc_lit(&state)));
        } else {
            out.push_str(&format!("{}b.raw(\"<span class=\\\"mv-field-hint\\\">\");\n", indent));
        }
        self.gen_attr_text(msg, out, indent);
        out.push_str(&format!("{}b.raw(\"</span>\");\n", indent));
    }
    out.push_str(&format!("{}b.raw(\"</div>\");\n", indent));
    Some(())
}
"Label" => {
    // Standalone Fluent Label. size = small|medium(default)|large; weight = regular(default)|semibold.
    let size = lit("size").unwrap_or_else(|| "medium".into());
    let weight = lit("weight").unwrap_or_else(|| "regular".into());
    let cls = format!("mv-label mv-label-{} mv-label-{}", esc_lit(&size), esc_lit(&weight));
    out.push_str(&format!("{}b.raw(\"<label class=\\\"{}\\\"\");\n", indent, cls));
    if let Some(f) = lit("htmlFor").or_else(|| lit("for")) {
        out.push_str(&format!("{}b.raw(\" for=\\\"{}\\\"\");\n", indent, esc_lit(&f)));
    }
    out.push_str(&format!("{}b.raw(\">\");\n", indent));
    if let Some(t) = prop("text") { self.gen_attr_text(t, out, indent); }
    self.gen_nodes(&c.children, out, indent);
    if prop("required").is_some() {
        out.push_str(&format!("{}b.raw(\"<span class=\\\"mv-required\\\" aria-hidden=\\\"true\\\"> *</span>\");\n", indent));
    }
    out.push_str(&format!("{}b.raw(\"</label>\");\n", indent));
    Some(())
}
"InfoLabel" => {
    // A Label plus a small (i) badge that reveals an info popover on hover/focus (CSS-only).
    let weight = lit("weight").unwrap_or_else(|| "regular".into());
    out.push_str(&format!("{}b.raw(\"<span class=\\\"mv-info-label mv-label mv-label-medium mv-label-{}\\\">\");\n", indent, esc_lit(&weight)));
    if let Some(l) = prop("label") { self.gen_attr_text(l, out, indent); }
    self.gen_nodes(&c.children, out, indent);
    if let Some(info) = prop("info") {
        out.push_str(&format!("{}b.raw(\"<span class=\\\"mv-info-tip\\\" tabindex=\\\"0\\\" role=\\\"note\\\"><span class=\\\"mv-info-ico\\\" aria-hidden=\\\"true\\\">i</span><span class=\\\"mv-info-pop\\\">\");\n", indent));
        self.gen_attr_text(info, out, indent);
        out.push_str(&format!("{}b.raw(\"</span></span>\");\n", indent));
    }
    out.push_str(&format!("{}b.raw(\"</span>\");\n", indent));
    Some(())
}
"SpinButton" => {
    // Number input with native up/down steppers, styled to Fluent. Optional label wraps it in a field.
    let name = lit("name").unwrap_or_default();
    let has_label = prop("label").is_some();
    if has_label {
        out.push_str(&format!("{}b.raw(\"<div class=\\\"mv-field\\\"><label class=\\\"mv-field-label\\\" for=\\\"{}\\\">\");\n", indent, esc_lit(&name)));
        self.gen_attr_text(prop("label").unwrap(), out, indent);
        out.push_str(&format!("{}b.raw(\"</label>\");\n", indent));
    }
    let mut spcls = "mv-input mv-spinbutton".to_string();
    if let Some(ap) = lit("appearance") { spcls.push_str(&format!(" mv-spinbutton-{}", ap)); }
    if let Some(sz) = lit("size") { spcls.push_str(&format!(" mv-spinbutton-{}", sz)); }
    out.push_str(&format!("{}b.raw(\"<input type=\\\"number\\\" class=\\\"{}\\\" name=\\\"{}\\\" id=\\\"{}\\\"\");\n", indent, spcls, esc_lit(&name), esc_lit(&name)));
    if let Some(v) = prop("value") {
        out.push_str(&format!("{}b.raw(\" value=\\\"\");\n", indent));
        self.gen_attr_text(v, out, indent);
        out.push_str(&format!("{}b.raw(\"\\\"\");\n", indent));
    }
    if let Some(m) = lit("min")  { out.push_str(&format!("{}b.raw(\" min=\\\"{}\\\"\");\n", indent, esc_lit(&m))); }
    if let Some(m) = lit("max")  { out.push_str(&format!("{}b.raw(\" max=\\\"{}\\\"\");\n", indent, esc_lit(&m))); }
    if let Some(s) = lit("step") { out.push_str(&format!("{}b.raw(\" step=\\\"{}\\\"\");\n", indent, esc_lit(&s))); }
    if prop("disabled").is_some() { out.push_str(&format!("{}b.raw(\" disabled\");\n", indent)); }
    out.push_str(&format!("{}b.raw(\">\");\n", indent));
    if has_label { out.push_str(&format!("{}b.raw(\"</div>\");\n", indent)); }
    Some(())
}
"Slider" => {
    // Native range input styled to the Fluent track + thumb + brand fill.
    let name = lit("name").unwrap_or_default();
    let min = lit("min").unwrap_or_else(|| "0".into());
    let max = lit("max").unwrap_or_else(|| "100".into());
    let step = lit("step").unwrap_or_else(|| "1".into());
    let mut cls = "mv-slider".to_string();
    if let Some(sz) = lit("size") { cls.push_str(&format!(" mv-slider-{}", sz)); }
    if prop("vertical").is_some() { cls.push_str(" mv-slider-vertical"); }
    out.push_str(&format!("{}b.raw(\"<input type=\\\"range\\\" class=\\\"{}\\\" name=\\\"{}\\\" min=\\\"{}\\\" max=\\\"{}\\\" step=\\\"{}\\\"\");\n", indent, cls, esc_lit(&name), esc_lit(&min), esc_lit(&max), esc_lit(&step)));
    if let Some(v) = prop("value") {
        out.push_str(&format!("{}b.raw(\" value=\\\"\");\n", indent));
        self.gen_attr_text(v, out, indent);
        out.push_str(&format!("{}b.raw(\"\\\"\");\n", indent));
    }
    if prop("vertical").is_some() { out.push_str(&format!("{}b.raw(\" orient=\\\"vertical\\\"\");\n", indent)); }
    if prop("disabled").is_some() { out.push_str(&format!("{}b.raw(\" disabled\");\n", indent)); }
    out.push_str(&format!("{}b.raw(\">\");\n", indent));
    Some(())
}
"Input" => {
    // Standalone single-line text input — the building block. appearance: outline(default)|underline|filled-lighter|filled-darker.
    let name = lit("name").unwrap_or_default();
    let ty = lit("type").unwrap_or_else(|| "text".into());
    let appearance = lit("appearance").unwrap_or_else(|| "outline".into());
    let mut cls = format!("mv-input mv-input-{}", esc_lit(&appearance));
    if let Some(sz) = lit("size") { cls.push_str(&format!(" mv-input-{}", esc_lit(&sz))); }
    out.push_str(&format!("{}b.raw(\"<input type=\\\"{}\\\" class=\\\"{}\\\" name=\\\"{}\\\"\");\n", indent, esc_lit(&ty), cls, esc_lit(&name)));
    if let Some(ph) = lit("placeholder") { out.push_str(&format!("{}b.raw(\" placeholder=\\\"{}\\\"\");\n", indent, esc_lit(&ph))); }
    if let Some(v) = prop("value") {
        out.push_str(&format!("{}b.raw(\" value=\\\"\");\n", indent));
        self.gen_attr_text(v, out, indent);
        out.push_str(&format!("{}b.raw(\"\\\"\");\n", indent));
    }
    if prop("disabled").is_some() { out.push_str(&format!("{}b.raw(\" disabled\");\n", indent)); }
    if prop("required").is_some() { out.push_str(&format!("{}b.raw(\" required\");\n", indent)); }
    out.push_str(&format!("{}b.raw(\">\");\n", indent));
    Some(())
}
            "CounterBadge" => {
                // Fluent CounterBadge: a small rounded numeric pill. appearance:
                // filled (default) / ghost / outline; color -> mv-cbadge-<color>;
                // size -> mv-cbadge-<size>; `dot` collapses to a bare dot; `showZero`
                // forces a literal 0 to render (otherwise a literal 0 is suppressed).
                let appearance = lit("appearance").unwrap_or_else(|| "filled".into());
                let color = lit("color").unwrap_or_else(|| "brand".into());
                let mut cls = format!("mv-cbadge mv-cbadge-{} mv-cbadge-{}", appearance, color);
                if let Some(sz) = lit("size") { cls.push_str(&format!(" mv-cbadge-{}", sz)); }
                if let Some(sh) = lit("shape") { cls.push_str(&format!(" mv-cbadge-{}", sh)); }
                if prop("disabled").is_some() { cls.push_str(" mv-cbadge-disabled"); }
                let is_dot = prop("dot").is_some();
                if is_dot { cls.push_str(" mv-cbadge-dot"); }
                // A literal count of 0 is hidden unless showZero is present (Fluent default).
                let show_zero = prop("showZero").is_some();
                if !is_dot {
                    if let Some(v) = lit("count") {
                        if v.trim() == "0" && !show_zero {
                            // suppressed: render nothing
                            return Some(());
                        }
                    }
                }
                out.push_str(&format!("{}b.raw(\"<span class=\\\"{}\\\" role=\\\"status\\\">\");\n", indent, cls));
                if !is_dot {
                    if let Some(c0) = prop("count") {
                        self.gen_attr_text(c0, out, indent);
                    }
                    self.gen_nodes(&c.children, out, indent);
                }
                out.push_str(&format!("{}b.raw(\"</span>\");\n", indent));
                Some(())
            }
            "PresenceBadge" => {
                // Fluent PresenceBadge: a small status dot. status -> mv-presence-<status>
                // drives color + glyph (busy/dnd filled, away ring, available check,
                // offline/oof hollow). size -> mv-presence-<size>. `outOfOffice` ring.
                let status = lit("status").unwrap_or_else(|| "available".into());
                let mut cls = format!("mv-presence mv-presence-{}", status);
                if let Some(sz) = lit("size") { cls.push_str(&format!(" mv-presence-{}", sz)); }
                if prop("outOfOffice").is_some() { cls.push_str(" mv-presence-oof-ring"); }
                out.push_str(&format!("{}b.raw(\"<span class=\\\"{}\\\" role=\\\"img\\\" aria-label=\\\"{}\\\"></span>\");\n", indent, cls, esc_lit(&status)));
                Some(())
            }
            "Title1" => {
                out.push_str(&format!("{}b.raw(\"<h1 class=\\\"mv-type-title1\\\">\");\n", indent));
                self.gen_nodes(&c.children, out, indent);
                out.push_str(&format!("{}b.raw(\"</h1>\");\n", indent));
                Some(())
            }
            "Title2" => {
                out.push_str(&format!("{}b.raw(\"<h2 class=\\\"mv-type-title2\\\">\");\n", indent));
                self.gen_nodes(&c.children, out, indent);
                out.push_str(&format!("{}b.raw(\"</h2>\");\n", indent));
                Some(())
            }
            "Title3" => {
                out.push_str(&format!("{}b.raw(\"<h3 class=\\\"mv-type-title3\\\">\");\n", indent));
                self.gen_nodes(&c.children, out, indent);
                out.push_str(&format!("{}b.raw(\"</h3>\");\n", indent));
                Some(())
            }
            "Subtitle1" => {
                out.push_str(&format!("{}b.raw(\"<h4 class=\\\"mv-type-subtitle1\\\">\");\n", indent));
                self.gen_nodes(&c.children, out, indent);
                out.push_str(&format!("{}b.raw(\"</h4>\");\n", indent));
                Some(())
            }
            "Subtitle2" => {
                out.push_str(&format!("{}b.raw(\"<h5 class=\\\"mv-type-subtitle2\\\">\");\n", indent));
                self.gen_nodes(&c.children, out, indent);
                out.push_str(&format!("{}b.raw(\"</h5>\");\n", indent));
                Some(())
            }
            "Body1" => {
                let tag = if prop("strong").is_some() { "strong" } else { "span" };
                let cls = if prop("strong").is_some() { "mv-type-body1 mv-type-strong" } else { "mv-type-body1" };
                out.push_str(&format!("{}b.raw(\"<{} class=\\\"{}\\\">\");\n", indent, tag, cls));
                self.gen_nodes(&c.children, out, indent);
                out.push_str(&format!("{}b.raw(\"</{}>\");\n", indent, tag));
                Some(())
            }
            "Body2" => {
                let tag = if prop("strong").is_some() { "strong" } else { "span" };
                let cls = if prop("strong").is_some() { "mv-type-body2 mv-type-strong" } else { "mv-type-body2" };
                out.push_str(&format!("{}b.raw(\"<{} class=\\\"{}\\\">\");\n", indent, tag, cls));
                self.gen_nodes(&c.children, out, indent);
                out.push_str(&format!("{}b.raw(\"</{}>\");\n", indent, tag));
                Some(())
            }
            "Caption1" => {
                let tag = if prop("strong").is_some() { "strong" } else { "span" };
                let cls = if prop("strong").is_some() { "mv-type-caption1 mv-type-strong" } else { "mv-type-caption1" };
                out.push_str(&format!("{}b.raw(\"<{} class=\\\"{}\\\">\");\n", indent, tag, cls));
                self.gen_nodes(&c.children, out, indent);
                out.push_str(&format!("{}b.raw(\"</{}>\");\n", indent, tag));
                Some(())
            }
            "Caption2" => {
                let tag = if prop("strong").is_some() { "strong" } else { "span" };
                let cls = if prop("strong").is_some() { "mv-type-caption2 mv-type-strong" } else { "mv-type-caption2" };
                out.push_str(&format!("{}b.raw(\"<{} class=\\\"{}\\\">\");\n", indent, tag, cls));
                self.gen_nodes(&c.children, out, indent);
                out.push_str(&format!("{}b.raw(\"</{}>\");\n", indent, tag));
                Some(())
            }
            "Display" => {
                out.push_str(&format!("{}b.raw(\"<h1 class=\\\"mv-type-display\\\">\");\n", indent));
                self.gen_nodes(&c.children, out, indent);
                out.push_str(&format!("{}b.raw(\"</h1>\");\n", indent));
                Some(())
            }
            "LargeTitle" => {
                out.push_str(&format!("{}b.raw(\"<h1 class=\\\"mv-type-largetitle\\\">\");\n", indent));
                self.gen_nodes(&c.children, out, indent);
                out.push_str(&format!("{}b.raw(\"</h1>\");\n", indent));
                Some(())
            }
            "CardHeader" => {
                // Card header row: optional leading image/avatar (passed as children),
                // a header line + an optional description line. Composes inside Card.
                out.push_str(&format!("{}b.raw(\"<div class=\\\"mv-card-header\\\">\");\n", indent));
                // Leading media slot: any children render before the text block.
                if !c.children.is_empty() {
                    out.push_str(&format!("{}b.raw(\"<div class=\\\"mv-card-header-media\\\">\");\n", indent));
                    self.gen_nodes(&c.children, out, indent);
                    out.push_str(&format!("{}b.raw(\"</div>\");\n", indent));
                }
                out.push_str(&format!("{}b.raw(\"<div class=\\\"mv-card-header-text\\\">\");\n", indent));
                if let Some(h) = prop("header") {
                    out.push_str(&format!("{}b.raw(\"<span class=\\\"mv-card-header-title\\\">\");\n", indent));
                    self.gen_attr_text(h, out, indent);
                    out.push_str(&format!("{}b.raw(\"</span>\");\n", indent));
                }
                if let Some(d) = prop("description") {
                    out.push_str(&format!("{}b.raw(\"<span class=\\\"mv-card-header-desc\\\">\");\n", indent));
                    self.gen_attr_text(d, out, indent);
                    out.push_str(&format!("{}b.raw(\"</span>\");\n", indent));
                }
                out.push_str(&format!("{}b.raw(\"</div></div>\");\n", indent));
                Some(())
            }
            "CardPreview" => {
                // Full-bleed media area inside a card. If `src` is given, emit an <img>;
                // otherwise wrap arbitrary children (e.g. a chart / custom media).
                out.push_str(&format!("{}b.raw(\"<div class=\\\"mv-card-preview\\\">\");\n", indent));
                if let Some(s) = prop("src") {
                    out.push_str(&format!("{}b.raw(\"<img class=\\\"mv-card-preview-img\\\" src=\\\"\");\n", indent));
                    self.gen_attr_text(s, out, indent);
                    out.push_str(&format!("{}b.raw(\"\\\" alt=\\\"\");\n", indent));
                    if let Some(a) = prop("alt") { self.gen_attr_text(a, out, indent); }
                    out.push_str(&format!("{}b.raw(\"\\\">\");\n", indent));
                }
                self.gen_nodes(&c.children, out, indent);
                out.push_str(&format!("{}b.raw(\"</div>\");\n", indent));
                Some(())
            }
            "CardFooter" => {
                // Actions row pinned to the bottom of a card. Children are the actions
                // (typically <Button/>s). Composes inside Card.
                out.push_str(&format!("{}b.raw(\"<div class=\\\"mv-card-footer\\\">\");\n", indent));
                self.gen_nodes(&c.children, out, indent);
                out.push_str(&format!("{}b.raw(\"</div>\");\n", indent));
                Some(())
            }
            "Tree" => {
                let mut trcls = "mv-tree".to_string();
                if let Some(sz) = lit("size") { trcls.push_str(&format!(" mv-tree-{}", sz)); }
                if let Some(ap) = lit("appearance") { trcls.push_str(&format!(" mv-tree-{}", ap)); }
                out.push_str(&format!("{}b.raw(\"<div class=\\\"{}\\\" role=\\\"tree\\\">\");\n", indent, trcls));
                self.gen_nodes(&c.children, out, indent);
                out.push_str(&format!("{}b.raw(\"</div>\");\n", indent));
                Some(())
            }
            "TreeItem" => {
                let open = if prop("open").is_some() { " open" } else { "" };
                // A leaf (no children) gets a modifier so its chevron is hidden.
                let leaf = if c.children.is_empty() { " mv-treeitem-leaf" } else { "" };
                out.push_str(&format!("{}b.raw(\"<details class=\\\"mv-treeitem{}\\\" role=\\\"treeitem\\\"{}><summary>\");\n", indent, leaf, open));
                if let Some(l) = prop("label") { self.gen_attr_text(l, out, indent); }
                out.push_str(&format!("{}b.raw(\"</summary>\");\n", indent));
                if !c.children.is_empty() {
                    out.push_str(&format!("{}b.raw(\"<div class=\\\"mv-treeitem-group\\\" role=\\\"group\\\">\");\n", indent));
                    self.gen_nodes(&c.children, out, indent);
                    out.push_str(&format!("{}b.raw(\"</div>\");\n", indent));
                }
                out.push_str(&format!("{}b.raw(\"</details>\");\n", indent));
                Some(())
            }
            "Toolbar" => {
                let size = lit("size").unwrap_or_else(|| "medium".into());
                let mut tbcls = format!("mv-toolbar mv-toolbar-{}", size);
                if lit("appearance").as_deref() == Some("surface") { tbcls.push_str(" mv-toolbar-surface"); }
                if lit("orientation").as_deref() == Some("vertical") { tbcls.push_str(" mv-toolbar-vertical"); }
                out.push_str(&format!("{}b.raw(\"<div class=\\\"{}\\\" role=\\\"toolbar\\\">\");\n", indent, esc_lit(&tbcls)));
                self.gen_nodes(&c.children, out, indent);
                out.push_str(&format!("{}b.raw(\"</div>\");\n", indent));
                Some(())
            }
            "ToolbarGroup" => {
                out.push_str(&format!("{}b.raw(\"<div class=\\\"mv-toolbar-group\\\" role=\\\"group\\\">\");\n", indent));
                self.gen_nodes(&c.children, out, indent);
                out.push_str(&format!("{}b.raw(\"</div>\");\n", indent));
                Some(())
            }
            "ToolbarDivider" => {
                out.push_str(&format!("{}b.raw(\"<span class=\\\"mv-toolbar-divider\\\" role=\\\"separator\\\" aria-orientation=\\\"vertical\\\"></span>\");\n", indent));
                Some(())
            }
            "Carousel" => {
                out.push_str(&format!("{}b.raw(\"<div class=\\\"mv-carousel\\\">\");\n", indent));
                for child in &c.children {
                    out.push_str(&format!("{}b.raw(\"<div class=\\\"mv-carousel-slide\\\">\");\n", indent));
                    self.gen_node(child, out, indent);
                    out.push_str(&format!("{}b.raw(\"</div>\");\n", indent));
                }
                out.push_str(&format!("{}b.raw(\"</div>\");\n", indent));
                Some(())
            }
            "TagGroup" => {
                let size = lit("size");
                let mut cls = String::from("mv-taggroup");
                if let Some(sz) = &size { cls.push_str(&format!(" mv-taggroup-{}", esc_lit(sz))); }
                out.push_str(&format!("{}b.raw(\"<div class=\\\"{}\\\" role=\\\"list\\\">\");\n", indent, cls));
                self.gen_nodes(&c.children, out, indent);
                out.push_str(&format!("{}b.raw(\"</div>\");\n", indent));
                Some(())
            }
            "InteractionTag" => {
                let mut cls = String::from("mv-tag mv-interactiontag");
                if let Some(sz) = lit("size") { cls.push_str(&format!(" mv-tag-{}", esc_lit(&sz))); }
                out.push_str(&format!("{}b.raw(\"<span class=\\\"{}\\\" role=\\\"listitem\\\">\");\n", indent, cls));
                // Primary clickable area (a button so it is keyboard-focusable).
                out.push_str(&format!("{}b.raw(\"<button type=\\\"button\\\" class=\\\"mv-interactiontag-primary\\\"\");\n", indent));
                if let Some(v) = lit("value") { out.push_str(&format!("{}b.raw(\" value=\\\"{}\\\"\");\n", indent, esc_lit(&v))); }
                // pass through @click on the primary area
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
                if prop("dismissible").is_some() {
                    out.push_str(&format!("{}b.raw(\"<button type=\\\"button\\\" class=\\\"mv-tag-dismiss mv-interactiontag-dismiss\\\" aria-label=\\\"Dismiss\\\"\");\n", indent));
                    if let Some(v) = lit("value") { out.push_str(&format!("{}b.raw(\" value=\\\"{}\\\"\");\n", indent, esc_lit(&v))); }
                    out.push_str(&format!("{}b.raw(\"></button>\");\n", indent));
                }
                out.push_str(&format!("{}b.raw(\"</span>\");\n", indent));
                Some(())
            }
            "MenuItemCheckbox" => {
                let name = esc_lit(&lit("name").unwrap_or_default());
                let value = esc_lit(&lit("value").unwrap_or_default());
                out.push_str(&format!("{}b.raw(\"<label class=\\\"mv-menu-item mv-menu-checkitem\\\"><input type=\\\"checkbox\\\" class=\\\"mv-menu-checkinput\\\" name=\\\"{}\\\" value=\\\"{}\\\"\");\n", indent, name, value));
                if prop("checked").is_some() { out.push_str(&format!("{}b.raw(\" checked\");\n", indent)); }
                out.push_str(&format!("{}b.raw(\"><span class=\\\"mv-menu-check\\\"></span><span class=\\\"mv-menu-itemlabel\\\">\");\n", indent));
                self.gen_nodes(&c.children, out, indent);
                out.push_str(&format!("{}b.raw(\"</span></label>\");\n", indent));
                Some(())
            }
            "MenuItemRadio" => {
                let name = esc_lit(&lit("name").unwrap_or_default());
                let value = esc_lit(&lit("value").unwrap_or_default());
                out.push_str(&format!("{}b.raw(\"<label class=\\\"mv-menu-item mv-menu-radioitem\\\"><input type=\\\"radio\\\" class=\\\"mv-menu-radioinput\\\" name=\\\"{}\\\" value=\\\"{}\\\"\");\n", indent, name, value));
                if prop("checked").is_some() { out.push_str(&format!("{}b.raw(\" checked\");\n", indent)); }
                out.push_str(&format!("{}b.raw(\"><span class=\\\"mv-menu-radio\\\"></span><span class=\\\"mv-menu-itemlabel\\\">\");\n", indent));
                self.gen_nodes(&c.children, out, indent);
                out.push_str(&format!("{}b.raw(\"</span></label>\");\n", indent));
                Some(())
            }
            // ---- Radial / arc chart family -----------------------------------
            // All render as server-side SVG via runtime/src/Charts.mo. Data is
            // passed as CSV-style STRING props (literal OR a single @(expr)).
            "PieChart" => {
                let values = str_expr("values");
                let labels = str_expr("labels");
                let opts = chart_opts(&[]);
                out.push_str(&format!("{}b.raw(Charts.pie({}, {}, {}));\n", indent, values, labels, opts));
                Some(())
            }
            "DonutChart" => {
                let values = str_expr("values");
                let labels = str_expr("labels");
                let opts = chart_opts(&[]);
                out.push_str(&format!("{}b.raw(Charts.donut({}, {}, {}));\n", indent, values, labels, opts));
                Some(())
            }
            "SemiDonutChart" => {
                let values = str_expr("values");
                let labels = str_expr("labels");
                let opts = chart_opts(&[]);
                out.push_str(&format!("{}b.raw(Charts.semiDonut({}, {}, {}));\n", indent, values, labels, opts));
                Some(())
            }
            "GaugeChart" => {
                // Single value (only the first is used). yMin/yMax bound the arc.
                let values = str_expr("values");
                let opts = chart_opts(&[]);
                out.push_str(&format!("{}b.raw(Charts.gauge({}, {}));\n", indent, values, opts));
                Some(())
            }
            "RadarChart" => {
                // Multi-series spec ("A:1,2,3;B:4,5,6") + shared axis labels.
                let series = str_expr("series");
                let labels = str_expr("labels");
                let opts = chart_opts(&[]);
                out.push_str(&format!("{}b.raw(Charts.radar({}, {}, {}));\n", indent, series, labels, opts));
                Some(())
            }
            "RadialBarChart" => {
                let values = str_expr("values");
                let labels = str_expr("labels");
                let opts = chart_opts(&[]);
                out.push_str(&format!("{}b.raw(Charts.radialBar({}, {}, {}));\n", indent, values, labels, opts));
                Some(())
            }
            // ---- Axis / rectangular chart family -----------------------------
            // Same string-prop convention (literal OR a single @(expr)).
            "BarChart" => {
                // Horizontal bars: values + labels.
                let values = str_expr("values");
                let labels = str_expr("labels");
                let opts = chart_opts(&[]);
                out.push_str(&format!("{}b.raw(Charts.bar({}, {}, {}));\n", indent, values, labels, opts));
                Some(())
            }
            "ColumnChart" => {
                // Vertical columns: values + labels.
                let values = str_expr("values");
                let labels = str_expr("labels");
                let opts = chart_opts(&[]);
                out.push_str(&format!("{}b.raw(Charts.column({}, {}, {}));\n", indent, values, labels, opts));
                Some(())
            }
            "GroupedColumnChart" => {
                // Multi-series spec ("A:1,2,3;B:4,5,6") + shared category labels.
                let series = str_expr("series");
                let labels = str_expr("labels");
                let opts = chart_opts(&[]);
                out.push_str(&format!("{}b.raw(Charts.groupedColumn({}, {}, {}));\n", indent, series, labels, opts));
                Some(())
            }
            "StackedColumnChart" => {
                // Multi-series spec stacked per category + shared category labels.
                let series = str_expr("series");
                let labels = str_expr("labels");
                let opts = chart_opts(&[]);
                out.push_str(&format!("{}b.raw(Charts.stackedColumn({}, {}, {}));\n", indent, series, labels, opts));
                Some(())
            }
            "ScatterChart" => {
                // XY points ("1,2;3,5;4,4").
                let points = str_expr("points");
                let opts = chart_opts(&[]);
                out.push_str(&format!("{}b.raw(Charts.scatter({}, {}));\n", indent, points, opts));
                Some(())
            }
            "BubbleChart" => {
                // XYZ points ("x,y,size"): the 3rd value is the bubble magnitude.
                let points = str_expr("points");
                let opts = chart_opts(&[]);
                out.push_str(&format!("{}b.raw(Charts.bubble({}, {}));\n", indent, points, opts));
                Some(())
            }
            "LineChart" => {
                let data = if c.props.iter().any(|a| a.name == "series") { str_expr("series") } else { str_expr("values") };
                let labels = str_expr("labels");
                let opts = chart_opts(&[]);
                out.push_str(&format!("{}b.raw(Charts.line({}, {}, {}));\n", indent, data, labels, opts));
                Some(())
            }
            "SplineChart" => {
                let data = if c.props.iter().any(|a| a.name == "series") { str_expr("series") } else { str_expr("values") };
                let labels = str_expr("labels");
                let opts = chart_opts(&[]);
                out.push_str(&format!("{}b.raw(Charts.spline({}, {}, {}));\n", indent, data, labels, opts));
                Some(())
            }
            "StepLineChart" => {
                let data = if c.props.iter().any(|a| a.name == "series") { str_expr("series") } else { str_expr("values") };
                let labels = str_expr("labels");
                let opts = chart_opts(&[]);
                out.push_str(&format!("{}b.raw(Charts.stepLine({}, {}, {}));\n", indent, data, labels, opts));
                Some(())
            }
            "AreaChart" => {
                let data = if c.props.iter().any(|a| a.name == "series") { str_expr("series") } else { str_expr("values") };
                let labels = str_expr("labels");
                let opts = chart_opts(&[]);
                out.push_str(&format!("{}b.raw(Charts.area({}, {}, {}));\n", indent, data, labels, opts));
                Some(())
            }
            "StackedAreaChart" => {
                let data = if c.props.iter().any(|a| a.name == "series") { str_expr("series") } else { str_expr("values") };
                let labels = str_expr("labels");
                let opts = chart_opts(&[]);
                out.push_str(&format!("{}b.raw(Charts.stackedArea({}, {}, {}));\n", indent, data, labels, opts));
                Some(())
            }
            "Sparkline" => {
                let values = str_expr("values");
                let opts = chart_opts(&[]);
                out.push_str(&format!("{}b.raw(Charts.sparkline({}, {}));\n", indent, values, opts));
                Some(())
            }
            "LollipopChart" => {
                // Stem+dot per (label,value). Same data as ColumnChart.
                let values = str_expr("values");
                let labels = str_expr("labels");
                let opts = chart_opts(&[]);
                out.push_str(&format!("{}b.raw(Charts.lollipop({}, {}, {}));\n", indent, values, labels, opts));
                Some(())
            }
            "BulletChart" => {
                // rows="name:value:target:b1,b2,b3;..." — measure vs target over bands.
                let rows = str_expr("rows");
                let opts = chart_opts(&[]);
                out.push_str(&format!("{}b.raw(Charts.bullet({}, {}));\n", indent, rows, opts));
                Some(())
            }
            "DotPlot" => {
                // One dot per category along a shared value axis. Same data as BarChart.
                let values = str_expr("values");
                let labels = str_expr("labels");
                let opts = chart_opts(&[]);
                out.push_str(&format!("{}b.raw(Charts.dotPlot({}, {}, {}));\n", indent, values, labels, opts));
                Some(())
            }
            "DumbbellChart" => {
                // rows="label:start,end;..." + named endpoints for the legend.
                let rows = str_expr("rows");
                let start_name = str_expr("startName");
                let end_name = str_expr("endName");
                let opts = chart_opts(&[]);
                out.push_str(&format!("{}b.raw(Charts.dumbbell({}, {}, {}, {}));\n", indent, rows, start_name, end_name, opts));
                Some(())
            }
            "RangePlot" => {
                // rows="label:low,high;..." — floating low..high bars.
                let rows = str_expr("rows");
                let opts = chart_opts(&[]);
                out.push_str(&format!("{}b.raw(Charts.rangePlot({}, {}));\n", indent, rows, opts));
                Some(())
            }
            "SlopeChart" => {
                // rows="label:before,after;..." + named columns.
                let rows = str_expr("rows");
                let before_name = str_expr("beforeName");
                let after_name = str_expr("afterName");
                let opts = chart_opts(&[]);
                out.push_str(&format!("{}b.raw(Charts.slope({}, {}, {}, {}));\n", indent, rows, before_name, after_name, opts));
                Some(())
            }
            "DivergingBarChart" => {
                // Signed bars L/R of zero. Same data as BarChart (negatives allowed).
                let values = str_expr("values");
                let labels = str_expr("labels");
                let opts = chart_opts(&[]);
                out.push_str(&format!("{}b.raw(Charts.divergingBar({}, {}, {}));\n", indent, values, labels, opts));
                Some(())
            }
            "WaterfallChart" => {
                // deltas="v,v,.." applied to a running total; labels per step.
                let deltas = str_expr("deltas");
                let labels = str_expr("labels");
                let opts = chart_opts(&[]);
                out.push_str(&format!("{}b.raw(Charts.waterfall({}, {}, {}));\n", indent, deltas, labels, opts));
                Some(())
            }
            "PictogramChart" => {
                // value/total drawn as a grid of repeated glyphs; cols per row optional.
                let value = str_expr("value");
                let total = str_expr("total");
                let cols = str_expr("cols");
                let opts = chart_opts(&[]);
                out.push_str(&format!("{}b.raw(Charts.pictogram({}, {}, {}, {}));\n", indent, value, total, cols, opts));
                Some(())
            }
            "WaffleChart" => {
                // 100-cell share grid: values + parallel labels.
                let values = str_expr("values");
                let labels = str_expr("labels");
                let opts = chart_opts(&[]);
                out.push_str(&format!("{}b.raw(Charts.waffle({}, {}, {}));\n", indent, values, labels, opts));
                Some(())
            }
            "TreemapChart" => {
                // Flat "label:value" semicolon list -> squarified tiles.
                let data = str_expr("data");
                let opts = chart_opts(&[]);
                out.push_str(&format!("{}b.raw(Charts.treemap({}, {}));\n", indent, data, opts));
                Some(())
            }
            "FunnelChart" => {
                // Descending stage values (widest first) + stage labels.
                let values = str_expr("values");
                let labels = str_expr("labels");
                let opts = chart_opts(&[]);
                out.push_str(&format!("{}b.raw(Charts.funnel({}, {}, {}));\n", indent, values, labels, opts));
                Some(())
            }
            "PyramidChart" => {
                // Ascending stage values (apex first, base last) + labels.
                let values = str_expr("values");
                let labels = str_expr("labels");
                let opts = chart_opts(&[]);
                out.push_str(&format!("{}b.raw(Charts.pyramid({}, {}, {}));\n", indent, values, labels, opts));
                Some(())
            }
            "MarimekkoChart" => {
                // Variable-width 100% stacked columns: one series = one column.
                let series = str_expr("series");
                let labels = str_expr("labels");
                let segments = str_expr("segments");
                let opts = chart_opts(&[]);
                out.push_str(&format!("{}b.raw(Charts.marimekko({}, {}, {}, {}));\n", indent, series, labels, segments, opts));
                Some(())
            }
            "PopulationPyramid" => {
                // Two opposing horizontal bar series (left/right) by age band.
                // `left` = a 2-series spec "Male:..;Female:.."; `labels` = bands.
                let pair = str_expr("left");
                let labels = str_expr("labels");
                let opts = chart_opts(&[]);
                out.push_str(&format!("{}b.raw(Charts.populationPyramid({}, {}, {}));\n", indent, pair, labels, opts));
                Some(())
            }
            "SunburstChart" => {
                // "l1/l2:value" semicolon paths -> concentric rings (1-2 levels).
                let paths = str_expr("paths");
                let opts = chart_opts(&[]);
                out.push_str(&format!("{}b.raw(Charts.sunburst({}, {}));\n", indent, paths, opts));
                Some(())
            }
            "Histogram" => {
                // Raw values CSV + optional bin COUNT ("" / "0" = auto Sturges).
                let values = str_expr("values");
                let bins = str_expr("bins");
                let opts = chart_opts(&[]);
                out.push_str(&format!("{}b.raw(Charts.histogram({}, {}, {}));\n", indent, values, bins, opts));
                Some(())
            }
            "BoxPlot" => {
                // Labelled groups of RAW values: "A:4,7,9;B:3,5,8".
                let series = str_expr("series");
                let opts = chart_opts(&[]);
                out.push_str(&format!("{}b.raw(Charts.boxPlot({}, {}));\n", indent, series, opts));
                Some(())
            }
            "ViolinPlot" => {
                // Labelled groups of RAW values -> mirrored KDE silhouettes.
                let series = str_expr("series");
                let opts = chart_opts(&[]);
                out.push_str(&format!("{}b.raw(Charts.violinPlot({}, {}));\n", indent, series, opts));
                Some(())
            }
            "StripPlot" => {
                // Labelled groups of RAW values; one jittered dot per datum.
                let series = str_expr("series");
                let opts = chart_opts(&[]);
                out.push_str(&format!("{}b.raw(Charts.stripPlot({}, {}));\n", indent, series, opts));
                Some(())
            }
            "BeeswarmChart" => {
                // Labelled groups of RAW values; non-overlapping packed dots.
                let series = str_expr("series");
                let opts = chart_opts(&[]);
                out.push_str(&format!("{}b.raw(Charts.beeswarm({}, {}));\n", indent, series, opts));
                Some(())
            }
            "DensityPlot" => {
                // Single raw-values CSV -> one smoothed KDE curve.
                let values = str_expr("values");
                let opts = chart_opts(&[]);
                out.push_str(&format!("{}b.raw(Charts.densityPlot({}, {}));\n", indent, values, opts));
                Some(())
            }
            "RidgelinePlot" => {
                // Labelled groups of RAW values -> stacked overlapping KDE ridges.
                let series = str_expr("series");
                let opts = chart_opts(&[]);
                out.push_str(&format!("{}b.raw(Charts.ridgelinePlot({}, {}));\n", indent, series, opts));
                Some(())
            }
            "CandlestickChart" => {
                let ohlc = if c.props.iter().any(|a| a.name == "ohlc") { str_expr("ohlc") } else { str_expr("data") };
                let opts = chart_opts(&[]);
                out.push_str(&format!("{}b.raw(Charts.candlestick({}, {}));\n", indent, ohlc, opts));
                Some(())
            }
            "OHLCChart" => {
                let ohlc = if c.props.iter().any(|a| a.name == "ohlc") { str_expr("ohlc") } else { str_expr("data") };
                let opts = chart_opts(&[]);
                out.push_str(&format!("{}b.raw(Charts.ohlc({}, {}));\n", indent, ohlc, opts));
                Some(())
            }
            "GanttChart" => {
                let tasks = if c.props.iter().any(|a| a.name == "tasks") { str_expr("tasks") } else { str_expr("data") };
                let opts = chart_opts(&[]);
                out.push_str(&format!("{}b.raw(Charts.gantt({}, {}));\n", indent, tasks, opts));
                Some(())
            }
            "StreamGraph" => {
                let series = if c.props.iter().any(|a| a.name == "series") { str_expr("series") } else { str_expr("data") };
                let labels = str_expr("labels");
                let opts = chart_opts(&[]);
                out.push_str(&format!("{}b.raw(Charts.streamGraph({}, {}, {}));\n", indent, series, labels, opts));
                Some(())
            }
            "BumpChart" => {
                let series = if c.props.iter().any(|a| a.name == "series") { str_expr("series") } else { str_expr("data") };
                let labels = str_expr("labels");
                let opts = chart_opts(&[]);
                out.push_str(&format!("{}b.raw(Charts.bump({}, {}, {}));\n", indent, series, labels, opts));
                Some(())
            }
            "BarcodeChart" => {
                let events = if c.props.iter().any(|a| a.name == "events") { str_expr("events") } else { str_expr("values") };
                let categories = str_expr("categories");
                let opts = chart_opts(&[]);
                out.push_str(&format!("{}b.raw(Charts.barcode({}, {}, {}));\n", indent, events, categories, opts));
                Some(())
            }
            "Heatmap" => {
                let matrix = str_expr("matrix");
                let row_labels = str_expr("rowLabels");
                let col_labels = str_expr("colLabels");
                let opts = chart_opts(&[]);
                out.push_str(&format!("{}b.raw(Charts.heatmap({}, {}, {}, {}));\n", indent, matrix, row_labels, col_labels, opts));
                Some(())
            }
            "HexbinChart" => {
                let points = str_expr("points");
                let opts = chart_opts(&[]);
                out.push_str(&format!("{}b.raw(Charts.hexbin({}, {}));\n", indent, points, opts));
                Some(())
            }
            "ConnectedScatterChart" => {
                let points = str_expr("points");
                let point_labels = str_expr("pointLabels");
                let opts = chart_opts(&[]);
                out.push_str(&format!("{}b.raw(Charts.connectedScatter({}, {}, {}));\n", indent, points, point_labels, opts));
                Some(())
            }
            "QuadrantChart" => {
                let data = str_expr("data");
                let axis_labels = str_expr("axisLabels");
                let quad_labels = str_expr("quadLabels");
                let opts = chart_opts(&[]);
                out.push_str(&format!("{}b.raw(Charts.quadrant({}, {}, {}, {}));\n", indent, data, axis_labels, quad_labels, opts));
                Some(())
            }
            "SankeyDiagram" => {
                // Flow links: one "Source>Target:value" per ';' segment.
                let links = str_expr("links");
                let opts = chart_opts(&[]);
                out.push_str(&format!("{}b.raw(Charts.sankey({}, {}));\n", indent, links, opts));
                Some(())
            }
            "ChordDiagram" => {
                // Square weight matrix: rows by ';', cells by ','. Optional labels CSV.
                let matrix = str_expr("matrix");
                let labels = str_expr("labels");
                let opts = chart_opts(&[]);
                out.push_str(&format!("{}b.raw(Charts.chord({}, {}, {}));\n", indent, matrix, labels, opts));
                Some(())
            }
            "ArcDiagram" => {
                // Edges "A>B" (optional ":weight") per ';' segment; optional node-order CSV.
                let edges = str_expr("edges");
                let nodes = str_expr("nodes");
                let opts = chart_opts(&[]);
                out.push_str(&format!("{}b.raw(Charts.arc({}, {}, {}));\n", indent, edges, nodes, opts));
                Some(())
            }
            "Dendrogram" => {
                // "parent>child" hierarchy edges per ';' segment; root = never-a-child.
                let edges = str_expr("edges");
                let opts = chart_opts(&[]);
                out.push_str(&format!("{}b.raw(Charts.dendrogram({}, {}));\n", indent, edges, opts));
                Some(())
            }
            "VennDiagram" => {
                // "Key:size" pairs by ';'; Key in {A,B,C,AB,AC,BC,ABC}. Optional labels CSV.
                let sets = str_expr("sets");
                let labels = str_expr("labels");
                let opts = chart_opts(&[]);
                out.push_str(&format!("{}b.raw(Charts.venn({}, {}, {}));\n", indent, sets, labels, opts));
                Some(())
            }
                        "NightingaleChart" => {
                // Rose / polar-area: equal-angle wedges, radius proportional to value.
                let values = str_expr("values");
                let labels = str_expr("labels");
                let opts = chart_opts(&[]);
                out.push_str(&format!("{}b.raw(Charts.nightingale({}, {}, {}));\n", indent, values, labels, opts));
                Some(())
            }
            "RadialHistogram" => {
                // Raw observations binned into polar bars; bins<=0 => Sturges.
                let values = str_expr("values");
                let bins = str_expr("bins");
                let opts = chart_opts(&[]);
                out.push_str(&format!("{}b.raw(Charts.radialHistogram({}, {}, {}));\n", indent, values, bins, opts));
                Some(())
            }
            "ParallelCoordinates" => {
                // One vertical axis per dimension (labels); one polyline per series.
                let series = str_expr("series");
                let labels = str_expr("labels");
                let opts = chart_opts(&[]);
                out.push_str(&format!("{}b.raw(Charts.parallelCoords({}, {}, {}));\n", indent, series, labels, opts));
                Some(())
            }
            "SmallMultiples" => {
                // Trellis grid of mini charts (one per series); kind = "bar" | "line".
                let series = str_expr("series");
                let labels = str_expr("labels");
                let kind = str_expr("kind");
                let opts = chart_opts(&[]);
                out.push_str(&format!("{}b.raw(Charts.smallMultiples({}, {}, {}, {}));\n", indent, series, labels, kind, opts));
                Some(())
            }
            "CircularTreemap" => {
                // Flat "label:value" semicolon list -> area-packed circles.
                let data = str_expr("data");
                let opts = chart_opts(&[]);
                out.push_str(&format!("{}b.raw(Charts.circularTreemap({}, {}));\n", indent, data, opts));
                Some(())
            }
            "HorizonChart" => {
                // Single value series + optional parallel labels -> folded bands.
                let values = str_expr("values");
                let labels = str_expr("labels");
                let opts = chart_opts(&[]);
                out.push_str(&format!("{}b.raw(Charts.horizon({}, {}, {}));\n", indent, values, labels, opts));
                Some(())
            }
            "BumpAreaChart" => {
                // Stacked ranking areas; accepts series=".." or data="..".
                let series = if c.props.iter().any(|a| a.name == "series") { str_expr("series") } else { str_expr("data") };
                let labels = str_expr("labels");
                let opts = chart_opts(&[]);
                out.push_str(&format!("{}b.raw(Charts.bumpArea({}, {}, {}));\n", indent, series, labels, opts));
                Some(())
            }
            "WordCloud" => {
                // "word:weight" semicolon list -> font-size-weighted text rows.
                let words = if c.props.iter().any(|a| a.name == "words") { str_expr("words") } else { str_expr("data") };
                let opts = chart_opts(&[]);
                out.push_str(&format!("{}b.raw(Charts.wordCloud({}, {}));\n", indent, words, opts));
                Some(())
            }
            "MatrixChart" => {
                // Row-major numeric matrix -> size+shade square glyphs.
                let matrix = str_expr("matrix");
                let row_labels = str_expr("rowLabels");
                let col_labels = str_expr("colLabels");
                let opts = chart_opts(&[]);
                out.push_str(&format!("{}b.raw(Charts.matrix({}, {}, {}, {}));\n", indent, matrix, row_labels, col_labels, opts));
                Some(())
            }
            "TableChart" => {
                // Row-major matrix -> styled HTML <table> (NOT svg).
                let values = if c.props.iter().any(|a| a.name == "values") { str_expr("values") } else { str_expr("data") };
                let row_labels = str_expr("rowLabels");
                let col_labels = str_expr("colLabels");
                let opts = chart_opts(&[]);
                out.push_str(&format!("{}b.raw(Charts.table({}, {}, {}, {}));\n", indent, values, row_labels, col_labels, opts));
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

/// HTML-escape a compile-time literal (`&<>"`) so it is safe both as HTML and
/// inside a generated `b.raw("…")` Motoko string (no raw `"` to break the literal).
fn esc_lit(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

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
