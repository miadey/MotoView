# MotoView Secure Visual Designer — Reflection & Phased Plan

> Status date: 2026-06-10. This document is grounded in the code on branch
> `feat/native-secure-client` (commit `7dd64a0`). Every capability claim below is
> tagged **REAL TODAY** (verified in the repo), **DESIGN-NOT-CODE** (the
> primitive exists but is not yet a deployed control), or **NEW WORK** (does not
> exist). The honesty discipline is non-negotiable: this is a security/privacy
> product, and an overstated claim is the single fastest way to get a user hurt
> and to collapse the project's credibility.

---

## 1. North Star

**A drag-and-drop visual designer that makes it fast and easy to build apps that
are secure and private *by construction* — apps that run as a real native binary
(no WebView, no PWA) and are connected to an Internet Computer canister, where
the UI and the money paths are cryptographically attributable to a known
canister, the user's confidential data is stored as ciphertext only, and there
is no application JavaScript to attack.**

The designer is not "yet another web builder." Its reason to exist is the
category of apps where *un-hackable + private is the product, not a feature*:
wallets/identity, confidential vaults (health/records), and
whistleblower/journalist-adjacent tooling. For everyone else, this stack is
deliberately over-engineered and they should pick FlutterFlow/Retool/Juno. The
product wins **if and only if** it is sold exclusively to buyers who genuinely
need the cryptographic guarantee badly enough to pay the ICP tax (smaller
ecosystem, ~2s update-call finality, niche Motoko language, and the disclosed
trust ceilings below).

---

## 2. Core Thesis — Security & Privacy *by Construction*

MotoView's wedge is a **chokepoint architecture**: every app passes through ONE
compiler, ONE runtime (`runtime/src/App.mo`), and ONE framework-fixed brain +
dumb hands. Security is therefore a property of the *toolchain*, not of developer
diligence. The four load-bearing structural facts:

1. **No application JavaScript.** All logic lives in `.mview` → Motoko (the
   server/canister) and a fixed Rust→WASM "brain." The JS/native "hands" are
   ~854 lines of dumb glue that only execute keyed DOM ops. **REAL TODAY.** This
   is what makes a native renderer possible (no per-app JS to port), which is
   what makes local certificate verification possible (a native binary can pin a
   root key), which is what makes intent-bound spends meaningful (the verified
   bytes *are* the confirmation screen). Pull out any one link and the chain
   breaks — the wedge is the whole chain, and no web2 or agent-js-SPA competitor
   can reproduce it without rebuilding from zero.

2. **Deny-by-default secure-form lint.** A state-mutating `<form @submit=...>`
   that is not marked `secure` is a hard build **Error** (`compiler/src/lint.rs`,
   `Severity::Error`), and the gate aborts both `build` AND `preview`
   (`compiler/src/project.rs`). **REAL TODAY.** You literally cannot render an
   insecure mutating form in preview, let alone ship one.

3. **Intent-bound server-side spend gate.** `runtime/src/WalletAuth.mo` +
   `App.mo`'s `authorizeSpend` re-derive `intentHash` from the canonical
   `(amount, dest, chain, nonce)` tuple; a token minted for intent X cannot
   authorize spend Y, plus a per-principal velocity limiter. **REAL TODAY**
   (server side). Honest gap: `WalletAuth.mo` itself (lines 27-28) flags the
   native `host_device_sign` hardware assertion as **NOT implemented** — the
   second factor does not exist yet.

4. **Ciphertext-only zero-trust storage.** `runtime/src/EncStore.mo` stores only
   ciphertext with no decrypt path; vetKeys IBE encryption/decryption happens in
   the *client* (`client-crypto/src/lib.rs`, round-trip tested vs a live
   replica). **REAL TODAY in the browser.** Honest gap: **no native client has
   any vetKeys crypto** (no `ic-vetkeys` in `clients/ios/Package.swift`,
   `clients/android/build.gradle`, or `apps/studio/native/Cargo.toml`). "Private
   apps that run from a native app" is **DESIGN-NOT-CODE** today.

The job of the designer is to make these properties *un-removable*: `secure` is
the default for mutating forms and the inspector cannot turn it off; a PII field
left in plaintext aborts the build; the only codegen path for a spend is through
`authorizeSpend`. Safety is enforced at the one chokepoint every app passes
through — not bolted on as a checkbox a user can forget.

---

## 3. How the Drag-and-Drop Designer Works (and round-trips with `.mview`/UI-IR)

### 3.1 The single highest-leverage decision

**`.mview` text is the source of truth; the canvas is a *span-anchored
bidirectional projection* over the real parser AST — NOT a separate document
model that emits text.** Visual edits become minimal, localized text splices at
AST spans, re-validated before they touch disk. This is the only model that (a)
never rewrites hand-authored `.mview` wholesale, (b) reuses the existing
parser/spans/codegen instead of inventing a parallel doc model that would drift,
and (c) keeps "designer output == what a human would write," so code and canvas
are literally one artifact.

MotoView has *already built the two hardest pieces of a real bidirectional
designer without calling them that*:

- **A self-verifying semantic-equivalence oracle.** `compiler/src/fmt.rs`'s
  `codegen_signature(source, kind)` runs the real parser + codegen and proves
  `codegen(candidate) == codegen(original)` byte-for-byte. Any visual edit can be
  auto-accepted or rejected with zero hand-reasoning. **REAL TODAY.**
- **Full source spans + a generated→source side-map.** The AST carries
  file-relative `Span`s on every Element/Component/Attr/EventBind
  (`compiler/src/ast.rs`), and `project.rs::build_source_map` already emits a
  `.mvbuild/main.mo.map` side artifact *without touching the generated
  `main.mo`*. **REAL TODAY.**

### 3.2 The canvas renders the live UI-IR forest

The canvas renders the same portable UI-IR forest (`runtime/src/Ir.mo`) that
ships to every renderer — web, iOS (SwiftUI `clients/ios/NativeView.swift`),
Android (Compose), and the desktop egui IDE (`apps/studio/native`). The native
egui studio *already* renders the page UI-IR as Fluent widgets
(`render_forest`/`render_node`) and dispatches clicks via deterministic no-deploy
replay (`backend.rs` shells out to `motoview preview --emit ir --json`). **REAL
TODAY** — it is a read-and-interact canvas today; it lacks only authoring
(selection, drag, drop, inspector). Because the canvas renders the IR forest,
multi-platform preview is *free* and the canvas is pixel-consistent with what
ships.

### 3.3 The ONE load-bearing new piece: the selection bridge

**`Ir.mo` carries no per-node span (`grep` for "span" = 0).** Every selection,
inspector, and drag feature depends on mapping a clicked widget back to its exact
`.mview` source region. This MUST be threaded as a **SIDE-MAP** (keyed exactly
like the existing `build_source_map`), **NOT a new IR attribute** — because
adding an attribute to the emitted IR would change the generated `main.mo` and
break the byte-identical-codegen invariant. This is **NEW WORK** and it is the
literal prerequisite for the whole designer. Until it exists there is no
authoring, only viewing.

### 3.4 Edit operations and their hard constraints

Each gesture is a minimal text splice at a span, gated by `codegen_signature`
before write, then the canvas re-renders via the preview path:

- `set-attr`, `set-bind`, `set-event`, `set-text`, toggle `secure` — single-span
  splices (safest, built first).
- `insert-child` (palette drop onto a `WidgetKind::Group` container).
- `move`/`reorder`/`delete` — the **hard correctness phase**.

Three constraints make move/reorder genuinely hard, and the plan budgets for them
explicitly (Phase D) rather than hand-waving:

- **Whitespace is semantic.** `Node::Text` whitespace is emitted verbatim into
  output (this is *why* `fmt.rs` is conservative). Splices must be surgically
  minimal and must not normalize surrounding whitespace, or the rendered output
  silently changes. The `codegen_signature` gate catches semantic drift, but a
  designer that *frequently* trips the gate and refuses the edit feels broken.
- **Dynamic regions resist direct manipulation.** Inside
  `@for deal in dealsIn(stage)` the canvas shows N rendered cards but there is
  ONE source template. A drop/delete/edit on an instance must map back to the
  single template node, and `@if`/`@switch` branches need an explicit authoring
  affordance. Webflow/Framer punt on this; we cannot.
- **Single-writer model.** The text buffer is truth; canvas edits go *through*
  the buffer so a hand-edit and a drag can never race.

The only thing that makes Phase D safe is a **golden round-trip CI suite**:
`canvas-edit → fmt → assert codegen unchanged`, with explicit
whitespace-semantic and `@for` template-vs-instance cases.

### 3.5 Data-binding and secure primitives are pickers, not free text

- **Bind-picker** dropdowns are populated from the project's real Motoko surface
  (`project.rs` already scans `Services`/`Models` for params + types): list-bind
  offers query-shaped funcs (`@for x in queryFn()`), form-submit offers async
  funcs (`@submit=updateFn`), input bind offers record fields (`bind="@field"`).
  **NEW WORK** on top of the existing scan. **Honest caveat to validate:**
  `async ≠ mutating`, and a query func can have a side-effecting-looking name —
  the heuristic can offer a binding that *compiles but is semantically wrong*, so
  the picker must be validated against real signatures, not shipped on faith.
- **Secure primitives** (secure-form, encrypted-field, wallet/spend block,
  II-auth block) are palette items that splice the *literal tokens the compiler
  already enforces* (`secure`, `data-mv-encrypt`/`data-mv-decrypt`, a submit
  routed through `authorizeSpend`, the `App.mo` session-bridge wiring). The
  **secure variant is the default**: drop a "Form" and it is `secure`; drop a
  "Secret field" and it carries `data-mv-encrypt`. The deny-by-default lint
  becomes a live red badge on the offending node, not a late build failure.

---

## 4. Security Model — Honest Threat Model

The headline must be the **TRUE, conditional** claim, never the aspirational one:

> **We have a fail-closed chain-key BLS verifier (`client/src/cert_verify.rs`),
> tested against TWO live unmodified mainnet certificates with a battery of
> fail-closed negatives, compiled into the iOS/Android native archives and
> reachable over the C ABI as `mv_ffi_verify_response` (hardwired to
> `RootKey::Mainnet`, so a native caller can never be downgraded to a local
> key). Wiring it into the render path is Phase 1 and is the single most
> important security task in the repo. Nothing yet gates rendering on it.**

### 4.1 What is REAL TODAY (verified primitives)

- The chain-key BLS12-381 verifier: pinned 133-byte NNS root key, HashTree
  reconstruction byte-identical to `CertV2.mo`, single-hop delegation +
  `canister_ranges`, `/time` freshness, strict CBOR (rejects trailing bytes +
  duplicate keys), every path fails closed with a named error
  (`client/src/cert_verify.rs`, `tests.rs`).
- The deny-by-default secure-form lint that aborts build AND preview
  (`lint.rs` + `project.rs`).
- The intent-bound spend gate + velocity limiter, server side
  (`WalletAuth.mo` + `App.mo authorizeSpend`).
- Threshold ECDSA/Schnorr via the mgmt canister, with a mainnet-key network gate
  that TRAPS on `dfx_test_key` (`ChainKey.mo`).
- The II session bridge: IC-verified principal → single-use nonce → HMAC session
  token in an httpOnly+Secure cookie never exposed to JS (`App.mo`).
- Response-cert v2 is **wired** for `@cacheable` pages: `App.mo` computes
  `responseHash`, calls `registerPage`, and installs it via
  `CertifiedData.set`/`CertV2.rootHash` (this *corrects* the deep-dives' outdated
  "CertV2 is pass-through" claim).
- The deterministic reproducible build of the brain wasm
  (`tools/release/reproducible-build.sh` emits `opt_sha256`).

### 4.2 What is DESIGN-NOT-CODE today (verified primitive, NOT a deployed control)

These are the overclaims that MUST NOT survive into any marketing:

- **"The native app verifies the UI bytes against the pinned root key." FALSE as
  deployed.** `mv_on_response` (`client/src/lib.rs:366-389`) calls `b.apply` with
  **zero** cert check; `cert_verify` is `mod`-declared but never invoked on the
  render path. The native transport is a stub (`StateHostBridge.fetch`: "No
  network in the reference bridge"). There is **no** transport that fetches a
  forest + certificate on-device and gates render on it.
- **"`verify_response` verifies a served page body." FALSE.**
  `cert_verify.rs:641-667` implements ONLY `certified_data == SHA256(body)`; its
  own doc-comment (lines 645-654) says it does **not** walk the v2
  `http_expr`/`expr_path` that `CertV2.mo` uses to certify served HTTP bodies.
  Even if wired in, it cannot verify a real served page body until the
  `http_expr` walker is ported.
- **"Dynamic per-user UI is locally verified." FALSE for the content that
  matters.** `App.mo` body-certifies **only `@cacheable` pages** as certified
  queries, and even those fall back to the always-correct **update path** the
  moment their body changed since last certify. Non-`@cacheable` (per-user,
  dynamic) pages — i.e. **most real app screens** — ALWAYS travel the update path
  (consensus-validated, NOT locally root-verified). Extending local verification
  to dynamic content requires folding per-response body hashes into
  `CertifiedData` on every mutation — a **canister-side redesign**, not a wiring
  task.

### 4.3 What becomes IMPOSSIBLE vs merely HARDER vs NOT stopped

**Becomes impossible (once Phase 1 lands and for the content it covers):**

- Shipping a state-mutating form without a secure HMAC token (compiler aborts —
  REAL TODAY).
- A compromised UI altering *what gets signed* after approval — the spend token
  is bound to the exact `(amount, dest, chain)` intent hash (server side — REAL
  TODAY).
- A native client being downgraded to a local/boundary root key —
  `mv_ffi_verify_response` is `Mainnet`-only by construction (REAL TODAY).
- (After Phase 1) a native client painting a static/`@cacheable` forest it cannot
  attribute to the pinned canister.

**Merely harder:**

- Replay (single-use nonces) and session-drain (per-principal velocity limiter) —
  real, but a stolen session cookie is still a bearer token until hardware-bound
  per-request signing lands.
- Serving tampered *dynamic* UI — consensus-validated via the update path, but
  not locally root-verified until §4.2's canister-side work ships.

**NOT stopped (must be stated in plain language inside the product):**

- **The malicious/compromised CONTROLLER.** A certificate proves a response
  *came from* the canister, not that the canister is *honest*. A coerced or
  compromised controller (or an NNS-governed upgrade) can mint sessions, forge
  intents, or rewrite spend logic server-side. Only blackholing / immutable
  governance / a real SNS DAO mitigates this, and a TOCTOU window exists between
  a controller check and a mid-session upgrade. **This makes "non-custodial"
  overstated: the threshold key is custodial-by-controller.**
- **General-purpose-OS compromise.** Keylogger/IME, screen-overlay malware, a
  rooted/jailbroken device, or a hooked runtime (Frida behind a *genuine* App
  Attest verdict) defeat input integrity and the in-binary pin. The missing
  `host_device_sign` (Secure Enclave/StrongBox) factor is exactly the bar that
  would raise this — and it is NOT built.
- **Human approving the wrong thing.** Intent-binding stops UI tampering *after*
  approval, not a user who is socially engineered into approving a malicious
  intent.
- **Bearer-token theft.** Session + secure-form + spend tokens all hang off ONE
  per-canister `mvSecret`. A controller-side state read or a future stable-memory
  leak forges everything; there is no key-rotation story beyond a per-principal
  epoch revoke. And vetKey *release* is bound to the HMAC **cookie**
  (`effectiveCaller`, `project.rs:1491,1499`), not a live II signature — so
  cookie theft yields the victim's vetKey and their plaintext.
- **TOFU / app-store gatekeeping.** The binary you install is whatever the
  store/OS handed you; the in-binary pin is rewritable on a rooted device; App
  Store 2.5.2 forbids runtime brain hot-swap (no OTA self-heal on iOS). The trust
  chain genuinely terminates *outside* the cryptography.

---

## 5. Privacy Model — vetKeys, and its real limits on a public ledger

**REAL TODAY (browser):** A field marked `data-mv-encrypt` is IBE-encrypted in
the client before it leaves the device; the canister stores only ciphertext via
`EncStore.mo` (no decrypt path); the client unwraps a per-principal vetKey to
decrypt locally (`data-mv-decrypt`). Node operators and canister controllers see
only ciphertext. The crypto is round-trip tested end-to-end against a live
replica (`client-crypto/src/lib.rs`, including negative wrong-identity/wrong-master
cases). `examples/vault` is the worked lighthouse.

**The privacy DESIGN layer to build:** a per-field classification
(**Public / Private-E2E / Derivable**) set in the designer. Marking a field
**Private** auto-emits the `data-mv-encrypt`/`data-mv-decrypt` wiring + an
`EncStore`-backed service + an `Audit` entry — the author writes zero crypto. A
**deny-by-default `private-field` lint** (PII heuristic + `Secret`-typed binding),
modeled on the existing secure-form rule, **aborts the build** if PII would ship
in plaintext. Encrypt to the **owner principal by default**; sharing is an
explicit, audited capability, never an implicit default.

**The hard limits, shown LOUDLY in-product (not footnotes):**

- **"Private" means private-CONTENT, never private-METADATA.** `EncStore.Meta`
  returns plaintext key/size/created/updated/version and `Audit` stores
  principal/action/timestamp. On a replicated **public** ledger,
  who-stored-what-when-how-big is readable by any node provider. A green
  "encrypted ✓" badge with no metadata disclosure is exactly the lie to avoid.
  The designer ships a per-screen **Privacy Ledger** that shows
  ciphertext-vs-visible-metadata-vs-derivable, with **metadata in RED**.
- **No native crypto today.** "Private native app" is browser-only privacy until
  `client-crypto` is recompiled with `ic-vetkeys` for iOS/Android/desktop behind
  the `host_*` seam.
- **Threshold-honest subnet + `dfx_test_key`.** vetKeys confidentiality rests on
  an honest >2/3 of the subnet. `dfx_test_key` on a small/colluding test subnet
  means a quorum can decrypt ALL ciphertext. `key_1` (a sufficiently large
  fiduciary subnet) is mandatory for any real privacy. The compiler's network
  gate (`project.rs enforce_network_gate`) already hard-fails `dfx_test_key` for
  `--network ic`; it must be **extended to also hard-fail for any native /
  value-bearing build**.
- **Key release bound to the cookie, not the live user** (see §4.3) — cookie
  theft → plaintext.
- **The replica is not a TEE.** Plaintext in canister memory is visible to node
  providers. Anything NOT routed through `EncStore` (a Private field also logged,
  a derived index) leaks. The minimization lint catches a lot, but not
  everything — so the UX must push *minimization* (collect less), not just
  *encryption* (collect-and-encrypt), or it manufactures a false sense of safety.
- **Key recovery is a real cliff.** vetKeys are deterministic from the principal:
  lose the II anchor → lose the data (and any chain-key-custodied funds). There
  is NO recovery primitive in-framework today. A one-click "Private" or "wallet"
  that silently creates unrecoverable loss is a launch-blocker; recovery
  (recovery-principal / social-recovery / multi-device / explicit
  acknowledged-unrecoverable) must be presented at **design time**.

---

## 6. Native ↔ Canister Channel, Attestation, and the TOFU Limit

**The architecture (REAL TODAY where noted):** Keep the brain↔hands `host_*` ABI
(~12 functions) as the native seam. The 84.6KB Rust core cross-compiles to
`aarch64-apple-ios` unchanged (no `js_sys`/`wasm_bindgen` in the default build);
the web wasm stays byte-identical. `HostBridge` mirrors `host_*` 1:1 on iOS
(Swift) and Android (Kotlin); the **renderer holds NO decision logic** (the
native sibling of the dumb JS glue). This is **native-lib**, *not* an embedded
WASM runtime (multi-MB bloat, interpreted on iOS, and OTA brain swap is exactly
what App Store 2.5.2 forbids) and *not* a WebView wrapper.

**The secure channel to build (NEW WORK, Phase 1 — the most important security
task in the repo):**

1. **Port `CertV2.responseHash` + the `http_expr`/`expr_path` walker** into
   `cert_verify.rs` so it can recompute `response_hash` over status + certified
   headers + `SHA256(body)`.
2. **Build a native transport** (`URLSession`/`OkHttp`) that does
   `read_state`/`query` against `icp-api.io` and **captures the certificate
   alongside the body** (today `StateHostBridge.fetch` is a stub).
3. **Gate render on `verify_response` BEFORE `applyTree`** — in BOTH the native
   client AND the web brain's `mv_on_response` (which today applies with zero
   checks). Fail closed on any non-certified response.

**Two independent pins** make a verified response provably from
known-canister/known-code/known-brain: the served-brain `PINNED_BRAIN_SHA256`
(already emitted by `reproducible-build.sh`) **and** the canister `module_hash`
(read via certified `read_state`). The native client must refuse to operate on
drift or un-pinned/un-blackholed controllers, and should verify `module_hash` in
the **same** certificate as the consequential response to close the TOCTOU
window.

**Attestation** (App Attest / Play Integrity) is documented but **inert** (no
verifying canister endpoint exists). It is the canister→client direction and is a
**risk signal feeding Roles, never an absolute gate** — it proves the on-disk
image, not a hooked runtime, and depends on Apple/Google backends.

**Distribution & its TOFU limit.** `bundle.sh` produces a real universal,
icon-ed, ad-hoc-signed `.app` + `.dmg`, published by a macOS-runner CI; the
store pipeline (`release.yml`) is reproducible-hash-gated and wired to
fastlane/Play with all account-touching steps gated on secrets. But **the trust
chain terminates at TOFU**: the binary you install is whatever the store/OS
handed you, the in-binary pin is rewritable on a rooted device, and no amount of
BLS verification closes that. A **transparency-logged signed release manifest**
tying app build + `module_hash` + brain `SHA256` to an audited commit is the
honest mitigation, not a guarantee.

**Anti-rollback.** Cert freshness is device-clock-anchored (±~5min window;
delegation TTL up to ~30 days). A rooted-device clock rollback or NTP MITM can
revive stale-but-valid certified balances. A **monotonic `/time` high-water mark**
in hardware storage is required (NEW WORK, Phase 6).

---

## 7. Competitive Wedge & Lighthouse Apps

MotoView is the **only product in the intersection** of native + ICP-native +
secure/private-by-construction + no-app-JS. The moat is the *integration*, not
any single feature — and that integration is gated by an architectural choice no
competitor can make without rebuilding from zero:

| Capability | FlutterFlow/Bubble/Webflow/Retool | Juno / agent-js SPA | **MotoView** |
| --- | --- | --- | --- |
| Local chain-key cert verification | No (no native binary, no canister) | No (sandboxed page) | **Yes** (primitive REAL; wired = Phase 1) |
| Native renderer, no WebView | No | No | **Yes** (SwiftUI/Compose REAL) |
| Ciphertext-only client-side decryption | No | Partial/manual | **Yes** (browser REAL; native = NEW) |
| No application JS attack surface | No | No (heavy app-JS) | **Yes** (REAL) |
| Intent-bound spends | No | No | **Yes** (server REAL; hardware factor = NEW) |

Lead the comparison table itself as marketing — *including* the disclosed ICP
limits — because with this buyer **the honesty is the moat**.

**Lighthouse sequencing:**

- **Lighthouse #1 — Confidential vault** (encrypted notes/records,
  health/identity/whistleblower-adjacent). Lower-risk than a wallet: it needs
  vetKeys confidentiality (REAL today, `examples/vault`) **without** the
  custodial-key/recovery/KYC problems. Shipping it **downloadable** forces
  recompiling `client-crypto` for native and an actual store submission — turning
  "no-WebView, ciphertext-only" from a fixture into a download.
- **Lighthouse #2 — Wallet / non-custodial signing.** Gated on the hardware
  factor + a recovery model + an independent audit. Never before.

**What would make it fail:** the target buyer doesn't need the guarantee badly
enough to pay the ICP tax; shipping any security claim before it is a *wired,
audited control*; never shipping a store binary; a designer round-trip so fragile
it rejects half the gestures; a recovery cliff that harms users; no independent
audit / bus-factor concentration; or marketing "private" without loudly
disclosing metadata exposure (Signal/SecureDrop own that category with audited
incumbents).

---

## 8. Phased Roadmap

Each phase names what it **builds on (REAL TODAY)** and the **genuinely new
work**. Phases 1–2 are sequenced FIRST and gate all security/privacy marketing —
**the cert-verify keystone (Phase 1) stays the non-negotiable first step regardless
of DX pressure.** The "best of all app builders" DX charter (§16) lands *on top of*
this sequence, not ahead of it: the **toolbox, property grid, and live lint badges**
land in Phase 3; the **double-click → `@code` handler** headline gesture and the
**typed bind-pickers** land in Phase 3–4; the **from-data-model scaffold** lands in
Phase 4; the **Privacy Ledger** posture surface lands in Phase 5; and the **optional
AI accelerator** is deferred to the cross-cutting track (and only ever as a
suggestion engine routed through the *same* `codegen_signature` + lint gate — it is
never on the deploy path). Each DX item below is tagged `[DX §16]`.

### Phase 1 — Wire the cert verifier into a real control (security keystone)
*Builds on:* `cert_verify.rs` (tested primitive), `CertV2.mo` (`@cacheable` body
binding), the `host_*` seam, `mv_ffi_verify_response`.
*New work:* (a) port `CertV2.responseHash` + the `http_expr`/`expr_path` walker
into `cert_verify.rs`; (b) build native `HostBridge.fetch` (URLSession/OkHttp)
that captures the certificate alongside the body; (c) gate render on
`verify_response` BEFORE `applyTree` in the native client AND in the web brain's
`mv_on_response`; fail closed. **No security marketing ships until this passes
its negative tests.** Explicitly: local verification covers static/`@cacheable`
pages; dynamic per-user content still travels the update path — say so.

### Phase 2 — The selection bridge + un-removable designer guardrails
*Builds on:* `ast.rs` spans, `build_source_map` side-map, `fmt.rs`
`codegen_signature`, the egui `render_forest` canvas, the `lint.rs` Error gate.
*New work:* thread the AST Element/Component span into the emitted IR via a
**SIDE-MAP** (NOT an IR attribute — preserve byte-identical `main.mo`); add
hit-testing so **click → source-highlight** works with no editing yet. Make
`secure` the inspector default that cannot be turned off; add the `private-field`
PII lint (Error). Stand up the **golden round-trip CI suite**
(`canvas-edit → fmt → assert codegen unchanged`) now, before any real editing.
`[DX §16]` This bridge is the keystone the whole DX rides on — it is the node→span
identity ("click-to-source") that lets the inspector, the double-click gesture, and
the bind-pickers exist at all (independently validated by Lovable's stable-ID
plugin, §16.1–16.2). No DX feature is buildable until it lands; **the cert-verify
keystone (Phase 1) still precedes it.**

### Phase 3 — Inspector + palette + drop (authoring begins)
*Builds on:* Phase 2 bridge, the Services/Models scan, the secure-token / vault
tokens.
*New work:* a property inspector driven by the selected node's
Attr/EventBind/bind/secure (single-span splices, gated by `codegen_signature`); a
Fluent + secure-primitive palette (secure variants are defaults); palette drop
onto `WidgetKind::Group`; live lint badges painted via the node→span index.
`[DX §16]` This is where the **VB4 loop becomes real**: the **curated secure-first
toolbox** (§16.3), the **honest typed property grid** (token-bound, no ghost props,
no `secure` off-toggle), the **live red lint badge** (anti-CVE-2025-48757 posture),
and the **headline double-click → `@code` Motoko handler** gesture (§16.4 — two
`codegen_signature`-gated splices + a caret move; no AI/network/deploy). The
**typed bind-pickers** over the `project.rs` scan begin here and complete in Phase 4.

### Phase 4 — Move/delete/reorder (the hard correctness phase)
*Builds on:* the golden round-trip suite from Phase 2.
*New work:* whitespace-preserving span splices; `@for` template-vs-instance and
`@if`/`@switch` authoring affordances; the bind-picker (with the `async ≠
mutating` heuristic **validated** so it never offers a semantically-wrong
binding); from-data-model scaffold (pick a Motoko type + service → generate a
secure list/detail/create page). Harden the **generic `render_node` path**
(today the canvas special-cases the CRM Kanban) and solve `moc -r` preview
latency with **optimistic-local-IR mutation + debounced authoritative
re-render**.
`[DX §16]` This phase delivers the **from-data-model 3-screen scaffold** (§16.3 —
record + Service → secure List/Detail/Create, Private fields auto-encrypted), the
**validated typed bind-pickers** (`async ≠ mutating` checked against real
signatures, with the live "update-call" cost badge), and the **F5 instant-feel**
loop (optimistic mutation hides preview latency; `Shift-F5` keeps the explicit
deploy visible). The connection-state Events tab + orphan/broken-wire lints
(§16.3) ride on the now-bidirectional side-map.

### Phase 5 — Native privacy + the downloadable vault lighthouse
*Builds on:* `client-crypto`, `EncStore`/`Audit`/vetkd endpoints, the
classification UX from Phase 2, `bundle.sh`/`release.yml`.
*New work:* recompile `client-crypto` (`ic-vetkeys`) for iOS/Android/desktop
behind the `host_*` seam; the per-screen **Privacy Ledger** + consent gate (no
green badge without metadata disclosure); extend `enforce_network_gate` to
hard-fail `dfx_test_key` for native/value-bearing builds; **ship Lighthouse #1 to
a store** so "no-WebView, ciphertext-only" is a download. Present the recovery
model at design time.
`[DX §16]` The **Privacy Ledger** is the DX posture surface (§16.3, §16.5): the
live, in-product, metadata-in-RED honesty panel that closes the 60-second first
run (§16.5) and is the structural inverse of Lovable 2.0's lying badge.

### Phase 6 — Hardware custody + the wallet lighthouse + audit gate
*Builds on:* `WalletAuth.mo`/`App.mo authorizeSpend`, `ChainKey.mo`, the II
session bridge.
*New work:* `host_device_sign` (Secure Enclave/StrongBox, biometric-gated) and a
fresh per-request assertion over a server challenge — verified in `WalletAuth`
before any signing; bind vetKey/session to the **live II principal**, not the
cookie; the anti-rollback `/time` high-water mark; native `read_state` of
`module_hash` + controllers with refuse-on-drift; the canister-side App
Attest/Play Integrity verifier as a risk signal. **Commission and PUBLISH an
independent audit** of `cert_verify.rs` + `CertV2.responseHash` + `WalletAuth.mo`,
add CI **cross-validation of the Rust verifier vs agent-js** on real mainnet
certs, and **gate ALL wallet/money features on it.** Ship the transparency-logged
signed release manifest. **Remove/hard-quarantine the legacy `motoview shell`
command** (`compiler/src/main.rs:1223`) so "no WebView" has no asterisk.

### Cross-cutting (every phase)
Off-chain integration reality (HTTPS-outcall pattern + a couple of reference
connectors) so a builder isn't dead-ended the moment they need Stripe/Twilio/SMS;
and the explicit, plain-language acknowledgement that KYC/AML/custody-licensing
are legal/operational blockers the crypto does not solve.
`[DX §16]` **Optional AI accelerator — deferred, never on the deploy path.** Only
after the direct-manipulation lanes (Phases 3–4) are solid does the optional
select-and-describe-in-English accelerator (§16.2–16.3) become worth adding. It is
a *suggestion engine*: it reads structured AST/IR/`project.rs` data (never pixels,
never the whole file), is scoped to the selected node's span, **proposes** span
splices that route through the **identical** `codegen_signature` + `lint.rs` Error
gate as a human edit, and **never touches a canister.** Trivial edits stay
model-free in the direct lane. This explicitly does NOT reorder the security-first
sequence — the gate it cannot route around (§16.6) is the whole point of building
it last.

---

## 9. What This Is NOT / Cannot Promise

- **NOT an "un-hackable app builder."** Today the strongest claim is *verified
  primitives, not deployed controls*: the cert verifier is tested and reachable
  over the C ABI but **not yet gating any render path** (Phase 1 fixes this for
  static/`@cacheable` content only).
- **NOT locally-verified for dynamic per-user UI.** Only static/`@cacheable`
  pages are body-certified; per-user screens travel the consensus-validated
  update path, not local root verification — until the canister-side per-response
  body-hash work lands.
- **NOT private at the metadata level.** Ciphertext-only protects *content*;
  principals, timing, ciphertext sizes, and access patterns are public on a
  replicated ledger. We show this in red, not hidden.
- **NOT a native-private app yet.** No native client has vetKeys crypto today;
  private-by-construction is browser-only until Phase 5.
- **NOT non-custodial in the strong sense.** A canister controller can upgrade
  spend logic / forge intents server-side; without blackholing or immutable
  governance it is custodial-by-controller. Chain-key proves *origin*, not
  *honesty*.
- **NOT a hardware wallet.** It runs on a general-purpose OS; keyloggers,
  overlays, rooted devices, and hooked runtimes defeat input integrity. The
  hardware second factor (`host_device_sign`) is not built.
- **NOT recoverable by default.** Lose the II anchor → lose the data and funds.
  No recovery primitive exists in-framework today.
- **NOT free of TOFU.** The install binary, the in-binary pin, and the OS update
  channel are trusted; App Store rules forbid OTA self-heal on iOS.
- **NOT audited.** No published third-party audit and no agent-js cross-validation
  in CI yet — so a sophisticated buyer should (correctly) distrust
  "secure-by-construction" until both exist. All money features are gated on them.
- **NOT for everyone.** Outside wallet/identity/health/whistleblower, this stack
  is over-engineered; those buyers should choose FlutterFlow/Retool/Juno.

---

## 10. First Concrete Step — A Thin Vertical Slice that Proves the Whole Loop

**Goal:** prove the end-to-end loop — *design a tiny secure + private app in the
designer → deploy to a canister → run it natively with cert-verified, encrypted
data* — on the smallest possible surface, so every later phase is hardening a
real path rather than chasing a slide.

**The slice: a single-screen confidential "Secret Note" app.**

1. **Design (proves the selection bridge + secure defaults):** in the egui
   studio, drop a `secure` form with one **Private** text field and a list that
   `data-mv-decrypt`s prior notes. Clicking the rendered field selects it and
   highlights its `.mview` span (the Phase 2 side-map bridge). The inspector
   cannot un-`secure` the form; the `private-field` lint stays green only because
   the field is encrypted.
2. **Deploy (proves the existing chokepoint):** `motoview build` (lint gate must
   pass) → deploy the canister with vetkd endpoints auto-generated and `key_1`
   enforced by the network gate.
3. **Run natively (proves the keystone):** the native client fetches the page
   forest **with its certificate**, runs `verify_response` (with the new
   `http_expr` walker for the `@cacheable` shell) and **refuses to render** on a
   bad cert; encrypts the note client-side via the native-recompiled
   `client-crypto`; the canister stores ciphertext only; the client decrypts the
   list locally.
4. **Prove the honesty surface:** the running app shows the **Privacy Ledger**
   (ciphertext vs visible metadata) and the controller/blackhole status — so the
   first thing the loop demonstrates is *exactly what it does and does not
   protect*.

This single slice exercises Phase 1 (cert-verify-on-render), Phase 2 (selection
bridge + secure default + private-field lint), and Phase 5 (native crypto +
Privacy Ledger) on one screen. If it works end-to-end and downloadable, the
entire native-secure-private thesis is demonstrated rather than asserted —
without overclaiming a single thing the code cannot yet back.

---

## 11. Designer DX — Best-in-Class Canvas/Inspector/Component System (Research Synthesis)

> Added 2026-06-10. Field research into the modern visual builders (Lovable,
> Webflow, Framer, Figma Dev Mode, Builder.io Visual Copilot, Plasmic, Anima) +
> the VB4 form-designer gold standard the project owner cites. The goal of this
> section is to make the **authoring experience** itself best-in-class, on top of
> the security/privacy layer that is our actual moat. The DX is table stakes; if
> the designer feels worse than Framer, the security story never gets a hearing.

### 11.1 The VB4 gold standard, restated as concrete loops

The Visual Basic 4 form designer is cited as the gold standard for one reason:
the **edit→run loop had near-zero ceremony and the code was always right there.**
Four loops define it, and each maps cleanly onto what we already have:

1. **Toolbox → draw a control → it exists.** A palette of primitives; click one,
   drag a rectangle, the control is real and selected. *Our analog:* the Fluent +
   secure-primitive palette drops a `WidgetKind` node onto a `Group` via a
   minimal span splice. The differentiator: our toolbox includes
   `secure-form`/`encrypted-field`/`spend-block`/`II-auth` as first-class
   primitives — controls no other toolbox can offer.
2. **Property grid edits the selected control.** A two-column grid (name | value)
   that is the *complete, honest* surface of the control — no hidden state. *Our
   analog:* the inspector is generated from the selected node's
   `Attr`/`EventBind`/`bind`/`secure`, each row a single-span splice gated by
   `codegen_signature`. Critically, the property grid must show *every* attribute
   the AST node carries (no designer-only ghost props), so canvas and code stay
   one artifact.
3. **DOUBLE-CLICK a control → jump straight into its event handler.** This is the
   single most-loved VB gesture and the one modern web builders mostly lost. In
   VB, double-clicking a Button dropped you into `Button1_Click()` with the cursor
   blinking. *Our analog (the headline DX move):* double-click a node in the
   canvas → if it has an `@event=handler`, **jump the code pane to that Motoko
   function's span**; if it has none, **scaffold a handler stub** (a correctly-
   typed Motoko function in the page's `@code`/Service, wired via a `set-event`
   splice) and land the cursor in it. Logic lives in Motoko, not JS — but the
   *gesture* is identical to VB, and it is the thing that makes "design and behave
   in one place" real instead of a slogan.
4. **F5 → it runs, now.** Instant, no build dialog. *Our analog:* the egui studio
   already does deterministic no-deploy replay (`motoview preview --emit ir`),
   so "F5" is `preview` re-render with optimistic-local-IR mutation + a debounced
   authoritative `moc -r` pass. The latency budget here (Phase 4) is what makes
   or breaks the "instant" feel.

The synthesis below pulls the *best modern refinements* of each loop from the
current tools, and rejects the modern mistakes.

### 11.2 What each modern builder actually does (concrete, not vague)

**Lovable (the closest philosophical cousin — same source-of-truth discipline).**
This is the most important reference because Lovable independently arrived at our
core decision: *the code is the source of truth and visual edits are AST splices,
not a separate document model.* Specifics worth stealing:

- A **custom Vite plugin stamps every JSX element with a stable, persistent ID**
  at compile time. Clicking a DOM element traces back to *the exact JSX that
  rendered it* via that ID. **This is exactly our node→span side-map** — and it
  validates our Phase-2 keystone as the right call rather than over-engineering.
  Their ID lives in a build-time plugin so generated source stays clean; *our*
  side-map lives in `.mvbuild/main.mo.map` so the emitted `main.mo` stays
  byte-identical. Same idea, and ours is arguably cleaner because the map is a
  side artifact, not an attribute that pollutes output.
- **Client-side AST mutation + optimistic preview.** They sync the app into the
  browser as a Babel/SWC AST, mutate it locally, and propagate to the DOM
  *optimistically* before any save — so edits feel instant, no network round-trip.
  On save they regenerate "clean, standard-compliant JSX/TSX" from the AST. *Our
  analog:* optimistic-local-IR mutation for instant canvas feedback, with the
  debounced `codegen_signature`-gated splice as the authoritative write (Phase 4).
- **Two-mode authoring: Visual Edits (no prompt, instant) vs. Plan/Chat
  (describe-it).** Visual Edits handle text/size/color/spacing directly on the
  canvas with zero prompt; Plan Mode handles broad/structural changes via
  conversation. The **Select feature** lets you highlight one component so an AI
  edit is scoped to it (less token use, no collateral damage elsewhere).
  *Lesson:* direct manipulation for the 80% of precise tweaks; reserve the AI/
  conversational path for structural changes — and always *scope* AI edits to a
  selected node + its span so a model can't rewrite the whole file. For us this
  is the safe way to add an optional AI assist later without surrendering the
  round-trip invariant.

**Webflow (the deepest, most honest layout model).**
- **Real flexbox/grid as first-class, not absolute positioning.** You build with
  flex and CSS grid and style each element; the canvas teaches the box model
  instead of hiding it. This is the *opposite* of Anima's div-soup.
- **Smart guides + grid overlay**: snap lines appear on drag, toggleable light/
  filled overlays, alignment hints — the canvas affordances that make pixel work
  feel precise.
- **Component Canvas (2024–25)**: a dedicated workspace where **all variants of a
  component are visible side-by-side**, so a change to the base cascades visibly
  across every variant in one view. This is the best variant-authoring UX in the
  category — you *see* the cascade.

**Framer (the best responsive model).**
- **Stacks = a visual Flexbox** (auto-arrange, gap/padding/direction/align/
  distribute as direct controls); **stacks wrap** so a row reflows to a column on
  narrow screens — "90% of responsive for free."
- **Breakpoints are visual variants** (desktop-first, 1200px default, changes
  cascade *down* unless overridden) — not hand-written media queries. You design
  the breakpoint, you don't write the query.
- **Stack Variables**: gap/padding/direction become *component props*, so one
  component flexes instead of needing five near-duplicate variants.

**Figma Dev Mode + Code Connect (the best design-token + design-system handoff).**
- **Inspect panel surfaces token *names*, not raw values**: a fill shows as
  `--color-action-primary` (the var), ready to paste — so the design system, not a
  hex literal, is the unit of edit. This is the model for *our* `@theme` ramp +
  Fluent tokens: the inspector should let you bind a color to a **token**, never
  poke a raw hex (raw hex is the div-soup of color).
- **Code Connect links a Figma component to its real coded component**, so the
  inspector shows *your* production snippet, not auto-generated markup. The lesson
  for us: the palette/inspector must reference our **real Fluent component set**
  and the project's **real Motoko surface**, never invent throwaway markup.
- **MCP feeds structured component/variant/token data to AI assistants** — i.e.
  the AI edits against *structured* design data, not a screenshot. If we ever add
  AI assist, it must read the AST + IR + project surface, never pixels.

**Builder.io Visual Copilot (the best "bring your own components" + sync story).**
- **Component Mapping**: a CLI maps a design's components to *your existing code
  components* before export, so generated output *uses your design system* instead
  of regenerating primitives. Directly analogous to making our palette emit
  **Fluent components + project Services/Models**, never anonymous `Group`s.
- **Round-trip sync that preserves code edits**: when the design changes it
  "intelligently syncs while preserving code edits." This is the bar for our
  hand-edit ↔ canvas-edit coexistence; our `codegen_signature` oracle is how we
  actually *prove* a sync didn't change semantics, which is stronger than "smart."

**Plasmic (the best round-trippable codegen contract — steal the file-split idea).**
- **Two-file split: `PlasmicButton.tsx` (generated, presentation, regenerated on
  every sync, owned by the tool) + `Button.tsx` (developer-owned wrapper,
  behavior/state/handlers, never clobbered).** Behavior attaches via variants,
  slots, and named-element *overrides* (e.g. `root={{ onClick: handleClick }}`).
  Design updates never destroy logic because they live in different files.
  *This is the single most important architectural idea in the cluster for us,*
  and we already have the *philosophy* (presentation in `.mview`, behavior in
  `@code` Motoko). The refinement to steal: make the **boundary explicit and
  enforced** — the designer edits the presentational `.mview` markup + attribute
  spans; it **never** edits the bodies of `@code` Motoko functions (only
  scaffolds/links them). The double-click gesture *crosses* the boundary (jumps
  you from presentation into behavior) without the designer ever *owning*
  behavior. That clean ownership line is what makes round-trip safe.
- **Register code components**: you hand Plasmic the real component + its prop
  metadata, and it generates the right-panel controls automatically. *Our analog:*
  drive the inspector's control set from the Fluent component's known
  attributes + the project scan — the property grid is *generated from the real
  schema*, not hand-curated per control.

**Anima (the cautionary tale — what NOT to do).**
- Converts Figma to **absolute-positioned divs with hardcoded pixels**
  (`top: 42px; left: 130px`), flattening everything to generic `<div>` with no
  component boundaries, no reuse, no token connection. It looks right in preview
  and is unmaintainable the instant the screen size changes. **This is the
  div-soup failure mode in its purest form, and it is structurally impossible for
  us** — because our canvas does not emit a separate document; it splices into the
  *real* `.mview` that a human would write, and there is no absolute-position
  primitive to fall back to. Our entire architecture is the anti-Anima.

### 11.3 The DX moves to steal, RANKED

1. **(P2, keystone) Node→span stable identity = click-to-source.** Lovable's
   stable-ID plugin and Figma's Code Connect both prove this is the foundation.
   We have it designed (the side-map). Without it there is no authoring. Rank #1
   because everything else is built on it.
2. **(P3, the headline gesture) Double-click → jump-to/scaffold the Motoko event
   handler.** This is the VB4 move the whole cluster mostly *lost*, and it's the
   one that makes "design + behavior in one place" visceral. Highest *delight*-
   per-effort: it's a span jump + a stub-scaffold splice on top of the bridge we
   already need.
3. **(P3) Property grid generated from the real schema + token-bound, never raw.**
   Plasmic's auto-generated controls + Figma's token-name inspect. Bind colors/
   spacing to `@theme`/Fluent tokens; raw-hex is a smell. The grid shows the
   node's *real* attributes, nothing hidden.
4. **(P3/P4) Optimistic-local-IR edit + debounced authoritative re-render.**
   Lovable's client-side-AST optimism is what makes edits feel instant; our
   `codegen_signature` gate is the authoritative truth. This is the latency
   battle that decides whether "F5" feels like VB.
5. **(P4) Stacks/auto-layout as the default container + wrap-to-responsive.**
   Framer's stack model is the best responsive UX in the category and avoids
   absolute positioning by construction. Map our `Group` container to a
   flex/stack with direct gap/padding/direction/align controls.
6. **(P3) Side-by-side variant view (Webflow Component Canvas) for our secure
   primitives' states.** Seeing the cascade across states (empty/error/encrypted/
   loading) in one view is the best variant DX; especially valuable for showing a
   secure-form's locked vs. unlocked vs. lint-failing states.
7. **(P4) Bring-your-own-components mapping (Builder.io) = palette is Fluent +
   project surface, never anonymous markup.** Guarantees output isn't div-soup
   because there are no anonymous primitives to soup.
8. **(later, optional) Scoped AI assist that edits the *selected node's span***
   *only* (Lovable Select). If we ever add AI, scope it to a span + feed it the
   AST/IR/project surface (Figma MCP model), and route its output through the
   *same* `codegen_signature` gate. AI never gets to bypass the round-trip oracle.

### 11.4 Why our canvas is *better-positioned* than all of them on fidelity

Every tool above fights one war: **the canvas is a different artifact than the
shipped code, so they spend enormous effort keeping a projection in sync** (stable
IDs, two-file splits, "intelligent" sync, round-trip reconciliation). **We do not
have that war.** Our canvas renders the *live UI-IR forest* — the exact same
`Ir.mo` forest that ships to web/iOS/Android/egui via `motoview preview --emit ir`
(`apps/studio/native/src/app.rs::render_forest`/`render_node` already does this with
Fluent widgets). So:

- **The canvas is pixel-consistent with what ships by construction** — it *is* the
  renderer, not a Figma-flavored approximation of it. No tool in the cluster can
  say this; Framer/Webflow render their own engine and hope the export matches.
- **There is no separate document model to drift.** Edits splice the real
  `.mview`; the only mapping artifact is the side-map, validated byte-for-byte by
  `codegen_signature`. Plasmic needs a two-file split to protect logic; we get the
  same protection *for free* because behavior already lives in `@code` Motoko that
  the designer never edits, only links/scaffolds.
- **Div-soup is structurally impossible.** There is no absolute-position primitive
  and no anonymous-`<div>` emission path; the palette emits known `WidgetKind`
  nodes / Fluent components / project surface bindings. The Anima failure mode
  cannot occur.
- **Multi-platform preview is free.** Because the canvas renders the portable IR,
  the same design previews natively on iOS/Android/egui with no per-platform
  export step — the thing Builder.io needs a per-framework generator for.

The honest gaps (already tracked in §3.4 and the roadmap): the generic
`render_node` path is thin today (the canvas special-cases the CRM Kanban in
`render_forest`), `moc -r` preview latency must be hidden behind optimistic
mutation, and `@for`/`@if` template-vs-instance authoring is the genuinely hard
part the pretty builders punt on.

### 11.5 Anti-patterns to refuse (each maps to a concrete tool failure)

- **Div soup / absolute positioning (Anima).** Never emit anonymous positioned
  divs. Containers are stacks/groups with real layout; the palette only emits
  known component kinds. *We are structurally immune; keep it that way — never add
  an "absolute position" escape hatch.*
- **Non-round-trippable export (Anima, naive Figma-to-code).** Code that looks
  right in preview but can't be hand-edited or re-imported. *Our `codegen_signature`
  oracle + side-map make every edit round-trippable by proof, not by hope — never
  ship an edit path that bypasses the oracle.*
- **Designer-owns-formatting / a parallel document model (most WYSIWYG tools).**
  If the designer reformats or owns the source, hand-authored `.mview` drifts and
  the two artifacts diverge. *The text buffer is the single writer; canvas edits
  go through it as minimal whitespace-preserving splices; the designer must never
  normalize formatting.*
- **Raw-value styling instead of tokens (the color div-soup).** A property grid
  that pokes hex/px literals everywhere. *Bind to `@theme`/Fluent tokens; raw
  values are an explicit, flagged escape, not the default.*
- **Hidden designer-only props / ghost state.** A property grid that shows
  attributes the code doesn't have (or hides attributes it does). *The grid is
  generated 1:1 from the node's real AST attributes — full honesty, no ghosts.*
- **Unscoped AI rewriting the whole file.** The Lovable lesson, inverted: an AI
  edit that isn't scoped to a selected span can clobber unrelated, hand-authored,
  *security-critical* code. *If AI is ever added, it edits a span and passes the
  same gate; it can never touch the `secure`/encrypt/`authorizeSpend` tokens the
  lint enforces.*
- **Letting the inspector weaken a security default.** None of the web builders
  have this concern; it's ours alone. *The inspector cannot un-`secure` a mutating
  form or un-encrypt a Private field — those toggles simply do not exist; the lint
  badge is live, not a late build failure.*

### 11.6 Net: the DX north star in one sentence

**A VB4-fast loop — toolbox, property grid, double-click-into-the-handler, instant
F5 — rebuilt on a canvas that renders the *real shipping UI-IR* (so it's pixel-
true and div-soup-proof by construction), where every edit is a proven-equivalent
splice into the one source-of-truth `.mview`, behavior lives in Motoko the
designer links but never owns, styling binds to tokens not raw values, and the one
thing no competitor can copy — secure/private-by-construction primitives in the
toolbox — rides on top.**

---

## 12. Classic RAD Direct-Manipulation Lineage — Deep Dive (VB / Delphi / WinForms / Interface Builder / Android)

> Added 2026-06-10. Companion to §11. Where §11 is breadth across *modern* web
> builders, this section goes deep on the **classic RAD direct-manipulation
> lineage the project owner explicitly cites as the gold standard** — the VB4/VB6
> form designer, Delphi, Windows Forms, Xcode Interface Builder, the Android
> Studio layout editor. The reason to dwell here: the *signature gesture* of this
> whole lineage — **double-click a control to auto-scaffold its event handler and
> jump into the code** — is the headline DX move for our Phase 3, and the precise
> mechanics of *why it felt instant* are the spec for getting our analog right.
> It also catalogs exactly **what RAD got wrong** so we can refuse those traps by
> construction.

### 12.1 Why VB-class RAD was the most productive UI tooling ever shipped

The VB form designer collapsed design→code→run into one window with **five
tightly-coupled mechanics**. The productivity is in the *coupling*, not any single
part — pull one out and the loop slows down:

1. **The Toolbox — a closed, curated tray of *real* controls.** A vertical strip
   of icons (Pointer, Label, TextBox, CommandButton, ListBox, Timer, …).
   **Double-click a Toolbox item** → it drops a default-sized control at a default
   location, already selected. Or **drag-rubber-band** it to size-as-you-place.
   The thing on the form is the *actual runtime control*, not a placeholder —
   WYSIWYG was literal. The set was *small and learnable*; every item was
   guaranteed to render; there was no blank-canvas paralysis.

2. **The Property grid (F4) — one inspector for everything, with typed editors.**
   Select a control → a two-column name/value grid shows *all* its properties
   (alpha or categorized). Crucially the value cells are **typed editors**: a
   color swatch + picker for `BackColor`, an enum **dropdown** for `BorderStyle`,
   a font dialog for `Font`, a `True`/`False` **combo** for booleans. You never
   typed a property name or guessed a legal value. With nothing selected the grid
   edited *the form itself*. One mental model covered every object. A second tab,
   the **Events list (lightning-bolt)**, listed every event the control could
   raise; double-clicking a row there *also* scaffolded that handler.

3. **Direct manipulation with snapping/align.** Drag-to-move, drag-handles to
   resize, a dotted **alignment grid** controls snapped to, and (Windows Forms
   era) **snap-lines** that flashed when an edge/baseline aligned with a neighbor,
   plus an Align / Make-Same-Size / Center-in-Form toolbar. The hands did the
   geometry; you nudged.

4. **The signature move — DOUBLE-CLICK A CONTROL → JUMP INTO ITS EVENT HANDLER.**
   The whole reason VB felt instant (mechanics in §12.2). Double-click `Command1`
   and you are *inside* `Command1_Click()` with the caret blinking in an empty
   body — no dialog, no name decision, no wiring step.

5. **F5 — instant run, no build ceremony.** Press F5 and the app is *running* in
   the same window. No deploy, no build dialog in the mental model, sub-second.
   Edit-and-continue let you change code while paused. The full loop —
   *drop a control → set a property → double-click → type 3 lines → F5 → click it →
   watch it work* — was **seconds, not minutes**.

### 12.2 The signature transition, mechanically: how design↔code felt instant

This is the spec, so it is worth being exact about each tool. The common thread:
**the gesture's endpoint resolves directly to a code location, the stub is
generated with a canonical name + exact signature, and the caret lands in the
empty body — zero intermediate decisions.**

**VB6 (`Command1_Click`).** Double-click the button → IDE switches from the form
designer to the **code editor** → if no handler exists it inserts
`Private Sub Command1_Click()` … `End Sub` (name = `{controlName}_{defaultEvent}`;
default event chosen *for you* — Click for a button, Change for a textbox, Load
for the form) → caret is placed inside the empty body. The handler↔control wiring
is *implicit by the name convention* — there is no separate "subscribe" line to
forget. Re-double-click an existing handler just navigates to it (idempotent).

**Windows Forms (`button1_Click`).** Same gesture, but the wiring is **explicit
and machine-managed**: double-click generates
`private void button1_Click(object sender, EventArgs e) { }` in `Form1.cs` (your
file) *and* inserts `this.button1.Click += new EventHandler(this.button1_Click);`
into `InitializeComponent()` in the **`Form1.Designer.cs`** partial-class file.
Two files, one gesture; the designer owns the wiring line, you own the body. The
Events tab of the Property grid does the same for any non-default event. (The
cost of that split is the anti-pattern in §12.3.2.)

**Xcode Interface Builder (`@IBAction` / `@IBOutlet`) — drag-to-connect.** The
most spatially-elegant variant. Open the **Assistant Editor** (storyboard left,
controller source right). **Control-drag a line from the control directly into the
source code**. A popover asks *Connection: Outlet | Action*, you name it, click
**Connect**, and Xcode **inserts the declaration at the exact drop point**:
`@IBOutlet weak var titleLabel: UILabel!` for an outlet, or
`@IBAction func tapped(_ sender: UIButton) { }` (empty body) for an action. The
connection itself is archived in the **nib/storyboard**, not in your code. The
genius: **the gesture's *endpoint* is the code location** — you don't pick a name
from a menu, you drag the wire to the spot in the file where the method should
live. Ever after, a **filled dot in the gutter** means "connected," an **empty
dot** means "declared but unwired" — *connection state is visible in the editor*.

**Delphi (`Button1Click`) and Android Studio.** Delphi's Object Inspector +
double-click-to-`OnClick` is the VB model in Pascal. Android Studio's layout
editor is the same five mechanics: a **Palette**, an **Attributes pane**, a
**Component Tree**, a **split Design/Code (XML) view**, and drag-to-constrain in
ConstraintLayout; `onClick` is wired via an attribute, and (with view-binding /
the assistant) jumping to the handler is a click away.

**Why it felt instant, distilled:** (a) **no decisions on the hot path** — name,
signature, default-event, and wiring were all defaulted; (b) **one gesture crossed
the design→code boundary** — no file-open, no scroll-hunt; (c) **the caret landed
ready to type** in an empty body; (d) **idempotent** — repeating the gesture
navigated instead of duplicating; (e) **the loop closed with F5** in the same
window with no build ceremony. Every one of those is reproducible by us as a
deterministic, no-AI, no-network operation.

### 12.3 What classic RAD got WRONG — anti-patterns we refuse by construction

1. **Absolute pixel positioning (`Top`/`Left`/`Width`/`Height`).** VB6 forms were
   hard-coded coordinates. They broke on resize, on high-DPI, on localization (a
   longer German caption clipped), on any screen that wasn't the developer's.
   Anchoring/Docking were *retrofits* to paper over a wrong default. **We never
   emit coordinates** — layout is flow/stack/grid containers (`WidgetKind::Group`).
   There is no absolute-position primitive to fall back to (this is also §11.5's
   anti-Anima point, reached from the RAD side).

2. **A generated "DO NOT EDIT" region the designer regenerates (`Designer.cs`).**
   WinForms put the designer's output inside `InitializeComponent()` behind the
   comment *"do not modify the contents of this method with the code editor."* Two
   failures followed: (a) the designer **round-trips by re-parsing and
   regenerating** that region, so a hand-edit inside it was silently reverted —
   or, worse, threw a **design-time exception** and the form refused to open in the
   designer at all; (b) the region was a notorious **merge-conflict** generator and
   an unreadable diff. The designer and the human were *two writers fighting over
   one region.* **Our architecture is built precisely to avoid this:** there is no
   separate generated artifact — `.mview` is the single file a human reads and
   writes, visual edits are minimal splices that produce *exactly what a human
   would type*, and `codegen_signature` proves equivalence. There is no
   protected region, so there is nothing to fight over.

3. **Designer-time ≠ runtime fidelity.** Custom-drawn controls showed a gray box
   at design time; data only appeared at runtime; the designer routinely lied
   about the running app. **We dodge this by construction** — the canvas renders
   the *same UI-IR forest* that ships, via `preview --emit ir` (§11.4). Design-time
   *is* the runtime renderer.

4. **Event-handler sprawl with no path back.** Double-click created a handler, but
   deleting the control **orphaned** the handler, and there was no map from a
   handler back to "which control(s) reference me." Interface Builder partly fixed
   this with the gutter dot, but VB never did. **We do better:** the node→span
   side-map is bidirectional, so the inspector shows *which `@code` function a
   control's `@click` targets*, jump works both directions, and we can **lint
   orphaned handlers** and **broken wires** (`@click=foo` where `foo` is undefined).

5. **The opinion-free blank canvas.** Pure form designers let you place anything
   anywhere with no layout opinion → inconsistent, un-resizable, inaccessible UIs.
   Our palette carries an opinion (Fluent components, flow containers) *and* a
   security opinion (secure-by-default primitives) no RAD tool ever had.

6. **Magic-string event wiring (VB's name convention).** VB tied handler→control
   purely by the `Name_Event` string; renaming the control silently broke the
   wire. We make the wire **explicit and checkable** (`@click=fn` is a real
   reference the lint validates), getting the convenience without the fragility.

### 12.4 The MotoView analog — double-click a Button → scaffold + jump to its `@code` Motoko handler

The headline Phase-3 gesture, specified as **two `codegen_signature`-gated span
splices + an editor jump** — fully deterministic, no AI, no network, no deploy.
(This expands §11.1 loop #3 into an implementable contract; logic lives only in
`@code` Motoko, there is no application JavaScript anywhere in the flow.)

**Trigger (ship both, exactly like VB):** double-click the `Button` on the canvas,
**or** double-click the **Click** row in the inspector's Events tab.

**Deterministic sequence:**

1. **Resolve node → span** via the Phase-2 side-map → the button's exact `.mview`
   element span + its current attributes.
2. **Derive the handler name** the way VB derived `Command1_Click`: a stable,
   canonical name from the control's id/label + event (button labeled "Save" →
   `onSaveClick`), **collision-checked** against existing `@code` symbols — suffix
   `_2` on collision, **never** silently overwrite.
3. **Splice A — wire the event** at the button's span: insert/replace
   `@click=onSaveClick` (the explicit, lint-checkable wire; the analog of WinForms'
   `Click += …` but as one local attribute splice in the *same* file, not a hidden
   `InitializeComponent`).
4. **Splice B — scaffold the handler** in the page `@code` block: if `onSaveClick`
   does not exist, insert a **typed Motoko stub** with the framework's fixed
   event-callback signature, empty body + a `// TODO` marker (the analog of VB's
   empty `Sub`). If it *already* exists → **skip scaffolding, just jump** (the
   gesture is **idempotent** — re-double-click is safe, like VB navigating to an
   existing handler).
5. **Both splices are pre-validated by `codegen_signature`** before they touch
   disk, so the result is byte-identical to hand-authored source and the lint gate
   still runs.
6. **Jump the caret into the empty body.** In the egui studio: flip the right pane
   to source, scroll to the new handler's span, place the caret inside. This is the
   literal one-gesture design→code seam — and because `.mview` is *one* artifact,
   there is **no file switch and no generated-region boundary** to cross. Strictly
   better than WinForms.
7. **Re-render** via the preview path; the Events tab now shows an
   Interface-Builder-style **filled dot** on Click (filled = wired to `onSaveClick`
   with a jump-to-def link; empty = no handler).

**Why ours can feel *as instant as VB, then beat it*:** every step is an
in-process text operation — the only latency is `codegen_signature` (fast,
in-process) plus the preview re-render, which Phase 4 makes sub-perceptible via
optimistic local-IR mutation + debounced authoritative re-render. And we close
RAD's worst gaps: **no `Designer.cs` to fight** (one artifact), **no orphaned
handlers** (bidirectional side-map + orphan lint), **no magic-string fragility**
(the wire is an explicit checked reference), and **no absolute coordinates** ever.

### 12.5 Ranked: classic-RAD moves to steal (and exactly where)

| Rank | Steal (classic-RAD) | Source | MotoView mapping |
| --- | --- | --- | --- |
| 1 | **Double-click control → scaffold + jump to handler** — idempotent, canonical derived name, typed stub, caret-in-body | VB6 / Delphi / WinForms | Two `codegen_signature`-gated splices (`@click=fn` + `@code` Motoko stub) + caret jump via node→span side-map. §12.4. The headline Phase-3 feature. |
| 2 | **Drag-to-connect + connection-state-in-gutter** (filled/empty dot = wired/unwired; jump both ways) | Xcode Interface Builder | Bidirectional side-map → inspector dot per event + jump-to-def from canvas; lint orphaned handlers & broken `@click` wires. Phase 3–4. |
| 3 | **Property grid with *typed* editors, one grid for everything, no free text** (swatch/enum-dropdown/bool-combo; edits the page when nothing selected) | VB F4 grid | Inspector generated 1:1 from the node's `Attr`/`EventBind`/`bind`/`secure`; each row a typed editor → single-span splice. `secure` is a default with no off-toggle. Phase 3. |
| 4 | **Closed, curated Toolbox of *real* controls** (double-click-to-drop default + drag-to-size; every item guaranteed to render) | VB / Delphi Toolbox | Fluent + secure-primitive palette; secure variants are the *defaults* (drop "Form" → `secure`). Drop onto `WidgetKind::Group`. Phase 3. |
| 5 | **Default-event + default-name + default-wiring chosen for you** (no decisions on the hot path) | VB `Name_Event` convention | Derive `on{Label}{Event}`, default event = Click for button / submit for form; pre-pick so the gesture asks nothing. §12.4 step 2. |
| 6 | **Snap-lines / align / make-same-size, NEVER absolute x/y** | WinForms snap-lines | Affordances act on child order + container props, emitting `Group` structure, never coordinates. (Anchoring/Docking were a *retrofit* — we start where they ended up.) |
| 7 | **F5 = instant run, no build ceremony in the mental model** | VB F5 | No-deploy replay (`preview --emit ir`) flips the canvas live on one keystroke; keep deploy a *separate, explicit* step so the security/lint gate stays visible (we do NOT hide it to fake "instant"). |

### 12.6 Net for §12

The classic-RAD lineage already solved the *feel* — toolbox, typed property grid,
**double-click straight into the handler**, instant F5 — and it tells us exactly
which sins to refuse: **absolute coordinates, a regenerated DO-NOT-EDIT region,
designer/runtime fidelity gaps, orphaned handlers, and magic-string wiring.** Our
span-anchored `.mview`-is-truth architecture is, almost line-for-line, the RAD
loop *with every one of those sins designed out* — and with one toolbox category
(secure/private-by-construction primitives) that VB, Delphi, Xcode, and Android
never had a reason to invent.

---

## 13. Data/Logic/Binding DX — The Low-Code Data Platform Cluster (Research Synthesis)

> Added 2026-06-10. Field research into the **low-code data/logic platforms**:
> FlutterFlow, Retool, Bubble, Budibase, Windmill, Microsoft Power Apps — plus the
> current AI builder **Lovable**. Sections 11–12 cover canvas/inspector/component
> DX and the classic RAD lineage; **this section is their complement: how you BIND
> a list to a query, a form to a mutation, build LOGIC/WORKFLOWS, and scaffold CRUD
> from a data model.** The constant constraint: in MotoView, logic is **Motoko in
> `@code`** (no app JS, ever) and binding is a **PICKER over the real Motoko
> service surface** (`project.rs` scans Services/Models for query funcs → lists,
> async funcs → submits, record fields → inputs). The VB4 pillar that anchors this
> section is **double-click a control → drop into its event handler**.

### 13.1 How each platform does binding & logic (exact gestures)

- **Power Apps — formula bar + delegation underline (closest VB heir).** A
  Gallery's `Items` property is set in a **formula bar** with IntelliSense:
  `Gallery.Items = Filter(Notes, Owner = User().Email)`. Killer detail: when a
  formula can't be pushed to the backend, Studio draws a **blue delegation
  underline at authoring time** (it would silently operate on only the first 500
  rows) — a query-shape lint surfaced in the binding UI, not a runtime surprise.
  **"Start with data"** generates a **3-screen Browse/Detail/Edit** app from one
  Dataverse table in a single gesture. *Anti-pattern: Power Fx is proprietary and
  host-locked.*
- **FlutterFlow — Backend Query + "Set from Variable" + real Flutter export.**
  Select a list widget → Properties → **Backend Query** → *Query Collection* →
  pick collection, *List of Documents*, add `+ Filter` rows and ordering. On the
  child widget, **Generate Dynamic Children**: bind the *first* child's fields via
  a **"Set from Variable" tree picker** (query result → document → field) and the
  bindings **propagate to every generated row**. Crucially, it **exports clean,
  standard Flutter source (MIT/BSD helpers) you own and can build without
  FlutterFlow** — multi-platform output is *real code*, not a runtime. This is the
  model for "owned code + visual binding."
- **Retool — `{{ }}` everywhere + GUI-mode changeset writes.** Every property is a
  `{{ }}` expression: `table1.data = {{ getNotes.data }}`; row selection exposes
  `{{ table1.selectedRow }}`. Writes use **GUI mode**: pick *Insert/Update/Bulk
  upsert*, map a **changeset** (often `{{ form1.data }}`) to columns, set
  **Filter-by** (the WHERE clause) as key/op/value rows — *no SQL typed*. Event
  handlers are a dropdown of **actions** (Run query, Control component, Open URL).
  *Anti-pattern: no code export; bindings are untyped strings that fail at
  runtime.*
- **Budibase — Data Provider + lightning-bolt binding drawer (cleanest picker).** A
  **Data Provider** fetches rows; a child's **Data** dropdown lists in-scope
  providers. Bindable fields show a **lightning-bolt icon** opening a **Binding
  drawer**: a left list of **context bindings** (provider rows, current user, app
  state, URL params), **click-to-insert** a readable token (`{{ Notes rows }}`),
  with a **live preview pane** evaluating as you type. **Autogenerated CRUD
  screens**: pick table(s) → Table + side-panel create/edit, fully wired.
- **Bubble — When→action workflow (powerful, fully locked-in).** Repeating Group
  `Data source = Search for Notes (Owner = Current User)`. Logic is the **workflow
  editor**: **When** [event] → ordered **action steps** (Create thing, Display
  data, Navigate), each a form, data flowing via "Result of step N". *Anti-pattern:
  zero code export; logic runs only on Bubble's servers (runtime lock-in).*
- **Windmill — visual DAG where every node is REAL owned code.** The **flow
  editor** is a visual DAG, but **every step is real TypeScript/Python/SQL** with a
  typed `main()`; inputs wired from prior steps via a picker. Its app editor binds
  components to **runnables** (scripts/flows). Proof that *visual orchestration +
  real owned code per node* coexist — the exact stance behind our `@code`-is-Motoko
  rule.
- **Lovable — Visual Edits (the single most directly stealable mechanism).** Click
  "Edit" → **select any rendered element** → change text/color/size/Tailwind class
  **instantly, no AI prompt, no credits**. Mechanism = *our exact problem solved
  their way*: a **Vite plugin injects a stable unique ID into every JSX node at
  compile time**; a click **traces the DOM element back to the exact source JSX**;
  the edit is a **surgical AST splice in the browser (Babel/SWC), not regex and not
  full AI regeneration**; the IDs **survive later AI edits**. Structurally
  identical to our **node→span side-map + `codegen_signature`-gated splice** —
  independent validation that our architecture is right (and we add the semantic-
  equivalence oracle they lack).

### 13.2 The VB4 "double-click → handler" pillar, mapped to Motoko `@code`

This is the headline data/logic steal:

- **Double-click a node with an `@submit`/`@click` binding → open its Motoko
  function** in the studio editor (the linked Service func or the `@code` body),
  cursor on the body. VB4's `Sub Button_Click()`, but the handler is **owned
  Motoko, not BASIC and not JS**.
- **If no handler is bound yet,** double-click offers the **bind-picker** (pick an
  existing async/query func) OR **"Create handler"** — which splices a
  `@submit=newFn` binding *and* scaffolds a **typed `newFn` stub** into the
  Service/`@code`, then drops you in. The signature is inferred from context (the
  form's argument record).
- **Implementation:** extend the existing node→span side-map (§3.3) to also carry
  the **bound-function span**, so the jump lands in logic, not just markup. No new
  architecture — the same map that powers selection now powers the handler jump.

### 13.3 Binding DX mapped to our PICKER-over-Motoko-surface model

The `project.rs` Services/Models scan already yields the typed surface; the
designer turns it into pickers mirroring the best-in-class gestures:

- **List bind (FlutterFlow Backend Query + Budibase Data Provider).** Select a
  `@for`-capable node → "Data source" dropdown lists **query-shaped funcs**
  (`query func`, non-mutating, returning `[T]`/`vec`). Picking `notesFor(owner)`
  splices `@for note in notesFor(owner)`. The **row scope** (`note`) becomes a
  context the child inspector offers: `note.title`, `note.body` (record fields of
  `T`) — exactly Budibase's context-binding drawer + FlutterFlow's "Set from
  Variable" tree. **No `{{ }}` strings; a typed dropdown.**
- **Form → mutation (Retool changeset + FlutterFlow Backend Call).** Select a
  `secure` form → "On submit" picker lists **async/mutating funcs**; picking
  `addNote(input)` splices `@submit=addNote`. Each input gets a `bind="@field"`
  picker over the **record fields of the function's argument type** — so the form's
  field set is *derived from the Motoko argument record* (inverse CRUD).
- **Data-model-first scaffold (Power Apps "Start with data" + Budibase autogenerate
  + FlutterFlow collection).** Pick a Motoko **record type** + its Service →
  **generate a secure List + Detail + Create page set**, pre-wired: list `@for`s
  the query func, Create is a `secure` form whose fields are the record fields,
  Private-classified fields auto-carry `data-mv-encrypt`. One gesture, three
  screens, secure-by-default — Phase 4's scaffold upgraded to the universal 3-screen
  pattern.
- **The honest heuristic caveat (reinforces §3.5).** `async ≠ mutating`, and a
  query-named func can side-effect. Classify off **real signatures** (return type,
  `query` keyword, effect), never the name, or the picker offers a binding that
  *compiles but is semantically wrong*. Mirror Power Apps' **delegation underline**:
  if a chosen `@for` source is *not* a real `query func` (pays a full update-call
  per render), draw a live **"update-call" badge** on the node at authoring time.

### 13.4 Ranked — what to STEAL (highest leverage first)

1. **Double-click-a-control → jump to its Motoko handler (+ "Create handler"
   stub).** The VB4 heart, applied to logic. Reuses the side-map + `codegen_
   signature`; just adds a bound-function span to the map. *Highest leverage,
   lowest new risk.*
2. **Lovable Visual-Edits surgical-splice model as our UX + validation bar.** We
   already chose the harder/correct version (text-is-truth, span splice, semantic-
   equivalence gate). Steal the **UX promise**: instant, no-AI, no-credit, click→
   change→done edits that never trip the user. Their stable-ID Vite plugin == our
   side-map; their AST splice == our span splice; our extra is the oracle.
3. **Budibase lightning-bolt binding drawer (context list + click-to-insert + live
   preview).** Cleanest binding-picker UI in the cluster. Adopt the drawer shape
   but populate from the **typed Motoko surface**, not stringly-typed handlebars;
   keep the live preview (no-deploy replay already powers it).
4. **FlutterFlow "Generate Dynamic Children" + "Set from Variable" tree → `@for`
   row-scope field pickers.** The gesture for binding a list row's children to
   record fields; `@for note in …` makes `note.*` the typed scope.
5. **Power Apps delegation-underline → our "update-call / non-query" badge.** A
   query-shape lint in the binding UI at authoring time. Cheap, and it serves the
   security story (no accidental expensive/incorrect bindings).
6. **Power Apps/Budibase data-model-first 3-screen scaffold (Browse/Detail/
   Create), secure-by-default.** Biggest time-to-first-app win; folds into Phase 4.
7. **Retool GUI-mode changeset → argument-record-derived form fields.** The form's
   inputs are generated from the mutation's Motoko argument type; submit maps the
   form to the call. No SQL, no JS, fully typed.
8. **Windmill's "every node is real owned code" as the anchor for any future visual
   workflow view.** If we add a multi-step action view, each step is real Motoko in
   `@code`, never a proprietary block.

### 13.5 Anti-patterns to AVOID (the lock-in traps)

- **Proprietary logic blocks (Bubble/Retool/Power Fx).** Bubble and Retool have
  **no code export** — logic lives only in their format and runtime; Power Fx runs
  only on Microsoft's host. *Hard line: all logic is Motoko in `@code`, plain text
  the user owns. The designer must never invent a visual logic block with no
  faithful Motoko source — if a gesture can't round-trip to readable Motoko via
  `codegen_signature`, it doesn't ship.*
- **Runtime lock-in (Bubble/Retool run only on vendor servers).** *Ours runs as the
  user's own canister + native binary; the designer is a tool, not a host.*
- **Stringly-typed binding expressions (`{{ }}` everywhere, Retool/Budibase).**
  Untyped, fail at runtime. *We have a typed Motoko surface — bindings are PICKERS
  validated against real signatures, with the semantic-equivalence gate, so a
  binding that compiles-but-is-wrong is caught at authoring time.*
- **Name-based query-vs-mutation heuristics.** Offering any `async` func as a "list
  source" guarantees wrong bindings. *Classify off real signatures/effects.*
- **AI-regenerates-the-whole-component for a small edit (Lovable's pre-Visual-Edits
  default).** Costly, non-deterministic, drifts hand-edits and could clobber
  security-critical tokens. *Our span-splice + `codegen_signature` is the
  deterministic antidote.*
- **A separate canvas document model that "emits" code.** How Webflow/Framer/Bubble
  drift from real code. *Already rejected in §3.1: text is truth, canvas is a
  projection. Do not regress.*
- **Punting on `@for` template-vs-instance and `@if`/`@switch` branches.** With a
  typed list scope we cannot punt (budgeted in Phase 4).
- **A green "bound/secure ✓" that hides cost or exposure.** Mirror the Privacy
  Ledger discipline: show the update-call cost badge and the secure/encrypted state
  truthfully, never a comforting checkmark that hides a round-trip or metadata leak.

### 13.6 Net for the roadmap

Folds into existing phases with no new architecture. **Phase 3** gains the
Budibase-style binding drawer (typed pickers, live preview), the Retool-style
changeset form (inputs derived from the mutation's argument record), and the
**double-click → Motoko-handler** jump (extend the side-map to carry the bound-
function span). **Phase 4** upgrades "from-data-model scaffold" to the Power Apps/
Budibase **3-screen Browse/Detail/Create** pattern and adds the **delegation-style
update-call badge** to the bind-picker. **F5 = no-deploy replay** is a keybinding
over an already-working path. Every steal lands on the established chokepoint; every
avoided anti-pattern is a lock-in trap we are **structurally immune to because logic
is owned Motoko and binding is a typed picker, not a proprietary block.**

---

## 14. The AI-Native Builder Cluster (Lovable-primary) — the "vibe" loop, on secure rails

> Added 2026-06-10. Companion to §11 (WYSIWYG/design-system cluster:
> Webflow/Framer/Figma/Plasmic/Anima) and §12 (classic RAD lineage). This section
> is the **AI-native / "vibe coding" cluster** — **Lovable (primary, per the
> brief), v0, Bolt.new, Replit Agent, Cursor** — researched against their own
> docs/blog and 2025–2026 third-party reviews (sources at end). §11 asks "what
> makes the *canvas* feel precise?"; §12 asks "what was the RAD loop?". This asks
> the orthogonal question the brief poses: **"what makes the first 90 seconds feel
> like *magic* (prompt → running app), how do they make iteration fearless, and
> where does that magic become a *liability* we can turn into our headline?"** The
> cluster shares our node→span keystone but draws the opposite security lesson:
> their momentum is real, and their absence of secure defaults is catastrophic in
> a way only MotoView can fix.

### 14.1 The prompt-to-app loop and the "vibe" feel — exactly what creates the magic

The dopamine is real; dissected into discrete, stealable mechanics (most of which
require **no LLM in the common path** — the key insight):

- **Type a sentence → a running, navigable app in ~60s.** Lovable spins up full
  UI + Supabase DB + auth + (sometimes) payments from one prompt; Bolt does it
  *in-browser* via **WebContainers (Node in the tab)** — no remote build, no cold
  start — the single fastest prompt→preview path. The magic is *time-to-first-
  running-thing*, not output quality.
- **The preview IS the workspace.** No separate "design file"; you edit the live,
  hot-reloading app (Vite HMR for Lovable, WebContainer for Bolt); edits appear
  without a page reload. §11.4 already notes our canvas *is* the renderer — same
  property; lean on it as "your preview is the real native app, not a mockup."
- **Conversational momentum.** Each turn builds on the last (the agent remembers
  context); v0 lets you **select a region and re-prompt** for a *targeted patch,
  not a regen*; Lovable lets you **queue/reorder prompts** while the agent works,
  so you pipeline ideas instead of waiting. Feels like "pairing with an instant
  full-stack engineer."
- **Voice/mobile capture (Lovable iOS/Android, Apr 2026):** capture an app idea by
  voice the moment it occurs. Pure top-of-funnel momentum.

**Honest read for MotoView:** the magic is *momentum + reversibility*, not AI per
se. Our no-deploy IR replay (F5, §11.1/§12) already gives the running-app loop
*without a cloud build* — structurally faster than Lovable/v0 (server round-trip)
and on par with Bolt's in-tab WebContainer, but producing a **real native binary
path** instead of a browser sandbox. We get the magic from determinism, not a
model.

### 14.2 "Select an element and tell it what to change" — the central gesture, dissected

The brief's named gesture and the cluster's signature move. Mechanics:

- **Lovable's preview toolbar = four layered modes over one selection:** (1)
  **Select element → describe in English** (attaches the element as chat context;
  patches only its JSX); (2) **inline text edit** — double-click, type — **free,
  no AI, no credits**, capped 100/day; (3) **draw annotation** — sketch a box/arrow
  as a spatial reference for the prompt; (4) **pin a comment** to an element.
  Multi-select with ⌘/Ctrl-click edits several at once.
- **The enabling tech is exactly our keystone:** a **custom Vite plugin stamps a
  unique, stable ID on every JSX element at compile time**, so a clicked DOM node
  maps back to the exact JSX span — **bidirectional, deterministic**. Select-and-
  talk is *worthless without the span map*, which independently validates that our
  node→span side-map is the literal prerequisite, not a nice-to-have.
- **Property tweaks are model-free.** Color/spacing/font are applied **client-side
  via AST manipulation + Tailwind generation, with NO AI round-trip** — Lovable
  built this explicitly to *cut token spend*. v0 applies region re-prompts as
  targeted patches. Replit's **Design Mode** goes further toward a property grid:
  intuitive controls for **padding / text-color / background**, not English.

**The inversion MotoView must make (the §14 thesis):** in Lovable, select-and-talk
is the *primary* path and deterministic tweaks are the *fast exception*. **We flip
it.** The **property grid + single-span splice is primary** (precise, typed, free,
instant, validated by `codegen_signature`); **select-and-describe-in-English is
the optional accelerator** that *emits the same span splices a human would* and
**must pass the secure/private lints + the codegen oracle before write**. Same
gesture, same feel — but the AI is a *suggestion engine that proposes splices*,
and the chokepoint is the *gate it cannot route around*. That is how MotoView gets
the vibe-coding feel without vibe-coding insecurity.

### 14.3 Fearless iteration — error self-healing, rollback, "you own the code"

The cluster's second magic: **mistakes don't hurt.** Three copyable mechanics:

- **Free auto-repair.** Lovable's **"Try-to-fix" button on an error costs NO
  credits** and runs a repair pass before you spend anything. Its **Build/Agent
  mode** can *inspect logs, runtime output, and network activity and iterate until
  resolved*, validating with browser/frontend tests. Replit/Bolt hand the agent
  the terminal+console so it sees real errors. *Map to MotoView:* a free,
  deterministic "fix" = re-run lint + `codegen_signature` and surface the *named,
  located* error (our lints already do this); an AI accelerator's repair output
  goes through the same gate. Crucially, **a "fix" must never silently broaden the
  splice region** (the collateral-regression failure, §14.4).
- **Lossless rollback / time-travel.** Lovable: preview any earlier working version
  and revert; **nothing after it is lost — it stays in the chat and can be
  reapplied.** Replit: **checkpoint-based rollback.** *Map to MotoView:* trivially
  cheap — `.mview` is plain text under git; studio history is git over the text
  buffer. Market it as "every edit is a commit you can preview and reapply."
- **You own the code / two-way GitHub sync.** Lovable: every AI edit is a GitHub
  commit; a push from Cursor/VS Code **syncs back into Lovable in seconds** — so
  Lovable is the rapid loop and **Cursor is the escape hatch for the hard 20%.**
  v0 has a Git panel (branch/PR); Bolt exports; Replit has a full IDE. *Map to
  MotoView:* we already win — `.mview` + generated `main.mo` are plain files, no
  proprietary doc model; "you own the Motoko" markets exactly like "you own the
  React." The Lovable↔Cursor handoff is our model for *MotoView studio ↔ hand-
  editing `.mview`/`@code` in any editor*, kept honest by the single-writer text
  buffer (§3.4).

### 14.4 Where the magic becomes a LIABILITY — ranked, and each is our opening

1. **NO security or privacy — catastrophic, not cosmetic. The headline.**
   **CVE-2025-48757 (CVSS 8.26):** a researcher scanned Lovable's *own public
   showcase* and found **170+ apps (10.3%) with missing/broken Supabase
   Row-Level-Security** — exposing emails, phone numbers, **home addresses**,
   payment status, personal debt amounts, Stripe endpoints accepting param
   injection, and **API keys bundled straight into the frontend**. A **Feb-2026
   re-audit found 170+ STILL exposed.** Kill shot: **Lovable 2.0's "security scan"
   only checked that RLS *existed*, not that it *worked* — a green badge that was a
   lie**, manufacturing false confidence. Root cause is **structural and unfixable
   in their model:** an AI optimizing for *speed-of-first-functionality* skips
   exactly the security defaults, because secure config is the boring part a
   speed-optimized generator omits. (Veracode 2025: **45% of AI-generated code
   fails basic security tests**; Georgia Tech tracked 35 CVEs traced to AI-
   generated code in March 2026 alone.) **This is the entire reason MotoView
   exists, stated by their own incident data.** Our answer is *structural, not a
   scanner*: the **secure-form lint is a hard build Error that aborts build AND
   preview** (REAL TODAY, `lint.rs` + `project.rs`), **secrets never reach a
   client** (no application JS bundle to leak an API key into), and the **backend
   is the canister, not a misconfigurable SaaS**. The badge can't lie because it
   reflects the *enforced token path*, not a keyword's presence; the Privacy Ledger
   shows real ciphertext-vs-metadata (§5).
2. **Precision/fidelity collapses in the last 20%.** Universal across reviews: all
   tools nail ~80% of layout; the final 20% needs repeated prompting that
   *degrades code quality*; screenshot/English edits produce "generic"
   approximations; you cannot pin "this padding is exactly 16px and must stay."
   *Cure (§11):* typed property grid + token-bound values + a deterministic,
   proven-equivalent splice. The thing they can't do, we do by construction.
3. **Collateral regressions.** "Change this button" can mutate unrelated code
   because the model rewrites broad regions. **MotoView is structurally immune:** a
   splice is bounded to one AST span and *rejected by `codegen_signature` if it
   perturbs anything else* — an edit can never silently break a `secure`/encrypt/
   `authorizeSpend` token elsewhere in the file.
4. **Credit/token anxiety.** Users report paying for the same failed bug-fix
   **three times** and **"5× the credit burn over time."** Root cause: routing
   trivial edits through a model. Our model-free splices + local IR replay remove
   this failure mode for the designer's core gestures (no per-edit token cost).
5. **Backend = a third-party SaaS you must configure correctly** (Supabase RLS,
   secret placement, security headers) — left to the user to retrofit. We have no
   such surface: secure is the default and *unremovable*.
6. **Autonomy that ignores guardrails (Replit/Lemkin).** The agent "ignored
   explicit production-safety instructions" and altered things it shouldn't. *Our
   rule:* the AI accelerator is **never a deploy actor** — it proposes splices; the
   human + the lint gate decide; **no agent touches a canister.**
7. **Lock-in gradient.** Base44 (proprietary backend, limited export) > v0 (Vercel
   ecosystem) > Replit (infra) > Bolt/Lovable (low; portable code). We sit at the
   *lowest* end: plain files, no proprietary runtime.

### 14.5 What to STEAL from the AI cluster, RANKED (complements §11.3)

1. **Select-element → optional describe-in-English accelerator** that emits the
   *same validated span splices* a manual edit would and **passes the secure/
   private lints + `codegen_signature` before write** (§14.2 inversion). *Maps to:*
   an LLM that outputs candidate splices → existing validate-before-write pipeline
   (`fmt.rs::codegen_signature`, `lint.rs` Error gate, the node→span side-map).
   **The single biggest "vibe" win available to us — and safe *only* because of the
   chokepoint.**
2. **Free, model-free, deterministic edits for the common case** (inline text,
   color, spacing, bind) — Lovable's no-credit inline-text + client-side property
   tweaks. *Maps to:* Phase 2/3 single-span splices; we have the safer version
   (proven-equivalent, not best-effort).
3. **Lossless time-travel + free "Try-to-fix."** Preview/reapply any version;
   error-repair that costs nothing. *Maps to:* git over the text buffer + re-running
   lints/oracle with named errors.
4. **Two-way "you own the code" sync (Lovable↔GitHub↔Cursor).** Studio ↔ hand-edit
   `.mview`/`@code` in any editor, single-writer buffer keeps it honest. *Maps to:*
   §3.4 single-writer model; already a structural win.
5. **Plan / Build / Chat mode separation** + queued, reorderable prompts you can
   pipeline while the agent works. *Maps to:* studio command surface (the AI
   accelerator's UX shell, kept off the deploy path).
6. **Prompt/voice/screenshot as a *scaffold seed*** (Lovable mobile, v0
   image-to-UI) → feed our **from-data-model scaffold** (Phase 4): "build me a
   secure CRUD for this Motoko record" → a secure-by-construction list/detail/
   create screen. *Maps to:* Phase 4 scaffold + Services/Models scan.
7. **Draw-annotation / pin-comment** as spatial context for the accelerator.
   *Maps to:* canvas overlay feeding the suggestion engine.

### 14.6 Anti-patterns to REFUSE (each maps to a concrete AI-cluster failure)

- **The security-scan-that-checks-existence-not-correctness (Lovable 2.0).** Never
  a green badge that means "the keyword is present." Our lints check the *enforced
  token path* and abort the build; the badge means "the control is wired," never
  "the field is named right." Most important refusal in the doc.
- **Routing trivial edits through an LLM** (credit-rage + latency + collateral
  regression). Deterministic splices first; AI only for genuine generation/refactor.
- **Broad-region regeneration that mutates unrelated code.** Splice-to-one-span +
  `codegen_signature` gate is the structural prohibition; never relax it for the
  accelerator.
- **Backend-as-misconfigurable-SaaS** ("remember to turn on RLS"). Secure is the
  default and unremovable; there is no frontend bundle to leak a key into.
- **Autonomy that can act on production** (Lemkin). The accelerator proposes; it
  never deploys; no agent touches a canister.
- **English-only editing with no typed inspector** (Lovable's primary path). The
  property grid (§11) is the precision layer the AI cluster is missing; AI is the
  accelerator *on top*, never the only way in.

### 14.7 Net, in one sentence

**MotoView should feel like *VB4 with a chat box* — the §11/§12 toolbox / property
grid / double-click-into-the-Motoko-handler / F5 loop, plus the AI cluster's
momentum (prompt-to-scaffold, select-and-talk, lossless time-travel, you-own-the-
code) — where the optional AI accelerator emits the *same proven-equivalent span
splices* a human would and the deny-by-default secure/private lint is the wall it
cannot route around, turning Lovable's 170-app, badge-that-lies security failure
into our single most defensible headline.**

**Sources (researched 2026-06-10):** Lovable Visual Edits blog/docs
(`lovable.dev/blog/visual-edits`, `docs.lovable.dev/features/design`,
`docs.lovable.dev/features/agent-mode`, `docs.lovable.dev/introduction/faq`,
`docs.lovable.dev/integrations/github`); v0 (`v0.app`, Vercel announcing-v0 blog);
Bolt.new / StackBlitz WebContainers (`github.com/stackblitz/bolt.new`); Replit
Agent 4 + Design Mode + Import (`replit.com/products/design`,
`blog.replit.com/import`); cross-tool comparisons (altar.io
Lovable-vs-Bolt-vs-v0-vs-Replit-vs-Base44; annaarteeva Medium prototyping-stack;
EPAM "best vibe coding tools" real-design test; vibecodingacademy 2026; getmocha
2026); security-failure analysis (CVE-2025-48757 writeups at superblocks.com,
vibeappscanner.com, ptkd.com, byteiota.com; VibeEval Feb-2026 re-audit; Veracode
2025 GenAI Code Security Report — 45% fail basic security tests).

---

## 15. THE UNIFIED DX MODEL — "Best of All App Builders," on One Span-Anchored Source

> Added 2026-06-10. §§11–14 each studied one cluster (modern WYSIWYG, classic RAD,
> low-code data platforms, AI-native vibe coding). This section is the **synthesis
> the brief asks for**: one coherent, opinionated DX where AI-draft (Lovable-style)
> + direct-manipulation RAD (VB4: toolbox / property grid / double-click-to-handler
> / F5) + a **real native live preview** + **secure-by-construction** all act on the
> **same `.mview` source of truth** through the **same `codegen_signature` gate**.
> Nothing here is a new architecture — every move below lands on the existing
> chokepoint (parser AST + spans, `fmt.rs::codegen_signature`, the IR canvas, the
> `project.rs` Services/Models scan, the `lint.rs` Error gate, `@code` Motoko). The
> point is to state the *combined* experience precisely and to be specific about how
> the four lanes never collide.

### 15.1 The thesis in one paragraph

Each cluster proved a different half of the DX and shipped a different fatal flaw.
The AI cluster proved the *feel* (prompt → running app in 60s, select-and-talk,
lossless time-travel, you-own-the-code) but ships insecure-by-default with a badge
that lied (CVE-2025-48757). Classic RAD proved the *zero-ceremony loop* (toolbox,
typed property grid, double-click-straight-into-the-empty-handler, instant F5) but
shipped absolute coordinates and a regenerated DO-NOT-EDIT region. Modern WYSIWYG
proved the *fidelity machinery* (stable IDs, token-bound styling) but burns its
budget keeping a canvas in sync with code it isn't. Low-code proved the *binding
gestures* (picker drawers, changeset writes, 3-screen scaffolds) but trapped logic
in a proprietary runtime. **MotoView already owns the two pieces that fix all four
flaws at once**: a real parser AST with a *byte-exact semantic-equivalence oracle*
(`codegen_signature`) for precision and zero collateral regressions, and a
*deny-by-default secure/private chokepoint* so the badge cannot lie. The unified DX
is therefore not an invention — it is **VB4's zero-ceremony loop, with Lovable's
momentum bolted on as an optional accelerator, rendered on a canvas that IS the
shipping UI-IR, where every edit (human drag OR AI suggestion) is the *same*
proven-equivalent span splice into the one `.mview`, and the secure lint is the
wall neither can route around.**

### 15.2 The four lanes and the ONE chokepoint they all pass through

The defining structural decision: **there are four ways to author, and exactly one
way an edit reaches disk.** This is what lets us combine clusters that elsewhere
fight each other.

```
  Lane 1: Direct manipulation   ─┐
  (drag/drop/inspector/dbl-click)│
  Lane 2: AI accelerator         │   candidate
  (select → describe-in-English) ├── span ───► [ codegen_signature gate ] ──► [ lint.rs Error gate ] ──► WRITE .mview ──► re-render IR canvas
  Lane 3: Hand-edit              │   splice         (byte-identical             (secure/private
  (.mview/@code in any editor)   │                   codegen proof)              deny-by-default)
  Lane 4: Scaffold               ─┘
  (data-model → 3 screens; prompt seed)
```

- **All four lanes emit the identical artifact: a minimal text splice at an AST
  span.** A drag, an AI suggestion, a hand-edit, and a scaffold are
  indistinguishable downstream — they are all candidate `.mview` byte-ranges.
- **The gate is non-negotiable and shared.** `codegen_signature(before) ==
  codegen_signature(after)` must hold for an *edit*-class splice (proves the change
  is semantics-preserving / structural-only where intended), and the `lint.rs`
  Error gate (secure-form, private-field) must pass for *any* write. Confirmed in
  `fmt.rs::format_source` (lines 144–159): a candidate that changes codegen or
  breaks parsing is **rejected and the original kept** — this is literally the
  mechanism the designer reuses per edit.
- **The single-writer buffer (§3.4) is what makes the four lanes coexist.** The
  `.mview` text buffer is the only writer; Lane 2's AI and Lane 1's drag both go
  *through* it, so an AI suggestion and a hand-edit can never race, and "you own the
  code" (Lane 3) is honest because there is no parallel document model.

**The inversion that makes it secure (the §14 thesis, generalized):** in Lovable,
English is primary and deterministic edits are the fast exception. **We flip it for
every lane.** Lane 1 (direct manipulation) is *primary*; Lane 2 (AI) is an *optional
accelerator that emits the same splices a human would*. The AI is a suggestion
engine; the chokepoint is the gate it cannot bypass. That is how we get the vibe
feel without the vibe insecurity.

### 15.3 How AI-draft and direct manipulation share ONE artifact (not two modes)

This is the crux of the brief — "how AI generation and direct manipulation combine
on ONE span-anchored `.mview` so both edit the same artifact." The answer is that
**they are not two editors; they are two input methods for one splice pipeline.**

- **Direct manipulation produces a splice directly.** Dragging a control, setting
  an inspector row, double-clicking to scaffold a handler — each computes its own
  byte-range edit from the node→span side-map and submits it to the gate. No model,
  no network, no credits. This is the deterministic core (Lovable's "Visual Edits"
  lane, but proven-equivalent rather than best-effort).
- **The AI accelerator produces a *candidate* splice — then takes the identical
  path.** "Select this card → make it a two-column secure form bound to `addNote`"
  yields *proposed span splices*, which are shown as a diff and then **must pass
  `codegen_signature` + the lint gate before write**, exactly like a human edit.
  The LLM never writes `.mview`; it proposes byte-ranges the existing pipeline
  validates. It reads the **AST + IR + the `project.rs` Services/Models surface**
  (structured data, à la Figma MCP) — **never pixels, never the whole file** — and
  is **scoped to the selected node's span** so it cannot rewrite unrelated,
  security-critical code (the structural fix for the cluster's collateral-regression
  and "rewrites broad regions" failures).
- **Because both lanes emit the same artifact, every AI-cluster "fearless
  iteration" feature comes for free over git:** lossless time-travel = `git` over
  the text buffer (preview/reapply any commit); "Try-to-fix" = re-run lint + the
  oracle and surface the *named, located* error (our lints already do this);
  two-way "you own the code" sync = the single-writer buffer reconciling studio ↔
  any external editor. None of these need a model.
- **The hard rule that keeps it honest:** an AI suggestion that touches a `secure`,
  `data-mv-encrypt`/`data-mv-decrypt`, or `authorizeSpend` token, or that would
  fail the secure-form / private-field lint, is **rejected at the gate** — the same
  wall a human edit hits. The badge reflects the *enforced token path*, so it
  cannot lie (the anti-CVE-2025-48757 property).

### 15.4 The signature feature set (concrete, grounded, opinionated)

The combined toolset, each item tagged with the cluster it best-of's and the real
repo primitive it rides on. Ordered by the loop a user lives in.

1. **Curated secure-first Toolbox** *(VB4 toolbox × Builder.io component-mapping ×
   our secure layer)*. A closed, learnable palette of Fluent components **plus**
   `secure-form` / `encrypted-field` / `spend-block` / `II-auth` — categories no
   other toolbox has a reason to invent. **Secure variants are the defaults**: drop
   "Form" → it is `secure`; drop "Secret field" → it carries `data-mv-encrypt`.
   Double-click-to-drop (default size) or drag-to-place; every item is a real
   `WidgetKind` node, dropped onto a `WidgetKind::Group`, never an anonymous
   positioned div. *Grounded in:* the palette splices the literal tokens
   `lint.rs`/`project.rs` already enforce.

2. **Honest, typed Property Grid** *(VB4 F4 grid × Plasmic auto-controls × Figma
   token-name inspect)*. One inspector for every object (selecting nothing edits the
   page). Generated **1:1 from the selected node's real `Attr`/`EventBind`/`bind`/
   `secure`** — no ghost props, no hidden designer-only state. Each row is a **typed
   editor** (color swatch bound to a `@theme`/Fluent **token**, never raw hex; enum
   dropdown; bool combo) → a single-span splice gated by `codegen_signature`.
   `secure` is a default with **no off-toggle** (the security opinion no RAD grid
   had). *Grounded in:* the AST carries these spans on every node (`ast.rs`); the
   inspector is the side-map made editable.

3. **Double-click → scaffold + jump to the `@code` Motoko handler** *(THE headline
   move — VB4 `Command1_Click`, lost by the whole AI cluster)*. Specified precisely
   in §15.5 below. Two `codegen_signature`-gated splices + a caret jump. No AI, no
   network, no deploy. Idempotent.

4. **Typed bind-pickers over the real Motoko surface** *(Budibase binding drawer ×
   FlutterFlow Set-from-Variable × Retool changeset — minus the lock-in)*. List bind
   offers **query-shaped funcs** (`@for x in notesFor(owner)`); form-submit offers
   **async/mutating funcs** (`@submit=addNote`); input bind offers the **record
   fields of the function's argument type**. Picked from a dropdown over the
   `project.rs` scan — **never a `{{ }}` string**. A **live "update-call" badge**
   (Power Apps' delegation underline, inverted) warns when a `@for` source is not a
   real `query func` and would pay a round-trip per render. The `async ≠ mutating`
   heuristic is **validated against real signatures**, never the name.

5. **AI accelerator as a suggestion engine** *(Lovable select-and-talk × v0 region
   re-prompt × Replit Design Mode — on secure rails)*. Optional. Select a node (or
   draw-annotate / pin-comment for spatial context) → describe in English → it
   **proposes span splices** that take the §15.3 path. Scoped to the selection,
   reads structured AST/IR/project data only, **never deploys**. The most-used edits
   (text/color/spacing/bind) stay **model-free** in Lane 1 so the accelerator is for
   genuine generation/refactor, not trivial tweaks (kills the credit-rage failure).

6. **From-data-model scaffold (the 3-screen win)** *(Power Apps "Start with data" ×
   Budibase autogenerate × FlutterFlow collection)*. Pick a Motoko **record type +
   its Service** → generate a **secure List + Detail + Create** page set, pre-wired:
   List `@for`s the query func, Create is a `secure` form whose fields are the record
   fields, **Private-classified fields auto-carry `data-mv-encrypt`**. One gesture,
   three screens, **secure-by-construction**. A prompt/voice/screenshot can act as
   the *seed* that selects the record + service (top-of-funnel momentum), but the
   output is the deterministic scaffold, not raw model text.

7. **Native live preview = F5** *(VB4 F5 × Bolt WebContainer speed — but a real
   native path)*. `F5` re-renders the canvas via the **already-working no-deploy IR
   replay** (`motoview preview --emit ir --json`; the egui studio already renders
   the forest and dispatches clicks, confirmed in `app.rs::render_forest`/
   `render_node`). **Optimistic-local-IR mutation** gives instant feedback;
   a **debounced authoritative re-render** is the truth. `Shift-F5` is the
   *separate, explicit* real build + deploy — we **never hide** the deploy step to
   fake "instant," because deploy crosses a trust boundary and the lint/cert posture
   must stay visible.

8. **Live security/privacy posture, not a late failure** *(the anti-Lovable-2.0
   badge)*. The deny-by-default secure-form and private-field lints paint a **live
   red badge** on the offending node via the node→span index — not a build-time
   surprise. The per-screen **Privacy Ledger** (§5) shows ciphertext-vs-metadata
   with metadata in RED. The badge means "the enforced token path is wired," never
   "a keyword is present."

9. **Fearless iteration over git** *(Lovable time-travel + Try-to-fix + you-own-the-
   code — all free because `.mview` is plain text)*. Every edit is a commit you can
   preview and reapply; "fix" re-runs the lints + oracle with named, located errors;
   the studio ↔ any-editor handoff is the single-writer buffer. No proprietary
   history format, no runtime lock-in.

10. **Connection-state visible in the editor** *(Xcode Interface Builder gutter
    dots)*. The bidirectional side-map lets the inspector's Events tab show a
    **filled dot** (wired to `onSaveClick`, jump-to-def) vs **empty dot** (no
    handler), and lets us **lint orphaned handlers and broken `@click=fn` wires** —
    closing the RAD sin (handler with no path back) that even IB only half-fixed.

### 15.5 The VB4 double-click → `@code` handler, as a precise span splice

This is the single most-loved RAD gesture and the one the entire AI cluster lost
(they have no typed logic surface to jump to — their "logic" is an English prompt).
It is the visceral proof of "design + behavior in one place." Specified as a
**deterministic, no-AI, no-network, no-deploy** operation — **two
`codegen_signature`-gated span splices plus a caret move** — that crosses the
presentation→behavior boundary *without the designer ever owning behavior* (the
Plasmic ownership line, enforced).

**Trigger (ship both, exactly like VB):** double-click the control on the canvas,
**or** double-click the **Click** row in the inspector's Events tab.

**Deterministic sequence:**

1. **Resolve node → span** via the Phase-2 side-map → the control's exact `.mview`
   element span + current attributes (and, if already wired, the bound-function
   span — §13.2 extends the side-map to carry it).
2. **If a handler is already bound** (`@click=onSaveClick` present): **skip
   scaffolding — just jump** the caret into the existing Motoko function's body.
   *Idempotent*, exactly like VB navigating to an existing handler. Done.
3. **Else derive the handler name** the way VB derived `Command1_Click`: a stable,
   canonical `on{Label}{Event}` from the control's id/label + its **default event**
   (Click for a button, submit for a form — *chosen for you*, zero decisions on the
   hot path). **Collision-check** against existing `@code` symbols from the
   `project.rs` scan; suffix `_2` on collision; **never silently overwrite**.
4. **Splice A — wire the event** at the control's span: insert `@click=onSaveClick`
   (the explicit, **lint-checkable** wire — the analog of WinForms'
   `Click += new EventHandler(...)`, but as one local attribute splice in the
   *same* file, not a hidden `InitializeComponent` region there is nothing to
   fight over).
5. **Splice B — scaffold the stub** in the page `@code` block: insert a **typed
   Motoko function** with the framework's fixed event-callback signature, empty body
   + a `// TODO` marker (VB's empty `Sub`). Inferred argument typing comes from
   context (e.g. a form's argument record).
6. **Both splices pre-validated by `codegen_signature` before they touch disk** — so
   the result is byte-identical to hand-authored source and the lint gate still
   runs. (Splice A/B are *insertions* that add a node + a function; the gate proves
   they parse and that no *other* codegen shifted — the structural-add variant of
   the equivalence check.)
7. **Jump the caret into the empty body.** Flip the right pane to source, scroll to
   the new handler's span, place the caret inside. Because `.mview` is **one
   artifact**, there is **no file switch and no generated-region boundary** to
   cross — strictly better than WinForms' two-file dance.
8. **Re-render** via the preview path; the Events tab now shows a **filled dot** on
   Click (wired, with jump-to-def). An orphan lint flags the handler if the control
   is later deleted, and a broken-wire lint flags `@click=foo` where `foo` is
   undefined.

**Why ours can feel as instant as VB and then beat it:** every step is an in-process
text op; the only latency is `codegen_signature` (fast, in-process) + the preview
re-render, made sub-perceptible by optimistic local-IR mutation. We close RAD's
worst gaps by construction — **no `Designer.cs` to fight** (one artifact), **no
orphaned handlers** (bidirectional side-map + orphan lint), **no magic-string
fragility** (the wire is an explicit checked reference), **no absolute coordinates
ever**, and **no JavaScript anywhere** in the flow (the handler is owned Motoko).

### 15.6 The 60-second first run — blank → a working, secure app

The brief asks for the concrete first-run. This is the thin vertical slice (§10)
told as a *timed DX walkthrough*, exercising every lane. Target: a downloadable,
cert-verified, ciphertext-only **Secret Note** app in ~60 seconds of authoring.

- **0–10s — Scaffold seed (Lane 4).** New project → "Start from a data model." The
  Services/Models scan is empty, so type one line of intent ("a private note with a
  title and body") **or** pick a starter record. The scaffold emits a Motoko `Note`
  record + an `EncStore`-backed service + **three secure screens** (List / Detail /
  Create). The Create screen's form is **already `secure`**; the body field is
  **already `data-mv-encrypt`** because it was classified Private. Zero crypto
  written by hand. *(Power Apps 3-screen × our secure-by-construction.)*
- **10–25s — Direct manipulation (Lane 1).** The canvas renders the **real UI-IR
  forest** (not a mockup). Click the title field → the inspector shows its honest,
  typed rows; the `secure` toggle on the form **has no off position**. Drag the
  body field above the title; the splice goes through the gate and the canvas
  re-renders. Change the heading color → it binds to a `@theme` **token**, not a hex
  literal. The `private-field` lint badge stays green only because the body is
  encrypted. *(VB4 grid × Figma token inspect × our live badge.)*
- **25–40s — Double-click → handler (Lane 1, the headline gesture).** Double-click
  the "Save" button → it derives `onSaveClick`, splices `@click=onSaveClick`,
  scaffolds a typed Motoko stub in `@code`, and drops the caret in the empty body.
  The bind-picker offers `addNote(input)` (an async/mutating func from the scan) for
  `@submit`; pick it and each input auto-binds to the argument record's fields.
  *(§15.5; VB4's most-loved move, on Motoko.)*
- **40–50s — F5 native preview (Lane 1).** Press F5 → no-deploy IR replay flips the
  canvas live; click "Save" in the running canvas → the deterministic replay
  dispatches the event. Sub-second, no build dialog in the mental model. *(VB4 F5 ×
  Bolt speed — real native path.)*
- **50–60s — Posture, then explicit deploy.** The per-screen **Privacy Ledger**
  shows ciphertext (the body) vs visible metadata (principal/timestamp/size) **in
  red** — the app tells the truth about what it does and does not protect before
  anything ships. `Shift-F5` runs the **explicit** build + deploy: the lint gate
  must pass, the network gate enforces `key_1`, the native client fetches the page
  **with its certificate** and **refuses to render on a bad cert**. *(Anti-CVE
  badge × the visible-deploy trust boundary.)*

At the end of 60 seconds the user has a **running, native, cert-verified,
ciphertext-only** app authored entirely through the four lanes — and has been shown,
in red, exactly the residual exposure. The momentum is Lovable's; the precision is
VB4's; the safety is structural and un-removable.

### 15.7 Why this combined DX beats Lovable / VB / FlutterFlow for the secure-dApp use case

- **vs Lovable (and the AI cluster):** same 60-second momentum and select-and-talk
  feel, but **secure-by-construction instead of insecure-by-default** (their
  170-app CVE is our headline), **typed precision in the last 20%** (property grid +
  proven-equivalent splice — the thing English-only editing can't do), **zero
  collateral regression** (one-span splice + `codegen_signature`), and **no credit
  anxiety** (the common edits are model-free).
- **vs VB4 (and classic RAD):** the same toolbox / typed grid / double-click-into-
  the-handler / F5 loop, but with **every RAD sin designed out** (no absolute
  coordinates, no regenerated DO-NOT-EDIT region, no design/runtime fidelity gap —
  the canvas *is* the shipping renderer — no orphaned handlers, no magic-string
  wiring), **plus** an optional AI accelerator VB never had **and** a secure/private
  toolbox category VB had no reason to invent.
- **vs FlutterFlow (and low-code):** the same binding gestures (typed pickers,
  changeset-derived forms, 3-screen scaffold) but **zero lock-in** — logic is owned
  Motoko, binding is a typed picker not a proprietary block, the app runs as the
  user's own canister + native binary, and the designer is a tool not a host.
- **The combined moat:** no competitor can copy the *integration* — a canvas that is
  the shipping UI-IR, a byte-exact equivalence oracle that makes every edit (human
  or AI) provably non-destructive, and a deny-by-default secure/private chokepoint
  that makes the security badge un-lieable — without rebuilding from zero. The DX is
  table stakes; **the DX *on these rails* is the product.**

### 15.8 The DX anti-goals (traps we refuse, consolidated from §§11.5/12.3/13.5/14.6)

- **A badge that means "the keyword is present" (Lovable 2.0).** Ours means "the
  enforced token path is wired," and the lint aborts the build otherwise.
- **Routing trivial edits through an LLM** (credit-rage, latency, collateral
  damage). Direct-manipulation splices are model-free and primary; AI is the
  accelerator, never the only way in.
- **Broad-region regeneration / an AI that rewrites unrelated code.** Every edit is
  one span, rejected by `codegen_signature` if it perturbs anything else — an AI can
  never silently break a `secure`/encrypt/`authorizeSpend` token.
- **An inspector that can weaken a security default.** The un-`secure` / un-encrypt
  toggles **do not exist**.
- **Absolute pixel positioning / a div-soup escape hatch (Anima/VB6).** Flow / stack
  / grid containers only; no coordinate primitive to fall back to.
- **A regenerated DO-NOT-EDIT region (`Designer.cs`).** One artifact, one writer;
  `codegen_signature` proves splice-equivalence, so there is no protected region to
  fight over.
- **A designer that owns formatting / a parallel document model.** The text buffer
  is the single writer; splices are whitespace-preserving and never normalize.
- **Stringly-typed `{{ }}` bindings / name-based query-vs-mutation heuristics.**
  Bindings are typed pickers validated against real signatures.
- **A round-trip so fragile it rejects half the gestures** (the #1 thing that would
  make us feel *worse* than VB). Whitespace-surgical splices + the golden round-trip
  CI suite keep the accept rate high.
- **Hiding the deploy/security step to fake "instant."** Fast *preview* yes;
  invisible *deploy* no — deploy crosses a trust boundary and stays visible.
- **An autonomous agent that can act on production.** The accelerator proposes; the
  human + the lint gate decide; **no agent ever touches a canister.**

### 15.9 Net, in one sentence

**The best-of-all-app-builders DX is VB4's zero-ceremony loop — toolbox, honest
typed property grid, double-click-straight-into-the-Motoko-handler, instant native
F5 — with Lovable's momentum (scaffold-from-a-sentence, select-and-talk, lossless
time-travel, you-own-the-code) bolted on as an *optional accelerator*, where all
four authoring lanes emit the *same* proven-equivalent span splice into the one
source-of-truth `.mview`, the canvas IS the shipping UI-IR so it can never lie, and
the deny-by-default secure/private chokepoint is the wall neither a human nor an AI
can route around — turning the precision and security every other builder loses in
the last 20% into the one thing only MotoView can guarantee.**

---

## 16. Developer Experience — Best of All App Builders

> Added 2026-06-10. §§11–15 each studied one cluster of builders and then §15
> synthesized the four authoring lanes. **This section is the consolidated DX
> charter** — the one place a reader can land to see the whole "best of all app
> builders" picture: the thesis, the AI-draft + direct-manipulation model on one
> source, the signature-feature table (inspiration × how-it-works-here ×
> what-it's-grounded-in), the VB4 double-click gesture spelled out, the 60-second
> first run, and the honest rule that none of it may weaken the security
> chokepoint. It restates, indexes, and tightens §§11–15; it does **not** override
> them. Every claim is grounded in a real repo primitive (cited inline), and
> nothing here introduces new architecture — the DX rides entirely on the existing
> chokepoint: the parser AST + `Span`s (`compiler/src/ast.rs`),
> `fmt.rs::codegen_signature` (`compiler/src/fmt.rs:53`) and `format_source`
> (`fmt.rs:144`), the `project.rs::build_source_map` side-map
> (`compiler/src/project.rs:271`), the IR canvas
> (`apps/studio/native/src/app.rs::render_forest`/`render_node`, lines 1154/1471),
> the no-deploy replay (`backend.rs::run_preview` → `motoview preview . --json`,
> `backend.rs:306`), the `project.rs` Services/Models scan, and the `lint.rs`
> `Severity::Error` gate (`compiler/src/lint.rs:208`).

### 16.1 The DX thesis — best of all builders, none of their flaws

Every cluster proved **half** the developer experience and shipped a **fatal
flaw**. The AI-native cluster (Lovable, v0, Bolt, Replit, Cursor) proved the
*feel* — prompt → running app in ~60s, select-and-talk editing, lossless
time-travel, "you own the code" — but ships *insecure-by-default* and even shipped
a security badge that **lied** (it checked that RLS *existed*, not that it
*worked*; the CVE-2025-48757 fallout was 170+ public apps with exposed databases).
Classic RAD (VB4/VB6, Delphi, WinForms, Interface Builder) proved the
*zero-ceremony loop* — toolbox, typed property grid, double-click-straight-into-
the-empty-handler, instant F5 — but shipped absolute coordinates and a regenerated
DO-NOT-EDIT region (`Designer.cs`) that the human and the tool fought over. Modern
WYSIWYG (Webflow, Framer, Figma Dev Mode, Builder.io, Plasmic) proved the
*fidelity machinery* (stable IDs, token-bound styling, presentation/behavior
splits) but burns its whole budget keeping a canvas in sync with code it *isn't*.
Low-code (FlutterFlow, Retool, Bubble, Budibase, Power Apps) proved the *binding
gestures* (picker drawers, changeset-derived forms, 3-screen scaffolds) but
trapped the logic in a proprietary, lock-in runtime.

**MotoView already owns the two pieces that fix all four flaws at once:** a real
parser AST with a *byte-exact semantic-equivalence oracle* (`codegen_signature`,
which proves `codegen(edit) == codegen(original)` and is the mechanism
`format_source` already uses to **reject a candidate and keep the original** when
codegen would change — `fmt.rs:144–159`) for precision and *zero collateral
regressions*; and a *deny-by-default secure/private chokepoint* so the badge can
never lie. The DX is therefore **not invented — it is VB4's loop, with Lovable's
momentum bolted on as an optional accelerator, on a canvas that IS the shipping
UI-IR, where the secure lint is the wall neither a human nor an AI can route
around.** (Full thesis: §15.1. The DX is table stakes; the DX *on these rails* is
the product — §15.7.)

### 16.2 AI-draft + direct manipulation on ONE source — two input methods, one pipeline

The crux of "best of all builders" is that **AI generation and direct manipulation
are not two modes or two editors — they are two input methods feeding ONE splice
pipeline over one artifact** (the `.mview` text buffer, the single writer of
§3.4). There are **four ways to author and exactly one way an edit reaches disk**:

```
  Lane 1: Direct manipulation   ─┐
  (drag / drop / inspector /     │
   double-click-to-handler)      │   candidate
  Lane 2: AI accelerator         ├── span ──► [ codegen_signature gate ] ──► [ lint.rs Error gate ] ──► WRITE .mview ──► re-render IR canvas
  (select → describe-in-English) │   splice      (byte-identical codegen)      (secure / private
  Lane 3: Hand-edit              │                                              deny-by-default)
  (.mview / @code, any editor)   │
  Lane 4: Scaffold               ─┘
  (data-model → 3 screens; prompt/voice/screenshot as seed)
```

- **Direct manipulation produces a splice directly** — no model, no network, no
  credits. A drag, an inspector row, a double-click each compute a byte-range edit
  from the node→span side-map and submit it to the gate. This is the
  *deterministic core* (Lovable's free "Visual Edits" lane, but proven-equivalent
  rather than best-effort AST-guessing). It is **primary**.
- **The AI accelerator produces a *candidate* splice and then takes the identical
  path.** "Select this card → make it a two-column secure form bound to `addNote`"
  yields *proposed* span splices, shown as a diff, that **must pass
  `codegen_signature` + the lint gate before write**, exactly like a human edit.
  The LLM never writes `.mview`; it reads structured **AST + IR + the `project.rs`
  Services/Models surface** (never pixels, never the whole file) and is **scoped to
  the selected node's span**, so it cannot rewrite unrelated, security-critical
  code. This is the structural fix for the AI cluster's collateral-regression and
  "rewrites broad regions" failures.
- **The inversion that makes it secure:** in Lovable, English is primary and
  deterministic edits are the exception; **we flip it** — Lane 1 is primary, Lane 2
  is the optional accelerator that emits the *same* splices a human would. The LLM
  is a suggestion engine; the chokepoint is the gate it cannot route around. That
  is "vibe feel without vibe insecurity" (§14, §15.2–15.3).
- **Because all lanes emit the same artifact, the AI cluster's "fearless
  iteration" comes free over git** — lossless time-travel = `git` over the buffer;
  "Try-to-fix" = re-run lint + oracle and surface the *named, located* error;
  two-way "you own the code" sync = the single-writer buffer reconciling studio ↔
  any external editor. None of these needs a model.

### 16.3 Signature features — inspiration × how it works here × grounded in

Each feature names the builder(s) it best-of's, the concrete MotoView mechanism,
and the real repo primitive it rides on. (Expanded in §15.4; the double-click is
§15.5; the scaffold is §13.4/§15.4.6.)

| Feature | Inspired by | How it works in MotoView | Grounded in (REAL primitive) |
| --- | --- | --- | --- |
| **Curated secure-first Toolbox** | VB4 toolbox × Builder.io component-mapping × our secure layer | Closed, learnable palette of Fluent components **plus** `secure-form`/`encrypted-field`/`spend-block`/`II-auth` as first-class categories. **Secure variant is the default** (drop "Form" → `secure`; drop "Secret field" → `data-mv-encrypt`). Every drop is a real `WidgetKind` node onto a `WidgetKind::Group` — never an anonymous positioned div. | Palette splices the literal tokens `lint.rs`/`project.rs` already enforce; no absolute-position primitive exists (anti-Anima by construction). |
| **Honest, typed Property Grid** | VB4 F4 grid × Plasmic auto-controls × Figma token-name inspect | One inspector for every object (selecting nothing edits the page); generated **1:1 from the node's real `Attr`/`EventBind`/`bind`/`secure`** — no ghost props. Each row is a **typed** editor (color bound to a `@theme`/Fluent **token**, never raw hex; enum dropdown; bool combo) → single-span splice. `secure` has **no off-toggle**. | `ast.rs` carries `Span`s on every Element/Component/Attr/EventBind; the inspector is the side-map made editable; each row passes `codegen_signature`. |
| **Double-click → scaffold + jump to the `@code` Motoko handler** | VB4 `Command1_Click` (the move the whole AI cluster *lost*) | Two `codegen_signature`-gated span splices + a caret move. Idempotent (jump if wired, else derive `on{Label}{Event}`, splice `@click=…` + a typed stub, land caret in body). **No AI, no network, no deploy.** Spelled out in §16.4. | Phase-2 node→span side-map (and §13.2's bound-function span); `project.rs` scan supplies the collision-check symbol table; the gate proves the structural-add is byte-identical. |
| **Typed bind-pickers over the real Motoko surface** | Budibase binding drawer × FlutterFlow Set-from-Variable × Retool changeset (minus lock-in) | List-bind offers query-shaped funcs (`@for x in notesFor(owner)`); form-submit offers async/mutating funcs (`@submit=addNote`); input-bind offers the **record fields of the func's argument type**. Dropdowns over the scan — **never a `{{ }}` string**. A live "update-call" badge (Power Apps delegation underline, inverted) warns when a `@for` source isn't a real `query func`. | `project.rs` already scans Services/Models for params + record types; `async ≠ mutating` is validated against real signatures, never the name. |
| **AI accelerator as a suggestion engine** | Lovable select-and-talk × v0 region re-prompt × Replit Design Mode (on secure rails) | Optional. Select a node (optionally draw-annotate/pin-comment) → describe in English → it **proposes** span splices that take the §16.2 path. Scoped to the selection, reads structured data only, **never deploys**. Common edits (text/color/spacing/bind) stay **model-free** in Lane 1, so the accelerator is for genuine generation/refactor, not trivial tweaks (kills credit-rage). | Output routed through `codegen_signature` + the `lint.rs` Error gate + the side-map; it cannot touch the `secure`/`data-mv-encrypt`/`authorizeSpend` tokens. |
| **From-data-model 3-screen scaffold** | Power Apps "Start with data" × Budibase autogenerate × FlutterFlow collection | Pick a Motoko **record type + its Service** → secure **List + Detail + Create** page set, pre-wired: List `@for`s the query func, Create is a `secure` form whose fields are the record fields, Private-classified fields auto-carry `data-mv-encrypt`. A prompt/voice/screenshot can be the *seed* that selects record + service; the output is the deterministic scaffold, not raw model text. | `project.rs` Services/Models scan + the secure-form and `private-field` lints; secure-by-construction folded into the generator. |
| **Native live preview = F5** | VB4 F5 × Bolt WebContainer speed (but a *real native* path) | `F5` re-renders the canvas via the **already-working no-deploy IR replay**; optimistic-local-IR mutation for instant feedback, debounced authoritative re-render for truth. `Shift-F5` is the **separate, explicit** real build + deploy — never hidden, because deploy crosses a trust boundary. | `backend.rs::run_preview` already shells out to `motoview preview . --json`; `app.rs::render_forest`/`render_node` already render the IR and dispatch clicks via deterministic replay. |
| **Live security/privacy posture (anti-lying badge)** | The inverse of Lovable 2.0's existence-not-correctness scanner | The deny-by-default secure-form and `private-field` lints paint a **live red badge** on the offending node via the node→span index (not a build-time surprise). The per-screen **Privacy Ledger** shows ciphertext-vs-metadata with metadata in RED. The badge means "the enforced token path is wired," never "a keyword is present." | `lint.rs` `Severity::Error` gate that aborts build AND preview (`project.rs`); `EncStore.Meta`/`Audit` metadata surfaced honestly (§5). |
| **Connection-state in the editor + orphan/broken-wire lint** | Xcode Interface Builder gutter dots | The bidirectional side-map drives an Events tab: **filled dot** (wired to `onSaveClick`, jump-to-def) vs **empty dot** (no handler); lints orphaned handlers and broken `@click=fn` wires — closing the RAD sin even IB only half-fixed. | The node→span side-map is bidirectional; `@click=fn` is an explicit, lint-checkable reference (not magic-string wiring), validated against the `project.rs` symbol table. |
| **Fearless iteration over git** | Lovable time-travel + Try-to-fix + you-own-the-code | Every edit is a commit you can preview and reapply; "fix" re-runs lints + oracle with named, located errors; studio ↔ any-editor handoff is the single-writer buffer. **No proprietary history, no runtime lock-in.** | `.mview` + generated `main.mo` are plain text under git; the single-writer text buffer (§3.4) keeps studio ↔ hand-edit honest. |

### 16.4 The headline gesture — double-click → `@code` Motoko handler

The single most-loved RAD gesture, and the one the **entire AI cluster lost** (they
have no typed logic surface to jump to — their "logic" is an English prompt). It is
the visceral proof of "design + behavior in one place." It is a **deterministic,
no-AI, no-network, no-deploy** operation — **two `codegen_signature`-gated span
splices plus a caret move** — that crosses the presentation→behavior boundary
*without the designer ever owning behavior* (the Plasmic ownership line, enforced).
Full spec is §15.5; the essential sequence:

1. **Resolve node → span** via the side-map → the control's exact `.mview` element
   span + attributes (and the bound-function span if already wired).
2. **If a handler is already bound** (`@click=onSaveClick` present): **skip
   scaffolding — just jump** the caret into the existing Motoko body. *Idempotent*,
   like VB navigating to an existing handler.
3. **Else derive `on{Label}{Event}`** the way VB derived `Command1_Click`: stable
   canonical name from the control's id/label + its **default event** (Click for a
   button, submit for a form — *chosen for you*, zero decisions on the hot path);
   collision-check against `@code` symbols from the `project.rs` scan; suffix `_2`
   on collision; **never silently overwrite**.
4. **Splice A — wire the event** at the control's span (`@click=onSaveClick`): the
   explicit, lint-checkable wire — the analog of WinForms' `Click += …` but **one
   local attribute splice in the same file**, not a hidden `InitializeComponent`
   region there is nothing to fight over.
5. **Splice B — scaffold the stub** in the page `@code` block: a typed Motoko
   function with the framework's fixed event-callback signature, empty body + a
   `// TODO` (VB's empty `Sub`); argument typing inferred from context (e.g. a
   form's argument record).
6. **Both splices pre-validated by `codegen_signature` before they touch disk** —
   byte-identical to hand-authored source; the lint gate still runs (structural-add
   variant: prove the additions parse and that **no other codegen shifted**).
7. **Jump the caret into the empty body.** Because `.mview` is **one artifact**,
   there is **no file switch and no generated-region boundary** to cross — strictly
   better than WinForms' two-file dance.
8. **Re-render** via the preview path; the Events tab now shows a **filled dot**
   with jump-to-def; an orphan lint flags the handler if the control is later
   deleted; a broken-wire lint flags `@click=foo` where `foo` is undefined.

**Why it can feel as instant as VB and then beat it:** every step is an in-process
text op; the only latency is `codegen_signature` (fast, in-process) + the preview
re-render, made sub-perceptible by optimistic local-IR mutation. RAD's worst gaps
are closed by construction — **no `Designer.cs` to fight** (one artifact), **no
orphaned handlers** (bidirectional side-map + orphan lint), **no magic-string
fragility** (an explicit checked reference), **no absolute coordinates ever**, and
**no JavaScript anywhere** (the handler is owned Motoko).

### 16.5 The 60-second first run — blank → a working, secure app

The thin vertical slice (§10) told as a *timed DX walkthrough*, exercising every
lane (full version: §15.6). Target: a downloadable, cert-verified, ciphertext-only
**Secret Note** app in ~60 seconds.

- **0–10s — Scaffold seed (Lane 4).** New project → "Start from a data model." The
  scan is empty, so type one line of intent ("a private note with a title and
  body") **or** pick a starter record. The scaffold emits a Motoko `Note` record +
  an `EncStore`-backed service + **three secure screens** (List / Detail / Create).
  The Create form is **already `secure`**; the body field is **already
  `data-mv-encrypt`** because it was classified Private. **Zero crypto by hand.**
- **10–25s — Direct manipulation (Lane 1).** The canvas renders the **real UI-IR
  forest** (not a mockup). Click the title field → the inspector shows honest typed
  rows; the form's `secure` toggle **has no off position**. Drag the body field
  above the title; the splice goes through the gate and the canvas re-renders.
  Change the heading color → it binds to a `@theme` **token**, not a hex literal.
  The `private-field` badge stays green only because the body is encrypted.
- **25–40s — Double-click → handler (Lane 1, the headline gesture).** Double-click
  "Save" → it derives `onSaveClick`, splices `@click=onSaveClick`, scaffolds a
  typed Motoko stub in `@code`, drops the caret in the empty body. The bind-picker
  offers `addNote(input)` (an async/mutating func from the scan) for `@submit`;
  pick it and each input auto-binds to the argument record's fields.
- **40–50s — F5 native preview (Lane 1).** Press F5 → no-deploy IR replay flips the
  canvas live; click "Save" in the running canvas → the deterministic replay
  dispatches the event. Sub-second, no build dialog in the mental model.
- **50–60s — Posture, then explicit deploy.** The per-screen **Privacy Ledger**
  shows ciphertext (the body) vs visible metadata (principal/timestamp/size) **in
  red** — the app tells the truth before anything ships. `Shift-F5` runs the
  **explicit** build + deploy: the lint gate must pass, the network gate enforces
  `key_1`, the native client fetches the page **with its certificate** and
  **refuses to render on a bad cert**.

End state: a **running, native, cert-verified, ciphertext-only** app authored
entirely through the four lanes, with the residual exposure shown in red. The
momentum is Lovable's; the precision is VB4's; the safety is structural and
un-removable.

### 16.6 The honest rule — great DX must NOT weaken the security chokepoint

This is the non-negotiable constraint on everything above, and it is the property
no other builder in any cluster has. **The DX is a layer of input methods on top of
the chokepoint; it is never allowed to become a bypass of it.** Concretely:

- **Every lane writes through the same gate.** A drag, an AI suggestion, a
  hand-edit, and a scaffold are indistinguishable downstream — all are candidate
  `.mview` byte-ranges that **must pass `codegen_signature` (byte-identical codegen
  proof) and the `lint.rs` `Severity::Error` gate (secure-form + `private-field`
  deny-by-default) before they touch disk.** AI-generated code is held to the
  *identical* bar as human code; the model is a suggestion engine, not a writer.
- **The security defaults have no off-switch in the designer.** The inspector
  cannot un-`secure` a mutating form or un-encrypt a Private field — **those
  toggles simply do not exist.** The secure variant is the *default* in the toolbox
  and stays the default through every edit path.
- **Collateral regression is structurally impossible.** A splice is bounded to one
  AST span and is **rejected by `codegen_signature` if it perturbs any other span**
  — so an AI (or a careless human drag) can **never silently break a
  `secure`/`data-mv-encrypt`/`authorizeSpend` token elsewhere** (the structural
  prohibition on the AI cluster's "change this button rewrote a region" failure).
- **The badge cannot lie.** It reflects the *enforced token path being wired*,
  never "a keyword is present" — the anti-CVE-2025-48757 property. The lint aborts
  build **and** preview, so you cannot even *render* an insecure mutating form.
- **No agent ever touches a canister.** The accelerator *proposes*; the human + the
  lint gate *decide*. Deploy is the explicit `Shift-F5` step, kept visible because
  it crosses a trust boundary — fast preview yes, invisible deploy no.

The full anti-goal list (a badge that means "keyword present"; routing trivial
edits through an LLM; broad-region regeneration; an inspector that weakens a
default; absolute positioning / div-soup; a regenerated DO-NOT-EDIT region; a
parallel document model; stringly-typed `{{ }}` bindings; a round-trip so fragile
it rejects half the gestures; hiding the deploy step; an autonomous production
agent) is consolidated in §15.8 and remains binding here.

**Net (§15.9, restated):** the best-of-all-app-builders DX is VB4's zero-ceremony
loop — toolbox, honest typed property grid, double-click-straight-into-the-Motoko-
handler, instant native F5 — with Lovable's momentum bolted on as an *optional
accelerator*, where all four lanes emit the *same* proven-equivalent span splice
into the one source-of-truth `.mview`, the canvas IS the shipping UI-IR so it can
never lie, and the deny-by-default secure/private chokepoint is the wall neither a
human nor an AI can route around.
