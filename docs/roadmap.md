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
- **Output** — escaped interpolation (`@count`, `@(expr)`) plus `@raw(expr)` for trusted, unescaped server-rendered HTML, and `@@` for a literal `@`. The compiler scans `Model`/`Service` record types so `@item.name` interpolates with the field's real type (`Text` directly, `Nat`/`Int`/`Float`/`Bool` converted) instead of a `debug_show` fallback.
- **Tooling** — `motoview check` builds and type-checks the generated actor, mapping any `moc` error back to the originating `.mview` (not the generated `main.mo`). A parser/codegen regression suite runs with `make test`.
- **Examples & apps** — `counter`, `contact` (secure form), `crm` (Kanban), `products` (CRUD), `blog` (SEO) and `svg-network` (interactive SVG) under `examples/`; two full apps under `apps/` — **bzzz** (a Discord × X × forum × WhatsApp super-app) and **this site** (the docs + marketing site, itself a MotoView canister).

## Production features

Built and verified end to end — locally and, where it matters, against the IC mainnet boundary (the playground):

- **Upgrade-stable persistence** — a service exposing `mvStableSave()`/`mvStableLoad(Blob)` (a Candid round-trip) gets an auto-generated `stable var` plus `preupgrade`/`postupgrade` hooks, so its state survives `dfx deploy --mode upgrade`. See [Persistence](persistence.md).
- **Internet Identity login** — hand-rolled, no npm and no agent-js: a browser IC agent makes one authenticated call, the runtime mints an httpOnly session cookie, and `ctx.caller` resolves from it. Served at `/mv-auth.js`; add `<button data-mv-signin>`. Includes per-principal session revocation.
- **Role stores** — `@authorize role="Admin"` is enforced against a persisted, per-principal role store. Manage roles from any handler via `ctx.hasRole / grantRole / revokeRole / claimRole / callerRoles`; `claimRole` is a first-come bootstrap for the first admin. Survives `--mode upgrade`. See [Security](security.md).
- **Certified query rendering** — static framework assets and pages marked `@cacheable` are served as fast **certified queries** (HTTP response-certification v2) instead of upgrading to an update call. Parameterized cacheable routes are covered by a single wildcard certificate (`/u/{handle}` → `/u/<*>`). Dynamic pages keep the consensus-validated update path.
- **Keyed-region DOM patches** — give list items a `key="..."` and the WASM brain diffs the keyed regions, patching precisely: content changes replace just those items; added/removed/reordered items are inserted/removed/moved (reorder moves the minimum number of nodes). Untouched and merely-moved nodes keep their live state (focus, selection, scroll, media). All the diffing runs in WASM — no application JavaScript. See [Keyed regions](directives-reference.md).

### Transport note

By default `http_request` returns `upgrade = true`, so a request is served by `http_request_update` — consensus-validated, always fresh. Static assets and pages you mark `@cacheable` are the exception: they're served as certified queries (no consensus round-trip).

## Roadmap

Not yet built. Do not design around these — they are labeled honestly as planned.

**Next**

- **Certifying the root `/` and exact-vs-wildcard prefix collisions** — `@cacheable` works for most routes; the root path and an exact route that collides with a wildcard prefix (e.g. `/docs` alongside `/docs/{slug}`) are rejected by the boundary today and safely fall back to the update path.

**Later**

- vetKeys-encrypted state.
- Theme packages & design tokens (the `@theme` directive exists; shareable packages do not).
- A richer animation engine (the `@effect` / `@animate` primitives exist).
- A visual page designer.
- An offline-first service-worker cache layer.

> **Realtime is not on this list.** The adaptive-polling render/event protocol *is* MotoView's communication layer; a canister cannot open a WebSocket without an external gateway. Polling is the design, not a placeholder.

## How to read a feature's status

When in doubt, the directive and CLI references describe only implemented behavior — if you find it documented there with a working snippet, it works. Anything on this page under **Roadmap** is the single source of truth for what is intentionally not done yet.

| Status | Meaning |
| --- | --- |
| Verified | Deployed and exercised in a real browser (or on the mainnet boundary) |
| Implemented / Production | In the compiler/runtime; documented with working snippets |
| Roadmap | Planned; not available |

If something you need is on the Roadmap, that is a great place to contribute. The protocol is intentionally small and the brain/hands split keeps the surface area honest.
