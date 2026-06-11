//! Abstract syntax tree for a parsed `.mview` file.
//!
//! `.mview` = template markup + Motoko `@code`. The compiler parses the
//! *template* fully and scans the `@code` block for its top-level declarations
//! (so it can wire up event dispatch and infer output types); function bodies
//! are emitted to Motoko almost verbatim — `moc` does the real Motoko work.

pub use crate::span::Span;

#[derive(Debug, Clone, PartialEq)]
pub enum FileKind {
    Page,
    Layout,
    Component,
}

#[derive(Debug, Clone)]
pub struct MviewFile {
    pub kind: FileKind,
    /// Module/type name derived from the file name (e.g. "Counter", "MainLayout").
    pub name: String,
    pub route: Option<String>,
    pub layout: Option<String>,
    pub title: Option<String>,       // raw Motoko expression (string literal or expr)
    pub description: Option<String>, // raw Motoko expression
    pub canonical: Option<String>,
    pub authorize: Option<Auth>,
    /// `@cacheable` — serve this (public, non-caller-specific) page as a fast
    /// certified query instead of upgrading to an update call.
    pub cacheable: bool,
    pub head_extra: Vec<HeadMeta>,
    pub template: Vec<Node>,
    pub sections: Vec<(String, Vec<Node>)>,
    pub code: CodeBlock,
    pub style: Option<String>,
    /// `@theme "name"` — a built-in theme preset to apply (before any overrides).
    pub theme_preset: Option<String>,
    /// `@theme brand="#hex"` — generate a full Fluent brand ramp + aliases.
    pub theme_brand: Option<String>,
    /// `@theme { --mv-x: y; }` — token overrides (applied on top of the preset).
    pub theme: Vec<(String, String)>,
}

impl MviewFile {
    pub fn new(name: String, kind: FileKind) -> Self {
        MviewFile {
            kind,
            name,
            route: None,
            layout: None,
            title: None,
            description: None,
            canonical: None,
            authorize: None,
            cacheable: false,
            head_extra: Vec::new(),
            template: Vec::new(),
            sections: Vec::new(),
            code: CodeBlock::default(),
            style: None,
            theme_preset: None,
            theme_brand: None,
            theme: Vec::new(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct Auth {
    pub role: Option<String>,
    /// Where to send an unauthorized caller. `None` => the framework default
    /// "/". Lets a route whose own path is the login target (or "/") gate
    /// itself without a redirect loop, e.g. `@authorize redirect="/welcome"`.
    pub redirect: Option<String>,
}

#[derive(Debug, Clone)]
pub struct HeadMeta {
    /// raw `<meta ...>`-style html (already templated) or a key=expr meta.
    pub raw: String,
}

/// Scanned contents of the `@code { ... }` block.
///
/// Spans on the declarations below ([`VarDecl::span`], [`FuncDecl::span`]) are
/// **FILE-relative** char offsets (R11): `scan_code`/`read_func` compute them over
/// the `@code` body substring, then [`crate::parser::parse_code_block`] rebases
/// them by adding the body's file start offset (`code_offset`). So `src[span]`
/// (on the whole-file `&[char]`) yields the declaration text, and the
/// generated->source map can turn each `func`'s file line into a `moc`-error
/// target line. (Before R11 these were `@code`-body-relative; the rebase happens
/// in one place — see `parse_code_block`.)
#[derive(Debug, Clone, Default)]
pub struct CodeBlock {
    pub vars: Vec<VarDecl>,
    pub params: Vec<ParamDecl>,
    pub funcs: Vec<FuncDecl>,
    /// Anything we didn't classify (let-bindings, types, helper exprs) — emitted verbatim.
    pub extra: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct VarDecl {
    pub stable: bool,
    pub name: String,
    pub ty: Option<String>,
    pub init: Option<String>,
    /// full raw text, e.g. "var count : Nat = 0;"
    pub raw: String,
    /// Source span of the whole declaration (`@code`-relative — see note below).
    /// Additive metadata for diagnostics; codegen ignores it.
    pub span: Span,
}

#[derive(Debug, Clone)]
pub struct ParamDecl {
    pub name: String,
    pub ty: String,
    pub default: Option<String>,
}

#[derive(Debug, Clone)]
pub struct FuncDecl {
    pub name: String,
    pub params: Vec<(String, String)>, // (name, type)
    pub ret: Option<String>,           // return type with `async` stripped
    pub body: String,                  // raw body WITHOUT braces, `await` stripped
    pub is_async: bool,
    /// Source span of the whole `func ... { ... }` (`@code`-relative — see note
    /// below). Additive metadata for diagnostics; codegen ignores it.
    pub span: Span,
}

// ---- template nodes -------------------------------------------------------

// R11: `Expr`/`Raw` now carry a file-relative [`Span`] (the directive's `@…`
// position) so lint's `raw-html` warning and `@expr` diagnostics get a real
// editor position instead of `(0,0)`. The span is ADDITIVE metadata — codegen
// matches `Node::Expr(e, _)`/`Node::Raw(e, _)` and emits the same bytes, so the
// golden actor is unchanged. (`If`/`For`/`Switch` still carry no span; the
// container nodes whose spans matter for diagnostics — Element/Component/Attr/
// EventBind/FuncDecl/VarDecl — already have them.)
#[derive(Debug, Clone)]
pub enum Node {
    Text(String),
    /// `@expr` — escaped output of a Motoko expression. The [`Span`] covers the
    /// `@…` directive (file-relative); additive, ignored by codegen.
    Expr(String, Span),
    /// `@raw(expr)` — TRUSTED, unescaped HTML output (escape hatch). The
    /// expression must already be `Text` of safe HTML; never use on user input.
    /// The [`Span`] covers the `@raw(…)` directive (file-relative); additive.
    Raw(String, Span),
    If(Vec<IfBranch>),
    For {
        var: String,
        iter: String,
        body: Vec<Node>,
    },
    Switch {
        subject: String,
        cases: Vec<SwitchCase>,
    },
    Element(Element),
    Component(Component),
    Yield,
    Head,
    SectionRef(String),
    Slot(String),
    Effect {
        kind: String,
        arg: String,
    },
}

#[derive(Debug, Clone)]
pub struct IfBranch {
    pub cond: Option<String>, // None => else
    pub body: Vec<Node>,
}

#[derive(Debug, Clone)]
pub struct SwitchCase {
    pub pattern: String, // raw, e.g. "#Draft" or "(#Ok(x))"
    pub body: Vec<Node>,
}

#[derive(Debug, Clone)]
pub struct Element {
    pub tag: String,
    pub attrs: Vec<Attr>,
    pub events: Vec<EventBind>,
    pub bind: Option<String>, // Motoko lvalue, e.g. "model.name"
    pub secure: bool,
    pub children: Vec<Node>,
    pub self_closing: bool,
    /// Source span covering the open `<` through the end of the open tag
    /// (`>` / `/>`). File-relative char offsets. Additive; codegen ignores it.
    pub span: Span,
}

#[derive(Debug, Clone)]
pub struct Component {
    pub name: String,
    pub props: Vec<Attr>,
    pub events: Vec<EventBind>,
    pub slots: Vec<(String, Vec<Node>)>,
    pub children: Vec<Node>, // default slot
    pub self_closing: bool,
    /// Source span covering the open `<` through the end of the open tag
    /// (`>` / `/>`). File-relative char offsets. Additive; codegen ignores it.
    pub span: Span,
}

#[derive(Debug, Clone)]
pub struct Attr {
    pub name: String,
    pub value: AttrValue,
    /// Source span covering `name` through the end of its value (or just `name`
    /// for a boolean attr). File-relative char offsets. Additive; ignored by codegen.
    pub span: Span,
}

#[derive(Debug, Clone)]
pub enum AttrValue {
    /// boolean attribute, e.g. `required`
    Bool,
    Literal(String),
    Expr(String),
    Concat(Vec<AttrPart>),
}

#[derive(Debug, Clone)]
pub enum AttrPart {
    Lit(String),
    Expr(String),
}

#[derive(Debug, Clone)]
pub struct EventBind {
    pub event: String,    // "click" | "submit" | "input" | "change"
    pub handler: String,  // function name
    pub args: Vec<String>, // raw Motoko arg expressions
    /// Source span covering the `@event[=handler]` binding within the open tag.
    /// File-relative char offsets. Additive; ignored by codegen.
    pub span: Span,
}
