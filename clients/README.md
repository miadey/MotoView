# MotoView native clients (Slice 8)

The OWN native client: one MotoView source -> native UI, **no WebView**. The Rust
core (`../client/`, the same crate that compiles to the 84.6 KB web brain) is
cross-compiled and called from Swift/Kotlin through a default-off `ffi` feature.

| Path | What | Built here? |
|------|------|-------------|
| `ios/` | SwiftUI client: C-ABI binding + `HostBridge` + `NativeView(UINode) -> some View` + host smoke test | **YES** — Rust cross-compiles for real iOS (`aarch64-apple-ios`), Swift package compiles, `swift test`/smoke run on the host |
| `android/` | Jetpack Compose scaffold: Kotlin core + `HostBridge` + `NativeView` composable + Gradle + cargo-ndk doc | **NO** — scaffold only (no Android NDK/SDK on this machine) |

## The `ffi` feature keeps the web build untouched

`../client/Cargo.toml` gains a default-OFF `ffi` feature (`src/ffi.rs`). The
default `wasm32-unknown-unknown` web build leaves it off, so the shipped web wasm
is **byte-identical (84634 bytes)** and pulls zero FFI/crypto code. The native
clients turn `ffi` on (which also turns on `cert-verify`) to get the IR
parse/diff and the chain-key certificate verifier.

## FFI surface (C ABI — `client/src/ffi.rs`, header `ios/Sources/MotoViewFFI/include/motoview_ffi.h`)

- `mv_ffi_parse_forest(json)` -> canonical UINode forest JSON (envelope)
- `mv_ffi_parse_node(json)` -> canonical UINode JSON
- `mv_ffi_render_forest(json)` -> HTML
- `mv_ffi_ir_diff(old, new)` -> keyed-diff Plan JSON (full | patch[ops])
- `mv_ffi_verify_response(cert, canister, path, body, now, max_offset)` ->
  certified `/time` or a named `CertError` (pinned NNS root key, fails closed)
- `mv_ffi_string_free(ptr)` — free any returned C string

Why a C ABI and not UniFFI: `uniffi-bindgen` is not on this build machine. A flat
C ABI (the lead-approved fallback) cross-compiles and binds to both Swift
(`import MotoViewFFI`) and Kotlin (JNI shim / future UniFFI) cleanly. See
`android/CARGO_NDK.md` for the UniFFI upgrade path.

See `ios/README.md` for exactly what compiled/ran and what needs full Xcode.
