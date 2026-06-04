---
title: Events
section: Interactivity
slug: events
---

# Events

In MotoView, interactivity is server-driven. You attach an event to an element in your `.mview` markup, write the matching Motoko function in `@code`, and the framework handles everything between the click and the updated DOM. There is no event listener to register, no fetch call to write, no JSON to parse, and no state to reconcile by hand. You write Motoko; the WASM client and the runtime do the wiring.

## Attaching events

Bind a handler to an element with one of the event directives. The value is the name of a function defined in your `@code` block.

```razor
@page "/counter"

@code {
  var count : Nat = 0;

  func increment() { count += 1; };
  func reset() { count := 0; };
}

<p>Count: @count</p>

<Button kind="primary" @click="increment">Increment</Button>
<Button @click="reset">Reset</Button>
```

The supported directives are:

- `@click` — fires when the element is clicked.
- `@submit` — fires when a form is submitted (use this on `<form>`).
- `@input` — fires as the user types into an input.
- `@change` — fires when an input's value is committed (blur or selection change).

That is the whole surface. Every directive maps to a typed Motoko function, and the same dispatch path serves all of them.

## Passing arguments

Handlers can take arguments. Arguments are written in the markup and evaluated **server-side at render time**, then baked into the element so the client can forward them back unchanged.

```razor
@code {
  var items : [Text] = ["Helmet", "Gloves", "Jacket"];
  var selected : Text = "";

  func select(name : Text) { selected := name; };
  func removeAt(i : Nat) { /* drop item i */ };
}

<ul>
  @for item in items {
    <li>
      <Button @click="select(item)">@item</Button>
    </li>
  }
</ul>

<p>Selected: @selected</p>
```

Because arguments are resolved during render, the value you pass is whatever the expression evaluates to on the server in that iteration. The developer never serializes anything manually — you write `@click="select(item)"` and the typed `select(name : Text)` function receives the right value.

## How events work internally

The flow is the same for every directive. Nothing here requires application JavaScript.

1. **Render.** When the page renders, each event directive produces a `data-mv-handler` attribute identifying the handler, plus `data-mv-arg*` attributes carrying any arguments that were evaluated server-side.
2. **Capture.** In the browser, the WASM client (the "brain") observes the interaction through the thin JS glue (the "hands"). It reads the handler id and args off the element.
3. **POST.** The client sends a form-encoded `POST` to `/_motoview/event` with the handler id and arguments.
4. **Dispatch.** On the canister, that request is served by `http_request_update`, which decodes the payload and dispatches to your typed Motoko function — `increment`, `select(name)`, and so on.
5. **Batch.** The handler mutates state, the page re-renders, and the server returns a new batch immediately in the response. The batch carries a `batchId` (a hash of the rendered state) and the updated HTML.
6. **Swap.** The client interprets the batch and swaps the content inside `<div id="mv-root">`, preserving focus and scroll where it can.

Because the event response returns the new batch right away, the UI updates without waiting for the next poll. Adaptive polling then continues in the background to pick up state changed by other callers.

```bash
# Illustrative: the event POST the WASM client makes for you
curl -X POST https://<canister>.localhost/_motoview/event \
  -d "path=/counter&handler=<handlerId>"
```

You do not write this request — it is shown only to make the protocol concrete. See [Protocol](protocol.md) for the full batch shape and status values.

## Events and forms

For forms, pair `@submit` with `bind` on your inputs and add the `secure` attribute so the submission is signed and verified server-side:

```razor
<form @submit="send" secure>
  <InputText name="name" label="Name" bind="@model.name" required />
  <Button kind="primary">Send</Button>
</form>
```

Validation runs inside the handler, and failures come back in the batch for re-render. See [Forms & Validation](forms.md) for the full pattern and [Security](security.md) for how `secure` tokens work.

## What you do not write

- No `addEventListener`.
- No `fetch` / agent-js call.
- No argument serialization or DOM patching.

Define the function, attach the directive, and MotoView connects the click to your Motoko.
