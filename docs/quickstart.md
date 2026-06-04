---
title: Quickstart
section: Getting Started
slug: quickstart
---

# Quickstart

This guide takes you from an empty folder to a running, interactive counter in a real Internet Computer canister. You will scaffold a project, tour what was created, deploy to a local replica, click a button in the browser, and then edit a handler and watch the change take effect.

If you have not installed the prerequisites and the `motoview` compiler yet, start with [Installation](installation.md). You need `dfx`, `rustup` with the `wasm32-unknown-unknown` target, `wasm-opt`, and the `motoview` binary on your `PATH`.

## Create a project

Scaffold a new app with `motoview new`:

```bash
motoview new hello-counter
cd hello-counter
```

This generates a complete project: pages, a layout, the runtime dependency, and the config files `dfx` needs.

## Project tour

Here is what `motoview new` lays down:

```
hello-counter/
├── src/
│   ├── Pages/        # .mview pages — one file per route
│   ├── Layouts/      # shared shells (@yield, @section)
│   ├── Components/   # reusable .mview components
│   ├── Services/     # plain Motoko (*.mo) business logic
│   └── Models/       # plain Motoko (*.mo) record/variant types
├── motoview.json     # MotoView project config
├── dfx.json          # canister + replica config for dfx
└── mops.toml         # Motoko package manifest (pulls in "motoview")
```

A page is a single `.mview` file holding template markup plus a `@code` block of Motoko. There is no separate JavaScript project, no React, and no `agent-js` glue to wire up — the only browser code is the MotoView WASM client, which the runtime serves for you.

## Run it

Start the dev loop:

```bash
motoview dev
```

`motoview dev` compiles your `.mview` files to Motoko, deploys to a local `dfx` replica, and watches for changes. When it finishes, it prints the canister URL. Open it in your browser:

```
http://<canister-id>.localhost:4943/
```

The first load is plain server-rendered HTML — the page content lives in a `<div id="mv-root">`. That is what makes MotoView pages SEO-friendly: the markup arrives complete, then the WASM client takes over and keeps it in sync.

## The Counter page

Open `src/Pages/Counter.mview`. It is the whole counter — template and logic in one file:

```razor
@page "/"
@title "Counter"

@code {
    stable var count : Nat = 0;

    func increment() {
        count += 1;
    };

    func decrement() {
        if (count > 0) { count -= 1; };
    };
}

<Card title="Counter">
    <PageHeader>You clicked @count times.</PageHeader>

    <Button kind="primary" @click="increment">Increment</Button>
    <Button kind="secondary" @click="decrement">Decrement</Button>
</Card>
```

A few things are worth pointing out:

- `@count` outputs the current value inline. Any `@expr` or `@(expr)` is evaluated server-side at render time.
- `@click="increment"` binds the click to the typed Motoko function `increment`. You never write event-wiring code.
- `stable var count` persists across calls — refresh the page and your count is still there.

## See it update

Click **Increment** in the browser. Under the hood, the WASM client posts the event to `/_motoview/event`; the canister runs `increment`, re-renders, and returns a new versioned batch; the client swaps the changed DOM. The click-to-update round trip happens immediately — no waiting for the next poll.

Adaptive polling also keeps the page fresh on its own: it runs hot (~350ms) for a few seconds after you interact, settles to warm (~2.5s) while the tab is visible, and backs off to cold (~15s) when idle. So if `count` changes from another caller, the page picks it up. For the full state machine and protocol, see [Architecture](protocol.md).

## Edit a handler, watch it change

With `motoview dev` still running, change `increment` to step by two:

```razor
    func increment() {
        count += 2;
    };
```

Save the file. `motoview dev` recompiles and redeploys automatically. Refresh the browser, click **Increment**, and the count now jumps by two.

That is the core loop: write Motoko, the markup renders on the server, events dispatch to typed functions, and the browser stays in sync through versioned batches — all without a line of application JavaScript.

## Next steps

- [Pages and Routing](pages-and-routing.md) — `@page`, typed route params like `/orders/{id:Nat}`, and layouts.
- [Events](events.md) — handler arguments, `@input`, `@change`, and `@submit`.
- [Forms and Validation](forms.md) — `secure` forms, `bind="@model.field"`, and `validate`.
- [Components](components.md) — the built-in semantic components and building your own.
