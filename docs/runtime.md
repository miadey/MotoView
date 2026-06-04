---
title: The Motoko Runtime
section: Architecture
slug: runtime
---

# The Motoko Runtime

When you run `motoview build`, your `.mview` files are compiled to Motoko and wired into a canister actor. That actor never works alone — it leans on the **MotoView runtime**, a Motoko library shipped in `runtime/` (the mops package `motoview`). The runtime is what turns an HTTP request from the browser into rendered HTML, dispatched events, and versioned UI batches.

This page walks through the modules in `runtime/src` and shows how a generated actor stitches pages, layouts, and assets together behind `http_request` / `http_request_update`.

## What lives in `runtime/src`

The runtime is deliberately small and dependency-free. Each module owns one concern:

- **App** — the orchestrator class. Holds registered pages and layouts, routes incoming requests, renders the matching page into its layout, dispatches events to typed handlers, and produces the batch JSON described in the [protocol](protocol.md).
- **Types** — the shared types: requests, responses, batches, route entries, render context, handler signatures.
- **Router** — matches a request path against registered routes, including parameters like `/products/{id}` and typed `/orders/{id:Nat}`.
- **Url** — parses paths, query strings, and form-encoded bodies (used to read `path`, `lastBatchId`, and event payloads).
- **Json** — minimal JSON building/escaping for batch responses.
- **Hash** — hashing helpers, including the `batchId` hash that lets the client skip unchanged renders.
- **Security / Sha256** — SHA-256 and HMAC-SHA256 in pure Motoko, verified against standard test vectors. This is what mints and re-derives the signed `secure` form token (path + handler + caller principal + nonce + expiry + field-schema hash).
- **Html.Builder** — a small typed builder for emitting escaped HTML, used by generated page code.
- **Base64** — encode/decode for tokens and binary asset payloads.
- **ClientAssets** — serves the only browser code: `/motoview.wasm` (the WASM brain), `/motoview.js` (the JS glue hands), and `/motoview.css`.

## How a generated actor wires everything

The compiler emits a Motoko actor that constructs an `App`, registers each compiled page and layout, and forwards the IC HTTP entry points to it.

```motoko
import App "mo:motoview/App";
import Types "mo:motoview/Types";

import HomePage "Generated/HomePage";
import MainLayout "Generated/MainLayout";

actor {
  let app = App.App();

  // Pages and layouts compiled from your .mview files
  app.layout("main", MainLayout.render);
  app.page("/", HomePage.render, HomePage.handlers);

  public query func http_request(req : Types.Request) : async Types.Response {
    // MVP transport: upgrade so every request is served by update.
    app.httpRequest(req);
  };

  public func http_request_update(req : Types.Request) : async Types.Response {
    app.httpRequestUpdate(req);
  };
};
```

Two entry points, two jobs:

- **`http_request`** — the IC query entry point. In the MVP it returns `upgrade = true`, which tells the replica to re-issue the call as an update. This sidesteps query response-certification while the protocol matures. (Certified query rendering for cacheable public pages is **Roadmap**.)
- **`http_request_update`** — does the real work. The `App` routes the path through the `Router`, then:
  - For a first load (`GET /page`), it renders the page into its layout and returns server-rendered HTML with content inside `<div id="mv-root">`.
  - For a sync poll (`GET /_motoview/render`), it compares the freshly computed `batchId` against `lastBatchId` and returns `"changed"` or `"unchanged"`.
  - For an event (`POST /_motoview/event`), it verifies the `secure` token via `Security`, dispatches `handlerId` + args to the typed Motoko handler, re-renders, and returns the new batch immediately.
  - For `/motoview.wasm`, `/motoview.js`, `/motoview.css`, it serves bytes from `ClientAssets`.

## The request lifecycle

```bash
# First load: full HTML document
GET /                         -> <html> ... <div id="mv-root"> ... </div>

# Background sync (adaptive polling)
GET /_motoview/render?path=/&lastBatchId=ab12  -> { "status": "unchanged" }

# An interaction
POST /_motoview/event         -> { "status": "changed", "batchId": "cd34", "html": "..." }
```

Because every response carries a `batchId` (a hash of rendered state from `Hash`), the WASM client only swaps the DOM when something actually changed.

## Where to go next

- See [Events](events.md) for how `@click` handlers are baked into `data-mv-arg*` attributes and dispatched server-side.
- See [Forms & Security](security.md) for the full `secure` token derivation.
- See [The Protocol](protocol.md) for batch statuses and the adaptive polling cadence.
