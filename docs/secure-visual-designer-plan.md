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
work**. Phases 1–2 are sequenced FIRST and gate all security/privacy marketing.

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

### Phase 3 — Inspector + palette + drop (authoring begins)
*Builds on:* Phase 2 bridge, the Services/Models scan, the secure-token / vault
tokens.
*New work:* a property inspector driven by the selected node's
Attr/EventBind/bind/secure (single-span splices, gated by `codegen_signature`); a
Fluent + secure-primitive palette (secure variants are defaults); palette drop
onto `WidgetKind::Group`; live lint badges painted via the node→span index.

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

### Phase 5 — Native privacy + the downloadable vault lighthouse
*Builds on:* `client-crypto`, `EncStore`/`Audit`/vetkd endpoints, the
classification UX from Phase 2, `bundle.sh`/`release.yml`.
*New work:* recompile `client-crypto` (`ic-vetkeys`) for iOS/Android/desktop
behind the `host_*` seam; the per-screen **Privacy Ledger** + consent gate (no
green badge without metadata disclosure); extend `enforce_network_gate` to
hard-fail `dfx_test_key` for native/value-bearing builds; **ship Lighthouse #1 to
a store** so "no-WebView, ciphertext-only" is a download. Present the recovery
model at design time.

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
