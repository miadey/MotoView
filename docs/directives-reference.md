---
title: Directive Reference
section: Reference
slug: directives-reference
---

# Directive Reference

Every MotoView feature is reachable through a small set of `@` directives and
component attributes. This page is the quick lookup: each row gives the syntax
and a tiny, copy-paste example. For deeper walkthroughs see
[Events](events.md), [Forms & Validation](forms.md), and
[Layouts & Components](components.md).

## Page & metadata directives

These declare what a `.mview` file *is* and what the browser sees in `<head>`.

| Directive | Syntax | Example |
|---|---|---|
| `@page` | `@page "/path"` | `@page "/about"` |
| `@page` (param) | `@page "/products/{id}"` | `@page "/products/{id}"` |
| `@page` (typed) | `@page "/orders/{id:Nat}"` | `@page "/orders/{id:Nat}"` |
| `@layout` | `@layout NAME` | `@layout Main` |
| `@cacheable` | `@cacheable` | serve a public page as a fast certified query (parameterized routes via a wildcard) |
| `@title` | `@title EXPR` | `@title "Product " # product.name` |
| `@description` | `@description EXPR` | `@description product.summary` |
| `@canonical` | `@canonical EXPR` | `@canonical "https://shop.ic0.app/p/" # id` |
| `@meta` | `@meta name=... content=...` | `@meta name="robots" content="index"` |
| `@head` | `@head { ... }` | extra tags injected into `<head>` |

```razor
@page "/orders/{id:Nat}"
@layout Main
@title "Order #" # Nat.toText(id)
@description "Details for order " # Nat.toText(id)
```

## Code, style & theming

| Directive | Syntax | Example |
|---|---|---|
| `@code` | `@code { ...Motoko... }` | handlers, state, route params |
| `@style` | `@style { ...css... }` | page-scoped CSS |
| `@theme` | `@theme "name"` / `@theme { tokens }` | apply a theme package and/or set design tokens |

```razor
@code {
  var count : Nat = 0;
  func increment() : async () { count += 1 };
}

@style { .total { font-weight: 600 } }
@theme "ocean" { --mv-primary: #0d9488 }
```

Theme packages: `midnight` (dark), `ocean`, `forest`, `sunset`, `slate`. Tokens
include `--mv-primary`, `--mv-bg`, `--mv-surface`, `--mv-text`, `--mv-border`,
`--mv-radius`, `--mv-font`, … (see [Styling & Themes](styling-and-themes.md)).

## Layout composition

Layouts define shared chrome; pages fill named regions.

| Directive | Syntax | Example |
|---|---|---|
| `@yield` | `@yield` | renders the page body (in a layout) |
| `@section` | `@section "name" { ... }` | page-side content for a named region |
| `@slot` | `@slot "name"` | layout-side placeholder for a section |

```razor
@section "sidebar" {
  <nav>Account menu</nav>
}
```

```razor
@slot "sidebar"
@yield
```

## Control flow

Standard branching and looping, evaluated server-side at render time.

| Directive | Syntax | Example |
|---|---|---|
| `@if` / `else` | `@if EXPR { } else { }` | conditional markup |
| `@for` | `@for X in EXPR { }` | iterate a collection |
| `@switch` | `@switch EXPR { case #Variant { } }` | match a Motoko variant |

```razor
@if cart.isEmpty() {
  <Alert type="info">Your cart is empty.</Alert>
} else {
  @for item in cart.items {
    <Card title=item.name>@item.price</Card>
  }
}

@switch order.status {
  case #Pending { <Badge type="warning">Pending</Badge> }
  case #Shipped { <Badge type="success">Shipped</Badge> }
}
```

## Output expressions

Print a value into the rendered HTML.

| Form | Syntax | Example |
|---|---|---|
| Field | `@user.name` | `Hello, @user.name` |
| Count | `@count` | `Total: @count` |
| Expression | `@(expr)` | `@(price * qty)` |
| Raw HTML | `@raw(expr)` | `@raw(doc.html)` |

`@user.name`, `@count` and `@(expr)` are **HTML-escaped** — safe by default.
`@raw(expr)` emits the expression **without escaping**, for trusted server-side
HTML (e.g. markdown you rendered yourself). The expression must already be
`Text`. Never pass user input to `@raw` — it bypasses escaping. To print a
literal `@`, write `@@`.

## Keyed regions

Give the elements of a list a `key` so the client can patch only what changed
instead of replacing the whole region. The key should be a stable identifier —
usually the item's id:

```mview
<ul>
@for item in items {
    <li key="@item.id">@item.name <input></li>
}
</ul>
```

`key="@item.id"` compiles to `data-mv-key`. On a re-render, the brain (the
Rust→WASM client) diffs the keyed regions and touches only what changed:
content-only changes **replace** just those items; added, removed and reordered
items are **inserted, removed and moved** (a reorder moves the minimum number of
nodes). Every node that doesn't change keeps its identity — so its focus,
selection, scroll and media state survive an update to its siblings, and even a
reorder *moves* a node rather than recreating it. A change to the surrounding
markup safely falls back to a full re-render. All of this logic runs in WASM;
there is no application JavaScript.

## Events

Bind DOM events to typed Motoko handlers. Handler arguments are evaluated
**server-side** at render time and baked into `data-mv-arg*` attributes; the
WASM client forwards them on dispatch. See [Events](events.md).

| Attribute | Syntax | Example |
|---|---|---|
| `@click` | `@click="handler"` | `<Button @click="save">Save</Button>` |
| `@click` (arg) | `@click="handler(arg)"` | `@click="remove(item.id)"` |
| `@submit` | `@submit="handler"` | `<form @submit="send">` |
| `@input` | `@input="handler"` | live field handling |
| `@change` | `@change="handler"` | select / checkbox changes |

```razor
<Button @click="increment">+1</Button>
<Button @click="remove(item.id)" kind="danger">Delete</Button>
```

## Forms, validation & security

The `secure` attribute mints a signed HMAC-SHA256 token binding path, handler,
caller principal, nonce, expiry, and field-schema hash; the server re-derives
the MAC and rejects mismatches, expired tokens, and replays. See
[Forms & Validation](forms.md).

| Feature | Syntax | Example |
|---|---|---|
| Secure form | `<form @submit="h" secure>` | hardened submit |
| Two-way bind | `bind="@model.field"` | `<InputText bind="@model.name" />` |
| Validation | `validate model { rules }` | in the handler |

```razor
<form @submit="send" secure>
  <InputText label="Name" bind="@model.name" required />
  <InputEmail label="Email" bind="@model.email" />
  <ValidationSummary />
  <Button kind="primary">Send</Button>
</form>
```

```motoko
func send() {
  validate model {
    name required "Name is required";
    email required email;
    price min 1 "Price must be at least 1";
  };
}
```

## Effects & animation

Imperative client effects returned in a batch and applied by the WASM client.

| Directive | Syntax | Example |
|---|---|---|
| `@effect Focus` | `@effect Focus("#x")` | move focus after render |
| `@effect ScrollTo` | `@effect ScrollTo("#id")` | scroll to an element |
| `@effect Toast` | `@effect Toast("msg")` | show a transient toast |
| `@animate` | `@animate` | mark a region for transition |

```razor
@effect Focus("#name")
@effect Toast("Saved!")
```

## Authorization

| Directive | Syntax | Example |
|---|---|---|
| `@authorize` | `@authorize` | require an authenticated caller |
| `@authorize` (role) | `@authorize role="Admin"` | require a role *(role stores: Roadmap)* |

```razor
@authorize role="Admin"
@page "/admin/orders"
```

Built-in semantic components (`Button`, `Card`, `Alert`, `Badge`, `InputText`,
`Table`, `Grid`, and more) are documented in
[Layouts & Components](components.md).
