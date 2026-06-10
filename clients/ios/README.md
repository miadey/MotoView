# MotoView native iOS client (Slice 8)

One MotoView source -> **native SwiftUI** (no WebView). This package proves the
iOS path end-to-end as far as a Command-Line-Tools-only machine allows, honestly.

## What this is

- `Sources/MotoViewFFI` — the hand-written **C ABI** module (`motoview_ffi.h`)
  over the Rust core (`client/`, built `--features ffi`, `client/src/ffi.rs`).
  **Not UniFFI** — `uniffi-bindgen` is not on this machine, so per the lead's
  approval we ship a flat cbindgen-style C ABI and decode the JSON envelopes with
  Foundation. (UniFFI remains a drop-in upgrade — see `../android/CARGO_NDK.md`.)
- `Sources/MotoViewKit` — the Swift surface:
  - `UINode.swift` — the Swift image of the IR node (decoded from the canonical
    JSON the Rust parser emits).
  - `MotoViewCore.swift` — the typed facade over the C ABI: `parseForest`,
    `parseNode`, `renderForest`, `irDiff`, `verifyResponse`.
  - `DiffPlan.swift` — the keyed-diff `Plan`/`Op` vocabulary (mirrors `diff.rs`).
  - `HostBridge.swift` — the **native equivalent of the brain's ~12 `host_*` ABI**
    (applyTree / replaceKeyed / insertKeyed / removeKeyed / moveKeyed / effect /
    navigate / setTitle / fetch / setTimer / now / log).
  - `StateHostBridge.swift` — a concrete in-memory `HostBridge` for tests/shells.
  - `NativeView.swift` — the recursive **`UINode -> some View`** renderer:
    `div/section -> VStack`, `button -> Button(+event)`, `text -> Text`,
    `raw -> RawHTMLView` fallback; keyed children preserved via `.id(key)`.
  - `BrandTokens.swift` + `Theme.swift` — the **Slice 7 token-theming hook**: the
    compiler-emitted Fluent brand ramp, wired into the renderer's accent color.
- `Sources/MotoViewSmoke` — a host executable that runs the **real Rust core**:
  parses the exact `Ir.mo` builder-forest golden into a `UINode` tree and asserts
  it, runs a keyed `irDiff`, and exercises `verifyResponse` (fails closed).
- `Tests/MotoViewKitTests` — `swift test` smoke covering the same.

## Linking

`MotoViewKit` links the Rust **static archive** `libmotoview_client.a`.
`swift build`/`swift test`/`swift run` on this machine target the host
(`aarch64-apple-darwin`), so they link `libs/host/libmotoview_client.a`.

The **real iOS cross-compiles** are vendored too:
- `libs/ios-arm64/libmotoview_client.a` — `aarch64-apple-ios` (device)
- `libs/ios-arm64-sim/libmotoview_client.a` — `aarch64-apple-ios-sim` (simulator)

Both were produced by:
```sh
cargo build --release --target aarch64-apple-ios     --manifest-path client/Cargo.toml --features ffi
cargo build --release --target aarch64-apple-ios-sim --manifest-path client/Cargo.toml --features ffi
```

## Build & test on this machine

```sh
cd clients/ios
swift build            # compiles the library + smoke executable (PASSES here)
swift run motoview-smoke   # runs the REAL Rust core on the host (PASSES here)
swift test             # NEEDS FULL XCODE — see below
```

`swift build` and `swift run motoview-smoke` both pass on this
Command-Line-Tools-only machine. **`swift test` does NOT run here**: the CLT ship
no SwiftPM-usable `XCTest` (and no SwiftPM-wired `Testing`) module, so
`import XCTest` fails to resolve under `swift test`. The XCTest suite in
`Tests/MotoViewKitTests` runs on a full-Xcode machine; on THIS machine the
identical assertions run through `motoview-smoke` (which passed: IR parse ->
UINode tree asserted, keyed `irDiff` -> Replace, and `verifyResponse` fails
closed with `Cbor`).

## What needs FULL XCODE (flagged — NOT available here)

This machine has only the Command-Line-Tools, so the following did **not** run:

- **Assembling an `.xcframework`** from the device + simulator archives
  (`xcodebuild -create-xcframework`).
- **Building an iOS `.app` target** and running it in the **simulator** or on a
  **device** (`xcodebuild`, a real app target, code signing).
- The `cdylib` (`.dylib`) crate-type does **not** link for the iOS targets
  without the iphoneos/iphonesimulator SDK linker — only the **`staticlib`**
  (`.a`) builds here. That `.a` IS a real `aarch64-apple-ios` device archive
  (verified: `lipo -info` -> `arm64`, exported `mv_ffi_*` symbols present), but
  it can only be linked into an app by full Xcode.
- A true HTML `RawHTMLView` (a `WKWebView`) for the `.raw` fallback — needs
  UIKit/WebKit in an app target. Here `.raw` renders stripped text.

**No on-device or simulator run happened.** What is proven here: the Rust core
**cross-compiles for real iOS**, the Swift package **compiles**, and a Swift
smoke test **runs the core over the C ABI on the host** (IR parse -> UINode,
keyed diff, and the chain-key `verify_response`).

## Token theming (Slice 7)

`BrandTokens.swift` is the compiler's `@theme` output
(`<app>/.mvbuild/native/BrandTokens.swift`), vendored verbatim. `Theme.swift`
resolves it per `ColorScheme` and `ThemedForest` applies the brand accent so the
native UI matches the web build's Fluent colors.
