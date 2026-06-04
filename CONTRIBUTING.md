# Contributing to MotoView

Thanks for your interest in MotoView! This document explains the layout of the
repository and how to build everything **without Node or npm** — the toolchain
is 100% the ICP/Rust ecosystem.

## Prerequisites

- [`dfx`](https://internetcomputer.org/docs/current/developer-docs/setup/install/) (the DFINITY SDK) — bundles `moc`
- `rustup` with the `wasm32-unknown-unknown` target: `rustup target add wasm32-unknown-unknown`
- `wasm-opt` (from [Binaryen](https://github.com/WebAssembly/binaryen))
- `python3` (only to build the documentation site)

## Repository layout

```
compiler/   the `motoview` compiler (Rust)        -> .mview to Motoko
runtime/    the Motoko runtime library (mops)     -> serves HTTP from the canister
client/     the browser client (Rust -> WASM)     -> the polling/protocol "brain"
            + glue/ (the minimal hand-written JS "hands")
examples/   working example apps (counter, contact, ...)
apps/       bzzz (reference super-app) + site (the marketing/docs website, itself a MotoView canister)
docs/       developer documentation (Markdown — the source apps/site renders)
skills/     AI assistant skill files (Claude/Copilot/Cursor live in .github/.cursor)
tools/      build + type-check helper scripts
```

## Common tasks

```bash
make client      # build the Rust->WASM client and embed it into runtime/src/ClientAssets.mo
make compiler    # build the `motoview` binary
make check       # type-check the Motoko runtime with moc
make example     # compile + deploy the counter to a local replica
make site        # regenerate + compile the docs/marketing site (apps/site)
```

## The architecture in one paragraph

A `.mview` file is template markup plus Motoko `@code`. The compiler parses the
template and scans the `@code` declarations, then emits a Motoko `object` per
page (state + handlers + a `render` that builds HTML) plus an actor that wires
everything into the runtime. The runtime serves the page over the canister's
`http_request`/`http_request_update` interface: **rendering is a query, events
are updates, and the browser synchronizes through versioned UI batches**. The
browser client is Rust compiled to WASM (the decision logic) with a tiny JS glue
layer for the DOM/`fetch`/timer primitives the browser only exposes to JS.

## Pull requests

- Keep the no-Node/no-npm rule: the only browser code is the WASM client + its
  minimal hand-written glue.
- Run `make check` and rebuild the affected example before submitting.
- If you change the WASM client, re-run `make client` so the embedded assets
  stay in sync.
