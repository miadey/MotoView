# Building the MotoView Android core (cargo-ndk) — NOT done on this machine

> **Status on the build machine: SCAFFOLD ONLY.** There is no Android NDK, no
> `ANDROID_HOME`, and no `cargo-ndk` here, so the `.so` is **not** produced and
> the Gradle/AAR build is **not** run. Everything below is the exact, tested
> recipe for a machine that *does* have the Android toolchain. The Kotlin/Compose
> sources in `motoview/src/main/kotlin` compile against this `.so`.

## What is built here vs. flagged

| Step | This machine | A machine with the NDK |
|------|--------------|------------------------|
| Rust core for Android (`.so`) | NOT built (no NDK) | `cargo ndk build` |
| Kotlin/Compose module compiles | NOT built (no Android SDK) | `./gradlew :motoview:assembleRelease` |
| AAR packaged | NOT built | yes (jniLibs auto-packaged) |
| On-device run | NOT run | yes |

The iOS side (`clients/ios`) *does* cross-compile the same Rust core for real
(`aarch64-apple-ios`) and links + runs a host smoke test — see `clients/ios`.

## 1. Install the Android Rust targets + cargo-ndk

```sh
rustup target add aarch64-linux-android armv7-linux-androideabi \
                  x86_64-linux-android i686-linux-android
cargo install cargo-ndk
export ANDROID_NDK_HOME=$HOME/Library/Android/sdk/ndk/<version>
```

## 2. Cross-compile the core with the `ffi` feature

The crate already exposes the flat C ABI under `--features ffi`
(`client/src/ffi.rs`). cargo-ndk drops the resulting `libmotoview_client.so`
straight into the Gradle `jniLibs` layout:

```sh
cd <repo>/client
cargo ndk \
  -t arm64-v8a \
  -o ../clients/android/motoview/src/main/jniLibs \
  build --release --features ffi
```

This produces:
```
clients/android/motoview/src/main/jniLibs/arm64-v8a/libmotoview_client.so
```
Add `-t armeabi-v7a -t x86_64 -t x86` for the other ABIs (and the matching
`abiFilters` in `motoview/build.gradle.kts`).

## 3. Bind the C ABI to Kotlin — two options

The Rust core exports **flat C symbols** (`mv_ffi_parse_forest`, ...), not
JNI-named ones. Kotlin's `external fun` expects `Java_<pkg>_<Class>_<method>`
symbols, so one shim layer is required. Pick one:

### Option A — UniFFI (preferred once `uniffi-bindgen` is on PATH)

`uniffi-bindgen` is **not** installed on the build machine, which is why the
shipped FFI is a hand-written C ABI. To switch to UniFFI later:

1. Add `uniffi` to `client/Cargo.toml` (optional, under the `ffi` feature) and a
   `[[bin]] name = "uniffi-bindgen"` that calls `uniffi::uniffi_bindgen_main()`.
2. Describe the surface in a `.udl` (or use proc-macro export) for
   `parse_forest` / `ir_diff` / `verify_response`.
3. Generate Kotlin:
   ```sh
   cargo run --features ffi --bin uniffi-bindgen generate \
     --library target/aarch64-linux-android/release/libmotoview_client.so \
     --language kotlin --out-dir ../clients/android/motoview/src/main/kotlin
   ```
4. The generated Kotlin replaces `MotoViewNative`'s `external fun`s with a typed
   binding; `MotoViewCore.kt` then calls the generated functions directly.

### Option B — a ~30-line JNI shim (works with the current C ABI today)

Add `client/src/jni_android.rs` (only compiled under `ffi` + `target_os =
"android"`) that re-exports each `mv_ffi_*` as a JNI symbol matching
`MotoViewNative`'s declarations, e.g.:

```rust
#[cfg(all(feature = "ffi", target_os = "android"))]
#[no_mangle]
pub extern "system" fn Java_dev_motoview_core_MotoViewNative_parseForest(
    mut env: jni::JNIEnv, _class: jni::objects::JClass, json: jni::objects::JString,
) -> jni::sys::jstring {
    let s: String = env.get_string(&json).unwrap().into();
    let c = std::ffi::CString::new(s).unwrap();
    let out = unsafe { crate::ffi::mv_ffi_parse_forest(c.as_ptr()) };
    let res = unsafe { std::ffi::CStr::from_ptr(out) }.to_str().unwrap().to_owned();
    unsafe { crate::ffi::mv_ffi_string_free(out) };
    env.new_string(res).unwrap().into_raw()
}
// ...one per mv_ffi_* (parseNode, renderForest, irDiff, verifyResponse)...
```

(Add the `jni` crate as an optional dep gated on `target_os = "android"`.) The
`external fun` names in `MotoViewCore.kt` already match these symbols.

## 4. Build the AAR

```sh
cd <repo>/clients/android
./gradlew :motoview:assembleRelease
# -> motoview/build/outputs/aar/motoview-release.aar
```

## 5. Token theming

The Slice 7 cross-compiler emits SwiftUI tokens at `.mvbuild/native/BrandTokens.swift`.
A matching Kotlin emitter (`BrandTokens.kt` with Compose `Color(...)`) is a small
follow-up in `compiler/src/color_native.rs`; until then the Compose renderer uses
Material3 defaults. See `clients/ios/README.md` for the iOS token wiring that is
already in place.
