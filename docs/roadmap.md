---
title: Roadmap & Status
section: Reference
slug: roadmap
---

# Roadmap & Status

MotoView is a real framework, built and verified end to end — but it is young. This page is the honest line between what works today and what is still ahead. We would rather you ship on solid ground than discover a "Roadmap" feature mid-project.

If a capability is listed under **Verified** or **Implemented**, you can use it now. If it is listed under **Roadmap**, treat it as planned and design around its absence.

## Verified end to end

The core loop is not a demo slide — it has been deployed to a local replica and exercised in a real browser:

- The counter example was deployed with `dfx` and clicked through a browser. Each click flows through the full pipeline: event to `http_request_update`, a fresh batch is returned, and the DOM is swapped.
- **Adaptive polling** picks up external state changes (a value mutated by another caller appears without a user interaction).
- **State persists** across calls — the canister is the source of truth.
- **SHA-256 and HMAC-SHA256** are implemented in Motoko and pass standard test vectors. This is what backs the `secure` form token (path + handler + caller principal + nonce + expiry + field-schema hash).

```bash
motoview new counter
cd counter
motoview dev          # build .mview -> Motoko, dfx deploy, watch
```

```razor
@page "/"
@code {
  var count = 0;
  func increment() { count += 1 };
}

<Button @click="increment">Count: @count</Button>
```

## Implemented

These ship in the current compiler and runtime:

- **The `.mview` pipeline** — the `motoview` Rust compiler parses templates plus `@code` and generates a Motoko actor. The runtime (mops package `motoview`) serves HTTP from the canister.
- **Directives** — `@page` (including `/products/{id}` and typed `/orders/{id:Nat}`), `@layout`, `@title`, `@description`, `@canonical`, `@meta`, `@section` / `@yield`, `@head`, `@slot`, `@code`, `@style`, `@theme`, control flow (`@if`/`else`, `@for`, `@switch`), inline output, `@effect`, `@animate`.
- **Events** — `@click`, `@submit`, `@input`, `@change`, with server-evaluated arguments baked into `data-mv-arg*`. See [Events](events.md).
- **Forms, validation, security** — `secure` forms, `bind="@model.field"`, the `validate model { ... }` block, `<ValidationSummary />`, and per-field errors. See [Forms & Validation](forms.md) and [Security](security.md).
- **Components** — capitalized app components in `src/Components/*.mview` plus built-ins (`Button`, `Card`, `Alert`, `Badge`, `InputText`, `InputEmail`, `InputNumber`, `TextArea`, `ValidationSummary`, `Table`, `PageHeader`, `Grid`). See [Components](components.md).
- **The WASM client** — the browser "brain" (the `motoview/1` protocol, adaptive polling state machine, batch interpretation, event sequencing) is Rust compiled to WebAssembly, with a tiny hand-written JS glue for DOM, fetch, timers, and focus/scroll/input preservation. No npm, no bundler.
- **Examples** — `todo`, `contact form`, and `products CRUD` are provided alongside the counter.

### MVP transport note

On the Internet Computer, `http_request` currently returns `upgrade = true`, so every request is served by `http_request_update`. This sidesteps query response-certification today. **Certified query rendering** for cacheable public pages is Roadmap (below).

## Roadmap

Not yet built. Do not design around these — they are labeled honestly as planned.

**v2**

- Keyed-region / granular DOM patches (today a changed batch swaps `#mv-root`).
- Full Internet Identity login over HTTP and role stores backing `@authorize role="Admin"`.
- Certified query rendering for cacheable public pages (removing the blanket `upgrade = true`).

**v3**

- vetKeys-encrypted state.
- Desktop / mobile / tablet shells.
- Visual designer.
- Push adapter (server-initiated updates beyond adaptive polling).

## How to read a feature's status

When in doubt, the directive and CLI references describe only implemented behavior — if you find it documented there with a working snippet, it works. Anything on this page under **Roadmap** is the single source of truth for what is intentionally not done yet.

| Status | Meaning |
| --- | --- |
| Verified | Deployed and exercised in a real browser |
| Implemented | In the compiler/runtime; documented with working snippets |
| Roadmap (v2/v3) | Planned; not available |

If something you need is on the Roadmap, that is a great place to contribute. The protocol is intentionally small and the brain/hands split keeps the surface area honest.
