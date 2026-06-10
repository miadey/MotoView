//! Recursive-descent parser for `.mview` files.
//!
//! It fully parses the template (markup, directives, control-flow, events,
//! components) and scans the `@code` block for its top-level declarations.

use crate::ast::*;

pub fn parse(source: &str, name: &str, kind: FileKind) -> Result<MviewFile, String> {
    let mut p = Parser::new(source, name, kind);
    p.parse_file()?;
    Ok(p.file)
}

struct Parser {
    src: Vec<char>,
    i: usize,
    file: MviewFile,
}

const VOID_TAGS: &[&str] = &[
    "area", "base", "br", "col", "embed", "hr", "img", "input", "link", "meta", "param", "source",
    "track", "wbr",
];

impl Parser {
    fn new(source: &str, name: &str, kind: FileKind) -> Self {
        Parser {
            src: source.chars().collect(),
            i: 0,
            file: MviewFile::new(name.to_string(), kind),
        }
    }

    // ---- low-level cursor helpers ----
    fn eof(&self) -> bool {
        self.i >= self.src.len()
    }
    fn peek(&self) -> char {
        if self.eof() { '\0' } else { self.src[self.i] }
    }
    fn peek_at(&self, off: usize) -> char {
        let j = self.i + off;
        if j >= self.src.len() { '\0' } else { self.src[j] }
    }
    fn bump(&mut self) -> char {
        let c = self.peek();
        self.i += 1;
        c
    }
    fn starts_with(&self, s: &str) -> bool {
        let chars: Vec<char> = s.chars().collect();
        if self.i + chars.len() > self.src.len() {
            return false;
        }
        for (k, ch) in chars.iter().enumerate() {
            if self.src[self.i + k] != *ch {
                return false;
            }
        }
        true
    }
    fn skip_ws(&mut self) {
        while !self.eof() && self.peek().is_whitespace() {
            self.i += 1;
        }
    }
    fn skip_spaces(&mut self) {
        while !self.eof() && (self.peek() == ' ' || self.peek() == '\t') {
            self.i += 1;
        }
    }
    fn read_line(&mut self) -> String {
        let mut s = String::new();
        while !self.eof() && self.peek() != '\n' {
            s.push(self.bump());
        }
        if !self.eof() {
            self.bump();
        }
        s.trim().to_string()
    }

    /// The identifier immediately following an `@` (without consuming).
    fn at_keyword(&self) -> Option<String> {
        if self.peek() != '@' {
            return None;
        }
        let mut k = String::new();
        let mut j = self.i + 1;
        while j < self.src.len() && (self.src[j].is_alphanumeric() || self.src[j] == '_') {
            k.push(self.src[j]);
            j += 1;
        }
        if k.is_empty() {
            None
        } else {
            Some(k)
        }
    }

    // ---- top-level ----
    fn parse_file(&mut self) -> Result<(), String> {
        loop {
            self.skip_ws();
            if self.eof() {
                break;
            }
            // Bare top-level `param NAME : TYPE [= DEFAULT]` — component parameters
            // (and any page-level params), declared outside @code.
            if self.starts_with("param") && matches!(self.peek_at(5), ' ' | '\t') {
                self.i += 5;
                let line = self.read_line();
                if let Some(pd) = parse_param_decl(line.trim()) {
                    self.file.code.params.push(pd);
                }
                continue;
            }
            if let Some(kw) = self.at_keyword() {
                match kw.as_str() {
                    "page" | "layout" | "title" | "description" | "canonical" | "authorize"
                    | "meta" => {
                        self.parse_directive_line(&kw)?;
                        continue;
                    }
                    "cacheable" => {
                        self.consume_keyword();
                        self.file.cacheable = true;
                        continue;
                    }
                    "code" => {
                        self.parse_code_block()?;
                        continue;
                    }
                    "style" => {
                        self.consume_keyword();
                        let body = self.read_brace_block()?;
                        self.file.style = Some(body);
                        continue;
                    }
                    "theme" => {
                        self.consume_keyword();
                        self.skip_spaces();
                        // optional brand ramp: @theme brand="#0f6cbd"
                        if self.starts_with("brand") {
                            for _ in 0..5 { self.bump(); }
                            self.skip_spaces();
                            if self.peek() == '=' { self.bump(); }
                            self.skip_spaces();
                            if self.peek() == '"' {
                                self.file.theme_brand = Some(self.read_quoted()?);
                            }
                            self.skip_spaces();
                        }
                        // optional preset name: @theme "ocean"
                        if self.peek() == '"' {
                            self.file.theme_preset = Some(self.read_quoted()?);
                            self.skip_spaces();
                        }
                        // optional token overrides: @theme { --mv-x: y; }
                        if self.peek() == '{' {
                            let body = self.read_brace_block()?;
                            self.file.theme = parse_theme(&body);
                        }
                        continue;
                    }
                    "section" => {
                        // top-level section WITH a body (page side)
                        if self.section_has_body() {
                            self.parse_section_block()?;
                            continue;
                        }
                        // else: a placeholder inside template -> fall through
                    }
                    _ => {}
                }
            }
            // template content
            let nodes = self.parse_nodes(StopAt::TopLevel)?;
            self.file.template.extend(nodes);
        }
        Ok(())
    }

    fn consume_keyword(&mut self) {
        // consume '@' + identifier
        self.bump(); // @
        while !self.eof() && (self.peek().is_alphanumeric() || self.peek() == '_') {
            self.bump();
        }
    }

    fn parse_directive_line(&mut self, kw: &str) -> Result<(), String> {
        self.consume_keyword();
        self.skip_spaces();
        let rest = self.read_line();
        match kw {
            "page" => self.file.route = Some(unquote(&rest)),
            "layout" => self.file.layout = Some(rest.trim().to_string()),
            "title" => self.file.title = Some(expr_or_literal(&rest)),
            "description" => self.file.description = Some(expr_or_literal(&rest)),
            "canonical" => self.file.canonical = Some(expr_or_literal(&rest)),
            "authorize" => {
                let role = extract_attr(&rest, "role");
                self.file.authorize = Some(Auth { role });
            }
            "meta" => self.file.head_extra.push(HeadMeta { raw: rest }),
            _ => {}
        }
        Ok(())
    }

    fn section_has_body(&self) -> bool {
        // peek past `@section "name"` to see if a `{` follows
        let mut j = self.i + "@section".len();
        while j < self.src.len() && self.src[j].is_whitespace() {
            j += 1;
        }
        if j < self.src.len() && self.src[j] == '"' {
            j += 1;
            while j < self.src.len() && self.src[j] != '"' {
                j += 1;
            }
            j += 1; // closing quote
        }
        while j < self.src.len() && self.src[j].is_whitespace() {
            j += 1;
        }
        j < self.src.len() && self.src[j] == '{'
    }

    fn parse_section_block(&mut self) -> Result<(), String> {
        self.consume_keyword(); // @section
        self.skip_spaces();
        let name = self.read_quoted()?;
        self.skip_ws();
        self.expect('{')?;
        let body = self.parse_nodes(StopAt::Brace)?;
        self.expect('}')?;
        self.file.sections.push((name, body));
        Ok(())
    }

    // ---- @code scanning ----
    fn parse_code_block(&mut self) -> Result<(), String> {
        self.consume_keyword(); // @code
        let body = self.read_brace_block()?;
        self.file.code = scan_code(&body);
        Ok(())
    }

    /// Read a `{ ... }` block (cursor anywhere before the `{`), returning the
    /// inner text. Brace counting respects Motoko strings and comments.
    fn read_brace_block(&mut self) -> Result<String, String> {
        self.skip_ws();
        self.expect('{')?;
        let start = self.i;
        let mut depth = 1;
        while !self.eof() {
            let c = self.peek();
            match c {
                '"' => self.skip_string('"'),
                '\'' => self.skip_string('\''),
                '/' if self.peek_at(1) == '/' => {
                    while !self.eof() && self.peek() != '\n' {
                        self.bump();
                    }
                }
                '/' if self.peek_at(1) == '*' => {
                    self.bump();
                    self.bump();
                    while !self.eof() && !(self.peek() == '*' && self.peek_at(1) == '/') {
                        self.bump();
                    }
                    self.bump();
                    self.bump();
                }
                '{' => {
                    depth += 1;
                    self.bump();
                }
                '}' => {
                    depth -= 1;
                    if depth == 0 {
                        let body: String = self.src[start..self.i].iter().collect();
                        self.bump(); // consume }
                        return Ok(body);
                    }
                    self.bump();
                }
                _ => {
                    self.bump();
                }
            }
        }
        Err("unterminated block".into())
    }

    fn skip_string(&mut self, q: char) {
        self.bump(); // opening quote
        while !self.eof() {
            let c = self.bump();
            if c == '\\' {
                self.bump();
            } else if c == q {
                break;
            }
        }
    }

    fn read_quoted(&mut self) -> Result<String, String> {
        self.skip_ws();
        if self.peek() != '"' {
            return Err("expected '\"'".into());
        }
        self.bump();
        let mut s = String::new();
        while !self.eof() && self.peek() != '"' {
            s.push(self.bump());
        }
        self.bump();
        Ok(s)
    }

    fn expect(&mut self, c: char) -> Result<(), String> {
        self.skip_ws();
        if self.peek() == c {
            self.bump();
            Ok(())
        } else {
            Err(format!("expected '{}' at position {}", c, self.i))
        }
    }

    // ---- template node parsing ----
    fn parse_nodes(&mut self, stop: StopAt) -> Result<Vec<Node>, String> {
        let mut nodes = Vec::new();
        let mut text = String::new();
        macro_rules! flush {
            () => {
                if !text.is_empty() {
                    nodes.push(Node::Text(std::mem::take(&mut text)));
                }
            };
        }
        loop {
            if self.eof() {
                break;
            }
            // stop conditions
            match stop {
                StopAt::Brace if self.peek() == '}' => break,
                StopAt::EndTag(_) if self.peek() == '<' && self.peek_at(1) == '/' => break,
                StopAt::TopLevel => {
                    if let Some(kw) = self.at_keyword() {
                        if is_top_level_kw(&kw) {
                            // a section placeholder still belongs to the template
                            if kw == "section" && !self.section_has_body() {
                                // handled below as a node
                            } else {
                                break;
                            }
                        }
                    }
                }
                _ => {}
            }

            let c = self.peek();
            if c == '<' {
                if self.peek_at(1) == '!' {
                    // doctype / comment passthrough as text
                    text.push(self.bump());
                    continue;
                }
                flush!();
                nodes.push(self.parse_tag()?);
            } else if c == '@' {
                if self.peek_at(1) == '@' {
                    text.push('@');
                    self.bump();
                    self.bump();
                    continue;
                }
                let kw = self.at_keyword().unwrap_or_default();
                match kw.as_str() {
                    "if" => {
                        flush!();
                        nodes.push(self.parse_if()?);
                    }
                    "for" => {
                        flush!();
                        nodes.push(self.parse_for()?);
                    }
                    "switch" => {
                        flush!();
                        nodes.push(self.parse_switch()?);
                    }
                    "yield" => {
                        flush!();
                        self.consume_keyword();
                        nodes.push(Node::Yield);
                    }
                    "head" => {
                        flush!();
                        self.consume_keyword();
                        nodes.push(Node::Head);
                    }
                    "slot" => {
                        flush!();
                        self.consume_keyword();
                        self.skip_spaces();
                        let n = self.read_quoted()?;
                        nodes.push(Node::Slot(n));
                    }
                    "section" => {
                        // placeholder (no body): @section "name"
                        flush!();
                        self.consume_keyword();
                        self.skip_spaces();
                        let n = self.read_quoted()?;
                        nodes.push(Node::SectionRef(n));
                    }
                    "effect" | "animate" => {
                        flush!();
                        nodes.push(self.parse_effect()?);
                    }
                    "raw" => {
                        // @raw(expr) — emit trusted HTML unescaped. Only a directive
                        // when followed by `(`; otherwise `raw` is a normal @expr.
                        flush!();
                        self.consume_keyword();
                        self.skip_spaces();
                        if self.peek() == '(' {
                            let e = self.read_paren_inner();
                            nodes.push(Node::Raw(e));
                        } else {
                            nodes.push(Node::Expr("raw".to_string()));
                        }
                    }
                    _ => {
                        // @expr
                        flush!();
                        let e = self.read_at_expr();
                        nodes.push(Node::Expr(e));
                    }
                }
            } else {
                text.push(self.bump());
            }
        }
        flush!();
        Ok(nodes)
    }

    fn parse_if(&mut self) -> Result<Node, String> {
        let mut branches = Vec::new();
        // @if COND { BODY }
        self.consume_keyword(); // if
        let cond = self.read_until_brace();
        self.expect('{')?;
        let body = self.parse_nodes(StopAt::Brace)?;
        self.expect('}')?;
        branches.push(IfBranch {
            cond: Some(cond.trim().to_string()),
            body,
        });
        // else / else if chains
        loop {
            let save = self.i;
            self.skip_ws();
            if self.starts_with("else") {
                self.i += 4;
                self.skip_ws();
                if self.starts_with("if") {
                    self.i += 2;
                    let cond = self.read_until_brace();
                    self.expect('{')?;
                    let body = self.parse_nodes(StopAt::Brace)?;
                    self.expect('}')?;
                    branches.push(IfBranch {
                        cond: Some(cond.trim().to_string()),
                        body,
                    });
                } else {
                    self.expect('{')?;
                    let body = self.parse_nodes(StopAt::Brace)?;
                    self.expect('}')?;
                    branches.push(IfBranch { cond: None, body });
                    break;
                }
            } else {
                self.i = save;
                break;
            }
        }
        Ok(Node::If(branches))
    }

    fn parse_for(&mut self) -> Result<Node, String> {
        self.consume_keyword(); // for
        self.skip_spaces();
        let mut var = String::new();
        while !self.eof() && (self.peek().is_alphanumeric() || self.peek() == '_') {
            var.push(self.bump());
        }
        self.skip_spaces();
        // expect "in"
        if self.starts_with("in") {
            self.i += 2;
        }
        let iter = self.read_until_brace().trim().to_string();
        self.expect('{')?;
        let body = self.parse_nodes(StopAt::Brace)?;
        self.expect('}')?;
        Ok(Node::For { var, iter, body })
    }

    fn parse_switch(&mut self) -> Result<Node, String> {
        self.consume_keyword(); // switch
        let subject = self.read_until_brace().trim().to_string();
        self.expect('{')?;
        let mut cases = Vec::new();
        loop {
            self.skip_ws();
            if self.peek() == '}' {
                break;
            }
            if self.starts_with("case") {
                self.i += 4;
                let pat = self.read_until_brace().trim().to_string();
                self.expect('{')?;
                let body = self.parse_nodes(StopAt::Brace)?;
                self.expect('}')?;
                cases.push(SwitchCase { pattern: pat, body });
                self.skip_ws();
                if self.peek() == ';' {
                    self.bump();
                }
            } else {
                break;
            }
        }
        self.expect('}')?;
        Ok(Node::Switch { subject, cases })
    }

    fn parse_effect(&mut self) -> Result<Node, String> {
        // @effect Focus("#x")  /  @animate FadeIn("#p")
        let is_animate = self.at_keyword().as_deref() == Some("animate");
        self.consume_keyword();
        self.skip_spaces();
        let rest = self.read_line();
        // rest like: Focus("#email")  or  Toast("Saved")
        let (kind, arg) = parse_call(&rest);
        let kind = if is_animate {
            format!("animate:{}", kind)
        } else {
            kind.to_lowercase()
        };
        Ok(Node::Effect { kind, arg })
    }

    fn read_until_brace(&mut self) -> String {
        let mut s = String::new();
        while !self.eof() && self.peek() != '{' {
            s.push(self.bump());
        }
        s
    }

    /// Read a balanced `( ... )` at the cursor, returning the trimmed inner text.
    fn read_paren_inner(&mut self) -> String {
        let mut depth = 0;
        let mut s = String::new();
        loop {
            let c = self.peek();
            if c == '\0' {
                break;
            }
            if c == '(' {
                depth += 1;
                self.bump();
                if depth == 1 {
                    continue;
                }
            } else if c == ')' {
                depth -= 1;
                self.bump();
                if depth == 0 {
                    break;
                }
                s.push(')');
                continue;
            } else {
                self.bump();
            }
            s.push(c);
        }
        s.trim().to_string()
    }

    /// Read an `@expr` (cursor at '@'). Supports `@(expr)` and member/call chains.
    fn read_at_expr(&mut self) -> String {
        self.bump(); // @
        if self.peek() == '(' {
            // balanced parens
            let mut depth = 0;
            let mut s = String::new();
            loop {
                let c = self.peek();
                if c == '\0' {
                    break;
                }
                if c == '(' {
                    depth += 1;
                    self.bump();
                    if depth == 1 {
                        continue;
                    }
                } else if c == ')' {
                    depth -= 1;
                    self.bump();
                    if depth == 0 {
                        break;
                    }
                    s.push(')');
                    continue;
                } else {
                    self.bump();
                }
                s.push(c);
            }
            return s.trim().to_string();
        }
        // ident ( . ident | ( ... ) )*
        let mut s = String::new();
        while !self.eof() && (self.peek().is_alphanumeric() || self.peek() == '_') {
            s.push(self.bump());
        }
        loop {
            if self.peek() == '.'
                && (self.peek_at(1).is_alphabetic() || self.peek_at(1) == '_')
            {
                s.push(self.bump()); // .
                while !self.eof() && (self.peek().is_alphanumeric() || self.peek() == '_') {
                    s.push(self.bump());
                }
            } else if self.peek() == '(' {
                let mut depth = 0;
                loop {
                    let c = self.peek();
                    if c == '\0' {
                        break;
                    }
                    if c == '(' {
                        depth += 1;
                    } else if c == ')' {
                        depth -= 1;
                    }
                    s.push(self.bump());
                    if depth == 0 {
                        break;
                    }
                }
            } else {
                break;
            }
        }
        s
    }

    // ---- elements & components ----
    fn parse_tag(&mut self) -> Result<Node, String> {
        self.bump(); // <
        let mut tag = String::new();
        while !self.eof() && (self.peek().is_alphanumeric() || self.peek() == '_' || self.peek() == '-') {
            tag.push(self.bump());
        }
        let mut attrs = Vec::new();
        let mut events = Vec::new();
        let mut bind = None;
        let mut secure = false;
        let mut self_closing = false;
        loop {
            self.skip_ws();
            let c = self.peek();
            if c == '\0' {
                break;
            }
            if c == '/' && self.peek_at(1) == '>' {
                self.bump();
                self.bump();
                self_closing = true;
                break;
            }
            if c == '>' {
                self.bump();
                break;
            }
            // read attribute name
            let mut name = String::new();
            while !self.eof()
                && !self.peek().is_whitespace()
                && self.peek() != '='
                && self.peek() != '>'
                && self.peek() != '/'
            {
                name.push(self.bump());
            }
            if name.is_empty() {
                self.bump();
                continue;
            }
            // value?
            let mut raw_val: Option<String> = None;
            self.skip_spaces();
            if self.peek() == '=' {
                self.bump();
                self.skip_spaces();
                raw_val = Some(self.read_attr_value());
            }
            // classify
            if let Some(ev) = name.strip_prefix('@') {
                let v = raw_val.unwrap_or_default();
                let (handler, args) = parse_call(&v);
                let args = if args.is_empty() {
                    Vec::new()
                } else {
                    split_args(&args)
                };
                events.push(EventBind {
                    // Normalize event names to lowercase so the security lint, the
                    // codegen submit/event wiring, and the DOM all agree. Without
                    // this, `@Submit`/`@SUBMIT` would slip past the secure-form
                    // lint (which matches `submit`) while still wiring a live
                    // submit handler — a CSRF bypass.
                    event: ev.to_ascii_lowercase(),
                    handler: handler.to_string(),
                    args,
                });
            } else if name == "bind" {
                let v = raw_val.unwrap_or_default();
                bind = Some(v.trim_start_matches('@').to_string());
            } else if name == "secure" {
                secure = true;
            } else {
                let value = match raw_val {
                    None => AttrValue::Bool,
                    Some(v) => parse_attr_value(&v),
                };
                attrs.push(Attr { name, value });
            }
        }

        let is_component = tag.chars().next().map(|c| c.is_uppercase()).unwrap_or(false);
        // For HTML elements (lowercase-first), normalize the tag name to
        // lowercase so the security lint and codegen match it consistently.
        // Without this, `<fOrm @submit>` parses as an Element with tag "fOrm",
        // dodges the secure-form lint (which matches "form"), yet the browser
        // still treats it as a <form> and submits it — a CSRF bypass. Components
        // (uppercase-first) keep their original casing; `is_component` was
        // computed above on the original tag, so this does not reclassify them.
        if !is_component {
            tag.make_ascii_lowercase();
        }
        let is_void = VOID_TAGS.contains(&tag.as_str());

        if is_component {
            // components keep `bind` as a prop so built-ins/binding can see it
            if let Some(b) = bind.take() {
                attrs.push(Attr {
                    name: "bind".to_string(),
                    value: AttrValue::Expr(b),
                });
            }
            let mut comp = Component {
                name: tag.clone(),
                props: attrs,
                events,
                slots: Vec::new(),
                children: Vec::new(),
                self_closing,
            };
            if !self_closing {
                let (children, slots) = self.parse_children_with_slots(&tag)?;
                comp.children = children;
                comp.slots = slots;
            }
            Ok(Node::Component(comp))
        } else {
            let mut el = Element {
                tag: tag.clone(),
                attrs,
                events,
                bind,
                secure,
                children: Vec::new(),
                self_closing: self_closing || is_void,
            };
            if !self_closing && !is_void {
                // `<style>` and `<script>` are raw-text elements (as in HTML):
                // their content is CSS/JS, not template markup. A bare `@` is a
                // literal (so `@media`, `@keyframes`, `@import`, `@font-face`
                // just work); only `@(expr)` interpolates, and `@@` is a literal @.
                if tag == "style" || tag == "script" {
                    el.children = self.parse_raw_text(&tag);
                } else {
                    el.children = self.parse_nodes(StopAt::EndTag(tag.clone()))?;
                }
                self.consume_end_tag(&tag)?;
            }
            Ok(Node::Element(el))
        }
    }

    /// Read the raw body of a `<style>`/`<script>` element until its end tag.
    /// Only `@(expr)` interpolates; `@@` -> `@`; every other `@` is literal.
    fn parse_raw_text(&mut self, tag: &str) -> Vec<Node> {
        let mut nodes = Vec::new();
        let mut text = String::new();
        let end = format!("</{}", tag);
        loop {
            if self.eof() {
                break;
            }
            if self.peek() == '<' && self.peek_at(1) == '/' && self.starts_with(&end) {
                break;
            }
            let c = self.peek();
            if c == '@' {
                if self.peek_at(1) == '@' {
                    text.push('@');
                    self.bump();
                    self.bump();
                    continue;
                }
                if self.peek_at(1) == '(' {
                    if !text.is_empty() {
                        nodes.push(Node::Text(std::mem::take(&mut text)));
                    }
                    let e = self.read_at_expr();
                    nodes.push(Node::Expr(e));
                    continue;
                }
                // bare '@' (e.g. a CSS at-rule) is literal
                text.push('@');
                self.bump();
                continue;
            }
            text.push(self.bump());
        }
        if !text.is_empty() {
            nodes.push(Node::Text(text));
        }
        nodes
    }

    fn parse_children_with_slots(&mut self, tag: &str) -> Result<(Vec<Node>, Vec<(String, Vec<Node>)>), String> {
        // Children may include `@slot "name" { ... }` definitions.
        let mut children = Vec::new();
        let mut slots = Vec::new();
        loop {
            if self.eof() {
                break;
            }
            if self.peek() == '<' && self.peek_at(1) == '/' {
                break;
            }
            if self.at_keyword().as_deref() == Some("slot") && self.slot_has_body() {
                self.consume_keyword();
                self.skip_spaces();
                let name = self.read_quoted()?;
                self.skip_ws();
                self.expect('{')?;
                let body = self.parse_nodes(StopAt::Brace)?;
                self.expect('}')?;
                slots.push((name, body));
            } else {
                let mut some = self.parse_nodes(StopAt::EndTag(tag.to_string()))?;
                if some.is_empty() {
                    break;
                }
                children.append(&mut some);
            }
        }
        self.consume_end_tag(tag)?;
        Ok((children, slots))
    }

    fn slot_has_body(&self) -> bool {
        let mut j = self.i + "@slot".len();
        while j < self.src.len() && self.src[j].is_whitespace() {
            j += 1;
        }
        if j < self.src.len() && self.src[j] == '"' {
            j += 1;
            while j < self.src.len() && self.src[j] != '"' {
                j += 1;
            }
            j += 1;
        }
        while j < self.src.len() && self.src[j].is_whitespace() {
            j += 1;
        }
        j < self.src.len() && self.src[j] == '{'
    }

    fn read_attr_value(&mut self) -> String {
        let q = self.peek();
        if q == '"' || q == '\'' {
            self.bump();
            let mut s = String::new();
            while !self.eof() && self.peek() != q {
                s.push(self.bump());
            }
            self.bump();
            s
        } else {
            // unquoted
            let mut s = String::new();
            while !self.eof() && !self.peek().is_whitespace() && self.peek() != '>' && self.peek() != '/' {
                s.push(self.bump());
            }
            s
        }
    }

    fn consume_end_tag(&mut self, tag: &str) -> Result<(), String> {
        self.skip_ws();
        if self.peek() == '<' && self.peek_at(1) == '/' {
            self.bump();
            self.bump();
            let mut t = String::new();
            while !self.eof() && self.peek() != '>' {
                t.push(self.bump());
            }
            self.bump();
            let _ = tag;
            Ok(())
        } else {
            Ok(()) // be lenient
        }
    }
}

#[derive(Clone)]
enum StopAt {
    TopLevel,
    Brace,
    EndTag(String),
}

fn is_top_level_kw(kw: &str) -> bool {
    matches!(
        kw,
        "page" | "layout" | "title" | "description" | "canonical" | "authorize" | "meta"
            | "cacheable" | "code" | "style" | "theme" | "section"
    )
}

// ---- small helpers --------------------------------------------------------

fn unquote(s: &str) -> String {
    let s = s.trim();
    s.trim_matches('"').to_string()
}

fn expr_or_literal(s: &str) -> String {
    // `@title "Counter"` keeps the quotes (Motoko string literal);
    // `@title product.name` stays as an expression.
    s.trim().to_string()
}

fn extract_attr(s: &str, key: &str) -> Option<String> {
    // find key="value"
    let needle = format!("{}=", key);
    if let Some(pos) = s.find(&needle) {
        let after = &s[pos + needle.len()..];
        let after = after.trim_start();
        let q = after.chars().next()?;
        if q == '"' || q == '\'' {
            let rest = &after[1..];
            if let Some(end) = rest.find(q) {
                return Some(rest[..end].to_string());
            }
        }
    }
    None
}

/// "increment(5)" -> ("increment", "5"); "save" -> ("save", "")
fn parse_call(s: &str) -> (String, String) {
    let s = s.trim();
    if let Some(open) = s.find('(') {
        if s.ends_with(')') {
            let name = s[..open].trim().to_string();
            let inner = s[open + 1..s.len() - 1].to_string();
            return (name, inner);
        }
    }
    (s.to_string(), String::new())
}

/// split top-level comma-separated args
fn split_args(s: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut depth = 0;
    let mut cur = String::new();
    for c in s.chars() {
        match c {
            '(' | '[' | '{' => {
                depth += 1;
                cur.push(c);
            }
            ')' | ']' | '}' => {
                depth -= 1;
                cur.push(c);
            }
            ',' if depth == 0 => {
                out.push(cur.trim().to_string());
                cur.clear();
            }
            _ => cur.push(c),
        }
    }
    if !cur.trim().is_empty() {
        out.push(cur.trim().to_string());
    }
    out
}

fn parse_attr_value(v: &str) -> AttrValue {
    if !v.contains('@') {
        return AttrValue::Literal(v.to_string());
    }
    let chars: Vec<char> = v.chars().collect();
    let mut parts: Vec<AttrPart> = Vec::new();
    let mut lit = String::new();
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '@' {
            if i + 1 < chars.len() && chars[i + 1] == '@' {
                lit.push('@');
                i += 2;
                continue;
            }
            if !lit.is_empty() {
                parts.push(AttrPart::Lit(std::mem::take(&mut lit)));
            }
            i += 1;
            let mut expr = String::new();
            if i < chars.len() && chars[i] == '(' {
                let mut depth = 0;
                while i < chars.len() {
                    let c = chars[i];
                    if c == '(' {
                        depth += 1;
                        i += 1;
                        if depth == 1 {
                            continue;
                        }
                    } else if c == ')' {
                        depth -= 1;
                        i += 1;
                        if depth == 0 {
                            break;
                        }
                        expr.push(')');
                        continue;
                    } else {
                        i += 1;
                    }
                    expr.push(c);
                }
            } else {
                while i < chars.len() && (chars[i].is_alphanumeric() || chars[i] == '_') {
                    expr.push(chars[i]);
                    i += 1;
                }
                loop {
                    if i < chars.len()
                        && chars[i] == '.'
                        && i + 1 < chars.len()
                        && (chars[i + 1].is_alphabetic() || chars[i + 1] == '_')
                    {
                        expr.push(chars[i]);
                        i += 1;
                        while i < chars.len() && (chars[i].is_alphanumeric() || chars[i] == '_') {
                            expr.push(chars[i]);
                            i += 1;
                        }
                    } else if i < chars.len() && chars[i] == '(' {
                        let mut depth = 0;
                        while i < chars.len() {
                            let c = chars[i];
                            if c == '(' {
                                depth += 1;
                            } else if c == ')' {
                                depth -= 1;
                            }
                            expr.push(c);
                            i += 1;
                            if depth == 0 {
                                break;
                            }
                        }
                    } else {
                        break;
                    }
                }
            }
            parts.push(AttrPart::Expr(expr));
        } else {
            lit.push(chars[i]);
            i += 1;
        }
    }
    if !lit.is_empty() {
        parts.push(AttrPart::Lit(lit));
    }
    if parts.len() == 1 {
        if let AttrPart::Expr(e) = &parts[0] {
            return AttrValue::Expr(e.clone());
        }
    }
    AttrValue::Concat(parts)
}

fn parse_theme(body: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for line in body.split(';') {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        // accept CSS `--mv-x: value` (preferred) or `--mv-x = value`. The first
        // `:`/`=` is the separator (token names never contain either).
        if let Some(sep) = line.find(':').or_else(|| line.find('=')) {
            let k = line[..sep].trim().to_string();
            let v = unquote(line[sep + 1..].trim());
            if !k.is_empty() && !v.is_empty() {
                out.push((k, v));
            }
        }
    }
    out
}

// ---- @code scanner --------------------------------------------------------

/// Scan a `@code` body into declarations. Function bodies are captured raw
/// (with `async`/`await` stripped) for verbatim emission.
pub fn scan_code(body: &str) -> CodeBlock {
    let chars: Vec<char> = body.chars().collect();
    let mut cb = CodeBlock::default();
    let mut i = 0;
    let n = chars.len();
    let skip_ws = |i: &mut usize| {
        while *i < n && chars[*i].is_whitespace() {
            *i += 1;
        }
    };
    while i < n {
        // skip whitespace & comments
        while i < n && chars[i].is_whitespace() {
            i += 1;
        }
        if i >= n {
            break;
        }
        if chars[i] == '/' && i + 1 < n && chars[i + 1] == '/' {
            while i < n && chars[i] != '\n' {
                i += 1;
            }
            continue;
        }
        if chars[i] == '/' && i + 1 < n && chars[i + 1] == '*' {
            i += 2;
            while i + 1 < n && !(chars[i] == '*' && chars[i + 1] == '/') {
                i += 1;
            }
            i += 2;
            continue;
        }
        let word = read_word(&chars, &mut i);
        match word.as_str() {
            "stable" => {
                skip_ws(&mut i);
                let w2 = read_word(&chars, &mut i);
                if w2 == "var" {
                    let (decl, raw) = read_statement(&chars, &mut i);
                    let mut vd = parse_var_decl(&decl);
                    vd.stable = true;
                    vd.raw = format!("stable var {}", raw);
                    cb.vars.push(vd);
                } else {
                    let (_d, raw) = read_statement(&chars, &mut i);
                    cb.extra.push(format!("stable {} {}", w2, raw));
                }
            }
            "var" => {
                let (decl, _raw) = read_statement(&chars, &mut i);
                let mut vd = parse_var_decl(&decl);
                vd.raw = format!("var {};", decl.trim());
                cb.vars.push(vd);
            }
            "param" => {
                let (decl, _raw) = read_statement(&chars, &mut i);
                if let Some(pd) = parse_param_decl(&decl) {
                    cb.params.push(pd);
                }
            }
            "func" => {
                let fd = read_func(&chars, &mut i);
                cb.funcs.push(fd);
            }
            "let" => {
                let (decl, _raw) = read_statement(&chars, &mut i);
                cb.extra.push(format!("let {};", decl.trim()));
            }
            "" => {
                i += 1;
            }
            other => {
                // unknown top-level construct (e.g. `type`, `class`): capture the
                // statement verbatim, restoring the terminating `;` that
                // read_statement strips (matches the `let` branch above).
                let (decl, _raw) = read_statement(&chars, &mut i);
                let combined = format!("{} {};", other, decl.trim());
                if combined.trim() != ";" {
                    cb.extra.push(combined);
                }
            }
        }
    }
    cb
}

fn read_word(chars: &[char], i: &mut usize) -> String {
    while *i < chars.len() && chars[*i].is_whitespace() {
        *i += 1;
    }
    let mut s = String::new();
    while *i < chars.len() && (chars[*i].is_alphanumeric() || chars[*i] == '_') {
        s.push(chars[*i]);
        *i += 1;
    }
    s
}

/// read until ';' at depth 0 (respecting (){}[] and strings). Returns the text
/// (without the trailing ';') and a copy.
fn read_statement(chars: &[char], i: &mut usize) -> (String, String) {
    let mut s = String::new();
    let mut depth = 0i32;
    while *i < chars.len() {
        let c = chars[*i];
        match c {
            // Skip comments verbatim (see read_func): an apostrophe inside a
            // comment must not be read as a char-literal delimiter.
            '/' if *i + 1 < chars.len() && chars[*i + 1] == '/' => {
                while *i < chars.len() && chars[*i] != '\n' {
                    s.push(chars[*i]);
                    *i += 1;
                }
            }
            '/' if *i + 1 < chars.len() && chars[*i + 1] == '*' => {
                s.push(chars[*i]);
                *i += 1;
                s.push(chars[*i]);
                *i += 1;
                while *i + 1 < chars.len() && !(chars[*i] == '*' && chars[*i + 1] == '/') {
                    s.push(chars[*i]);
                    *i += 1;
                }
                if *i + 1 < chars.len() {
                    s.push(chars[*i]);
                    *i += 1;
                    s.push(chars[*i]);
                    *i += 1;
                }
            }
            '"' | '\'' => {
                let q = c;
                s.push(c);
                *i += 1;
                while *i < chars.len() {
                    let d = chars[*i];
                    s.push(d);
                    *i += 1;
                    if d == '\\' {
                        if *i < chars.len() {
                            s.push(chars[*i]);
                            *i += 1;
                        }
                    } else if d == q {
                        break;
                    }
                }
            }
            '(' | '[' | '{' => {
                depth += 1;
                s.push(c);
                *i += 1;
            }
            ')' | ']' | '}' => {
                depth -= 1;
                s.push(c);
                *i += 1;
            }
            ';' if depth == 0 => {
                *i += 1;
                break;
            }
            _ => {
                s.push(c);
                *i += 1;
            }
        }
    }
    let raw = format!("{};", s.trim());
    (s.trim().to_string(), raw)
}

fn parse_var_decl(decl: &str) -> VarDecl {
    // "count : Nat = 0"  /  "count : Nat"
    let (lhs, init) = match decl.find('=') {
        Some(eq) => (decl[..eq].trim(), Some(decl[eq + 1..].trim().to_string())),
        None => (decl.trim(), None),
    };
    let (name, ty) = match lhs.find(':') {
        Some(co) => (
            lhs[..co].trim().to_string(),
            Some(lhs[co + 1..].trim().to_string()),
        ),
        None => (lhs.trim().to_string(), None),
    };
    VarDecl {
        stable: false,
        name,
        ty,
        init,
        raw: String::new(),
    }
}

fn parse_param_decl(decl: &str) -> Option<ParamDecl> {
    let (lhs, default) = match decl.find('=') {
        Some(eq) => (decl[..eq].trim(), Some(decl[eq + 1..].trim().to_string())),
        None => (decl.trim(), None),
    };
    let co = lhs.find(':')?;
    Some(ParamDecl {
        name: lhs[..co].trim().to_string(),
        ty: lhs[co + 1..].trim().to_string(),
        default,
    })
}

/// read a `func name(params) : ret { body }` starting just after the `func` word.
fn read_func(chars: &[char], i: &mut usize) -> FuncDecl {
    let name = read_word(chars, i);
    // params
    while *i < chars.len() && chars[*i] != '(' {
        *i += 1;
    }
    let mut params_raw = String::new();
    if *i < chars.len() && chars[*i] == '(' {
        let mut depth = 0;
        loop {
            if *i >= chars.len() {
                break;
            }
            let c = chars[*i];
            if c == '(' {
                depth += 1;
            } else if c == ')' {
                depth -= 1;
            }
            if depth > 0 || c == ')' {
                if !(depth == 1 && c == '(') {
                    params_raw.push(c);
                }
            }
            *i += 1;
            if depth == 0 {
                break;
            }
        }
    }
    // strip leading '(' if captured
    let params_raw = params_raw.trim().trim_start_matches('(').trim_end_matches(')').to_string();

    // return type: between ')' and '{'
    let mut ret_raw = String::new();
    while *i < chars.len() && chars[*i] != '{' {
        ret_raw.push(chars[*i]);
        *i += 1;
    }
    // body
    let mut body = String::new();
    if *i < chars.len() && chars[*i] == '{' {
        *i += 1; // {
        let mut depth = 1;
        while *i < chars.len() {
            let c = chars[*i];
            match c {
                // Skip comments verbatim so an apostrophe inside one (e.g. a
                // word like `peer's`) is NOT mistaken for a char-literal/string
                // delimiter — which would swallow code and corrupt brace counting.
                '/' if *i + 1 < chars.len() && chars[*i + 1] == '/' => {
                    while *i < chars.len() && chars[*i] != '\n' {
                        body.push(chars[*i]);
                        *i += 1;
                    }
                }
                '/' if *i + 1 < chars.len() && chars[*i + 1] == '*' => {
                    body.push(chars[*i]);
                    *i += 1; // /
                    body.push(chars[*i]);
                    *i += 1; // *
                    while *i + 1 < chars.len() && !(chars[*i] == '*' && chars[*i + 1] == '/') {
                        body.push(chars[*i]);
                        *i += 1;
                    }
                    if *i + 1 < chars.len() {
                        body.push(chars[*i]);
                        *i += 1; // *
                        body.push(chars[*i]);
                        *i += 1; // /
                    }
                }
                '"' | '\'' => {
                    let q = c;
                    body.push(c);
                    *i += 1;
                    while *i < chars.len() {
                        let d = chars[*i];
                        body.push(d);
                        *i += 1;
                        if d == '\\' {
                            if *i < chars.len() {
                                body.push(chars[*i]);
                                *i += 1;
                            }
                        } else if d == q {
                            break;
                        }
                    }
                }
                '{' => {
                    depth += 1;
                    body.push(c);
                    *i += 1;
                }
                '}' => {
                    depth -= 1;
                    if depth == 0 {
                        *i += 1;
                        break;
                    }
                    body.push(c);
                    *i += 1;
                }
                _ => {
                    body.push(c);
                    *i += 1;
                }
            }
        }
    }
    // optional trailing ';'
    let save = *i;
    while *i < chars.len() && chars[*i].is_whitespace() {
        *i += 1;
    }
    if *i < chars.len() && chars[*i] == ';' {
        *i += 1;
    } else {
        *i = save;
    }

    let ret_clean = ret_raw.replace(':', " ");
    let is_async = ret_clean.contains("async");
    let ret = {
        let r = ret_clean.replace("async", " ");
        let r = r.trim().to_string();
        if r.is_empty() {
            None
        } else {
            Some(r)
        }
    };
    let params = parse_params(&params_raw);
    let body = strip_await(&body);
    FuncDecl {
        name,
        params,
        ret,
        body,
        is_async,
    }
}

fn parse_params(s: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for part in split_args(s) {
        if let Some(co) = part.find(':') {
            out.push((part[..co].trim().to_string(), part[co + 1..].trim().to_string()));
        } else if !part.trim().is_empty() {
            out.push((part.trim().to_string(), "Any".to_string()));
        }
    }
    out
}

/// Strip `await` keywords (MVP handlers run synchronously in-canister).
fn strip_await(body: &str) -> String {
    // replace word-boundary "await " with ""
    let mut out = String::new();
    let chars: Vec<char> = body.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        if chars[i..].iter().collect::<String>().starts_with("await")
            && (i == 0 || !chars[i - 1].is_alphanumeric())
            && (i + 5 >= chars.len() || !chars[i + 5].is_alphanumeric())
        {
            i += 5;
            // skip following whitespace
            while i < chars.len() && chars[i].is_whitespace() {
                i += 1;
            }
        } else {
            out.push(chars[i]);
            i += 1;
        }
    }
    out
}
