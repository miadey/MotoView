---
title: Building a Zero-Trust App
section: Architecture
slug: zero-trust
---

# Building a Zero-Trust App on MotoView

Zero trust is a posture, not a product: *never trust, always verify*. No request is trusted because of where it comes from; every request is authenticated, authorized, and minimized, and sensitive data is encrypted so that even the systems holding it cannot read it.

MotoView is unusually well-positioned for this because of its shape. There is no application JavaScript and no separate API tier, so authorization and validation live in exactly one place — the canister — and cannot drift out of sync between a client and a server. The browser is a renderer (and, for vetKeys, a local crypto engine); the canister is the source of truth.

This page maps the seven pillars of a zero-trust architecture to concrete MotoView primitives. It is deliberately honest about the line between **what ships today** and **what the encrypted-storage change adds**, because designing around a feature that does not exist yet is the opposite of zero trust.

---

## The seven pillars, mapped

| Pillar | MotoView primitive | Status |
| --- | --- | --- |
| 1. Identity | Internet Identity login → session cookie → `ctx.caller` | **Ships today** |
| 2. Authorization | `@authorize` + the persisted per-principal role store | **Ships today** |
| 3. Integrity of served content | Certified query rendering (response-certification v2) | **Ships today** |
| 4. Request authenticity | `secure` forms: HMAC token binding path + handler + principal + nonce + expiry + schema | **Ships today** |
| 5. Server-side enforcement | `validate model { … }` + handlers run in the canister; no client-only rules | **Ships today** |
| 6. Data confidentiality (key custody) | vetKeys endpoints: `mvVetkdPublicKey` / `mvVetkdDeriveKey`, threshold-derived, per-principal | **Ships today** (canister foundation, verified) |
| 7. Encrypted-only storage + audit | `EncStore` (ciphertext-only state) + server-side `Audit` log + **in-browser** vetKeys decrypt | **Added by this change** (opt-in) |

The first six are verified and in the runtime/compiler now. The seventh is what turns "we *could* encrypt" into "the canister *only ever holds ciphertext*, and every sensitive transition is logged" — and it is opt-in, because the in-browser crypto is not free (see [the size caveat](#the-opt-in-cost)).

---

## Pillar 1 — Identity: who is calling

MotoView ships a hand-rolled Internet Identity login with **no npm and no agent-js**. A small browser IC agent (served at `/mv-auth.js`) makes one authenticated `mvEstablish(nonce)` update call; the IC itself verifies the caller's signature, the runtime records the principal under a server-issued nonce, and a following `GET /mv-session` mints a short-lived **httpOnly** session cookie bound to that principal.

From then on, every handler resolves the real signed-in principal from the cookie via `effectiveCaller` (`runtime/src/App.mo`), exposed to your code as `ctx.caller`. The session token is never readable by JavaScript (XSS-resistant), the nonce is server-chosen and never travels in a URL (login-CSRF-resistant), and sessions are revocable per principal (a logout bumps a stored epoch, invalidating every outstanding token for that user).

This is the anchor for everything else: roles attach to a principal, vetKeys are derived for a principal, and the secure-form MAC is bound to a principal.

```razor
@page "/vault"
@authorize
<!-- ctx.caller is now the authenticated II principal -->
```

## Pillar 2 — Authorization: least privilege

`@authorize` (bare) requires an authenticated caller. `@authorize role="Admin"` additionally requires the caller to **hold that role** in the runtime's role store; otherwise the page redirects and its content never renders — the data is never sent, not merely hidden.

Roles are per-principal, persisted across upgrades (`stable var`, see `runtime/src/Roles.mo`), and managed from any handler via the context: `hasRole`, `callerRoles`, `grantRole`, `revokeRole`, and `claimRole` (a first-come bootstrap for seating the first admin safely). Because authorization runs server-side before render, there is no client gate to bypass.

## Pillar 3 — Integrity of served content

Static framework assets and pages you mark `@cacheable` are served as **certified queries** using HTTP response-certification v2 (`runtime/src/CertV2.mo`). The boundary node validates the certificate against the subnet's root key, so a single malicious replica or boundary node cannot tamper with the bytes a user receives. Dynamic pages keep the consensus-validated `http_request_update` path, which is fresh by construction. Either way, the content a user sees is attributable to the canister, not to whatever served it.

## Pillar 4 — Request authenticity: every mutation is a bound request

Mark any state-mutating form `secure`. On render, the canister mints an **HMAC-SHA256** token over a tuple that pins the submission to one exact request: path, handler, caller principal, single-use nonce, expiry, and a hash of the field schema. On submit, the canister re-derives the MAC from the *live* request and rejects any mismatch before the handler runs.

This defeats CSRF (no secret, no forgery), replay (single-use nonce + expiry), principal substitution (the MAC is bound to `ctx.caller`), and field tampering (the schema hash binds the allowed field set). SHA-256/HMAC are implemented in Motoko and pass standard test vectors — there is no JavaScript crypto in the trust path. See [Security](security.md) for the full token shape.

## Pillar 5 — Server-side enforcement: no trusted client

There is no client-side-only validation to bypass, because there is no application JavaScript to hold it. The `validate model { … }` block and every event handler execute in the canister. A client can lie about anything; the canister re-checks all of it. This is the structural reason MotoView's "verify" is cheap: there is exactly one enforcement point.

## Pillar 6 — Data confidentiality: keys nobody can steal

Every generated MotoView actor exposes vetKeys endpoints for identity-based encryption (IBE), so each user can have state that **only they** can decrypt (`compiler/src/project.rs` generates these into every app):

- `mvVetkdContext()` → the app's derivation context (`"motoview"`).
- `mvVetkdPublicKey()` → the 96-byte BLS12-381 G2 master public key for that context. Free; reveals no secret.
- `mvVetkdDeriveKey(transportKey)` → a 192-byte vetKey for the **caller's principal** (the derivation `input` is `Principal.toBlob(msg.caller)`), encrypted to a client-generated transport key.

The threshold key is derived across the subnet — **no single node ever holds it**. The canister mediates the derivation (it attaches the cycles and calls the management canister `aaaaa-aa`) but never sees the plaintext key: it only forwards a 192-byte blob that is already encrypted to the client's transport key. The BLS unwrap and the IBE encrypt/decrypt run in the **client**, via [`ic-vetkeys`](https://crates.io/crates/ic-vetkeys).

This foundation is verified end to end against a real local replica with the real client crypto — see [`tools/vetkeys-roundtrip`](../tools/vetkeys-roundtrip), which prints `ROUND_TRIP_OK` after a full derive → unwrap (BLS-signature-checked `decrypt_and_verify`) → IBE encrypt → IBE decrypt that recovers the exact plaintext. It also confirms `ic-vetkeys` compiles to `wasm32-unknown-unknown`, which is the prerequisite for putting that crypto in the browser brain.

---

## What this change adds (Pillar 7)

The six pillars above mean a user is authenticated, authorized, served verified content, and *can* obtain a per-user key the infrastructure cannot read. This change closes the loop so that the canister holds **only ciphertext** and every sensitive transition is **audited** — and it wires the vetKeys decrypt into the browser brain so an app does this with no external tool.

Three pieces, all opt-in:

1. **`EncStore` — ciphertext-only storage.** A store whose values are IBE ciphertexts keyed by principal. It deliberately offers no "read plaintext" method: the canister cannot decrypt, by design. It stores what the browser hands it (already encrypted) and returns it on request. The threat surface of a compromised canister or a curious node operator collapses to "they see ciphertext."

2. **`Audit` — a server-side append-only log.** Every sensitive transition (store, fetch, key derive, role change) appends an immutable record — principal, action, timestamp, and a content hash (never the plaintext). This is the "always verify" half of zero trust: even though the canister cannot read the data, it can prove *who did what, when*. The log is `stable`, so it survives upgrades, and there is no method to mutate or delete past entries.

3. **In-browser vetKeys decryption.** The same `ic-vetkeys` crypto that `tools/vetkeys-roundtrip` proves on the host, shipped inside the MotoView WASM brain. A page generates a transport key, calls `mvVetkdDeriveKey`, unwraps the vetKey, and does the IBE encrypt/decrypt **locally** — the plaintext exists only in the browser tab. The JS glue stays dumb hands: it loads a wasm, calls an export with bytes, gets randomness from `window.crypto`, and `fetch`es. All crypto logic is in Rust → WASM. (This is the framework's hard rule: no application JavaScript; crypto is brain, not hands.)

See the worked example under [`examples/vault`](../examples/vault) — a per-user secret store where the browser encrypts, the canister stores only ciphertext via `EncStore`, every access is recorded in `Audit`, and the browser decrypts on read.

### The client API (what you actually write)

The whole client side is two declarative attributes — no JavaScript:

```mview
@* encrypt a field in the browser before the secure form is sent *@
<form @submit="addNote" secure>
    <textarea name="note" data-mv-encrypt required></textarea>
    <Button appearance="primary" type="submit">Encrypt &amp; save</Button>
</form>

@* render stored ciphertext; the browser decrypts it in place on load *@
@for n in notes {
    <span data-mv-decrypt="@n.ciphertext">🔒 decrypting…</span>
}
```

On submit, the glue IBE-encrypts every `[data-mv-encrypt]` field (so the canister only ever receives ciphertext); on render, it decrypts every `[data-mv-decrypt]` element in place. Both go through `window.mvCrypto`, which loads `/motoview-crypto.wasm` on first use. The canister auto-exposes two endpoints the brain calls — `GET /_motoview/vetkd/public-key` (the master key) and `POST /_motoview/vetkd/derive` (the vetKey, bound to the session caller via the II session cookie). Your Motoko handler reads `ctx.form` (already ciphertext) and stores it; it never sees plaintext.

> **Verified end to end** in a real browser on a deployed canister: a note typed into the vault is encrypted before submit (the served HTML contains only the `IbeCiphertext`, never the plaintext), stored on-chain, then decrypted back to the exact text locally on render — `ok: true`.

---

## End-to-end data flow

A single sensitive value, from login to plaintext-in-tab and back:

1. **Login.** The user signs in with Internet Identity; the runtime mints an httpOnly session cookie. `ctx.caller` now resolves to their principal (Pillar 1).
2. **Derive.** The browser brain generates a 48-byte transport key (`window.crypto` RNG → `ic-vetkeys`), calls `mvVetkdPublicKey()` for the 96-byte master key, and calls `mvVetkdDeriveKey(transportKey)`. The canister attaches cycles, calls the management canister, and returns the 192-byte encrypted vetKey — still encrypted to the transport key.
3. **Unwrap.** In the browser, `decrypt_and_verify` checks the BLS signature and yields the user's vetKey. The plaintext key never leaves the tab and never touches the canister.
4. **Encrypt + store.** The browser IBE-encrypts the secret to the user's principal and submits the **ciphertext** through a `secure` form (Pillar 4). The handler writes it to `EncStore` and appends an `Audit` record. The canister stores ciphertext only.
5. **Fetch.** Later, `ctx.caller` (Pillar 1) is checked, the ciphertext is read from `EncStore`, an `Audit` "fetch" record is appended, and the ciphertext is rendered into the page.
6. **Decrypt.** The browser brain re-derives/uses the vetKey and IBE-decrypts locally, recovering the plaintext for display. The canister never saw it.

The only point at which the plaintext exists is inside the authenticated user's browser. Everything in transit and at rest is ciphertext; everything is attributable; nothing is trusted on the client's word.

---

## Threat model: what the canister and node operator can and cannot see

**Cannot see:**

- The **plaintext** of any value stored through `EncStore`. The canister holds IBE ciphertext only and has no decrypt path.
- The **vetKey** itself. It is threshold-derived (no single node holds it) and returned encrypted to a transport key the browser generated; the canister forwards an opaque blob.
- Anything by **impersonating a user**: derivation `input` is the caller's principal, so a node cannot ask for someone else's key, and the secure-form MAC is bound to the caller.

**Can see (and you must account for):**

- **Metadata.** Which principal stored or fetched which key, when, and how large the ciphertext is. The `Audit` log makes this explicit on purpose. If access patterns or sizes are themselves sensitive, pad and/or partition.
- **Ciphertext at rest**, available to anyone who can read canister state (including a malicious replica). Confidentiality rests entirely on the IBE encryption, not on access control.
- **A coerced subnet.** vetKeys' guarantee is *threshold* — it assumes an honest majority of the subnet's nodes. A subnet-level compromise that breaks that threshold is outside what any vetKeys-based scheme defends against.

**Not solved here:** transport security (handled by the IC boundary and `http_request_update`), denial of service, and traffic analysis beyond the metadata note above. Zero trust reduces the blast radius of a compromise; it does not make the system invulnerable.

---

## The dfx_test_key vs key_1 + cycle caveat

vetKeys derivation is keyed by name and costs cycles:

- **Local dfx** ships the test key `dfx_test_key` (the generated endpoints use it). The local replica charges roughly **26.2 billion cycles per derive**; `runtime/src/VetKeys.mo` attaches a margin (30 billion) on every `mvVetkdDeriveKey` call.
- **Mainnet** uses the production key `key_1`. Switch the key name in the generated `mvVetkdPublicKey` / `mvVetkdDeriveKey` (the codegen in `compiler/src/project.rs`, backed by `runtime/src/VetKeys.mo`) before you deploy.

Two practical consequences: budget cycles for derivation (it is not free, and it is an **update** call, not a query), and cache the unwrapped vetKey in the browser for the session rather than deriving per operation. `mvVetkdPublicKey` is free and safe to call anywhere; only the derive costs.

---

## The opt-in cost

The in-browser vetKeys crypto is **opt-in**, and the reason is honest: it is not small. The default MotoView brain is roughly **76 KB** of WASM. Bundling the `ic-vetkeys` BLS12-381 + IBE machinery adds on the order of **~300 KB**. Apps that don't need client-side encryption keep the lean default brain and pay nothing; apps that opt in accept the larger download in exchange for plaintext that never leaves the browser.

This is the right default. Zero trust is a cost-benefit decision per app, not a tax on every page.

---

## Where to go next

- **Proof it works:** [`tools/vetkeys-roundtrip`](../tools/vetkeys-roundtrip) — the verified derive → unwrap → IBE round-trip against a real replica with the real `ic-vetkeys` crypto.
- **Worked example:** [`examples/vault`](../examples/vault) — encrypt-in-browser, store-ciphertext, audit-every-access, decrypt-in-browser.
- **Foundations:** [Security](security.md) (secure forms, HMAC binding, roles, the vetKeys endpoints) and the [Roadmap](roadmap.md) (the honest status line).

Build on the verified pillars; opt into the encrypted-storage pillar where the data warrants it. That is zero trust on MotoView — not a slogan, but a small set of primitives that each refuse to trust the layer below them.