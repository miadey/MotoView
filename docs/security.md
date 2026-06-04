---
title: Security
section: Architecture
slug: security
---

# Security

MotoView pushes rendering, validation, and authorization into the canister. Because there is no application JavaScript and no separate API layer, there is also no second place for security rules to drift out of sync. The browser is a renderer; the canister is the source of truth.

This page covers secure forms, the HMAC token MotoView mints for them, replay and principal binding, the threat model, and what is still on the roadmap.

## Secure forms

Mark any form that mutates state with the `secure` attribute. Inputs bind to your model with `bind`, and the handler runs server-side.

```razor
<form @submit="send" secure>
  <InputText  name="name"  label="Name"  bind="@model.name"  required />
  <InputEmail name="email" label="Email" bind="@model.email" required />
  <ValidationSummary />
  <Button kind="primary">Send</Button>
</form>
```

When the form is rendered, MotoView mints a signed token and embeds it in the form. The token is not a session cookie and it is not a CSRF nonce alone — it is a MAC over the exact request that is allowed to follow.

## The token

The token is an **HMAC-SHA256** value computed by the canister over a tuple that binds the submission to a specific context:

- **path** — the page the form was rendered on
- **handler** — the event handler that is permitted to run (`send`)
- **principal** — the caller's IC principal at render time
- **nonce** — a single-use random value
- **expiry** — an absolute deadline after which the token is dead
- **schema hash** — a hash of the form's field schema (names and constraints)

SHA-256 and HMAC are implemented in Motoko and verified against standard test vectors, so the MAC is the same primitive a server would compute anywhere else — there is no JavaScript crypto in the trust path.

On submit, the [event](events.md) is delivered as a `POST` to `/_motoview/event`, served by `http_request_update`. The canister re-derives the MAC from the live request — the actual path, the resolved handler, the *current* caller principal, and the field schema it expects — and compares it to the token. Any mismatch is rejected before the handler runs.

```motoko
// Conceptual shape of what the canister checks before dispatch.
let expected = HMAC.sha256(secretKey, encode(path, handler, caller, nonce, expiry, schemaHash));
if (token != expected)          return reject("bad token");
if (now() > expiry)             return reject("expired");
if (nonceStore.seen(nonce))     return reject("replay");
```

### Replay protection

Each token carries a single-use **nonce** and an **expiry**. A captured submission cannot be replayed: once a nonce is consumed it is rejected, and once the expiry passes the token is worthless even if the nonce were never seen. This closes the window on both double-submits and captured-then-resent requests.

### Principal binding

The token is bound to the **caller principal** present when the form was rendered. The canister compares that to the principal making the submit call. A token minted for one caller cannot be lifted and submitted by another — the re-derived MAC will not match. This means authorization is not advisory: it is cryptographically tied to who is allowed to act.

### Schema binding

The **schema hash** covers the form's declared fields and their constraints. If an attacker tampers with field names or adds fields the handler never declared, the re-derived hash diverges and the request is rejected. Validation still runs server-side regardless — see [Forms & Validation](forms.md) — but schema binding stops malformed submissions before that point.

## Threat model

What MotoView defends against today:

- **CSRF / cross-origin submits** — the token cannot be forged without the canister's secret key.
- **Replay** — single-use nonces plus expiry.
- **Principal substitution** — the MAC is bound to the caller principal.
- **Field tampering** — the schema hash binds the allowed field set.
- **Client-side validation bypass** — there is no client-side-only validation to bypass; rules live in the handler.

What it does **not** attempt to solve: it is not a transport layer (IC boundary nodes and `http_request_update` handle that), and it does not by itself encrypt stored state. Tokens authenticate *requests*; they do not authenticate *users* across sessions — that is the job of authentication, below.

## Authorization

Pages and handlers can require authorization with `@authorize`, optionally scoped to a role:

```razor
@page "/admin"
@authorize role="Admin"
```

Today `@authorize` gates on the caller principal. Full role stores are on the roadmap.

## Roadmap

These are planned and **not yet implemented**:

- **Internet Identity login over HTTP** — full II authentication flow so handlers can identify users, not just principals.
- **Role stores** — durable role assignment backing `@authorize role="..."`.
- **vetKeys encrypted state** — encrypting per-user state at rest using IC vetKeys.
- **Certified query rendering** — response certification for cacheable public pages (today every request upgrades to `http_request_update`).

Until those land, build on what is verified: secure forms, HMAC token binding, replay and principal protection, and server-side validation.
