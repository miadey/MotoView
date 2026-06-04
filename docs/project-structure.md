---
title: Project Structure
section: Getting Started
slug: project-structure
---

# Project Structure

When you scaffold an app with `motoview new`, you get a layout that maps cleanly onto how MotoView thinks about an application: templates that render, code that runs on the canister, and configuration that ties it all to ICP. Everything is Motoko-native — there is no Node, no npm, and no JavaScript build step to manage.

```bash
motoview new shop
```

```
shop/
├── motoview.json
├── dfx.json
├── mops.toml
└── src/
    ├── Pages/          # routable .mview templates
    ├── Layouts/        # shared page shells
    ├── Components/      # reusable .mview components
    ├── Services/        # plain Motoko (*.mo)
    ├── Models/          # plain Motoko types (*.mo)
    └── Generated/       # compiler output (do not edit)
```

## src/Pages

Pages are `.mview` files that own a route. A page declares its URL with `@page`, optionally an SEO title and description, and contains template markup plus a `@code` block of Motoko.

```razor
@page "/products/{id:Nat}"
@layout Main
@title "Product " # Nat.toText(id)

<PageHeader title=@product.name />
<p>@product.summary</p>

@code {
  var product = Catalog.get(id);
}
```

Routes can be static (`@page "/about"`), dynamic (`@page "/products/{id}"`), or typed (`@page "/orders/{id:Nat}"`). See [Routing](routing.md) for the full path syntax.

## src/Layouts

Layouts are the shared shell around your pages — the `<html>`, navigation, and footer that you do not want to repeat. A layout marks where the page drops in with `@yield`, and can expose named regions with `@section`.

```razor
@code { }

<!doctype html>
<html>
  <head>@head</head>
  <body>
    <nav>...</nav>
    <main>@yield</main>
  </body>
</html>
```

A page opts in with `@layout Main`. Pages without a `@layout` directive render standalone.

## src/Components

Capitalized tags in your templates are components. App components live in `src/Components/*.mview` and declare typed parameters with `param`.

```razor
param title : Text
param featured : Bool = false

<Card title=@title>
  @if featured { <Badge type="success">Featured</Badge> }
</Card>
```

```razor
<ProductCard title=@product.name featured=true />
```

Alongside your own components, MotoView ships built-in semantic components — `Button`, `Card`, `Alert`, `Badge`, `InputText`, `InputEmail`, `InputNumber`, `TextArea`, `ValidationSummary`, `Table`, `PageHeader`, and `Grid` — so you can write `<Button kind="primary">Save</Button>` instead of long class strings. See [Components](components.md).

## src/Services and src/Models

Not everything belongs in a template. `src/Services/*.mo` and `src/Models/*.mo` are plain Motoko files that you import from your `@code` blocks.

- **Models** hold your types and records — the shapes your pages bind to and your forms validate against.
- **Services** hold business logic, stable state, and the functions your pages call.

```motoko
// src/Models/Product.mo
public type Product = {
  id : Nat;
  name : Text;
  summary : Text;
};
```

```motoko
// src/Services/Catalog.mo
import Product "../Models/Product";

module {
  public func get(id : Nat) : Product.Product { ... };
}
```

## src/Generated

`motoview build` parses every `.mview` file and emits a Motoko actor. That generated code lands under `src/Generated/` (with an entry actor such as `src/main.mo`). Treat this directory as build output — do not edit it by hand, and let it regenerate on every build. Your hand-written code lives in Services and Models; the compiler wires it into the actor.

## motoview.json

The project config. It declares your app metadata and where the compiler should look and write.

```json
{
  "name": "shop",
  "version": "0.1.0",
  "src": "src",
  "out": "src/Generated"
}
```

## dfx.json

The standard DFINITY SDK manifest. It defines the canister, points dfx at the generated `main.mo`, and adds the runtime as a package argument.

```json
{
  "canisters": {
    "shop": {
      "type": "motoko",
      "main": "src/main.mo",
      "args": "--package motoview ../../runtime/src"
    }
  }
}
```

## mops.toml

MotoView ships its runtime as the [mops](https://mops.one) package `motoview`. Declare it here so the runtime that serves HTTP from your canister is on the dependency path.

```toml
[dependencies]
motoview = "..."
```

With the structure in place, run `motoview dev` to build, deploy to a local replica, and watch for changes. Next, head to [Events](events.md) to wire up `@click` and `@submit`.
