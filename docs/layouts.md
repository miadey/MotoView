---
title: Layouts
section: The .mview Format
slug: layouts
---

# Layouts

Most pages share the same chrome: a `<head>` full of meta tags, a navigation bar, a footer, maybe an analytics include. Instead of repeating that markup in every page, you define it once in a **layout** and let your pages slot their content into it.

A layout is just a `.mview` file in `src/Layouts/`. Pages opt in with the `@layout` directive, and the layout decides where page content lands with `@yield`.

## A page that uses a layout

```razor
@page "/products"
@layout MainLayout
@title "Products"
@description "Browse the catalog"

<PageHeader title="Products" />

<Grid columns="3">
  @for p in products {
    <Card title=@p.name>@p.summary</Card>
  }
</Grid>
```

The page never mentions `<html>`, `<head>`, or `<body>`. It only describes its own content. The `@title` and `@description` values are passed up to the layout's head so the rendered HTML stays SEO-friendly. (For more on metadata directives, see [Pages & Routing](pages.md).)

## The MainLayout example

Here is a complete layout. Note the single `@yield`, which is where the page's content is rendered.

```razor
@code {
  // Layouts can run Motoko too — anything in scope is available below.
}

<!DOCTYPE html>
<html lang="en">
<head>
  <meta charset="utf-8" />
  <meta name="viewport" content="width=device-width, initial-scale=1" />
  <title>@title</title>
  <meta name="description" content=@description />
  <link rel="stylesheet" href="/motoview.css" />
  @section "head"
</head>
<body>
  <header class="site-nav">
    <a href="/" class="brand">MotoView Store</a>
    <nav>
      <a href="/products">Products</a>
      <a href="/cart">Cart</a>
    </nav>
  </header>

  <main id="mv-root">
    @yield
  </main>

  <footer class="site-footer">
    <p>Built with MotoView.</p>
  </footer>

  <script type="module" src="/motoview.js"></script>
  @section "scripts"
</body>
</html>
```

A few things to call out:

- `@title` and `@description` resolve to whatever the page declared with `@title` / `@description`. They are plain expressions, so you can supply a fallback in the layout.
- `@yield` is the single insertion point for the page's body. The page content lives inside `<div id="mv-root">` — this is the element the WASM client swaps when a new batch arrives, so keep it on the element that wraps `@yield`.
- `/motoview.css`, `/motoview.js`, and `/motoview.wasm` are served by the runtime. Including the stylesheet and module script is what wires up the browser client.

## Sections: `@section` and `@head`

A layout exposes named **sections** that pages can fill in. Two are conventional: `head` (for per-page `<link>`/`<meta>` tags) and `scripts` (for per-page script tags placed at the end of the body). You declare the insertion point in the layout with `@section "name"`:

```razor
  @section "head"
  ...
  @section "scripts"
```

A page contributes content to a section by defining it:

```razor
@page "/products/{id:Nat}"
@layout MainLayout
@title product.name

@section "head" {
  <link rel="canonical" href=@canonical />
  <meta property="og:title" content=@product.name />
}

<PageHeader title=@product.name />
<p>@product.description</p>

@section "scripts" {
  <!-- per-page markup that belongs at the end of <body> -->
}
```

If a page does not define a given section, the layout simply renders nothing there.

For the common case of adding to the document head from a page without naming a section, use `@head`:

```razor
@page "/about"
@layout MainLayout
@title "About"

@head {
  <meta name="robots" content="index,follow" />
}

<p>About us.</p>
```

`@head` is shorthand that targets the layout's head region — useful when you just want one or two extra tags and do not need a dedicated named slot.

## Layout inheritance

Layouts can nest. A layout may itself declare `@layout` to wrap inside a parent layout, with its own `@yield` providing the slot for the child. This lets you build, for example, a broad `MainLayout` (full HTML document) and a narrower `DashboardLayout` that adds a sidebar:

```razor
@layout MainLayout
@title "Dashboard"

<div class="dashboard">
  <aside class="sidebar">
    <a href="/dashboard">Overview</a>
    <a href="/dashboard/orders">Orders</a>
  </aside>
  <section class="dashboard-body">
    @yield
  </section>
</div>
```

A page then chooses `@layout DashboardLayout`, which renders inside `MainLayout`. Sections like `head` propagate up the chain, so a page can still contribute `<head>` tags even when wrapped two layouts deep.

## Building

Layouts compile alongside pages. The compiler resolves `@layout` references and the named sections at build time:

```bash
motoview build
```

No layout is selected by default — a page without `@layout` renders its markup as the full response. From here, wire up interactivity with [Events](events.md) and reusable markup with [Components](components.md).
