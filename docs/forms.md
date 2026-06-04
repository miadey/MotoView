---
title: Forms
section: Interactivity
slug: forms
---

# Forms

Forms in MotoView are plain HTML wired straight to typed Motoko handlers. You bind inputs to a model, submit to a server-side function, and validate without writing a single line of frontend JavaScript. The browser synchronizes the result through the same batch protocol that powers every other [event](events.md).

## Building a form

A form declares the handler that runs on submit and binds each input to a field on a `@code` model. Use `bind="@model.field"` to connect an input to state — MotoView renders the current value into the input and routes changes back into your model.

```razor
@page "/contact"
@title "Contact us"

@code {
  type Message = { name : Text; email : Text; body : Text };
  var model : Message = { name = ""; email = ""; body = "" };
  var sent : Bool = false;

  func send() : async () {
    validate model {
      name required "Please tell us your name";
      email required email "A valid email is required";
      body required "Message can't be empty";
    };
    // persist, notify, etc.
    sent := true;
  };
}

@if sent {
  <Alert type="success">Thanks — we'll be in touch.</Alert>
} else {
  <form @submit="send" secure>
    <ValidationSummary />
    <InputText  name="name"  label="Name"  bind="@model.name" required />
    <InputEmail name="email" label="Email" bind="@model.email" required />
    <TextArea   name="body"  label="Message" bind="@model.body" required minLength={10} />
    <Button kind="primary">Send message</Button>
  </form>
}
```

That is the whole contact form: markup, model, and handler in one `.mview` file. On submit the client POSTs the field values to `/_motoview/event`, the server hydrates `model`, runs `send`, and returns a fresh batch. If everything passes, the success branch renders; the DOM swaps without a full page reload.

## bind="@model.x"

`bind` is two-way. On render, MotoView writes `model.x` into the input's value. On submit, the posted form fields are decoded back onto the model before your handler runs, so inside `send` you read `model.name`, `model.email`, and so on directly. The built-in input components (`InputText`, `InputEmail`, `InputNumber`, `TextArea`) accept `bind` alongside `name`, `label`, `required`, and `minLength`, so you avoid hand-writing `<input>` markup and class strings. Prefer them over raw elements.

## Validation

Call `validate` inside the handler with one rule line per field:

```motoko
validate model {
  name  required "Name is required";
  email required email;
  price min 1 "Price must be at least 1";
};
```

When a rule fails, the errors are returned in the batch and the page re-renders with them in place — no exception, no lost input. Surface them with `<ValidationSummary />` for a grouped list, and per-field messages appear next to the bound inputs automatically. Because validation lives in your Motoko handler, there is no duplicated client/server logic to keep in sync.

## The `secure` attribute

Add `secure` to a form and MotoView mints a signed token when the form is rendered:

```razor
<form @submit="send" secure>
```

The token is an **HMAC-SHA256** MAC binding the request path, the handler, the caller principal, a nonce, an expiry, and a hash of the field schema. On submit the server re-derives the MAC from the incoming request and compares. This protects against:

- **Tampering** — changing the handler, path, or submitted field set breaks the schema/handler binding, so the MAC won't match.
- **Cross-context replay** — the token is bound to the caller principal and the specific form, so a token minted for one user or page can't be reused elsewhere.
- **Replay & stale submits** — the nonce and expiry cause repeated or late submissions to be rejected.

Mismatched, expired, and replayed tokens are all refused before your handler runs. SHA-256 and HMAC are implemented in Motoko and verified against the standard test vectors, so the guarantee is real, not decorative.

Use `secure` on any form that mutates state or accepts untrusted input. For forms that only read or filter, you can omit it.

## Next steps

- Learn how handler arguments are evaluated in [Events](events.md).
- Compose richer inputs with the built-in [Components](components.md).
