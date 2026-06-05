---
title: Deployment
section: Deployment
slug: deployment
---

# Deployment

MotoView apps deploy the same way any Internet Computer canister does: with `dfx`. There is no Node build step, no bundler, and no JavaScript pipeline to maintain. You compile your `.mview` files to Motoko, then ship the canister. This page covers building, deploying locally and to mainnet, the transport model, asset serving, and SEO.

## Build, then deploy

The two-step rhythm is **compile** and **deploy**. `motoview build` turns your `.mview` files into Motoko; `dfx` installs the resulting canister.

```bash
# Compile .mview -> Motoko
motoview build

# Deploy to a local replica
dfx start --background
dfx deploy
```

During development, `motoview dev` wraps both steps and watches for changes:

```bash
motoview dev
```

This builds, runs `dfx deploy` against your local replica, and recompiles on save so you can iterate without restarting anything.

## Deploying to mainnet

Mainnet deployment is the same `dfx deploy` with the `ic` network selected. The difference is that mainnet runs on **cycles**, the IC's compute and storage fuel.

```bash
motoview build
dfx deploy --network ic
```

Your identity needs a cycles balance to create and top up the canister. Convert ICP to cycles or use a faucet, then check and top up as needed:

```bash
# How many cycles does the canister hold?
dfx canister --network ic status YOUR_CANISTER

# Top up from your wallet
dfx canister --network ic deposit-cycles 2_000_000_000_000 YOUR_CANISTER
```

A MotoView canister keeps UI state in memory across calls, so it consumes cycles for both compute and the storage backing that state. Watch the balance after launch and budget top-ups accordingly.

## The transport model (upgrade=true)

MotoView serves every request through the canister's HTTP interface. In the MVP, `http_request` returns `upgrade = true`, which tells the replica to re-run the request as `http_request_update`. This sidesteps query response-certification entirely: every page render and every event is handled as an update call.

```motoko
public query func http_request(req : HttpRequest) : async HttpResponse {
  // MVP: defer everything to the update path
  { status_code = 200; headers = []; body = ""; upgrade = ?true };
};

public func http_request_update(req : HttpRequest) : async HttpResponse {
  // Real rendering and event dispatch happen here
};
```

The practical consequences: first loads, sync polls (`/_motoview/render`), and events (`/_motoview/event`) all flow through the update path, so responses reflect the latest state immediately. The tradeoff is that requests pay update-call latency rather than query speed. **Certified query rendering for cacheable public pages is Roadmap.** See [Protocol](protocol.md) for the full request/batch lifecycle.

## Serving assets

The browser client ships as three files served by the canister itself:

- `/motoview.wasm` — the Rust client compiled to WebAssembly (the "brain")
- `/motoview.js` — the small hand-written glue that loads the wasm and provides DOM, fetch, and timer primitives
- `/motoview.css` — base styles for the built-in components

These are emitted by the build and served from your canister's HTTP handler. There is no separate asset canister to wire up and no application JavaScript beyond the glue loader. Your layout references them in `@head`:

```razor
@head {
  <link rel="stylesheet" href="/motoview.css" />
  <script src="/motoview.js" defer></script>
}
```

## SEO: sitemap.xml and robots.txt

Because the first load is fully server-rendered HTML with page content inside `<div id="mv-root">`, crawlers see real markup. Set per-page metadata with the directives in [Pages](pages.md):

```razor
@page "/products/{id:Nat}"
@title "Product " # product.name
@description product.summary
@canonical "https://example.com/products/" # Nat.toText(id)
```

Serve `robots.txt` and `sitemap.xml` as routes from your canister so crawlers can discover and index your pages. Point the sitemap at your canonical mainnet origin and list the URLs you want indexed.

```
User-agent: *
Allow: /
Sitemap: https://example.com/sitemap.xml
```

## mops packaging

The MotoView runtime is distributed as the mops package **`motoview`**. Add it to your project's `mops.toml`:

```toml
[dependencies]
motoview = "..."
```

For local development against a checkout of the runtime, you can instead pass it directly to the Motoko compiler through your `dfx.json` build args:

```json
{
  "canisters": {
    "app": {
      "type": "motoko",
      "main": ".mvbuild/main.mo",
      "args": "--package motoview ../../runtime/src"
    }
  }
}
```

Either way, the runtime provides the HTTP serving, batch protocol, and the SHA-256/HMAC primitives used by `secure` forms. With the package resolved, `motoview build` followed by `dfx deploy` is all it takes to ship.
