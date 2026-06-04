---
title: CLI Reference
section: Deployment
slug: cli
---

# CLI Reference

The `motoview` binary is your single entry point for scaffolding, compiling, and deploying a MotoView app. It is a Rust crate you install with `cargo`, and it speaks to `dfx` under the hood for the deploy step. There is no Node, no npm, and no JavaScript build tooling anywhere in the pipeline — that is the point.

```bash
motoview <command> [options]
```

All commands run from your project root, where `motoview.json`, `dfx.json`, and `mops.toml` live.

## motoview new

Scaffold a new project.

```bash
motoview new my-shop
```

This creates the standard [project layout](quickstart.md): `src/Pages/`, `src/Layouts/`, `src/Components/`, `src/Services/`, `src/Models/`, plus `motoview.json`, `dfx.json`, and `mops.toml`. It wires the runtime (the `motoview` mops package) so the generated actor can serve HTTP, and it drops in a working starter page so your first `motoview dev` shows something real in the browser.

## motoview build

Compile every `.mview` file in `src/` to Motoko.

```bash
motoview build
```

`build` is the heart of the toolchain. It parses your templates and `@code { ... }` blocks and emits a Motoko actor as `src/main.mo`. It does **not** deploy — use it in CI or when you want to inspect generated Motoko before shipping.

### The build pipeline

Each `motoview build` runs these steps in order:

1. **Discover** all `.mview` files in `src/Pages`, `src/Layouts`, and `src/Components`.
2. **Parse** template markup and directives — `@page`, `@layout`, `@code`, `@if`, `@for`, `@switch`, components, and the inline `@expr` outputs.
3. **Resolve routes** from `@page "/path"` directives, including parameterized routes like `@page "/orders/{id:Nat}"`.
4. **Type-bind handlers**: each `@click="save"` / `@submit="send"` is matched to a typed Motoko function, and handler arguments are lowered into `data-mv-arg*` attributes so the WASM client can forward `handlerId + args`.
5. **Generate Motoko**: render functions, the event dispatcher, and the [`motoview/1` protocol](protocol.md) wiring (`http_request` / `http_request_update`, the `/_motoview/render` and `/_motoview/event` endpoints).
6. **Write output** to the `src/main.mo` actor (override with `--out`).

If a template references a handler that does not exist, or a `bind="@model.field"` points at an unknown field, the build fails here with a clear error — before you ever reach the replica.

## motoview dev

Build, deploy locally, and watch.

```bash
motoview dev
```

This is your day-to-day loop. It runs `motoview build`, hands the generated Motoko to `dfx` for a local deploy, and then watches your `.mview` files. Edit a page, save, and the project recompiles and redeploys so you can refresh the browser. The adaptive polling client (hot ~350ms after an interaction, warm ~2.5s while visible) means external state changes show up without a manual reload — see [Protocol](protocol.md) for the cadence details.

## motoview compile

Compile a single file. Useful for debugging the compiler or inspecting one template's output.

```bash
motoview compile src/Pages/Products.mview
```

Where `motoview build` walks the whole project, `compile` targets one `.mview` and emits its Motoko so you can read exactly what a given template produces. Reach for this when a page misbehaves and you want to see the generated render and dispatch code in isolation.

## Prerequisites

Before any of the above, install the toolchain:

```bash
# DFINITY SDK
sh -ci "$(curl -fsSL https://internetcomputer.org/install.sh)"

# Rust + the wasm target used by the browser client
rustup target add wasm32-unknown-unknown

# wasm-opt (binaryen)
brew install binaryen

# The MotoView compiler
cargo install motoview
```

The runtime is added per-project as the `motoview` mops package, or as a local path dependency in `dfx.json` build args:

```json
"args": "--package motoview ../../runtime/src"
```

## Where to go next

- New to the framework? Start with [Getting Started](quickstart.md).
- Wiring up handlers? See [Events](events.md).
- Curious how the browser stays in sync? Read the [Protocol](protocol.md) page.
