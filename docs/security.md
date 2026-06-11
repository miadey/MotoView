---
title: Security
section: Security
slug: security
---

# Security

MotoView is **secure by construction**. Rendering, validation, and authorization all live inside the canister, there is **no application JavaScript** and **no separate API layer**, and the responses the browser receives are **certified by the chain**. There is simply no second place for security rules to drift out of sync: the browser is a renderer, the canister is the source of truth.

This page walks the whole posture — the attack surface, certified responses, secure forms, authentication, authorization and roles, and zero-trust encryption — and ends with a threat-to-defense table.

## The attack surface (what *isn't* there)

A lot of a web app's security problems come from things MotoView doesn't have:

- **No application JavaScript.** You don't ship app JS, so there's no app-level XSS sink, no `eval`, no dynamic code, no place to leak a key. The only JavaScript on the page is MotoView's small, audited framework glue (DOM sync, fetch, focus/scroll preservation) — it carries no business logic and is identical for every app.
- **No npm / build-time supply chain.** No `node_modules`, no transitive dependency tree, no bundler plugins running arbitrary code at build time. The client "brain" is Rust → WASM, compiled reproducibly.
- **No separate API to guard.** There's no REST/GraphQL layer that can disagree with the UI about who may do what. Handlers are typed Motoko functions in the same actor that renders the page.
- **Output is escaped by default.** `@value` interpolation HTML-escapes; raw HTML requires the explicit, opt-in `@raw(...)`. So untrusted data can't become markup by accident.

## Certified responses (integrity)

What the browser renders is **provably what the canister produced**:

- Static framework assets and pages you mark `@cacheable` are served as **certified queries** (HTTP response certification v2). The IC boundary verifies the canister's signature over the response bytes against the subnet's certified state — a boundary node or a man-in-the-middle cannot tamper with them undetected.
- Every dynamic page and every mutation goes through **`http_request_update`** — an update call validated by subnet **consensus**, so it is always fresh and agreed-upon, never a stale or forged read.

In short: certified for the static, consensus for the dynamic. There is no "trust the CDN" step.

## Secure forms

Mark any form that mutates state with `secure`. Inputs `bind` to your model; the handler runs server-side.

```razor
<form @submit="send" secure>
  <InputText  name="name"  label="Name"  bind="@model.name"  required />
  <InputEmail name="email" label="Email" bind="@model.email" required />
  <ValidationSummary />
  <Button appearance="primary">Send</Button>
</form>
```

When the form renders, MotoView mints a signed token and embeds it. It is **not** a bare CSRF nonce and **not** a session cookie — it is a MAC over the exact request that is allowed to follow.

### The token

An **HMAC-SHA256** value the canister computes over a tuple that binds the submission to a specific context:

- **path** — the page the form was rendered on
- **handler** — the one event handler permitted to run (`send`)
- **principal** — the caller's IC principal at render time
- **nonce** — a single-use random value
- **expiry** — an absolute deadline after which the token is dead
- **schema hash** — a hash of the form's field schema (names + constraints)

SHA-256 and HMAC are implemented in Motoko and verified against standard test vectors — there is no JavaScript crypto in the trust path. On submit, the event is delivered as a `POST` to `/_motoview/event` (served by `http_request_update`); the canister re-derives the MAC from the *live* request and rejects any mismatch before the handler runs.

```motoko
let expected = HMAC.sha256(secretKey, encode(path, handler, caller, nonce, expiry, schemaHash));
if (token != expected)      return reject("bad token");
if (now() > expiry)         return reject("expired");
if (nonceStore.seen(nonce)) return reject("replay");
```

This single token gives you four protections at once:

- **CSRF / cross-origin submits** — it can't be forged without the canister's secret key (minted from `raw_rand` at first boot; never leaves the canister).
- **Replay** — single-use nonce + expiry: a captured submission can't be resent.
- **Principal substitution** — bound to the caller principal at render time; a token minted for one caller won't verify for another. Authorization is *cryptographic*, not advisory.
- **Field tampering** — the schema hash binds the allowed field set; added or renamed fields diverge the hash and are rejected.

And there's no client-side-only validation to bypass — the `validate { … }` rules run in the handler. See [Forms & Validation](forms.md).

**Verification is mandatory, not opt-in.** The compiler records which handlers are bound to a `secure` form and bakes that set into the page. On dispatch the server **requires** a valid token for any of those handlers — an attacker cannot skip the check by omitting the request's `__mv_secure` flag. A request to a secure handler with a missing or invalid token is rejected before the handler runs.

## Authentication — Internet Identity

Sign-in uses **Internet Identity** (passkeys / WebAuthn) — **no passwords**, hand-rolled with no npm or `agent-js`:

- The browser makes **one** authenticated update call the IC has cryptographically verified; the canister records the real signed-in principal.
- The canister then mints a short, HMAC-signed **httpOnly session cookie**. Because it's `httpOnly`, page JavaScript cannot read or steal it; `ctx.caller` resolves from it on every later request.
- The login nonce is **server-generated and browser-bound** (an httpOnly `mv_login` cookie), which prevents login-CSRF and nonce theft, and the II `postMessage` flow is pinned to the exact II origin.
- Sessions are **revocable per principal** (an epoch bump invalidates outstanding cookies) and survive canister upgrades.

Drop in a `<button data-mv-signin>` and it's wired automatically.

## Authorization — roles & scopes

Gate pages and handlers with `@authorize`, optionally scoped to a role:

```razor
@page "/admin"
@authorize role="Admin"
```

`@authorize` (no role) requires an authenticated caller; `role="Admin"` additionally requires the caller to **hold that role** in the runtime's persisted role store — otherwise the page redirects and its content never renders (server-side; not a hidden div).

An unauthorized caller is sent to `/` by default. Point them somewhere else — your sign-in route — with `redirect`:

```razor
@page "/feed"
@authorize redirect="/welcome"
```

Because the target is configurable, even `/` itself (or the login route) can carry `@authorize` without a redirect loop.

> **Gate on the page, not the layout.** `@authorize` is enforced on *every* path that can serve a page: the full-document `GET`, the `/_motoview/render` poll, and the `/_motoview/event` dispatch. A layout that merely hides content behind `@if (ctx.isAuthenticated) { … @yield … }` is **presentation only** — the render-poll and event endpoints render the page *without* its layout, so a layout-only gate leaks the page body and lets its handlers run for unauthenticated callers. Always put `@authorize` on the page. The compiler emits a `layout-auth-gate` warning when a page relies on an auth-gating layout but declares no `@authorize` of its own.

Manage roles from any handler via the request context — `hasRole(who, role)`, `callerRoles()`, `grantRole`, `revokeRole`, and `claimRole(role)` (first-come bootstrap for the first admin). The store is a `stable var`, so grants survive `dfx deploy --mode upgrade`.

```motoko
@code {
  func claimAdmin(ctx : Context) : async () { ignore ctx.claimRole("Admin"); };
  func makeEditor(ctx : Context) : async () {
    if (ctx.hasRole(ctx.caller, "Admin")) { ctx.grantRole(somePrincipal, "Editor"); };
  };
}
```

For a fuller picture, apps can build a **scoped** role model on top of this — global tiers (e.g. SuperAdmin / Admin / Moderator) plus per-resource roles (e.g. per-server Owner/Admin/Moderator/Member), with the page combining the two so a global staffer can act in any scope. The flagship Bzzz app ships exactly this, with an Admin Console; the framework primitive (`@authorize` + the role store) is what backs it.

## Zero-trust encryption — vetKeys, the browser decrypts locally

For data the canister itself must never read, MotoView ships **end-to-end encryption with vetKeys** — the **browser decrypts locally** and the canister holds **only ciphertext**.

- Every actor exposes the endpoints `GET /_motoview/vetkd/public-key` (the BLS12-381 master key) and `POST /_motoview/vetkd/derive` (a vetKey bound to the **session caller's** principal — only they can unwrap it). The threshold key is derived across the subnet; **no single node holds it**, and the canister never sees the plaintext key.
- All the crypto (BLS unwrap + identity-based encrypt/decrypt) runs in a **Rust → WASM** module in the browser (`ic-vetkeys`). Two declarative attributes make it JS-free for apps: `data-mv-encrypt` (a form field is IBE-encrypted *before* it's sent) and `data-mv-decrypt="<ciphertext>"` (decrypted in place on render). The canister stores ciphertext and audits each access; it can prove *who did what, when* without ever reading the data.

So a compromised canister or a curious node operator sees only ciphertext. This is verified end to end on a deployed canister, and there's a worked `examples/vault` (a per-user encrypted notes app). Full walkthrough: [Building a Zero-Trust App](zero-trust.md). (Local dfx uses `dfx_test_key`; mainnet uses `key_1`. The crypto is opt-in — it's only fetched when an app uses it — so non-crypto apps keep the lean default brain.)

## Denial-of-service

The update path enforces a request-size cap, and the certified-query path serves static assets without consuming a consensus round. Heavy reads should be `@cacheable` so they're served as queries rather than updates.

## Threat → defense

| Threat | Defense |
|---|---|
| Cross-site request forgery | secure-form HMAC token (can't forge without the canister secret) + browser-bound login nonce |
| Replay / double-submit | single-use nonce + token expiry |
| Acting as another user | token bound to the caller principal; httpOnly session resolves `ctx.caller` |
| Field tampering / over-posting | schema-hash binding rejects unexpected fields |
| Client validation bypass | validation runs server-side in the handler |
| Response tampering / stale reads | certified queries (static/`@cacheable`) + consensus updates (dynamic) |
| XSS from app code | no application JavaScript; output escaped by default (`@raw` is explicit) |
| Supply-chain / dependency attacks | no npm; reproducible Rust→WASM client |
| Session theft via JS | httpOnly session cookie; per-principal revocation |
| Privilege escalation | `@authorize` + persisted role store enforced server-side |
| Canister/operator reading private data | vetKeys E2E encryption — only ciphertext on-chain, browser decrypts locally |

Build on what is verified end to end: secure forms, certified responses, Internet Identity, role-based authorization, and zero-trust vetKeys encryption.
