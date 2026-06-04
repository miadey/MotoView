# MotoView Reference

Dense cheat-sheet for `.mview` directives, attributes, components, the CLI, and the `motoview/1` batch
protocol. Authoritative for syntax. Do not invent anything not listed here; Roadmap items are marked.

---

## Directives

| Directive | Form(s) | Purpose |
|---|---|---|
| `@page` | `@page "/path"` · `@page "/products/{id}"` · `@page "/orders/{id:Nat}"` | Route the page; supports plain and typed route params |
| `@layout` | `@layout NAME` | Render page inside a named layout |
| `@title` | `@title EXPR` | Page `<title>` |
| `@description` | `@description EXPR` | Meta description |
| `@canonical` | `@canonical EXPR` | Canonical URL |
| `@meta` | `@meta` | Emit additional meta tags |
| `@head` | `@head` | Inject content into `<head>` |
| `@section` | `@section "name" { ... }` | Define a named section |
| `@yield` | `@yield` | Render page body in a layout |
| `@slot` | `@slot "name"` | Named slot (layout exposes / page fills) |
| `@code` | `@code { ...Motoko... }` | Motoko state, handlers, helpers |
| `@style` | `@style { ...css... }` | Scoped CSS |
| `@theme` | `@theme { tokens }` | Design tokens |
| `@authorize` | `@authorize` · `@authorize role="Admin"` | Require auth / role (role stores = Roadmap) |

### Control flow
| Directive | Form |
|---|---|
| Conditional | `@if EXPR { } else { }` |
| Loop | `@for X in EXPR { }` |
| Switch | `@switch EXPR { case #Variant { } }` |

### Output
| Form | Meaning |
|---|---|
| `@count` | inline output of a value |
| `@user.name` | inline output of a field |
| `@(expr)` | inline output of an arbitrary Motoko expression |

### Effects & animation
| Form | Meaning |
|---|---|
| `@effect Focus("#x")` | focus an element |
| `@effect ScrollTo(...)` | scroll |
| `@effect Toast(...)` | toast notification |
| `@animate` | animation hook |

---

## Event attributes

| Attribute | Form(s) |
|---|---|
| `@click` | `@click="handler"` · `@click="handler(arg)"` |
| `@submit` | `@submit="handler"` |
| `@input` | `@input="handler"` |
| `@change` | `@change="handler"` |

Argument semantics:
- Handler args are evaluated **server-side at render time** and baked into `data-mv-arg*` attributes.
- WASM client forwards `handlerId` + args; server dispatches to the typed Motoko function.
- Developer writes only `@click="save"` / `@click="remove(id)"`.

---

## Form / input attributes

| Attribute | Used on | Meaning |
|---|---|---|
| `secure` | `<form>` | Mint + verify a signed token (see Security) |
| `@submit="handler"` | `<form>` | Submit handler |
| `bind="@model.field"` | inputs | Two-way bind to a model field |
| `name` | inputs | Field name |
| `label` | inputs | Field label |
| `required` | inputs | Required field |
| `minLength` | inputs | Minimum length |

### Validation DSL (in handlers)

```motoko
validate model {
  name  required "Name is required";
  email required email;
  price min 1 "Price must be at least 1";
};
```

Rules seen in facts: `required`, `email`, `min N`. Each rule may take a trailing message string.
On failure → batch status `validation-error`; re-render with `<ValidationSummary />` and per-field errors.

---

## Security model (`secure` form)

The `secure` attribute mints a token via **HMAC-SHA256** binding:

```
path + handler + caller principal + nonce + expiry + field-schema hash
```

On submit the server **re-derives the MAC** and rejects:
- MAC mismatch (tamper)
- expired tokens
- replays (nonce)

SHA-256 and HMAC are implemented in Motoko and verified against standard test vectors.

---

## Components

Capitalized tags = components. App components: `src/Components/*.mview`, declare `param name : T [= default]`.

| Built-in | Params |
|---|---|
| `Button` | `kind`, `size` |
| `Card` | `title` |
| `Alert` | `type` |
| `Badge` | `type` |
| `InputText` | `name`, `label`, `bind`, `required`, `minLength` |
| `InputEmail` | `name`, `label`, `bind`, `required`, `minLength` |
| `InputNumber` | `name`, `label`, `bind`, `required`, `minLength` |
| `TextArea` | `name`, `label`, `bind`, `required`, `minLength` |
| `ValidationSummary` | — |
| `Table` | — |
| `PageHeader` | — |
| `Grid` | `columns` |

---

## CLI

| Command | Action |
|---|---|
| `motoview new <app>` | Scaffold a project |
| `motoview build` | Compile `.mview` → Motoko |
| `motoview dev` | Build + `dfx deploy` + watch (local) |
| `motoview compile <file>` | Compile a single file (debugging) |

Prereqs: `dfx`; `rustup` + `wasm32-unknown-unknown` target; `wasm-opt` (binaryen); the `motoview`
compiler via `cargo`; the runtime as mops package `motoview` or a local path dep in `dfx.json`
(`--package motoview ../../runtime/src`). **No Node/npm/JS build tooling.**

---

## Project layout

| Path | Contents |
|---|---|
| `src/Pages/*.mview` | Routed pages |
| `src/Layouts/*.mview` | Layouts |
| `src/Components/*.mview` | Components |
| `src/Services/*.mo` | Motoko services/state |
| `src/Models/*.mo` | Motoko types/models |
| `src/main.mo` · `src/Generated/` | Compiler output (generated actor) |
| `motoview.json` | MotoView config |
| `dfx.json` | DFINITY SDK config |
| `mops.toml` | Motoko package manifest |

---

## Protocol: `motoview/1`

### Endpoints

| Phase | Method | URL | Notes |
|---|---|---|---|
| First load | `GET` | `/page` | Server-rendered HTML; body in `<div id="mv-root">` |
| Sync (poll) | `GET` | `/_motoview/render?path=...&lastBatchId=...` | Returns a batch JSON |
| Event | `POST` (form-encoded) | `/_motoview/event` | Served by `http_request_update`; returns a new batch immediately |
| Client assets | `GET` | `/motoview.wasm` · `/motoview.js` · `/motoview.css` | Rust→WASM client + JS glue + CSS |

### Batch status

| `status` | Meaning |
|---|---|
| `changed` | New HTML; client swaps `#mv-root` |
| `unchanged` | `batchId` matched; HTML omitted; client skips re-render |
| `redirect` | Navigate elsewhere |
| `validation-error` | Validation failed; errors in batch, re-render with messages |

`batchId` = hash of the rendered state. Matching ids let the client skip DOM work.

### Client roles

- **WASM (brain):** adaptive polling state machine, the protocol, batch interpretation, event sequencing.
- **JS glue (hands):** DOM/fetch/timer primitives + focus/scroll/input preservation. Tiny, hand-written,
  no bundler/npm.

### Adaptive polling cadence

| State | Cadence | Trigger |
|---|---|---|
| hot | ~350ms | for ~3s after an interaction |
| warm | ~2.5s | tab visible |
| cold | ~15s | idle |
| hidden | ~45s | tab backgrounded |
| offline | exponential backoff | network unavailable |

Event responses return the new batch immediately; polling only catches **external** changes.

### IC transport note (MVP)

`http_request` returns `upgrade = true`, so every request is served by `http_request_update` (avoids
query response-certification). **Certified query rendering** for cacheable public pages = Roadmap.

---

## Roadmap (NOT shipped)

Keyed-region / granular DOM patches · full Internet Identity login over HTTP · role stores · vetKeys
encrypted state · certified query rendering · desktop/mobile/tablet shells · visual designer · push adapter.

## Verified

Runtime + WASM client + dfx pipeline (counter deployed to a local replica, exercised in a real browser:
event → update → batch → DOM swap; adaptive polling picks up external changes; state persists across
calls). SHA-256/HMAC pass standard test vectors. Examples: counter, todo, contact form, products CRUD.
