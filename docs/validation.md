---
title: Validation
section: Interactivity
slug: validation
---

# Validation

Validation in MotoView runs where your data lives: inside your Motoko handlers, on the canister. There is no client-side validation library to keep in sync, no duplicated rules, and no JavaScript. You declare your rules once, and when they fail the server re-renders the page with the errors attached to the very batch the browser is already polling for.

If you have not read [Forms](forms.md) and [Events](events.md) yet, start there — validation is the natural next step once a `@submit` handler is receiving a bound model.

## The `validate` block

Inside a handler, call `validate` against your model. Each line is a field name, one or more rules, and an optional custom message.

```motoko
@code {
  var model = { name = ""; email = ""; price = 0 };

  public func send() : async () {
    validate model {
      name required "Name is required";
      email required email;
      price min 1 "Price must be at least 1";
    };

    // Reached only when every rule passes.
    await Store.save(model);
  };
}
```

When any rule fails, MotoView stops the handler, collects the failures, and returns a batch with status `validation-error`. Your `await Store.save(...)` never runs. The browser receives the batch, swaps `#mv-root`, and the page is now showing your errors — same render path as any other update.

## Rule syntax

Rules are written after the field name, separated by spaces. A trailing string literal is the custom message for that field.

```motoko
validate model {
  name     required "Name is required";
  email    required email;
  bio      minLength 10 "Tell us a little more";
  price    min 1 "Price must be at least 1";
}
```

Built-in rules:

| Rule | Checks |
| --- | --- |
| `required` | The field is present and not empty. |
| `email` | The value is a syntactically valid email address. |
| `min N` | A numeric field is at least `N`. |
| `minLength N` | A text field has at least `N` characters. |

When you omit the message, MotoView supplies a sensible default. When you provide one, it is used verbatim — so write it the way you want the user to read it.

## Showing all errors with `ValidationSummary`

Drop `<ValidationSummary />` anywhere in your template to render every current error as a list. It produces nothing when there are no errors, so it is safe to leave in place permanently.

```razor
@page "/products/new"

<form @submit="send" secure>
  <ValidationSummary />

  <InputText  name="name"  label="Name"  bind="@model.name" />
  <InputEmail name="email" label="Email" bind="@model.email" />
  <InputNumber name="price" label="Price" bind="@model.price" />

  <Button kind="primary">Save</Button>
</form>
```

## Per-field errors

Errors are also keyed by field, so you can place messages next to the input that produced them. The `InputText`, `InputEmail`, `InputNumber`, and `TextArea` components render their own field error automatically when one exists for their `name`, so the markup above already shows inline messages without extra wiring.

The bound values survive the round trip: when the batch re-renders, each input is repopulated from the model, so the user never loses what they typed while fixing the one field that failed.

## The re-render-with-errors flow

Validation rides the same protocol as every other interaction:

1. The user submits the form. The browser POSTs to `/_motoview/event`, handled by `http_request_update`.
2. Your handler runs `validate`. A rule fails.
3. MotoView halts the handler and builds a batch with status `validation-error`, carrying the error map and the freshly rendered HTML.
4. The response returns that batch immediately — no waiting for the next poll.
5. The WASM client swaps `#mv-root`. `<ValidationSummary />` and per-field messages appear; inputs keep their values.

Because the errors travel inside an ordinary batch, focus, scroll position, and in-progress input are preserved exactly as they are for any other update. See the [Protocol](protocol.md) page for the full batch shape and status values.

## Validation and `secure`

Use `validate` together with the `secure` form attribute. The signed token rejects tampered, expired, and replayed submissions before your handler runs; `validate` then enforces that the data itself is well-formed. The two are complementary — see [Forms](forms.md) for how `secure` mints and verifies its token.

```razor
<form @submit="send" secure>
  <ValidationSummary />
  <InputText name="name" label="Name" bind="@model.name" />
  <Button kind="primary">Save</Button>
</form>
```

With that, every rule lives in one place, runs on the canister, and the browser stays in sync through the same versioned batches it already speaks.
