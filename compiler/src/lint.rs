//! AST lint pass — security gates that run BEFORE codegen.
//!
//! These are *structural* checks on the parsed `.mview` tree (no Motoko type
//! info needed). They catch security footguns the type-checker can't see:
//!
//!   * `secure-form` (Error): a state-mutating `<form @submit=...>` that is NOT
//!     marked `secure`. Secure forms mint an HMAC token binding the request, so
//!     an unsecured mutating form is a CSRF + over-posting hole. The build is
//!     aborted (see `project::build`).
//!   * `raw-html` (Warning): every `@raw(...)` is an unescaped-HTML / XSS sink;
//!     advisory only — it never blocks the build.
//!
//! The walker mirrors the node/element shape `codegen.rs` uses, so forms/raw
//! nested anywhere in the tree (inside `@if`/`@for`/`@switch`/components/slots)
//! are still seen.

use crate::ast::{MviewFile, Node};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Severity {
    Error,
    Warning,
}

#[derive(Debug, Clone)]
pub struct Diagnostic {
    pub severity: Severity,
    pub message: String,
    /// Source location: file path plus the element/handler that triggered it.
    pub location: String,
    /// Stable rule id (e.g. "secure-form", "raw-html").
    pub rule: String,
}

/// Run all lint rules over a parsed file, returning every diagnostic found.
/// `path` is the `.mview` source path, used to build the `location` string.
pub fn lint_file(file: &MviewFile, path: &str) -> Vec<Diagnostic> {
    let mut diags = Vec::new();
    walk_nodes(&file.template, path, &mut diags);
    for (_, body) in &file.sections {
        walk_nodes(body, path, &mut diags);
    }
    diags
}

fn walk_nodes(nodes: &[Node], path: &str, diags: &mut Vec<Diagnostic>) {
    for node in nodes {
        walk_node(node, path, diags);
    }
}

fn walk_node(node: &Node, path: &str, diags: &mut Vec<Diagnostic>) {
    match node {
        Node::Raw(_) => {
            // RULE raw-html (advisory): @raw emits unescaped HTML.
            diags.push(Diagnostic {
                severity: Severity::Warning,
                message: "@raw emits unescaped HTML — ensure the expression is \
                          trusted server-generated HTML, never user input (XSS sink)."
                    .to_string(),
                location: format!("{} (@raw)", path),
                rule: "raw-html".to_string(),
            });
        }
        Node::Element(el) => {
            // RULE secure-form: a <form> that MUTATES (has an @submit event) must
            // be `secure`. A <form> WITHOUT @submit (e.g. a GET search form) is
            // not mutating and is never flagged.
            // Case-insensitive matching mirrors codegen (codegen.rs) exactly:
            // the parser already lowercases element tags and event names, but we
            // match case-insensitively here too so the lint can never diverge
            // from what codegen wires as a live submit form (CSRF-relevant).
            let submit = el.events.iter().find(|e| e.event.eq_ignore_ascii_case("submit"));
            if el.tag.eq_ignore_ascii_case("form") {
                if let Some(ev) = submit {
                    if !el.secure {
                        diags.push(Diagnostic {
                            severity: Severity::Error,
                            message:
                                "state-mutating <form @submit=...> must be marked \
                                 `secure` (or remove the submit handler). Secure forms \
                                 mint an HMAC token binding the request; an unsecured \
                                 mutating form is a CSRF + over-posting hole."
                                    .to_string(),
                            location: format!("{} (<form @submit=\"{}\">)", path, ev.handler),
                            rule: "secure-form".to_string(),
                        });
                    }
                }
            }
            walk_nodes(&el.children, path, diags);
        }
        Node::Component(c) => {
            // @raw / forms can live inside a component's default slot or named slots.
            walk_nodes(&c.children, path, diags);
            for (_, body) in &c.slots {
                walk_nodes(body, path, diags);
            }
        }
        Node::If(branches) => {
            for br in branches {
                walk_nodes(&br.body, path, diags);
            }
        }
        Node::For { body, .. } => walk_nodes(body, path, diags),
        Node::Switch { cases, .. } => {
            for c in cases {
                walk_nodes(&c.body, path, diags);
            }
        }
        // Leaf / non-container nodes carry no nested template to walk.
        Node::Text(_)
        | Node::Expr(_)
        | Node::Yield
        | Node::Head
        | Node::SectionRef(_)
        | Node::Slot(_)
        | Node::Effect { .. } => {}
    }
}
