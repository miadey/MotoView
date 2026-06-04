---
title: Why MotoView
section: Prologue
slug: why-motoview
---

# Why MotoView

Building an interactive app on the Internet Computer usually means building two apps.

You write a Motoko canister for your data, your authorization, and your business rules. Then you write a *second* application — a React or Svelte frontend — that talks to the canister through agent-js. You wire up Candid bindings, manage a build pipeline of npm packages and bundlers, and hand-write `fetch`-style calls into the replica. And because the browser cannot be trusted, you write your validation rules twice: once in TypeScript to keep the form snappy, and once in Motoko because the TypeScript copy is only a suggestion.

The result is a split architecture with a seam running right down the middle:

- **Two languages, two mental models.** Motoko on one side, JavaScript on the other.
- **agent-js glue** to translate every interaction across the wire.
- **Duplicated validation** that drifts the moment someone forgets to update both copies.
- **A JavaScript build chain** — npm, a bundler, a dozen transitive dependencies — that has nothing to do with your product.

Every feature you ship has to cross that seam. That is where the bugs live.

## How MotoView collapses it

MotoView removes the seam. You write `.mview` files — template markup and Motoko in a single file — and the `motoview` compiler turns them into a Motoko actor that you deploy with `dfx`. There is no application JavaScript, no React, no agent-js, and no duplicated validation. Rendering is a query, events are updates, and the browser stays in sync through versioned UI batches.

Here is a counter — the example we deployed to a local replica and clicked through in a real browser:

```razor
@page "/"
@title "Counter"

<Card title="Counter">
  <p>Count: @count</p>
  <Button kind="primary" @click="increment">+1</Button>
</Card>

@code {
  var count : Nat = 0;

  func increment() {
    count += 1;
  };
}
```

Clicking the button posts an event to the canister, which dispatches to your typed Motoko `increment` function, recomputes the page, and returns a new batch. The client swaps the DOM. No `useState`, no API route, no Candid binding written by hand. State lives in the canister and persists across calls. See [Events](events.md) for how handlers and arguments work.

Validation happens once, in Motoko, where it is enforced:

```razor
<form @submit="send" secure>
  <InputText name="name" label="Name" bind="@model.name" />
  <InputEmail name="email" label="Email" bind="@model.email" />
  <ValidationSummary />
  <Button kind="primary">Send</Button>
</form>

@code {
  func send() {
    validate model {
      name required "Name is required";
      email required email;
    };
    // ...persist the message
  };
}
```

The `secure` attribute mints a signed token (HMAC-SHA256) binding the path, handler, caller principal, nonce, expiry, and field-schema hash; the server re-derives the MAC on submit and rejects tampering, expired tokens, and replays. There is no second validation layer to keep in sync because there is no second language. See [Forms & Validation](forms.md) and [Security](security.md).

## Design philosophy

**Readable templates over class-name soup.** A template should read like the page it produces. MotoView ships semantic components — `Button`, `Card`, `Alert`, `Table`, `InputText`, `ValidationSummary` — so your markup says what it *means*, not which forty utility classes happen to produce the look:

```razor
<Button kind="primary" size="lg">Save</Button>
```

not a wall of utility classes you have to decode every time you read it.

**No `.map()` JSX.** Control flow is markup, not a JavaScript expression smuggled into your view. You loop and branch with directives that read top to bottom:

```razor
@for product in products {
  <Card title="@product.name">
    <p>@product.price</p>
  </Card>
}
```

If you have written Blade, Razor, or LiveView, this will feel familiar — the difference is that the language underneath is Motoko, running in your canister.

**No JavaScript build chain.** The only browser code is a small Rust-to-WebAssembly client (the polling state machine, the protocol, batch interpretation) plus a hand-written JS glue for the unavoidable DOM, fetch, and timer primitives. No npm, no bundler. Your toolchain is `dfx`, `rustup`, and the `motoview` compiler — and nothing else gets between you and your app.

One language. One source of truth for validation. One build. Ready to write some Motoko? Start with [Getting Started](quickstart.md).
