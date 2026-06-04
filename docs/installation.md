---
title: Installation
section: Getting Started
slug: installation
---

# Installation

MotoView compiles your `.mview` files to Motoko and deploys them to an Internet Computer canister with `dfx`. There is no Node, no npm, and no JavaScript build step to set up — that absence is the whole point. The toolchain is just three things: the DFINITY SDK, a Rust toolchain, and the `motoview` compiler.

This page walks you from an empty machine to a verified toolchain. When you're ready to build something, head to the [Quickstart](quickstart.md).

## Prerequisites

You need three tools on your `PATH` before installing MotoView.

### dfx (DFINITY SDK)

`dfx` builds and deploys your canister to a local replica or to mainnet.

```bash
sh -ci "$(curl -fsSL https://internetcomputer.org/install.sh)"
dfx --version
```

### Rust and the WebAssembly target

MotoView's browser client is written in Rust and compiled to WebAssembly — it's the only browser-side code in the framework. You need `rustup` and the `wasm32-unknown-unknown` target.

```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
rustup target add wasm32-unknown-unknown
```

### wasm-opt (binaryen)

The WASM client is optimized with `wasm-opt` from the [binaryen](https://github.com/WebAssembly/binaryen) toolkit.

```bash
# macOS
brew install binaryen

# Debian/Ubuntu
apt-get install binaryen
```

Confirm it resolves:

```bash
wasm-opt --version
```

## Installing the compiler

The `motoview` binary is a Rust crate. Build and install it with `cargo`, which `rustup` provided above.

```bash
cargo install --git https://github.com/your-org/motoview motoview
```

If you're working from a local checkout of the repository instead:

```bash
cargo install --path compiler
```

Verify the install:

```bash
motoview --version
```

You now have the four CLI commands available — `motoview new`, `motoview build`, `motoview dev`, and `motoview compile`. See the [CLI reference](cli.md) for details.

## Adding the runtime

The compiler turns `.mview` files into Motoko, but the generated actor depends on the MotoView **runtime** — the Motoko library that serves HTTP from your canister, interprets events, and emits versioned UI batches.

The recommended way to add it is through [mops](https://mops.one), the Motoko package manager. In your `mops.toml`:

```toml
[dependencies]
motoview = "0.1.0"
```

Then install:

```bash
mops install
```

If you're developing against a local checkout of the runtime, point `dfx.json` at it directly with a package argument instead of using mops:

```json
{
  "canisters": {
    "app": {
      "type": "motoko",
      "main": "src/main.mo",
      "args": "--package motoview ../../runtime/src"
    }
  }
}
```

Both approaches make `import MotoView "mo:motoview"` resolve when the generated actor is compiled.

## Verifying the toolchain

Scaffold a project and run it once to confirm every piece is wired up correctly.

```bash
motoview new hello
cd hello
motoview dev
```

`motoview dev` compiles your `.mview` files to Motoko, runs `dfx deploy` against the local replica, and watches for changes. When it finishes it prints the canister URL. Open it in a browser and you should see the rendered page served straight from the canister at `<canister-url>/`.

The scaffold ships with a counter page. Clicking the button sends an event to the canister's update method and swaps in a new batch — this is the same `event -> update -> batch -> DOM swap` cycle described in [Events](events.md), and it's verified end to end. If the count increments in the browser, your install is good.

To compile a single file without deploying — handy when debugging a template — use:

```bash
motoview compile src/Pages/Index.mview
```

A clean compile and a working counter confirm all four moving parts agree: the `dfx` replica, the Rust-built WASM client served at `/motoview.wasm`, the runtime serving the `motoview/1` protocol, and the compiler that generated the actor.

## Next steps

- [Quickstart](quickstart.md) — build your first page from scratch.
- [Project Layout](project-layout.md) — where `.mview` files, services, and config live.
- [Events](events.md) — how `@click`, `@submit`, and `@input` flow to typed Motoko handlers.
