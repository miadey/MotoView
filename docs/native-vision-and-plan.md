# MotoView Native: Vision and Plan

**Audience:** the MotoView creator.
**Scope:** the OWN fully-native MotoView client + OWN design system + MotokoStudio, engineered for wallets, identity, and banking.
**Binding constraints (yours):** NO PWA, NO WebView, NO third-party shells. Always linked to the backend. Most-advanced security via every applicable ICP primitive.
**Tone:** decisive where the architecture is sound; brutally honest where the security claims are not yet true. This targets money. Overstated security is worse than none.

> **Honesty header — read this first.** Large parts of the security story circulating in the design notes are *design, not code*. I verified the repo on 2026-06-09. The following do **not exist**: `client/src/cert_verify.rs`, `runtime/src/ChainKey.mo`, `compiler/src/lint.rs`, `apps/studio/`, any `clients/` native tree, and any attestation/`host_device_sign` code. The shipped certification (`runtime/src/CertV2.mo:6,126`) is the `no_certification` pass-through whose own comment says responses are **"not body-certified."** The generated vetKD key is hardcoded `dfx_test_key` (`compiler/src/project.rs:358,362,387,390`) with only a comment to switch to `key_1`. After login, **one HMAC bearer cookie alone authorizes a spend** (`App.mo` `effectiveCaller`→`serveEvent`). And `motoview shell` today emits **Tauri/Capacitor WebView wrappers** pointing at the canister URL (`compiler/src/main.rs:399,402`, `shell_redirect_html`) — i.e. the exact PWA/WebView path you forbid. This document treats all of that as work to be built, gated, and audited — never as a present-tense guarantee.

---

## 1. The thesis

**Go native because a fully-native MotoView client can make trust *local and cryptographic*, where the browser is forced to make it *remote and social*. That difference is the entire reason to build this, and it is strictly stronger — but only once the verifier is actually built.**

The browser client trusts the HTTPS boundary node. It renders whatever bytes the gateway returns. There is no on-device check that those bytes came from your canister, that the canister runs the code you shipped, or that a displayed balance reflects consensus state. For a content site that is acceptable. For a wallet it is a direct path to a forged "send to address X" confirmation screen. I confirmed the browser brain (`client/src/lib.rs`) parses JSON and applies it to the DOM with **zero** signature, root-key, or certificate checks; `host_fetch` in `client/glue/motoview.js` is a bare `fetch()`.

A native client can close that gap with primitives the browser cannot reach:

- **Local chain-key certificate verification.** Pin the 96-byte NNS root BLS12-381 G2 key as a compile-time constant in the Rust core. Verify every consequential response: CBOR-decode the `Certificate`, reconstruct the `HashTree` root, BLS-verify `domain_sep("ic-state-root") || root` against the delegated subnet key, verify the single-hop NNS delegation, assert the target canister id is inside the subnet's certified `canister_ranges`, enforce the `/time` freshness window. **Never `fetchRootKey()` on mainnet** — that trusts one replica. The exact hash/`domainSep`/CBOR algorithm already exists, byte-correct, in `runtime/src/CertV2.mo` (`hash()` 49–57, `domainSep()` 40–43, CBOR 60–90); port it 1:1 to Rust so the client verifies with *identical* logic to what the canister signs. This is the single highest-trust-payoff piece and it is the work of `client/src/cert_verify.rs` (new).

- **Canister-hash pinning + controller pinning.** `read_state` `/canister/<id>/module_hash` and `/canister/<id>/controllers` (both delivered inside a certificate, so trustworthy without the boundary). Refuse to operate unless `module_hash` matches a reproducibly-built reference and controllers are blackholed or a *specific pinned* governance principal. This closes the "is this even the code I audited?" gap the browser cannot answer.

- **Hardware key custody.** iOS Secure Enclave / Android StrongBox hold a non-exportable P-256 key behind a biometric. The II session key and the unwrapped vetKey (today held in plain JS/WASM memory — `client/glue/mv-auth.js` generates an **extractable** Ed25519 key via `crypto.subtle.generateKey(..., true, ...)`) get sealed under that hardware key. This is hardware-*gated usage*, not in-silicon storage of the BLS/Ed25519 material — be precise about that (Enclave is P-256-only).

- **Threshold signing.** The canister becomes the on-chain signer via threshold ECDSA/Schnorr (`aaaaa-aa`), so a wallet's private key never exists in one place. The hardware key is a *second authorization factor*, not the chain signer (IC has no P-256 threshold curve).

- **vetKeys confidentiality.** The canister stores ciphertext only; the device decrypts locally. Already wire-correct in `client-crypto/src/lib.rs` (ic-vetkeys 0.7) and `runtime/src/{VetKeys,EncStore}.mo`.

**Why native specifically:** every one of these primitives needs code running outside a sandboxed page — a pinned root key in the binary, a hardware keystore, a local BLS verifier. The browser physically cannot pin the NNS key and verify a threshold-BLS certificate before painting a balance. A native client can refuse to render UI it cannot attribute to the pinned canister. That makes the native client **strictly more backend-bound than the web client**, which is exactly your "always linked to the backend" requirement expressed as a cryptographic invariant rather than a slogan.

**The honest caveat that makes this real and not theater:** the strength is *conditional on building the verifier and certifying the bytes*. As shipped, the render path is an uncertified query and the server uses `no_certification`. Until both ends exist, the native client is no more trustworthy than the browser — it just has a place to put the trust. The thesis is the destination, not the current state.

---

## 2. Unified architecture

**Decided brain-execution model: native-lib (Option A).** Compile the shared Rust core to a native library via **UniFFI** for store builds, while the *identical source* keeps building to `wasm32-unknown-unknown` for web. **No on-device WASM runtime. No WebView. No third-party shell.**

```
                         ┌────────────────────────────────────────────┐
                         │           ONE .mview SOURCE (per app)        │
                         │   template + @code{} handlers + typed state  │
                         └───────────────────────┬──────────────────────┘
                                                  │  compiler/src/parser.rs → codegen.rs
                          ┌───────────────────────┴───────────────────────┐
                          │           EmitMode  (one AST walker)            │
                          │   gen_node / gen_element / gen_builtin / charts │
                          └───────┬─────────────────────────────┬──────────┘
                                  │ HTML (today)                 │ UI-IR (new, additive)
                                  ▼                              ▼
                       runtime/src/Html.mo            runtime/src/Ir.mo  (NEW)
                                  └──────────────┬───────────────┘
                                                 ▼
                              CANISTER  (single source of truth + logic)
            App.mo render=query / events=update · Security.mo · Roles.mo · Audit.mo
            VetKeys.mo · EncStore.mo · CertV2.mo (+ body-cert) · ChainKey.mo (NEW)
                Batch carries  html : Text   AND   ui : ?Json   (motoview/1, additive)
                                                 │
                                                 ▼  motoview/1 wire
                    ┌────────────────────────────┴────────────────────────────┐
                    │                                                          │
       ┌────────────┴─────────────┐                          ┌─────────────────┴────────────────┐
       │   SHARED RUST CORE        │  ← REUSED, ~unchanged →  │       SHARED RUST CORE             │
       │  client/src/lib.rs        │                          │   (same crate, native target)      │
       │  diff.rs · json.rs        │                          │   + cert_verify.rs (NEW)           │
       │  client-crypto (vetKeys)  │                          │   + 13th host_device_sign          │
       └────────────┬──────────────┘                          └─────────────────┬─────────────────┘
                    │ 12 host_* over (ptr,len) WASM mem          UniFFI foreign trait (HostBridge)
                    ▼                                                            ▼
        ┌───────────────────────┐         ┌──────────────────────┐   ┌──────────────────────┐
        │  WEB                  │         │  iOS                 │   │  Android             │
        │  wasm32 + glue.js     │         │  native lib (xcfwk)  │   │  native lib (AAR)    │
        │  → DOM (NEW host = no) │         │  → SwiftUI NativeView│   │  → Compose NativeView│
        └───────────────────────┘         │  Keychain · SE       │   │  Keystore · StrongBox│
                                          └──────────────────────┘   └──────────────────────┘
                                                  desktop = SwiftUI macOS / Compose Desktop
                                                  (~free off the same renderers)
```

**Reused vs new:**

| Reused ~verbatim | Newly built |
|---|---|
| `client/src/lib.rs` polling state machine, event seq/idempotency, batch interpretation | `client-ir` types (`UINode`/`Attr`/`EventBinding`/`Chart`) + structural keyed diff |
| `client/src/diff.rs` LIS reconcile (`lis_stable_set`) — algorithm transfers verbatim | `client/src/cert_verify.rs` (chain-key + module-hash/controllers verifier) |
| Entire canister: `App.mo`, `Security.mo`, `Roles.mo`, `Audit.mo`, `VetKeys.mo`, `EncStore.mo`, `CertV2.mo` (UI-agnostic) | `runtime/src/Ir.mo` (mirror of `Html.mo` builder) + `Types.mo` `ui:?Json` + `Json.mo` arm |
| `client-crypto/src/lib.rs` (links natively as-is; OS RNG takes over) | Second codegen backend (EmitMode) in `codegen.rs` |
| `compiler/src/color.rs` `brand_ramp()` + `brand_aliases.rs` | `compiler/src/color_native.rs` (`brand_theme_swift`/`brand_theme_kotlin`) |
| Page render/dispatch/validate logic in every `.mview` | Swift/Kotlin `HostBridge` + recursive `NativeView` renderers; UniFFI wrap; native II bridge; `ChainKey.mo` |

**Why native-lib, not embedded WASM:** I verified the brain is a pure-protocol cdylib whose **default** build has an empty normal dependency graph (the only deps are `optional`, behind the off-by-default `cert-verify` feature) and zero `wasm_bindgen`/`js_sys` (`client/Cargo.toml`), so it cross-compiles to `aarch64-*` unchanged. The default-feature wasm artifact is 84,634 bytes (measured 2026-06-09); embedding Wasmtime/Wasmer to host an ~85KB program is multi-MB of bloat for nothing. iOS forbids JIT, so an embedded runtime would run the brain *interpreted* (Pulley) — slower than native, with no portability gain the brain doesn't already have. Embedded-WASM's only real differentiator is hot-swapping `brain.wasm` at runtime — which is precisely what App Store **2.5.2** forbids. So you either never use it (cost for nothing) or use it and risk removal. Native-lib is dominant for a store-bound banking client. (Reserve embedded-WASM only for a future requirement to run *untrusted third-party* brain logic in-process, or Android-only off-store OTA — not this product.)

---

## 3. The security trust link

Five layers, each = (ICP primitive) × (existing MotoView module it extends) × (what is NEW). Then the honest threat model.

### Layer 1 — Transport / Integrity (untrust the boundary)
- **Primitive:** chain-key threshold-BLS over the state-tree root; response-certification v2; pinned NNS root key.
- **Extends:** `CertV2.mo` (port its `hash`/`domainSep`/CBOR to Rust; widen server cert from path-set to **body hashes**).
- **New:** `client/src/cert_verify.rs`; wire it into `lib.rs` *before* `apply()` touches the view tree.

### Layer 2 — Identity / Custody (hardware key + II + attestation)
- **Primitive:** Secure Enclave / StrongBox P-256, biometric-gated; II canister-signature delegation (pin signer `rdmx6-jaaaa-aaaaa-aaadq-cai`); App Attest / Play Integrity.
- **Extends:** the `mvEstablish` → `/mv-session` → HMAC-cookie bridge (`App.mo` 154–211) stays; native stores the token in Keychain/Keystore (the httpOnly-cookie equivalent). `establish(nonce, who)` is extended to require + verify an attestation token.
- **New:** native `device_identity`, `ii_bridge` (ASWebAuthenticationSession / Custom Tabs + Universal/App Links), attestation verification (off-canister via HTTPS outcall or independently-audited Motoko X.509/ECDSA stack).

### Layer 3 — Confidentiality (vetKeys IBE, now hardware-sealed)
- **Primitive:** threshold IBE on BLS12-381; ciphertext-only storage.
- **Extends:** `VetKeys.mo` + `EncStore.mo` (verbatim) + `client-crypto` (recompiled native; OS CSPRNG backs `getrandom`).
- **New:** seal the unwrapped vetKey under the hardware key; **switch key name to `key_1`** for any value-bearing build.

### Layer 4 — Signing (threshold ECDSA/Schnorr wallet)
- **Primitive:** `sign_with_ecdsa` (secp256k1: BTC/ETH) / `sign_with_schnorr` (bip340/ed25519); per-user addresses via `derivation_path = [callerPrincipalBytes, chainTag]`.
- **Extends:** the *authorization path* — II session + `Roles.mo` + `Security.mo` token + `Audit.mo` — which is the real custody surface, because the threshold key signs whatever it is told.
- **New:** `runtime/src/ChainKey.mo` (sibling of `VetKeys.mo`); 13th `host_device_sign`; intent-bound assertion verified server-side before signing.

### Layer 5 — Hardening
- **Primitive:** SPKI pinning (+ offline backup pin), canister module-hash pin, secure storage, anti-tamper signals.
- **Extends:** all of the above. Store compliance (native, no WebView, no embedded interpreter) is itself a security property — removes the hot-swap attack surface.

### HONEST THREAT MODEL — red-team summary

This is the section that matters most. Every claim below was attacked against the actual code. **Do not ship banking on a "held" badge that is really "designed."**

| # | Claim | Verdict | Why (grounded) | Residual risk after correct build | Required mitigation | Severity if broken |
|---|---|---|---|---|---|---|
| 1 | Transport integrity: rendered bytes are threshold-BLS-signed; boundary cannot forge UI/balance/redirect | **BROKEN (as deployed)** | No client verifier exists; render is an uncertified query; `CertV2.mo:6` = `no_certification` ("not body-certified"). Boundary can serve any body. | Stale-but-valid certified-query balances within `/time` window; missed `certified_data` re-set; a wrong-passing BLS port bug; `<1/3` Byzantine subnet | Build `cert_verify.rs` AND replace `no_certification` with real body-hash certification; consequential reads via UPDATE + verify `/request_status`; cross-validate the Rust verifier vs agent-js on real mainnet certs with fail-closed negative tests | **Critical** |
| 2 | Canister authenticity: refuse unless `module_hash` pinned and controllers blackholed/governed | **BROKEN (design + as deployed)** | No `read_state`/pinning code. "Governance-controlled" *permits* upgrades; blackhole = unpatchable funds. TOCTOU between periodic checks. In-binary pin is rewritable on a rooted device. | Governance compromise/coerced DAO proposal silently swaps signing logic; mid-session upgrade before re-check; pin void on compromised device | Two-tier model; verify hash in the SAME cert as the consequential response; transparency-logged signed release manifest; move the real gate server-side via attestation; never show "verified" from a client-only check | **Critical** |
| 3 | Delegation soundness: subnet delegation verified to root key, single-hop, canister in `canister_ranges` | **BROKEN (unimplemented)** | No `cert_verify`, no `ic_bls12_381` in the *client* deps (dev-only in crypto crate), nothing checks a signature on the render path | `<1/3` Byzantine on authoritative subnet; ≤30-day delegation replay window; wire-format port error can wrongly-pass | Implement `check_canister_ranges` exactly as agent-js; reject multi-hop delegation; pin root key; tighten `/time`; bind to module-hash pin | **Critical** (absence of the whole control) |
| 4 | Freshness: old-but-valid cert rejected outside `/time` window | **BROKEN (unimplemented + device-clock anchored)** | No `/time` read exists. Even as designed, the anchor is the device clock — rollback on a rooted device or NTP MITM revives old certs; ±5min is a 10-min replay corridor; 30-day delegation ceiling | Clock rollback defeats the bound; in-window replay of stale-but-favorable balances; stolen delegation valid for the window | Monotonic last-seen `/time` high-water mark in hardware storage (anti-rollback); consequential reads via UPDATE; tighten windows; cap delegation TTL ≤ hours bound to the device key; reject out-of-order `batchId` | **High** (critical-leaning as-is) |
| 5 | Identity custody: every signature needs a current biometric; key never in software (Path B) | **BROKEN (session bearer bypass)** | Hardware key signs only the *one* `mvEstablish` ingress; thereafter a 24h HMAC `mv_session` cookie authorizes everything via `effectiveCaller`. Cookie must live in memory to be attached per request → stealable | Stolen cookie = full impersonation for TTL, no biometric, on a compromised process | Make the *session* hardware-bound: fresh `host_device_sign` over a server challenge on every consequential request; cut TTL to minutes; store under `.biometryCurrentSet`; bind to attested device instance | **High** (critical if any value path is cookie-only) |
| 6 | Device/binary integrity: session minted only on attested unmodified binary | **BROKEN (unbuilt + contradictory)** | `establish(nonce, who)` takes no token; zero attestation code; Motoko has no X.509/ECDSA verifier. Design also says "never an absolute gate," contradicting "minted ONLY when proves." Attestation proves the on-disk image, not hooked runtime; `Audit.mo` is in-memory and `mvStableLoad` clears it | Rooted device + Frida runs hooked logic behind a genuine verdict; token relay/farming; Apple/Google backend dependency; non-tamper-evident audit log | Treat as a *risk signal*, not a gate; bind token to server nonce + device key; verify off-canister; hash-chain Audit and fold its head into `CertifiedData` | **High** |
| 7 | Confidentiality: canister sees only ciphertext; vetKey never leaves device, sealed by biometric; wrong-key substitution rejected | **PARTIAL** | `decrypt_and_verify` genuinely rejects wrong keys (`client-crypto/src/lib.rs:249`, tested) and `EncStore` is ciphertext-only — **these hold**. But: vetKey sits in plain JS memory (`mv-auth.js`); generated code hardcodes `dfx_test_key`; key release is gated by the HMAC cookie, not the II signature; transport seed uses `crypto.getRandomValues` not hardware CSPRNG | On `dfx_test_key`, a test-subnet quorum/key-holder decrypts all ciphertext; cookie theft yields the victim's key; boundary-injected JS reads in-memory key; `EncStore` metadata (size/timestamps/key text) leaks | **Hard-fail the compiler** if a `--network ic`/banking build emits `dfx_test_key`; bind derive input to the verified II principal not the cookie; build the native sealing layer before claiming it; pad/opaque metadata | **Critical** |
| 8 | Non-custodial signing: no signature unless {session, role/limit, single-use token bound to spend intent, fresh hardware assertion} all verify | **BROKEN (value not bound; replay window)** | The secure-form token's `schemaHash` is over field *names*, not *values* (`Security.mo:9,28`) — amount/dest are unsigned POST values. `consumed` nonce store is a non-stable HashMap capped at 4096 (`App.mo:53,57`) → replay across upgrade or via flood-eviction. `ChainKey.mo`/`host_device_sign` don't exist; "limit" has no implementation | Even built: canonicalization gaps (decimal/hex, address case, units) re-open substitution; no on-chain velocity limit | Add `intentHash = SHA256(canonical(amount)‖canonical(dest)‖chain‖nonce‖expiry)` to BOTH token payload and device assertion; make `consumed` stable; one canonical serializer shared by mint+verify; server-side per-principal velocity limits | **Critical** |
| — | Wallet "funds safe and recoverable" | **BROKEN** | After login, ONE bearer cookie suffices to spend; `ChainKey.mo`/`cert_verify.rs`/`host_device_sign` unbuilt; `.biometryCurrentSet` self-locks on re-enrollment; no seed / no social recovery → device+II loss = permanent loss | Controller/upgrade = total custody loss; subnet collusion; NNS seizure path (the canister IS the custodian, so "non-custodial" is overstated) | Per-tx IC ingress signature for spends (not cookie); real recovery (multi-device II / SLIP-39 social shares); reproducible build + tightly-governed controllers; `key_1`; disclose subnet + seizure assumptions in plain language | **Critical** |

**The one-line summary for the builder:** the *architecture* of the trust link is sound and the canister-side primitives are real and wire-correct. The *current security posture is the browser's* — boundary-trusting, cookie-bearer, test-keyed. Every "Critical" above is the gap between the design notes and the code. Build Layer 1 first; do not market any layer until its negative tests pass.

---

## 4. MotokoStudio

**What it is:** a visual + AI design studio that is itself a MotoView app (`apps/studio/`, NEW), dogfooded and served from a canister. The unit of design is an advanced `.mview` file; the unit of "preview" is a **real local dfx canister**. There is no mockup layer — every edit round-trips through the existing `parser.rs → codegen.rs → moc → dfx deploy` pipeline into the same canister the canvas renders from.

**How AI stays bound to the backend (structural, not aspirational):** the LLM is constrained to emit ONE complete artifact — template + `@code{}` handlers + typed state. Client/server drift becomes a `moc` type error caught by `motoview check`, not a runtime surprise: a `@click="save"` with no `func save`, an `@price` on an undeclared var, a handler returning the wrong type — all fail the type-check. **The studio refuses to "save" an artifact that fails `check`.** A green canvas is, by definition, a type-checked, deployed, backend-bound app. The AI is given the project's real Motoko service signatures as a palette, so generated handlers can only bind to functions/types that exist.

**Live preview against a real canister:** the canvas embeds the user's local dfx canister (the bound port from the scaffold's `dfx.json`). Form submits run real server-side handlers, real `validate{}`, real `Audit` appends. No fake data path.

**Security-by-construction — and its red-teamed limit (stated honestly):** the studio owns no rendering/validation engine; it only emits `.mview` that the unmodified `codegen.rs` compiles. So `novalidate` (kills HTML5 client validation, `codegen.rs:605`), server-side `validate{}` translation, secure-token minting (`codegen.rs:623–628`), and `@authorize` (enforced server-side in `App.mo`) are inherited — the AI *cannot route validation client-side because that code path does not exist in the compiler.*

> **But the red team broke the strong claim, and you must not repeat it.** MotoView security is **opt-in at every layer the author controls**: `secure` per-form (`codegen.rs:623` mints the token only `if el.secure`), `@authorize` per-page (`App.mo` `authorized()` returns true when `not page.authorize`), `validate{}` per-handler. An LLM that *omits* `secure` on a transfer form, *omits* `@authorize` on an admin page, or *omits* `validate{}` ships a type-correct, deploy-passing, **insecure** app. `@raw(expr)` emits unescaped HTML (`codegen.rs:490`) — an AI emitting `@raw(userText)` ships stored XSS. And IDOR is invisible to any structural check: a handler that writes another principal's `EncStore` record is type-correct and "secure-formed" yet wrong. **The correct claim is: "the studio provides secure defaults and surfaces security posture; it does NOT guarantee the absence of insecure apps."** The mitigation is to make security **deny-by-default in the compiler** (the one place that cannot be bypassed): hard `moc` error if a state-mutating `<form>` lacks `secure`; require an explicit `@authorize` (even `@authorize public` as a deliberate opt-out) on any page with a mutation handler; gate `sign_with_*` behind a typed capability requiring an intent-bound token + device assertion; ban `@raw` on dynamic user-data unless explicitly `@raw unsafe`; scope all owner-keyed writes to `ctx.caller` in the runtime. Build `compiler/src/lint.rs` + an adversarial `.mview` test suite and gate CI on it.

**Templates wired to the trust link:** wallet/identity/banking scaffolds pair generated `.mview` with the *existing verified modules* (`Security.mo`, `Roles.mo`, `EncStore.mo`, `VetKeys.mo`, `Audit.mo`, II login, and — once built — `ChainKey.mo` + `cert_verify.rs`). The studio composes proven crypto; it never invents it.

**Output to web + native:** one source. Web is the canister URL. Native is the UI-IR codegen backend the studio toggles per target. **Honest correction:** the current `motoview shell` command emits Tauri/Capacitor **WebView wrappers** (`main.rs:399,402`) — that is the forbidden PWA/WebView path and must NOT be presented as native output. True SwiftUI/Compose output ships only with the UI-IR backend (Section 2). Label native-native as roadmap until it exists.

**The dependency you must disclose:** on-canister LLM does not exist. AI runs off-canister (HTTPS outcall or a local studio daemon) — a trust/availability dependency outside the canister. The *generated app* stays fully on-chain and LLM-free at runtime; the *design-time* AI does not.

---

## 5. The own design system

**Single token source → three render targets.** `compiler/src/color.rs` `brand_ramp()` produces the 16-stop WCAG-checked ramp; `brand_theme_css()` emits CSS `--vars` today. The new `compiler/src/color_native.rs` consumes the **identical** ramp + `brand_aliases.rs` and emits:
- SwiftUI: `static let colorBrandBackground = Color(red:.., green:.., blue:..)`
- Compose: `val colorBrandBackground = Color(0xFF..)`

including the light/dark pair already encoded in the `(tok, light, dark)` alias tuples, with dark mode driven by OS appearance. The **IR carries semantic tokens** (`Attr::Token("primary")`), not hex or CSS classes, so the native renderer maps token → precompiled `Color` at draw time. Pixel-identical theming, zero per-app duplication, `@theme brand="#hex"` (already parsed) and `@theme "midnight"` presets work unchanged.

**Native component set:** the recursive `NativeView(node: UINode)` maps the IR to native widgets — `Element("div")` → `VStack`/`Column`, `Element("button")` → `Button`, `Component("Button", appearance:.primary)` → Fluent-styled native button. The Fluent token foundation MotoView already adopted is the contract; SwiftUI/Compose are new skins over the same tokens.

**Charts natively:** the SVG chart suite in `runtime/src/Charts.mo` (95 `public func`) is data-driven. The IR chart arm emits `Chart{ kind, series, labels, opts }`. Day-one parity: ship the existing SVG and render it in a native SVG view (SVGKit / AndroidSVG) keyed by the same node — honest perf/fidelity compromise. Progressively replace high-traffic charts with native `Canvas` (Swift Charts / Vico) painting from `{kind,data,opts}`, no SVG parsing. Either way, charts use the same semantic theme tokens.

---

## 6. Phased roadmap

Each phase ends in something shippable. Effort: S(days) / M(weeks) / L(month+). Dependencies in brackets.

- **Phase 0 — IR foundation (M).** `client-ir` crate (`UINode`/`Attr`/`EventBinding`/`Chart`); generalize `lib.rs` `Bridge::apply` payload from `String` to `UINode`; structural keyed diff in `diff.rs` (keep `lis_stable_set` verbatim; port `diff.rs` tests as a golden spec). *Ships:* the brain reconciles an IR tree in a Rust test harness. **[no deps]**

- **Phase 1 — REAL native app + ONE local cert verify (L). The de-risking milestone.** Second codegen backend (EmitMode) + `runtime/src/Ir.mo` + `Types.mo` `ui:?Json` + `Json.mo` arm; UniFFI-wrap the core; iOS xcframework / Android AAR; **minimal SwiftUI `NativeView` + `HostBridge`** rendering a real `.mview` page **natively (no WebView)**; AND `cert_verify.rs` verifying **one** chain-key certificate against the pinned NNS root key on-device (start with `/time` + a single certified path). *Ships:* an iOS app that renders a MotoView page as native SwiftUI and proves one certificate locally. **[Phase 0]**

- **Phase 2 — full renderer + theme + web parity (M).** Complete `NativeView` (all components + charts via SVG-view fallback) on both platforms; `color_native.rs`; ensure web (`html`) and native (`ui`) serve from one canister; golden tests asserting HTML and IR describe the same tree. *Ships:* feature-parity native client minus native-Canvas charts. **[Phase 1]**

- **Phase 3 — body certification + hardened transport (L).** Replace `CertV2.mo` `no_certification` with real response-cert v2 body-hash commitment; fold body hashes into `CertifiedData.set` on every mutation; route consequential reads through UPDATE + verify `/request_status`; full delegation + `canister_ranges` + anti-rollback `/time` in `cert_verify.rs`; cross-validate vs agent-js in CI. *Ships:* a client that refuses unverified consequential bytes. **[Phase 2]** — *this is the first phase a wallet may even be prototyped on.*

- **Phase 4 — hardware identity (L).** Native `device_identity` (SE/StrongBox, biometric-gated), native II bridge (ASWebAuthenticationSession/Custom Tabs + Universal/App Links, II signer pinned), Keychain/Keystore session, hardware-bound session (fresh assertion per consequential request), attestation as a risk signal feeding `Roles`. *Ships:* login + session that survives the red team's cookie-bearer attack. **[Phase 3]**

- **Phase 5 — threshold-sig wallet (L).** `runtime/src/ChainKey.mo`; 13th `host_device_sign`; `intentHash`-bound token + device assertion verified before `sign_with_*`; stable `consumed` store; per-principal velocity limits; `key_1`; recovery (multi-device II / social shares). *Ships:* a wallet whose spend requires {session + role/limit + intent-bound token + hardware assertion}. **[Phase 4]**

- **Phase 6 — MotokoStudio (L).** `apps/studio/` + LLM orchestration + `lint.rs` (deny-by-default) + security posture panel + wallet/identity/banking templates wired to Phases 3–5. *Ships:* AI-designed, backend-bound, secure-by-default apps to web + native. **[Phase 5]**

- **Phase 7 — store submission (M).** Reproducible builds + published module hash; fastlane (iOS) / Play Publisher (Android); App Attest / Play Integrity in CI; submission. *Ships:* the app in both stores. **[Phase 5; Phase 6 optional]**

---

## 7. Concrete repo changes

New dirs/files and the existing files each touches:

- **NEW `client-ir/`** — `UINode`/`Attr`/`EventBinding`/`Chart`/`Series` + structural keyed diff. *Replaces the HTML-byte-scanner role of `client/src/diff.rs` `find_key`.*
- **EDIT `client/src/lib.rs`** — generalize `Bridge::apply` over payload (`String`|`UINode`); polling/seq/backoff untouched; route consequential responses through `cert_verify` before apply.
- **EDIT `client/src/diff.rs`** — `Op` variants carry `UINode`; keep `lis_stable_set`/`reconcile` verbatim.
- **EDIT `client/src/abi.rs`** — native build: structured node payloads + UniFFI foreign trait; add 13th `host_device_sign`.
- **NEW `client/src/cert_verify.rs`** — pinned NNS root key const, CBOR decode, `HashTree` reconstruct (ported from `CertV2.mo`), BLS12-381 verify, delegation + `canister_ranges` + `/time`, `read_state` of `module_hash`/`controllers`. *Add `ic_bls12_381` (pairings) as a real `[dependencies]` of the client crate (today it is dev-only in `client-crypto`).*
- **NEW `runtime/src/Ir.mo`** — mirror of `Html.mo` `Builder` emitting a JSON node tree.
- **EDIT `runtime/src/Types.mo`** — `Batch` gains `ui : ?Json` next to `html`.
- **EDIT `runtime/src/Json.mo`** — `encodeBatch` `#changed`/`#validationError` arms emit `ui` when `fmt=ir`.
- **EDIT `runtime/src/App.mo`** — render path selects html vs ir by capability hint; **widen `ensureCert`/`certifiedPage` to certify dynamic Batch body hashes**; extend `establish(nonce, who)` to require/verify attestation; verify `host_device_sign` assertion before `ChainKey` signing; make `consumed` a stable structure.
- **EDIT `runtime/src/CertV2.mo`** — replace the `no_certification` pass-through with real body-hash response-cert v2 (the comment at line 6 stops being true).
- **EDIT `runtime/src/Security.mo`** — bind the token to `intentHash` (extend payload 6→7 fields); require the matching hardware assertion for sign handlers.
- **NEW `runtime/src/ChainKey.mo`** — threshold ECDSA/Schnorr (sibling of `VetKeys.mo`; `aaaaa-aa`; env-switched key name; cycles margin).
- **EDIT `compiler/src/codegen.rs`** — `EmitMode` flag + IR-emitting parallels of `gen_node`/`gen_element`/`gen_builtin` incl. chart arms; opt-in `data-mv-studio-id` behind a feature flag; **invert `el.secure` to deny-by-default** (hard error on mutating form without `secure`).
- **NEW `compiler/src/color_native.rs`** — `brand_theme_swift`/`brand_theme_kotlin` from `color.rs` `brand_ramp` + `brand_aliases.rs`.
- **NEW `compiler/src/lint.rs`** + **EDIT `compiler/src/main.rs`** — AST linter + `motoview lint`/`motoview studio` subcommands; **`motoview build --target ios|android`** (real native build via UniFFI/cargo-ndk, NOT the WebView `shell` scaffold).
- **EDIT `compiler/src/project.rs`** — **hard-fail** if a `--network ic` build emits `dfx_test_key`; require `key_1` for production (today `:358,362,387,390` only carry a comment).
- **NEW `clients/ios/`** — SwiftUI `HostBridge` + `NativeView(node:)` + Keychain + ASWebAuthenticationSession II bridge + xcframework of the core.
- **NEW `clients/android/`** — Compose `HostBridge` + `NativeView` composable + Keystore + Custom Tabs II bridge + cargo-ndk AAR.
- **NEW UniFFI wrapper** (foreign trait for the 13 host_*; async `host_fetch`) + Mozilla-megazord build scripts for `aarch64-apple-ios` / `aarch64-linux-android`.
- **NEW `apps/studio/`** — the studio as a MotoView app + `Studio.mo` (LLM orchestration).
- **CI guard** — a test asserting `client-crypto`'s `getrandom` custom hook stays `cfg(target_arch="wasm32")`-only so it can never shadow the OS CSPRNG on device (the gate is currently correct — keep it).

---

## 8. App-store path

**Why genuinely-native clears Apple 4.2 / 4.7 and makes 2.5.2 N/A — where a wrapper would not:**

- **4.2 (Minimum Functionality) / 4.7 (mini-apps & HTML5):** a real SwiftUI/Compose view tree driven by the Rust core is the *opposite* of a "repackaged website." The current `motoview shell` Tauri/Capacitor scaffold (a WebView at the canister URL) is exactly the wrapper that trips 4.2/4.7 — which is why it is forbidden here and replaced by the native renderer.
- **2.5.2 (no downloaded executable code):** the brain ships *inside* the signed binary as a native lib. Nothing is downloaded or executed at runtime. So 2.5.2 does not apply. This is the decisive reason the brain-exec model is native-lib, not embedded-WASM: an embedded interpreter that hot-swaps `brain.wasm` is the textbook 2.5.2 violation.
- **Bonus:** no WebView, no JIT (iOS forbids it anyway), no interpreter — also removes the hot-swap attack surface, so store compliance and security align.

**Brain-exec store implications:** UniFFI changes the ABI — `host_fetch` becomes an async foreign-trait callback; `mv_alloc`/`mv_dealloc` disappear (UniFFI manages lifting/lowering). Re-express the `(ptr,len)` convention as UniFFI bytes/strings without regressing borrow semantics. This is the well-trodden Signal libsignal / Mozilla AppServices path.

**Automation & logistics:** fastlane (match for certs, gym/deliver for build+submit) on iOS; Gradle Play Publisher on Android. Accounts: Apple Developer Program ($99/yr) + App Store Connect; Google Play Console ($25 one-time). App Attest needs the DeviceCheck entitlement; Play Integrity needs a Cloud project + default 10k checks/day quota (request an increase before launch). Universal Links need a valid HTTPS `apple-app-site-association`; App Links need `assetlinks.json` — **neither supports custom ports/localhost**, so the II dev-login path must stay separate from the store build.

---

## 9. Honest constraints

What ICP does **not** give you — state these to users in plain language for a banking product:

- **The replica is not a TEE.** Canister state is visible to node providers **unless** vetKeys-encrypted. Confidentiality of plaintext in canister memory is zero. EncStore is the only confidential path, and it leaks metadata (sizes, timestamps, access patterns).
- **Threshold trust is `<1/3` Byzantine, per subnet.** vetKD confidentiality and chain-key signing/certification both rest on it. On a small or node-provider-colluding subnet the claims degrade. Use `key_1` (34-node fiduciary) for value; verify the app canister sits on a sufficiently large subnet.
- **Finality latency.** A threshold signature is ~2s (consensus-round-bound, async cross-subnet). Spends are an UPDATE flow with a pending UI state, never a query. Presignature pools can deplete under load — retry with backoff.
- **"Non-custodial" is overstated.** The canister IS the custodian: the threshold key signs whatever the canister code says, and NNS governance / a governed controller / legal pressure on node providers can freeze or upgrade it. There is no user-held key that moves funds independent of the canister.
- **No P-256 threshold curve.** The hardware Enclave key can never be the on-chain signer — only an authorization factor. "Hardware custody" = hardware-gated *usage* of a software-wrapped key, not in-silicon storage.
- **Regulatory/banking compliance is not a crypto feature.** KYC/AML, custody licensing, consumer-protection, and key-recovery obligations are legal and operational, not solved by chain-key or vetKeys.

**Hardest unknowns (what to prove first):**
1. Does the ported `CertV2.mo`→Rust verifier match agent-js byte-for-byte on real mainnet certs, *and fail-closed* on tampered body / swapped G1↔G2 / wrong `\x0Dic-state-root` separator / out-of-range canister id / multi-hop delegation? A wrongly-*passing* verifier is worse than none.
2. Can `App.mo` certify dynamic Batch body hashes within the 32-byte `certified_data` cap and the boundary's response-verification constraints (the existing rejected route shapes)?
3. Is the native II bridge reliable across Universal/App-Link edge cases, with the II signer pinned?

**Prove these before any value is at risk.**

---

## 10. First spike

**The single smallest experiment that de-risks the whole path:**

> Build a throwaway iOS app that (a) links the existing `client/src` Rust core compiled to `aarch64-apple-ios` via UniFFI — proving the brain cross-compiles and the 12 host_* map to a foreign trait — and (b) in `host_fetch`, performs ONE real chain-key certificate verification against the **pinned NNS root key** on a single `read_state` of `/time` from a known mainnet canister, using the `HashTree`/`domainSep`/CBOR logic ported from `CertV2.mo` and BLS12-381 verify, **cross-checked against agent-js** on the same captured certificate.

This one spike simultaneously proves: the native-lib brain-exec model works (no WASM runtime), the UniFFI seam is viable, and the *hardest, highest-value, most error-prone* security primitive — local chain-key verification with a byte-exact ported verifier — actually verifies (and fail-closes on a tampered copy). It needs no UI renderer, no II, no wallet. If it passes, the thesis in Section 1 is real and Phase 1 is unblocked. If the ported verifier can't match agent-js, you have found the project's biggest risk on day one, before a line of wallet code.
