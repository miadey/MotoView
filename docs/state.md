---
title: State
section: Interactivity
slug: state
---

# State

State is what makes a MotoView page feel alive. A counter that climbs, a form that remembers what you typed, a list that grows — all of it is ordinary Motoko state living inside your canister. There is no client store to sync, no JavaScript model to mirror. You declare state in `@code`, your handlers mutate it, and MotoView re-renders.

Rendering is a query over your state; events are updates to it. When state changes, the new render is hashed into a versioned batch and the WASM client swaps the DOM. See [Events](events.md) for how an interaction flows from the browser back to your Motoko.

## `var` vs `stable var`

State is declared in the `@code` block like any Motoko field.

```razor
@page "/counter"

@code {
    var count : Nat = 0;

    public func increment() {
        count += 1;
    };
}

<p>The count is @count.</p>
<Button kind="primary" @click="increment">Add one</Button>
```

A plain `var` is mutable canister state. It is part of the actor the compiler generates, so it persists between requests for as long as the canister is running — every render and every event sees the same `count`.

A `stable var` is the same, but it also survives a canister **upgrade**. In Motoko, `stable` variables are preserved across `dfx canister install --mode upgrade`; plain `var` state is re-initialized.

```razor
@code {
    stable var count : Nat = 0;
}
```

> **MVP vs Roadmap.** Plain `var` state is **verified**: it persists across calls within a running canister, which is exactly what the counter example demonstrated on a local replica. Reliable upgrade-stable page state — the full `stable var` story for generated pages, including stable migration of richer types — is **Roadmap**. For now, treat `var` as your working model and keep durable data in a [Service](services.md) you control.

## The page state lifecycle

A page goes through the same shape on every request:

1. **Resolve** the route and any params (see [Pages & Routing](pages-and-routing.md)).
2. **Initialize** the `@code` block. `let` bindings and `var` initializers run, and `onLoad(ctx)` is called if you defined it.
3. **Render** the template against that state, producing HTML and a `batchId`.
4. On an **event**, the matching handler runs, mutates state, and the page re-renders into a fresh batch.
5. The client compares `batchId`s — `unchanged` batches are skipped, `changed` batches swap the DOM.

Because step 3 is a pure query over your state, the same state always produces the same HTML and the same `batchId`.

## Loading data with `onLoad(ctx)`

For anything that needs to run before the first render — fetching a record, reading the caller, preparing a model — define `onLoad`. It receives the request context and runs during initialization.

```razor
@page "/orders/{id:Nat}"

@code {
    import Orders "../Services/Orders";

    var order : ?Orders.Order = null;

    public func onLoad(ctx : Context) {
        order := Orders.find(id);
    };
}

@switch order {
    case (?o) { <PageHeader>Order #@Nat.toText(o.id)</PageHeader> }
    case null { <Alert type="warning">Order not found.</Alert> }
}
```

Keep `onLoad` focused on loading. Mutations belong in event handlers, so the page stays cheap to re-render on every poll.

## Reading context: caller, params, query

`onLoad` (and your handlers) can read the request **context** — who is calling and what they asked for.

- **`ctx.caller`** — the calling `Principal`. Use it to scope state to a user or to gate behavior.
- **Route params** — declared on `@page` and available directly by name, already typed. With `{id:Nat}`, `id` is a `Nat`.
- **Query string** — read named query values from the context.

```razor
@page "/dashboard"
@authorize

@code {
    import Notes "../Services/Notes";

    var mine : [Notes.Note] = [];
    var filter : Text = "all";

    public func onLoad(ctx : Context) {
        mine := Notes.forOwner(ctx.caller);
        filter := ctx.query("filter");
    };
}

<PageHeader>Your notes (@filter)</PageHeader>
```

Here `ctx.caller` scopes the data to the signed-in principal, and `ctx.query("filter")` reads `?filter=...` from the URL. Route params are simply in scope, so a `{id:Nat}` segment is used as `id` with no parsing.

> Authenticated callers depend on `@authorize`. The directive works today; full Internet Identity login over HTTP and role stores are **Roadmap**, as noted in [Pages & Routing](pages-and-routing.md).

## Putting it together

Declare state, load it in `onLoad`, mutate it in handlers, and let MotoView re-render:

```bash
motoview build
motoview dev
```

Next, wire interactions to your state with [Events](events.md), or move durable data into a [Service](services.md).
