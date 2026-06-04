# MotoView — GitHub Copilot Instructions

These instructions tell Copilot how to generate correct code for **MotoView**. Follow them
exactly. When in doubt, prefer fewer assumptions: if a capability is not listed here, treat it
as **not implemented** and label it "Roadmap" rather than inventing it.

## What MotoView is

MotoView is a **Motoko-native, server-driven UI framework for Internet Computer (ICP) canisters**.

- You write `.mview` files: template markup + Motoko code in one file (Blazor/Blade/LiveView feel).
- It **compiles to Motoko** and deploys to ICP with `dfx`.
- **No application JavaScript, no React, no agent-js glue, no duplicated validation.**
- Mental model: *rendering is a query, events are updates, and the browser synchronizes through
  versioned UI batches.*

Taglines you can echo: "Write Motoko. Ship interactive, SEO-friendly ICP apps. No frontend JavaScript."

## Hard rule: NO Node / npm / JS build tooling

This is a **core selling point**. Do **NOT** suggest, scaffold, or reference any of the following:

- `package.json`, `npm`, `yarn`, `pnpm`, `npx`
- Webpack, Vite, Rollup, esbuild, Babel, or any JS bundler
- React, Vue, Svelte, agent-js, or any frontend JS framework
- Hand-written application JavaScript

The **only** browser code is a Rust→WebAssembly client shipped by the framework itself
(`/motoview.wasm`, `/motoview.js`, `/motoview.css`). The `motoview.js` is a tiny hand-written
glue file the framework provides — application developers never write or edit browser JS.

## Stack

- **Compiler**: a Rust crate that builds the `motoview` binary (installed via `cargo`). Parses
  `.mview`, generates Motoko.
- **Runtime**: a Motoko library in `runtime/` (distributed as the mops package `motoview`). Serves
  HTTP from the canister.
- **Dev/deploy**: `dfx` (DFINITY SDK).
- **Browser client**: Rust compiled to WebAssembly (the brain: adaptive polling state machine,
  protocol, batch interpretation, event sequencing). The JS glue is the hands: DOM/fetch/timer
  primitives plus focus/scroll/input preservation.

## Project layout

```
src/Pages/*.mview
src/Layouts/*.mview
src/Components/*.mview
src/Services/*.mo
src/Models/*.mo
motoview.json      # config
dfx.json
mops.toml
```

Compiler output is a Motoko actor (e.g. `src/main.mo` / `src/Generated/`).

## CLI

```bash
motoview new <app>        # scaffold a project
motoview build            # compile .mview -> Motoko
motoview dev              # build + dfx deploy + watch (local)
motoview compile <file>   # compile a single file (debugging)
```

Prereqs: `dfx`, `rustup` + the `wasm32-unknown-unknown` target, `wasm-opt` (binaryen). Build/install
the `motoview` compiler with `cargo`. Add the runtime via the mops package `motoview` or a local
path dependency in `dfx.json` args (`"--package motoview ../../runtime/src"`).

## `.mview` directives

Top-of-file / structural:

- `@page "/path"` — also `"/products/{id}"` and typed `"/orders/{id:Nat}"`
- `@layout NAME`, `@title EXPR`, `@description EXPR`, `@canonical EXPR`, `@meta`, `@head`
- `@section "name" { ... }`, `@yield`, `@slot "name"`
- `@authorize` and `@authorize role="Admin"`
- `@code { ...Motoko... }` — Motoko code block (state, handlers)
- `@style { ...css... }`, `@theme { tokens }`

Control flow and output (inside markup):

- `@if EXPR { } else { }`
- `@for X in EXPR { }`
- `@switch EXPR { case #Variant { } }`
- Inline output: `@count`, `@user.name`, `@(expr)`
- `@effect Focus("#x")` / `ScrollTo` / `Toast`, `@animate`

## Events

- `@click="handler"`, `@click="handler(arg)"`, `@submit="handler"`, `@input="handler"`, `@change`.
- Handler arguments are evaluated **server-side at render time** and baked into `data-mv-arg*`
  attributes. The WASM client forwards `handlerId` + args; the server dispatches to typed Motoko
  functions. The developer only writes `@click="save"`.

Example:

```razor
@page "/counter"
@code {
  var count : Nat = 0;
  func increment() { count += 1 };
}

<h1>Count: @count</h1>
<Button kind="primary" @click="increment">Add one</Button>
```

## Forms, validation, security

- Use `<form @submit="send" secure>` with inputs bound via `bind="@model.field"`.
- The `secure` attribute mints a **signed token (HMAC-SHA256)** binding path + handler + caller
  principal + nonce + expiry + field-schema hash. On submit the server re-derives the MAC and
  rejects mismatches, expired tokens, and replays. (SHA-256 and HMAC are implemented in Motoko and
  verified against standard test vectors.)
- Validate inside handlers:

```motoko
validate model {
  name required "Name is required";
  email required email;
  price min 1 "Price must be at least 1";
};
```

- On failure, errors are returned in the batch and re-rendered. Use `<ValidationSummary />` and
  per-field errors.

```razor
<form @submit="send" secure>
  <ValidationSummary />
  <InputText name="name" label="Name" bind="@model.name" required />
  <InputEmail name="email" label="Email" bind="@model.email" required />
  <Button kind="primary">Save</Button>
</form>
```

## Components

- **Capitalized tags are components.** App components live in `src/Components/*.mview` and declare
  params with `param name : T [= default]`.
- **Prefer built-in semantic components over utility-class soup.** Use `<Button kind="primary">Save</Button>`,
  not long `class="..."` strings.
- Built-in components: `Button` (kind, size), `Card` (title), `Alert` (type), `Badge` (type),
  `InputText` / `InputEmail` / `InputNumber` / `TextArea` (name, label, bind, required, minLength),
  `ValidationSummary`, `Table`, `PageHeader`, `Grid` (columns).

## Protocol (`motoview/1`)

- **First load**: `GET /page` → server-rendered HTML; page content lives in `<div id="mv-root">`.
- **Sync**: `GET /_motoview/render?path=...&lastBatchId=...` → batch JSON with status
  `"changed"` | `"unchanged"` | `"redirect"` | `"validation-error"`. `"unchanged"` omits html
  when `batchId` matched.
- **Events**: `POST` (form-encoded) `/_motoview/event` → served by `http_request_update` → returns
  a new batch immediately.
- `batchId` is a hash of the rendered state; unchanged batches let the client skip re-rendering.
- **Adaptive polling cadence**: hot ~350ms (for ~3s after an interaction), warm ~2.5s while
  visible, cold ~15s when idle, hidden ~45s, offline = exponential backoff. The event response
  returns the new batch immediately (no wait for the next poll).
- **IC transport note (MVP)**: `http_request` returns `upgrade=true` so every request is served by
  `http_request_update`; this avoids query response-certification. Certified query rendering for
  cacheable public pages is **Roadmap**.

## Verified status (be honest)

- Runtime + WASM client + dfx pipeline are **VERIFIED end to end**: the counter example was deployed
  to a local replica and exercised in a real browser (click → event → update → batch → DOM swap),
  adaptive polling picks up external changes, and state persists across calls. SHA-256/HMAC pass
  standard vectors.
- Additional examples provided: todo, contact form, products CRUD.

## Roadmap (NOT done — do not present as working)

Keyed-region / granular DOM patches, full Internet Identity login over HTTP, role stores, vetKeys
encrypted state, certified query rendering, desktop/mobile/tablet shells, visual designer, push
adapter.

## DO

- Write `.mview` files combining markup + `@code { }` Motoko.
- Use the documented directives, events, and built-in components exactly as named above.
- Use `<form ... secure>` + `bind="@model.field"` + `validate model { ... }` for forms.
- Use `dfx` and `motoview` CLI commands for build/deploy.
- Label anything not listed here as "Roadmap" / "Planned".

## DON'T

- Don't introduce Node, npm, bundlers, React, or any application JavaScript.
- Don't invent directives, attributes, CLI flags, file names, components, or numbers.
- Don't write client-side validation logic — validation lives in Motoko handlers.
- Don't hand-write or edit the browser WASM/JS glue; the framework ships it.
- Don't claim Roadmap features are implemented.
