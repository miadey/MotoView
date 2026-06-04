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
| `@theme` | `@theme { tokens }` | design tokens for the page/layout |

```razor
@code {
  stable var count : Nat = 0;
  func increment() { count += 1 };
}

@style { .total { font-weight: 600 } }
@theme { --mv-accent: #2563eb }
```

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
