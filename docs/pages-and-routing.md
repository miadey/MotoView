---
title: Pages & Routing
section: The .mview Format
slug: pages-and-routing
---

# Pages & Routing

Every screen in a MotoView app is a `.mview` file under `src/Pages/`. A page is template markup plus the Motoko that backs it, living side by side in one file. The first directive you reach for is `@page`, which maps a URL to that file.

```razor
@page "/about"

@title "About us"

<PageHeader>About us</PageHeader>
<p>MotoView renders this from your canister. No frontend JavaScript.</p>
```

When the browser requests `/about`, the canister server-renders the HTML into `<div id="mv-root">` and ships it. From there the WASM client keeps the page in sync. See [Events](events.md) for how interactions flow back to your Motoko handlers.

## Route Parameters

Routes can capture segments with curly braces. Use `{name}` for a string segment, or `{name:Nat}` to require and parse a typed value.

```razor
@page "/products/{id}"
```

```razor
@page "/orders/{id:Nat}"
```

With `{id}` the captured segment arrives as `Text`. With `{id:Nat}` MotoView parses the segment for you; a request whose segment is not a valid `Nat` does not match the route.

## Reading Params in @code

Route parameters are available to your page logic inside `@code`. Read them, load your data, and expose values to the template.

```razor
@page "/orders/{id:Nat}"
@title "Order #" # Nat.toText(id)

@code {
  import Nat "mo:base/Nat";
  import Orders "../Services/Orders";

  let order = Orders.find(id);
}

@switch order {
  case (?o) {
    <PageHeader>Order #@Nat.toText(o.id)</PageHeader>
    <Card title="Summary">
      <p>Total: @(Nat.toText(o.total))</p>
    </Card>
  }
  case null {
    <Alert type="warning">Order not found.</Alert>
  }
}
```

Because `id` was declared `{id:Nat}`, it is already a `Nat` here — no manual `Text` parsing, no failed conversions to guard against.

## Authorization

Add `@authorize` to require an authenticated caller before the page renders. Pass `role="..."` to require a specific role.

```razor
@page "/admin"
@authorize role="Admin"

<PageHeader>Admin</PageHeader>
```

> Full Internet Identity login over HTTP and role stores are **Roadmap**. `@authorize` is the directive you'll use; the backing identity and role infrastructure is still being built out.

## SEO Directives

MotoView renders real HTML on the server, so pages are crawlable and shareable out of the box. The SEO directives let you fill in the document head. Each takes a Motoko expression, so titles and descriptions can be computed from your data.

```razor
@page "/products/{id:Nat}"

@code {
  import Nat "mo:base/Nat";
  import Catalog "../Services/Catalog";

  let product = Catalog.get(id);
}

@title product.name # " — Acme Store"
@description "Buy " # product.name # " on Acme, deployed on the Internet Computer."
@canonical "https://acme.example/products/" # Nat.toText(id)

<PageHeader>@product.name</PageHeader>
```

The available head directives are:

- `@title EXPR` — sets the document `<title>`.
- `@description EXPR` — the meta description.
- `@canonical EXPR` — the canonical URL.
- `@meta` — additional meta tags.

For anything else you need in `<head>`, use the `@head` directive to emit raw markup.

```razor
@head {
  <meta property="og:type" content="product" />
}
```

## Putting It Together

A typical detail page combines a typed route, a data load in `@code`, computed SEO, and a template that handles the missing case.

```razor
@page "/products/{id:Nat}"
@layout Main

@code {
  import Nat "mo:base/Nat";
  import Catalog "../Services/Catalog";

  let product = Catalog.get(id);
}

@title @switch product { case (?p) p.name; case null "Not found" }
@description "Product details on Acme."

@switch product {
  case (?p) {
    <PageHeader>@p.name</PageHeader>
    <p>@(Nat.toText(p.price))</p>
  }
  case null {
    <Alert type="warning">That product doesn't exist.</Alert>
  }
}
```

## Building Your Routes

Compile your `.mview` files to Motoko, then deploy with `dfx`:

```bash
motoview build
motoview dev
```

`motoview dev` builds, deploys to your local replica, and watches for changes. Once it's running, open the route in your browser and the server-rendered HTML is there immediately.

Next, wire up interactivity with [Events](events.md), or compose your pages from reusable [Components](components.md).
