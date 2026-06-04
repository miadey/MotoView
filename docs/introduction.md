---
title: Introduction
section: Prologue
slug: introduction
---

# Introduction

MotoView is a Motoko-native, server-driven UI framework for Internet Computer (ICP) canisters. You write `.mview` files — template markup and Motoko code living together in a single file — and MotoView compiles them to Motoko and deploys them to ICP with `dfx`. There is no application JavaScript, no React, no agent-js glue, and no validation you have to write twice.

If you have used Blazor, Laravel Blade, or Phoenix LiveView, the feel will be familiar. The difference is where it runs: your UI lives inside the canister, rendered by Motoko, served straight from the chain.

> **Write Motoko. Ship interactive, SEO-friendly ICP apps. No frontend JavaScript.**

## The core promise

The mental model is small enough to fit in one sentence: **rendering is a query, events are updates, and the browser synchronizes through versioned UI batches.**

- **Render as query.** When the page needs to show its current state, the server renders it. The first load is server-rendered HTML — real markup that search engines and link previews can read — with the page content living inside `<div id="mv-root">`.
- **Event as update.** When a user clicks a button or submits a form, the browser posts the event to the canister. An update call dispatches it to a typed Motoko handler, your state changes, and a freshly rendered batch comes back immediately.
- **Sync through versioned batches.** Every rendered state carries a `batchId` that hashes its content. The browser keeps polling on an adaptive cadence; if the `batchId` hasn't changed, the server answers `unchanged` and skips re-sending HTML. When it has changed, the client swaps in the new markup.

You never wire up fetch calls, diff state by hand, or re-implement your validation rules on a client. You write Motoko, and the protocol moves it to the screen.

## The one browser component

MotoView ships exactly one piece of browser code, and you don't write it. The client is written in Rust and compiled to WebAssembly — served at `/motoview.wasm` — accompanied by a tiny hand-written JS glue (`/motoview.js`, no bundler, no npm). The WASM is the brain: the adaptive polling state machine, the `motoview/1` protocol, batch interpretation, and event sequencing. The JS glue is just the hands: the unavoidable DOM, fetch, and timer primitives, plus focus, scroll, and input preservation. There is no JS build step in your project, and that is on purpose.

## Who it's for

MotoView is for developers building real, interactive ICP applications who would rather stay in one language than stitch a frontend toolchain onto a canister. It suits dashboards, CRUD apps, forms, and content-driven pages that need to be SEO-friendly and stay fast — without a Node/npm pipeline anywhere in sight. If you already know Motoko, you already know most of MotoView.

## One file: the `.mview` idea

A `.mview` file is template markup plus Motoko in the same place. Markup describes the view; an `@code { ... }` block holds the state and handlers; directives like `@if`, `@for`, and inline output (`@count`) bind them together. Here is a complete, interactive counter — the example that has been deployed to a local replica and exercised in a real browser:

```razor
@page "/counter"
@title "Counter"

@code {
    var count : Nat = 0;

    func increment() { count += 1 };
    func decrement() { if (count > 0) count -= 1 };
}

<h1>Counter</h1>
<p>Current value: @count</p>

<Button kind="primary" @click="increment">+</Button>
<Button @click="decrement">-</Button>
```

The `@click` handlers map to the `increment` and `decrement` functions you wrote right above the markup. Clicking sends an event, the update call runs the handler, the new state is rendered into a batch, and the DOM swaps. State persists across calls because it lives in the canister.

To build and run it locally:

```bash
motoview new my-app
motoview dev
```

`motoview dev` compiles your `.mview` files to Motoko, deploys to the local replica with `dfx`, and watches for changes.

## Where to go next

- [Events](events.md) — how `@click`, `@submit`, and `@input` map to typed Motoko handlers.
- [Forms & Validation](forms.md) — `secure` forms, `bind`, and `validate` blocks.
- [Components](components.md) — built-in semantic components and your own under `src/Components`.
- [Protocol](protocol.md) — the `motoview/1` batch format and adaptive polling cadence.

Write Motoko. Ship the page.
