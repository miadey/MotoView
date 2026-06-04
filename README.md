# MotoView

**Write Motoko. Ship interactive, SEO-friendly ICP apps. No frontend JavaScript.**

![status: counter verified end-to-end](https://img.shields.io/badge/status-counter%20verified%20end--to--end-success)
![compiler: Rust](https://img.shields.io/badge/compiler-Rust-orange)
![runtime: Motoko](https://img.shields.io/badge/runtime-Motoko-7048e8)
![client: WASM](https://img.shields.io/badge/client-WebAssembly-654ff0)
![platform: Internet Computer](https://img.shields.io/badge/platform-Internet%20Computer-29abe2)
![protocol: motoview/1](https://img.shields.io/badge/protocol-motoview%2F1-blue)
![license: MIT](https://img.shields.io/badge/license-MIT-green)

MotoView is a Motoko-native, server-driven UI framework for Internet Computer (ICP) canisters. You write `.mview` files — template markup and Motoko code together in a single file — and MotoView compiles them to Motoko and deploys them to the IC with `dfx`. If you've enjoyed Blazor, Blade, or LiveView, this will feel familiar.

There is no application JavaScript, no React, no agent-js glue, and no duplicated validation. Your canister renders the page, your handlers run on-chain, and the browser stays in sync automatically.

> Rendering is a query, events are updates, and the browser synchronizes through versioned UI batches.

## Why MotoView

Building a frontend for an ICP canister usually means standing up a second project: a JavaScript bundler, a React app, `agent-js` to call the canister, and a second copy of your validation logic that has to stay in lockstep with the backend. MotoView removes that whole layer.

- **One language, one source of truth.** Markup and Motoko live side by side in `.mview` files. Your validation runs in your handlers — there is no second copy to keep in sync.
- **No Node, no npm, no JS build tooling.** This is a core design goal, not an accident. The compiler is a Rust binary, the runtime is a Motoko library, and the only browser code is a small WebAssembly client. You never run a JavaScript bundler.
- **Server-driven UI.** The canister renders HTML. The first load is fully server-rendered for SEO and fast paint; after that, the browser synchronizes through versioned UI batches.
- **Events are real Motoko functions.** A `@click="save"` dispatches to a typed Motoko handler that runs on-chain. Handler arguments are evaluated server-side and baked into the markup — you just write the handler.
- **Security in the platform.** Secure forms mint a signed HMAC-SHA256 token binding the path, handler, caller principal, a nonce, an expiry, and the field-schema hash. The server re-derives the MAC and rejects mismatches, expired tokens, and replays. SHA-256 and HMAC are implemented in Motoko and verified against standard test vectors.
- **Adaptive, considerate polling.** The client polls fast right after an interaction and backs off when the tab is idle or hidden, so live updates feel instant without hammering the canister.

## Quick Start

### 1. Install the prerequisites

MotoView builds on the standard IC toolchain plus the Rust WebAssembly target.

```bash
# dfx — the DFINITY SDK
sh -ci "$(curl -fsSL https://internetcomputer.org/install.sh)"

# Rust + the wasm32 target (for the browser client)
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
rustup target add wasm32-unknown-unknown

# wasm-opt (from binaryen) for optimizing the client wasm
brew install binaryen   # or your platform's package manager
```

### 2. Build and install the compiler

The `motoview` compiler is a Rust crate. Build and install it from this repository with `cargo`.

```bash
cargo install --path compiler
```

### 3. Scaffold a project

```bash
motoview new my-app
cd my-app
```

### 4. Run it locally

`motoview dev` builds your `.mview` files to Motoko, deploys to a local replica with `dfx`, and watches for changes.

```bash
motoview dev
```

Then open the local URL printed by `dfx` in your browser. Click a button — the event hits the canister, your Motoko handler runs, a new UI batch comes back, and the DOM updates. No page reload, no JavaScript you had to write.

## A counter in one file

This is `src/Pages/Counter.mview` from the verified counter example — markup and Motoko, together:

```razor
@page "/"
@layout MainLayout
@title "Counter"
@description "A live counter built with MotoView — rendering is a query, the +/- clicks are updates."

<section class="mv-container">
    <h1>Counter</h1>

    <p class="counter-value">Current value: <strong>@count</strong></p>

    <div class="counter-actions">
        <button class="mv-btn mv-btn-primary" @click="increment(1)">+1</button>
        <button class="mv-btn mv-btn-primary" @click="increment(5)">+5</button>
        <button class="mv-btn mv-btn-secondary" @click="decrement">-1</button>
        <button class="mv-btn mv-btn-ghost" @click="reset">Reset</button>
    </div>
</section>

@code {
    var count : Nat = 0;

    func increment(by : Nat) : async () {
        count += by;
    };

    func decrement() : async () {
        if (count > 0) { count -= 1 };
    };

    func reset() : async () {
        count := 0;
    };
}
```

The `@click="increment(5)"` handler argument is evaluated server-side at render time and baked into the markup; the WASM client forwards the handler id and its args, and the server dispatches to the typed `increment` function.

## Verified status

The runtime, the WASM client, and the `dfx` pipeline are **verified end to end**: the counter example above was deployed to a local replica and exercised in a real browser. Clicking updates state via `event -> update -> batch -> DOM swap`, adaptive polling picks up external changes, and state persists across calls. SHA-256 and HMAC pass standard test vectors.

Additional examples — a todo list, a contact form, and a products CRUD — are included under `examples/`.

## Features

- `.mview` files: template markup plus Motoko in one file.
- Directives: `@page` (including typed routes like `@page "/orders/{id:Nat}"`), `@layout`, `@title`, `@description`, `@canonical`, `@meta`, `@authorize`, `@section`/`@yield`, `@slot`, `@code`, `@style`, `@theme`, `@if`/`else`, `@for`, `@switch`, inline output (`@count`, `@user.name`, `@(expr)`), `@effect` (Focus / ScrollTo / Toast), and `@animate`.
- Events: `@click`, `@submit`, `@input`, `@change` — dispatched to typed Motoko functions, with handler args evaluated server-side.
- Secure forms with `bind="@model.field"`, signed HMAC-SHA256 tokens, and replay/expiry rejection.
- Handler-side validation (`validate model { ... }`) with `<ValidationSummary />` and per-field errors.
- Built-in semantic components — `Button`, `Card`, `Alert`, `Badge`, `InputText` / `InputEmail` / `InputNumber` / `TextArea`, `ValidationSummary`, `Table`, `PageHeader`, `Grid` — plus your own components in `src/Components/*.mview`.
- The `motoview/1` protocol: server-rendered first load, batch-based sync with `changed` / `unchanged` / `redirect` / `validation-error` statuses, and content-hashed `batchId`s so unchanged batches skip re-rendering.
- Adaptive polling: hot (~350ms after an interaction), warm (~2.5s while visible), cold (~15s when idle), hidden (~45s), and exponential backoff when offline — with event responses returning the new batch immediately.

## Repository layout

```text
compiler/   Rust crate — the motoview binary (parses .mview, generates Motoko)
runtime/    Motoko library (the "motoview" mops package) — serves HTTP from the canister
client/     Rust → WebAssembly browser client + tiny hand-written JS glue (no bundler)
examples/   counter, todo, contact, products
docs/        documentation
site/        project site
skills/      AI agent skills for working with MotoView
```

A typical generated project uses: `src/Pages/*.mview`, `src/Layouts/*.mview`, `src/Components/*.mview`, `src/Services/*.mo`, `src/Models/*.mo`, plus `motoview.json`, `dfx.json`, and `mops.toml`. The compiler emits a Motoko actor.

## Using AI

MotoView ships an agent skill under [`skills/motoview/`](skills/motoview/) so AI coding tools can scaffold pages, wire up events, and write secure forms that match the framework's real APIs. See [`docs/ai-tools.md`](docs/ai-tools.md) for how to point your assistant at it.

## Documentation

Full documentation lives in [`docs/`](docs/) — start with [`docs/introduction.md`](docs/introduction.md) and [`docs/quickstart.md`](docs/quickstart.md), then dig into [pages and routing](docs/pages-and-routing.md), [events](docs/events.md), [forms](docs/forms.md), [validation](docs/validation.md), [security](docs/security.md), [components](docs/components.md), and the [protocol](docs/protocol.md).

## Roadmap

The following are planned and **not yet implemented**:

- Keyed-region / granular DOM patches (instead of root swaps).
- Full Internet Identity login over HTTP and role stores.
- vetKeys-encrypted state.
- Certified query rendering for cacheable public pages.
- Desktop / mobile / tablet shells.
- A visual designer.
- A push adapter.

## License

MotoView is released under the [MIT License](LICENSE).
