---
name: motoview
description: >-
  Build server-driven UI apps for Internet Computer (ICP) canisters with MotoView. Use this skill
  whenever the user works with .mview files, mentions MotoView, the `motoview` CLI/compiler, the
  `motoview` mops runtime package, or asks to build Motoko-native / server-driven UI on ICP (pages,
  layouts, components, secure forms, validation, adaptive polling, the "motoview/1" batch protocol).
  Triggers on @page/@layout/@code/@click/@submit directives, bind="@model.field", <form ... secure>,
  and questions about deploying interactive ICP frontends without React/agent-js/JavaScript.
---

# MotoView

MotoView is a **Motoko-native, server-driven UI framework for Internet Computer (ICP) canisters**.
You write `.mview` files — template markup + Motoko code in one file (a Blazor/Blade/LiveView feel) —
and they compile to Motoko that you deploy with `dfx`.

**There is no application JavaScript, no React, no agent-js glue, and no duplicated client/server validation.**

> Write Motoko. Ship interactive, SEO-friendly ICP apps. No frontend JavaScript.
>
> Technically: rendering is a query, events are updates, and the browser synchronizes through versioned UI batches.

This skill teaches you to build MotoView apps correctly. Stick to the facts here. Do **not** invent
directives, attributes, CLI flags, file names, or numbers. If a capability is on the roadmap, say so.

---

## The mental model

Hold these four ideas and most of MotoView follows:

1. **Rendering is a query.** A page is pure server-side Motoko that produces HTML from canister state.
   The first request returns fully server-rendered HTML (good for SEO) with the page body inside
   `<div id="mv-root">`.
2. **Events are updates.** A user interaction (`@click`, `@submit`, …) POSTs to the canister, runs a
   typed Motoko handler that mutates state, and immediately returns a fresh batch.
3. **The browser synchronizes through versioned UI batches.** The client re-renders by swapping in new
   HTML when the `batchId` changes, and skips work when it hasn't.
4. **The only browser code is a Rust→WebAssembly client** plus a tiny hand-written JS glue. You never
   write either. WASM is the "brain" (adaptive polling state machine, protocol, batch interpretation,
   event sequencing); the JS glue is the "hands" (DOM/fetch/timer primitives, focus/scroll/input
   preservation). Served at `/motoview.wasm`, `/motoview.js`, `/motoview.css`.

You write `.mview`. The `motoview` compiler (a Rust binary) turns it into a Motoko actor. The `motoview`
mops runtime package serves HTTP from inside the canister. `dfx` deploys it.

**No Node, no npm, no JS bundler.** That absence is a core selling point — never introduce them.

---

## Project layout

```
src/
  Pages/*.mview        # routed pages (@page "/...")
  Layouts/*.mview      # shared shells (@yield, @section)
  Components/*.mview    # reusable components (Capitalized tags)
  Services/*.mo        # plain Motoko services/state
  Models/*.mo          # plain Motoko types/models
  main.mo              # compiler output (generated actor); also src/Generated/
motoview.json          # MotoView config
dfx.json               # DFINITY SDK config
mops.toml              # Motoko package manifest (depends on the "motoview" runtime)
```

The compiler reads your `.mview` sources and emits a Motoko actor (e.g. `src/main.mo` / `src/Generated/`).

---

## The CLI

```bash
motoview new <app>        # scaffold a new project
motoview build            # compile .mview -> Motoko
motoview dev              # build + dfx deploy + watch (local)
motoview compile <file>   # compile a single file (for debugging)
```

These four commands are the whole CLI. Do not invent others or add flags that aren't documented here.

### Install / prerequisites

MotoView builds on standard ICP + Rust tooling — there is no JS toolchain:

```bash
# 1. dfx (DFINITY SDK)  -- deploys the canister
# 2. rustup + the wasm32 target  -- builds the compiler and browser client
rustup target add wasm32-unknown-unknown
# 3. wasm-opt (binaryen)  -- optimizes the WASM browser client
# 4. the motoview compiler, installed via cargo
cargo install ...   # the motoview binary

# Add the runtime to your canister, either:
#  - as the mops package "motoview", or
#  - as a local path dependency in dfx.json args, e.g.:
#      "--package motoview ../../runtime/src"
```

---

## .mview directives

A `.mview` file mixes template markup with Motoko. Directives start with `@`.

### Routing & metadata
- `@page "/path"` — routes the page. Supports route params: `@page "/products/{id}"` and typed params
  `@page "/orders/{id:Nat}"`.
- `@layout NAME` — render this page inside a named layout.
- `@title EXPR` — page `<title>`.
- `@description EXPR` — meta description.
- `@canonical EXPR` — canonical URL.
- `@meta` — emit additional meta tags.
- `@head` — inject content into `<head>`.

### Layout composition
- `@section "name" { ... }` — define a named section (in a page).
- `@yield` — render the page body inside a layout.
- `@slot "name"` — a named slot a layout exposes / a page fills.

### Code & styling
- `@code { ...Motoko... }` — Motoko code for the page/component (state, handlers, helpers).
- `@style { ...css... }` — scoped CSS.
- `@theme { tokens }` — design tokens.

### Control flow (in markup)
- `@if EXPR { } else { }`
- `@for X in EXPR { }`
- `@switch EXPR { case #Variant { } }`

### Output
- `@count`, `@user.name` — inline output of a value/field.
- `@(expr)` — inline output of an arbitrary Motoko expression.

### Auth
- `@authorize` — require an authenticated caller.
- `@authorize role="Admin"` — require a role. (Full Internet Identity login and role stores are Roadmap;
  see "Honest status".)

### Effects & animation
- `@effect Focus("#x")` / `@effect ScrollTo(...)` / `@effect Toast(...)` — declarative client effects.
- `@animate` — animation hook.

---

## Events

```razor
@click="handler"
@click="handler(arg)"
@submit="send"
@input="onInput"
@change="onChange"
```

How event arguments work (important and easy to get wrong):

- **Handler arguments are evaluated SERVER-SIDE at render time** and baked into `data-mv-arg*`
  attributes on the element.
- The WASM client forwards `handlerId` + the baked args to the server.
- The server dispatches to typed Motoko functions you wrote in `@code { }`.

So you write `@click="save"` or `@click="remove(item.id)"`; the framework handles serialization,
transport, and typed dispatch. You never write fetch calls, JSON, or event wiring by hand.

---

## Forms, validation & security

### Secure forms

```razor
<form @submit="send" secure>
  <InputText name="name" label="Name" bind="@model.name" required />
  <InputEmail name="email" label="Email" bind="@model.email" required />
  <ValidationSummary />
  <Button kind="primary">Send</Button>
</form>
```

- `bind="@model.field"` two-way binds an input to a field on your model.
- The `secure` attribute **mints a signed token** (HMAC-SHA256) binding:
  **path + handler + caller principal + nonce + expiry + field-schema hash.**
- On submit the server **re-derives the MAC** and rejects mismatches, expired tokens, and replays.
- SHA-256 and HMAC are implemented in Motoko and verified against standard test vectors.

Use `secure` on any form that mutates state or is sensitive. The developer writes one word; the
framework does CSRF/replay/tamper protection.

### Validation (in handlers)

Validation lives **server-side, in your Motoko handler** — there is no separate client validation to
duplicate or drift:

```motoko
validate model {
  name required "Name is required";
  email required email;
  price min 1 "Price must be at least 1";
};
```

On failure, the errors are returned in the batch (status `validation-error`) and the page re-renders
with messages. Surface them with `<ValidationSummary />` and per-field errors.

---

## Components

- **Capitalized tags are components.** App components live in `src/Components/*.mview` and declare
  params: `param name : T [= default]`.
- Prefer the built-in **semantic components** over long utility-class strings:

| Component | Notable params |
|---|---|
| `Button` | `kind`, `size` |
| `Card` | `title` |
| `Alert` | `type` |
| `Badge` | `type` |
| `InputText` / `InputEmail` / `InputNumber` / `TextArea` | `name`, `label`, `bind`, `required`, `minLength` |
| `ValidationSummary` | — |
| `Table` | — |
| `PageHeader` | — |
| `Grid` | `columns` |

Write `<Button kind="primary">Save</Button>` — not a soup of CSS classes.

A simple component:

```razor
@* src/Components/StatCard.mview *@
param label : Text
param value : Nat

<Card title=@label>
  <p class="stat">@value</p>
</Card>
```

---

## The protocol ("motoview/1")

You normally don't touch the wire protocol — but understanding it explains the behavior.

- **First load:** `GET /page` → server-rendered HTML; page content is inside `<div id="mv-root">`.
- **Sync (poll):** `GET /_motoview/render?path=...&lastBatchId=...` → a batch JSON with `status` of
  `"changed"` | `"unchanged"` | `"redirect"` | `"validation-error"`. An `"unchanged"` batch omits the
  HTML (the `batchId` matched, so the client skips re-rendering).
- **Events:** `POST` (form-encoded) `/_motoview/event` → served by `http_request_update` → returns a
  **new batch immediately** (no waiting for the next poll).
- **`batchId`** is a hash of the rendered state; matching ids let the client skip DOM work.

### Adaptive polling cadence

| State | Cadence |
|---|---|
| hot (≈3s after an interaction) | ~350ms |
| warm (visible) | ~2.5s |
| cold (idle) | ~15s |
| hidden (tab backgrounded) | ~45s |
| offline | exponential backoff |

The event response returns the new batch immediately; polling exists to pick up **external** state
changes (e.g. another caller mutating the canister).

### IC transport note (MVP)

`http_request` returns `upgrade = true`, so every request is served by `http_request_update`. This
avoids query response-certification for now. **Certified query rendering** for cacheable public pages
is Roadmap.

---

## DO / DON'T rules

**DO**
- Write all logic in `@code { }` as typed Motoko; let handlers mutate state and return.
- Put route params in `@page` and read them as typed values (`{id:Nat}` when you need a `Nat`).
- Use `secure` on every state-mutating/sensitive form.
- Validate in the handler with `validate model { ... }`; render errors with `<ValidationSummary />`.
- Use built-in semantic components (`Button`, `Card`, `InputText`, …).
- Trust the protocol: write `@click="save"` and let event→update→batch→DOM swap happen.
- Use `dfx` to deploy and `motoview dev` for the local loop.
- Be honest about Roadmap items.

**DON'T**
- **Don't add Node, npm, a JS bundler, React, or agent-js.** Their absence is the point.
- **Don't write client-side JavaScript** or hand-roll fetch/event wiring — the WASM client does it.
- **Don't duplicate validation** on the client; validation is server-side only.
- **Don't invent directives, attributes, CLI flags, or files** beyond those documented here.
- **Don't claim Roadmap features work today** (see "Honest status").
- **Don't hand-write HMAC/CSRF tokens** — `secure` mints and verifies them.
- **Don't reach for utility-class soup** when a semantic component exists.

---

## Copy-paste-ready snippets

### 1. Counter (verified end-to-end)

The counter was deployed to a local replica and exercised in a real browser: clicking updates state via
event → update → batch → DOM swap, and adaptive polling picks up external changes.

```razor
@page "/"
@title "Counter"

@code {
  var count : Nat = 0;

  func increment() { count += 1 };
  func decrement() { if (count > 0) { count -= 1 } };
  func reset() { count := 0 };
}

<PageHeader title="Counter" />

<Card title="Count">
  <p class="count">@count</p>

  <Button kind="primary" @click="increment">+</Button>
  <Button @click="decrement">-</Button>
  <Button kind="secondary" @click="reset">Reset</Button>
</Card>
```

### 2. Contact form (secure + validate)

```razor
@page "/contact"
@title "Contact us"
@description "Get in touch"

@code {
  type Message = { name : Text; email : Text; body : Text };

  var model : Message = { name = ""; email = ""; body = "" };
  var sent : Bool = false;

  func send() {
    validate model {
      name required "Name is required";
      email required email;
      body required "Please write a message";
    };
    // persist the message in canister state here...
    sent := true;
  };
}

<PageHeader title="Contact us" />

@if sent {
  <Alert type="success">Thanks — we got your message.</Alert>
} else {
  <form @submit="send" secure>
    <ValidationSummary />
    <InputText  name="name"  label="Name"  bind="@model.name"  required />
    <InputEmail name="email" label="Email" bind="@model.email" required />
    <TextArea   name="body"  label="Message" bind="@model.body" required minLength="10" />
    <Button kind="primary">Send</Button>
  </form>
}
```

### 3. CRUD page (list + create + delete + detail route)

```razor
@page "/products"
@title "Products"

@code {
  type Product = { id : Nat; name : Text; price : Nat };

  var products : [Product] = [];
  var nextId : Nat = 1;
  var draft : { name : Text; price : Nat } = { name = ""; price = 0 };

  func create() {
    validate draft {
      name required "Name is required";
      price min 1 "Price must be at least 1";
    };
    let p : Product = { id = nextId; name = draft.name; price = draft.price };
    products := Array.append(products, [p]);
    nextId += 1;
    draft := { name = ""; price = 0 };
  };

  func remove(id : Nat) {
    products := Array.filter<Product>(products, func (p) { p.id != id });
  };
}

<PageHeader title="Products" />

<Card title="New product">
  <form @submit="create" secure>
    <ValidationSummary />
    <InputText   name="name"  label="Name"  bind="@draft.name"  required />
    <InputNumber name="price" label="Price" bind="@draft.price" required />
    <Button kind="primary">Add</Button>
  </form>
</Card>

@if products.size() == 0 {
  <Alert type="info">No products yet.</Alert>
} else {
  <Table>
    <thead><tr><th>Name</th><th>Price</th><th></th></tr></thead>
    <tbody>
      @for p in products {
        <tr>
          <td><a href=@("/products/" # Nat.toText(p.id))>@p.name</a></td>
          <td>@p.price</td>
          <td><Button kind="danger" size="sm" @click="remove(p.id)">Delete</Button></td>
        </tr>
      }
    </tbody>
  </Table>
}
```

Matching typed detail route:

```razor
@page "/products/{id:Nat}"
@title "Product"

@code {
  // `id` is available as a typed Nat route parameter
  func product() : ?Product { /* look up by id in canister state */ };
}

@switch product() {
  case (?p) {
    <PageHeader title=@p.name />
    <Card title="Price"><p>@p.price</p></Card>
  }
  case null {
    <Alert type="error">Product not found.</Alert>
  }
}
```

---

## Honest status (verified vs. roadmap)

**Verified end to end:**
- The runtime + WASM client + dfx pipeline. The counter example was deployed to a local replica and
  exercised in a real browser: clicking updates state via event → update → batch → DOM swap, adaptive
  polling picks up external changes, and state persists across calls.
- SHA-256 / HMAC pass standard test vectors.
- Additional examples are provided: todo, contact form, products CRUD.

**Roadmap (do NOT present as working today):**
- Keyed-region / granular DOM patches (currently full `#mv-root` swap on change).
- Full Internet Identity login over HTTP; role stores.
- vetKeys encrypted state.
- Certified query rendering for cacheable public pages.
- Desktop / mobile / tablet shells.
- Visual designer.
- Push adapter.

See `reference.md` (next to this file) for a dense cheat-sheet of every directive, attribute, and the
batch protocol.
