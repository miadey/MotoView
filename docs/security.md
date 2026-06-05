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

`@authorize` (no role) requires an authenticated caller — a principal resolved
from the [Internet Identity session](security.md). `@authorize role="Admin"`
additionally requires the caller to **hold that role** in the runtime's role
store; otherwise the page redirects to `/` and its content never renders.

### The role store

Roles are assigned per principal and persist across upgrades. Manage them from
any handler through the request context:

```razor
@code {
  // First-come bootstrap: grant "Admin" to the caller iff nobody holds it yet.
  func claimAdmin(ctx : Context) : async () {
    ignore ctx.claimRole("Admin");
  };

  // An existing admin grants/revokes others.
  func makeEditor(ctx : Context) : async () {
    if (ctx.hasRole(ctx.caller, "Admin")) {
      ctx.grantRole(somePrincipal, "Editor");
    };
  };
}
```

The context exposes `hasRole(who, role)`, `callerRoles()`, `grantRole(who, role)`,
`revokeRole(who, role)`, and `claimRole(role)` (claims for the caller, first-come).
Reads are safe anywhere; mutations should run in event handlers (updates). The
store is backed by a `stable var` in the generated actor, so role assignments
survive `dfx deploy --mode upgrade`.

## vetKeys (threshold encryption)

Every MotoView actor exposes two vetKeys endpoints for identity-based encryption
(IBE), so an app can give each user encrypted state only they can read:

- `mvVetkdPublicKey()` → the 96-byte BLS12-381 master public key for the app's
  context. Free; reveals no secret.
- `mvVetkdDeriveKey(transportKey)` → a 192-byte vetKey for the **caller's
  principal**, encrypted to a client-generated transport key. The canister
  attaches the cycles and calls the management canister; it never sees the
  plaintext key.

The threshold key is derived across the subnet (no single node holds it). The
BLS unwrap and the IBE encrypt/decrypt run in the **client** (the Rust brain via
[`ic-vetkeys`](https://crates.io/crates/ic-vetkeys)), never in the canister.
Because the derivation `input` is the caller's principal, each user gets their
own key — encrypt a secret to a principal, store the ciphertext, and only that
principal can recover it.

This canister foundation is verified end to end against a real local replica with
the real client crypto — see [`tools/vetkeys-roundtrip`](../tools/vetkeys-roundtrip)
(`ROUND_TRIP_OK`: derive → unwrap → IBE encrypt/decrypt recovers the plaintext).
Local dfx uses the `dfx_test_key`; switch the key name to `key_1` for mainnet
(`runtime/src/VetKeys.mo`).

> **Next:** shipping the same `ic-vetkeys` crypto *inside the browser brain* (so a
> page encrypts/decrypts with no external tool) is the in-progress step. It is
> opt-in: the crypto adds ~300 KB to the wasm, so apps that don't use vetKeys
> keep the lean ~76 KB brain.

## Roadmap

These are planned and **not yet implemented**:

- **In-browser vetKeys brain** — the client-side IBE (above) wired into the
  shipped brain + an example app; the canister endpoints and crypto are proven,
  the browser integration is the remaining work.
- **Role hierarchies / wildcard scopes** — today roles are flat string grants.

Until those land, build on what is verified: secure forms, HMAC token binding, replay and principal protection, and server-side validation.
