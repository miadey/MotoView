---
title: The .mview File
section: The .mview Format
slug: mview-files
---

# The .mview File

A `.mview` file is the heart of MotoView. It holds your **template markup** and your **Motoko code** side by side in a single file — the way a Blade or Razor view does, but the code is real, type-checked Motoko that runs inside your Internet Computer canister.

If you have written Blazor, Blade, or Phoenix LiveView, this will feel immediately familiar: the markup describes what to render, the code says how to compute it, and MotoView's compiler stitches the two together into a Motoko actor.

## Anatomy of a .mview file

Here is a small but complete page. It declares a route, sets the page title, holds some state, renders markup, and handles a click — all in one file.

```razor
@page "/counter"
@title "Counter"

@code {
    var count : Nat = 0;

    public func increment() {
        count += 1;
    };
}

<PageHeader title="Counter" />

<Card>
    <p>The count is @count.</p>
    <Button kind="primary" @click="increment">Add one</Button>
</Card>
```

Three things are happening:

- **Directives** (lines starting with `@`) configure the page — its route, metadata, layout, and embedded code.
- The **`@code` block** holds plain Motoko: state (`var count`) and handlers (`public func increment`).
- The **template** below is HTML plus MotoView tags, components, and inline output like `@count`.

When the browser clicks the button, the WASM client forwards the event to the canister, `increment()` runs, the page re-renders, and the new state flows back as a versioned batch. See [Events](events.md) for how `@click` is wired end to end.

## The directive list

Directives always begin with `@` and configure the page or its structure.

| Directive | What it does |
| --- | --- |
| `@page "/path"` | Declares the route. Supports params: `"/products/{id}"` and typed `"/orders/{id:Nat}"`. |
| `@layout NAME` | Renders this page inside the named layout. |
| `@title EXPR` | Sets the document title (used for SEO). |
| `@description EXPR` | Sets the meta description. |
| `@canonical EXPR` | Sets the canonical URL. |
| `@meta` | Emits additional meta tags. |
| `@authorize` | Requires an authenticated caller; `@authorize role="Admin"` requires a role. |
| `@section "name" { ... }` | Defines content a layout pulls in by name. |
| `@yield` | In a layout, the slot where page content is rendered. |
| `@head` | Injects markup into the document `<head>`. |
| `@slot "name"` | A named insertion point a component can fill. |
| `@code { ... }` | Embeds Motoko: state, handlers, and helpers. |
| `@style { ... }` | Scoped CSS for this view. |
| `@theme { tokens }` | Declares theme tokens. |
| `@if EXPR { } else { }` | Conditional rendering. |
| `@for X in EXPR { }` | Loops over a collection. |
| `@switch EXPR { case #Variant { } }` | Matches a Motoko variant. |
| `@count` / `@user.name` / `@(expr)` | Inline output of a value or expression. |
| `@effect Focus("#x")` / `ScrollTo` / `Toast` | Requests a client-side effect after render. |
| `@animate` | Marks an element for animation. |

For the full event set (`@click`, `@submit`, `@input`, `@change`) and how handler arguments are baked in, see [Events](events.md).

## How template and @code combine

The `@code` block and the template are two views of the same actor. Anything you declare in `@code` is in scope in the template.

```razor
@code {
    var products : [Product] = Store.all();

    func total() : Nat {
        Array.size(products);
    };
}

<PageHeader title="Products" />
<p>We have @total() items.</p>

@for p in products {
    <Card title="@p.name">
        <p>@(p.price) credits</p>
    </Card>
}
```

`products` and `total()` are ordinary Motoko members, used directly as `@total()` and looped over with `@for`. The compiler generates a Motoko actor where the template becomes a render function and the `@code` members become actor state and methods — so rendering is a query over your state, and events are updates to it.

Because everything is Motoko, your validation, types, and business logic live in one place. There is no JavaScript layer to keep in sync.

## Where files live

Pages, layouts, and components each have a home:

```bash
src/Pages/Counter.mview      # @page routes
src/Layouts/Main.mview       # @layout targets
src/Components/Card.mview     # reusable components
```

Run the compiler to turn them into Motoko:

```bash
motoview build
```

Next, read about [Events](events.md) to handle user input, or [Components](components.md) to build reusable pieces.
