# MotoView — Release & Store Submission Pipeline (Slice 11)

End-to-end store automation + reproducible builds for the native MotoView
clients (Slice 8: `clients/ios` SwiftUI, `clients/android` Compose) and the
certified web brain. This document states **exactly** which steps are
automated-and-verified on the build machine versus which require the user's
paid accounts, full Xcode, the Android NDK, and store projects.

> **HARD HONESTY.** No app was built into an `.ipa`/`.aab`, signed, or uploaded
> on this machine. Store submission is **impossible here** — it needs paid Apple
> + Google accounts, signing material, full Xcode, and the Android NDK, none of
> which exist on a Command-Line-Tools-only box. What runs here is the
> reproducible client-wasm build, the Rust core's iOS cross-compile, and the
> static/YAML/Ruby validation of the automation. Everything else is **complete,
> wired, and flagged** for the user to run with their credentials.

---

## Pipeline at a glance

```
   source (audited commit)
        │
        ▼
  ┌─────────────────────────────┐   tools/release/reproducible-build.sh
  │ 1. Reproducible build        │   -> deterministic /motoview.wasm
  │    + module sha256 (PIN)     │      opt sha256 = d98e924c…0315c0  (verified
  └─────────────────────────────┘      stable across clean rebuilds here)
        │  pin ties to client/src/cert_verify.rs (chain-key verifier)
        ▼
  ┌─────────────────────────────┐   tools/native-build.sh ios|android
  │ 2. Native core cross-build   │   iOS cargo build: RUNS here (arm64 .a)
  │    (Rust → iOS .a / Android  │   xcframework / Android .so: FLAGGED
  │     .so)                     │      (need full Xcode / cargo-ndk)
  └─────────────────────────────┘
        │
        ▼
  ┌─────────────────────────────┐   clients/ios/fastlane (gym/pilot/deliver/match)
  │ 3. Assemble + sign           │   clients/android :app Gradle Play Publisher
  │    (.ipa / .aab)             │   NEEDS: full Xcode, signing certs, keystore
  └─────────────────────────────┘
        │
        ▼
  ┌─────────────────────────────┐   .github/workflows/release.yml (macOS runner)
  │ 4. Upload / submit           │   iOS → TestFlight/App Store; Android → Play
  │                             │   ALL gated on repository SECRETS
  └─────────────────────────────┘
```

---

## 1. Reproducible build + module hash (VERIFIED HERE)

`tools/release/reproducible-build.sh` deterministically rebuilds the Rust→WASM
client brain (the same artifact `tools/build-client.sh` embeds and every
canister serves at `/motoview.wasm`) and prints the SHA-256 to **pin**.

```sh
bash tools/release/reproducible-build.sh
# -> opt wasm: 76318 bytes  sha256=d98e924ca722bc32fa6e966b77a6305d5a929f5a1eb75ce7f0522ca58e0315c0
# -> MODULE HASH TO PIN: d98e924ca722bc32fa6e966b77a6305d5a929f5a1eb75ce7f0522ca58e0315c0
bash tools/release/reproducible-build.sh --check <expected_sha256>   # gate (exit 2 on drift)
```

**Verified on this machine:** two clean rebuilds produce the **same** opt hash
(`d98e924c…0315c0`). Determinism comes from the fixed `[profile.release]` in
`client/Cargo.toml` (opt-level=z, lto, codegen-units=1, panic=abort, strip), a
fixed `SOURCE_DATE_EPOCH`, and `--remap-path-prefix` so the binary does not
encode the checkout path.

### Why the hash matters (banking-grade integrity, ties to `cert_verify.rs`)

The native client verifies IC responses against the **pinned NNS root key**
(`client/src/cert_verify.rs::IC_ROOT_KEY`) — chain-key proof the bytes came from
the named canister. To also prove the canister runs the **audited code** serving
the **audited brain**, pin two more values:

1. **Served-asset pin** — the `opt_sha256` above. The v2 HTTP response
   certification binds `SHA256(body)`; pin the brain hash next to `IC_ROOT_KEY`
   and reject a `/motoview.wasm` whose certified body hash drifts.
   (cert_verify.rs today verifies `certified_data == SHA256(body)`; the v2
   `http_expr` walker is the documented follow-up in that file.)
2. **Canister `module_hash` pin** — the WASM the replica executes:
   ```sh
   dfx canister --network ic info <CANISTER_ID>   # -> "Module hash: 0x…"
   ```
   or via a chain-key-verified `read_state` on `["canister",<id>,"module_hash"]`.

Anyone can reproduce both by cloning the audited commit and re-running the
script — that is what makes the pin auditable rather than a number to trust.

---

## 2. Native core cross-build (iOS VERIFIED HERE; rest FLAGGED)

`tools/native-build.sh` is the `motoview build --target ios|android` equivalent
(a wrapper — the compiler and its green test suite are untouched).

```sh
bash tools/native-build.sh ios       # cargo aarch64-apple-ios(+sim): RUNS here
bash tools/native-build.sh android   # cargo-ndk: FLAGGED (no NDK here)
```

**Verified on this machine:** `tools/native-build.sh ios` cross-compiles the
Rust core for `aarch64-apple-ios` (device) and `aarch64-apple-ios-sim`, vendors
the archives into `clients/ios/libs/…`, and `lipo` confirms the device archive
is real `arm64`. The `.xcframework` assembly (`xcodebuild -create-xcframework`)
is **flagged** with the exact command because it needs **full Xcode**, which is
not installed (Command-Line-Tools only). The Android branch is flagged because
`cargo-ndk` + `ANDROID_NDK_HOME` are absent (see `clients/android/CARGO_NDK.md`).

---

## 3 & 4. Assemble, sign, submit (AUTOMATION COMPLETE; REQUIRES USER ACCOUNTS)

### iOS — `clients/ios/fastlane/`
- `Fastfile` lanes: `build_core`, `beta` (gym → TestFlight via pilot),
  `release` (gym → App Store via deliver), `certs` (match).
- `Appfile` / `Matchfile`: app id + team ids + the private `match` certs repo,
  all from environment/secrets (no credentials committed).
- `ruby -c` on all three: **Syntax OK** (verified here). They do **not run**
  here — `gym`/`xcodebuild`/`-create-xcframework` need full Xcode.

### Android — `clients/android/app/`
- New `:app` application module with the **Gradle Play Publisher**
  (`com.github.triplet.play`) plugin: `./gradlew :app:publishReleaseBundle`
  ships `bundleRelease` to the Play **internal** track. Release signing + the
  Play service-account JSON are read entirely from the environment.
- Optional fastlane `supply` path in `clients/android/fastlane/` (`ruby -c`:
  Syntax OK). Does **not run** here — no Android SDK/NDK.

### CI — `.github/workflows/release.yml`
- macOS-runner pipeline: `reproducible-hash` (build + hash gate) → `ios`
  (cargo + native-build + Fastlane) and `android` (cargo-ndk + Gradle Play
  Publisher). **Every** account-touching step is gated on a repository secret;
  the file contains **no** secrets.
- **Validated here:** `python3 yaml.safe_load` → YAML_OK, and `actionlint` →
  clean (0 findings). It does **not execute** here (no runner, no secrets).

---

## App Attest / Play Integrity (documented stubs)

- iOS: `clients/ios/APP_ATTEST.md` — `DCAppAttestService` flow, the client
  call-site (`HostBridge.fetch`), entitlement + App ID capability, and the
  canister-side verifier the user must build.
- Android: `clients/android/PLAY_INTEGRITY.md` — Play Integrity client flow (the
  `com.google.android.play:integrity` dep is already in `:app`), the Cloud
  project link, and the verifier.

Both are **inert without** the respective Apple/Google project + a verifying
endpoint, so neither is exercised here.

---

## EXACT external prerequisites + costs (USER must supply)

| Need | Cost | Used by |
|------|------|---------|
| Apple Developer Program | $99/yr | iOS signing, TestFlight, App Store, App Attest |
| App Store Connect API key (.p8 + key id + issuer) | — (account) | Fastlane auth (`ASC_*`) |
| `match` certs repo + `MATCH_PASSWORD` | — (private git) | iOS code signing |
| **Full Xcode** | free, ~40 GB, not on this box | `gym`, `.xcframework`, `.app`, `.ipa` |
| Google Play Console | $25 once | Android upload, Play Integrity |
| Play service-account JSON (release perm) | — (account) | Play Publisher / supply |
| Android upload keystore (.jks) | — | release signing |
| **Android SDK + NDK + cargo-ndk** | free, not on this box | `tools/native-build.sh android` |
| App Attest project (App ID capability + entitlement) | — | iOS device attestation |
| Play Integrity project (Cloud link) | — | Android device attestation |
| First store listing created manually | — | both (plugins only UPDATE existing apps) |

---

## What is automated-and-verified-here vs. requires the user

| Step | Here | Needs the user |
|------|------|----------------|
| Reproducible client-wasm build + stable sha256 | ✅ runs, hash stable | — |
| Module-hash pin doc (served asset + canister module_hash) | ✅ documented | run `dfx canister info` against their canister |
| iOS Rust core cross-compile (device + sim `.a`) | ✅ runs (arm64 verified) | — |
| `.xcframework` assembly | ⛔ flagged | full Xcode |
| Android Rust core (`.so`) | ⛔ flagged | NDK + cargo-ndk |
| Fastfile/Appfile/Matchfile syntax | ✅ `ruby -c` OK | — |
| `.aab` build + Play Publisher config | ✅ scaffolded | Android SDK + accounts |
| `release.yml` YAML + actionlint | ✅ valid | runner + secrets to execute |
| App Attest / Play Integrity wiring | ✅ documented stub | Apple/Google projects + verifier |
| Build an `.ipa`/`.aab`, sign, **upload/submit** | ⛔ NOT done | accounts + Xcode/NDK + secrets |

---

## How a maintainer ships a release (with credentials)

1. Set the repository secrets listed in `.github/workflows/release.yml`.
2. Pin the brain hash: run `tools/release/reproducible-build.sh`, store the
   `opt_sha256` as `PINNED_BRAIN_SHA256` and (separately) pin the canister
   `module_hash`.
3. Push a tag `vX.Y.Z` (or run the workflow manually, choosing `beta`/`release`
   and the Play track). CI rebuilds + hash-gates, cross-compiles, signs, and
   uploads to TestFlight + the Play internal track.
4. Promote from TestFlight / the Play console to production.
