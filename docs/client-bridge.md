---
title: The Client Bridge
section: Architecture
slug: client-bridge
---

# The Client Bridge

Every MotoView app ships with the same small client. It is the only code that runs in the browser, and it is deliberately split into two parts: a **brain** written in Rust and compiled to WebAssembly, and a thin pair of **hands** written by hand in JavaScript. The canister serves both, plus a stylesheet, from three fixed URLs:

```
/motoview.wasm   the brain  (Rust -> WASM)
/motoview.js     the hands  (DOM/fetch/timer glue)
/motoview.css    base styles
```

You never write any of this. Your `.mview` pages compile to Motoko, the runtime serves the bridge automatically, and the bridge keeps the page in sync with the canister.

## The brain: Rust compiled to WASM

The WASM module owns everything that needs real logic. It runs the [protocol](protocol.md) (`motoview/1`), interprets the batch JSON the server returns, sequences events, and drives the adaptive polling state machine that decides *when* to ask the server for a fresh render.

That cadence is the brain's most visible job. After you interact with the page it runs **hot** (~350ms for about 3 seconds), settles to **warm** (~2.5s while the tab is visible), drops to **cold** (~15s) when idle, slows to **hidden** (~45s) when the tab is backgrounded, and backs off exponentially when offline. When a batch arrives whose `batchId` matches what is already on screen, the server reports `unchanged` and the brain skips the work entirely.

Keeping this in Rust/WASM means the protocol logic is one compiled artifact, the same in every app, fast, and not something you can accidentally fork or monkey-patch.

## The hands: the JS glue

WASM cannot touch the DOM, open a `fetch`, or set a timer on its own. Those capabilities only exist on the JavaScript side of the boundary, so the brain reaches them through a small, hand-written glue file. No bundler, no npm, no framework — just the primitives the brain asks for:

- swap the `#mv-root` content when a batch says `changed`
- issue `GET /_motoview/render` and `POST /_motoview/event`
- start and stop timers for the polling cadence
- preserve focus, scroll position, and in-flight input across a DOM swap

The glue is intentionally dumb. It holds no application state and makes no decisions; it does what the brain tells it and reports DOM events back.

## Preserving focus, scroll, and input

A naive server-driven framework feels broken because every refresh blows away what the user was doing. MotoView's glue handles the three things people notice immediately. Before a swap it records which element is focused, the caret/selection position in text inputs, the current scroll offsets, and any values the user has typed but not yet submitted. After the brain applies the new HTML into `#mv-root`, the glue restores them.

This is why a form stays usable while the page is polling behind it:

```razor
<form @submit="send" secure>
  <InputEmail name="email" label="Email" bind="@model.email" required />
  <InputText name="name" label="Name" bind="@model.name" required />
  <Button kind="primary">Send</Button>
</form>
```

You type into `email`, a warm poll lands, the page re-renders — and your text, caret, and focus are exactly where you left them. See [Forms & Validation](forms.md) for how `bind` and `secure` work, and [Events](events.md) for how the submit becomes an update.

## Why WASM, and the honest tradeoffs

Putting the brain in WebAssembly buys real things: one shared, compiled implementation of the protocol and polling logic; no JavaScript application code to write, audit, or keep in sync with your Motoko; and a clean line between *logic* (portable, testable Rust) and *side effects* (the unavoidable browser primitives).

The honest cost is the boundary itself. WASM has no direct access to the DOM, network, or timers — every one of those calls has to cross into the JS glue and back. So the glue is not optional, and it is the one piece of the client that is plain JavaScript. We keep it as small and side-effect-driven as we can, but it exists because the browser gives WASM no other door to the page.

The current build also makes a transport tradeoff worth knowing: on IC, `http_request` returns `upgrade = true`, so every request is served by `http_request_update`. That sidesteps query response-certification and keeps the bridge simple. Certified query rendering for cacheable public pages is **Roadmap**, as are keyed-region DOM patches that would let the glue update a sub-tree instead of swapping all of `#mv-root`.

Next: read the [Protocol](protocol.md) for the exact batch shapes the brain consumes.
