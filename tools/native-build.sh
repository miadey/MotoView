#!/usr/bin/env bash
# ============================================================================
# MotoView — native core build wrapper (the `motoview build --target ios|android`
# equivalent, Slice 11). A thin wrapper instead of a compiler edit, so the
# compiler test suite stays untouched and green.
# ============================================================================
#
#   tools/native-build.sh ios       cross-compile the Rust core for iOS device
#                                    + simulator (these BUILD on Command-Line-
#                                    Tools), then assemble a .xcframework IF
#                                    full Xcode (xcodebuild) is present; else
#                                    print the exact command and FLAG it.
#
#   tools/native-build.sh android    run the cargo-ndk build IF cargo-ndk +
#                                    ANDROID_NDK_HOME are present; else print the
#                                    exact command and FLAG it.
#
# The cargo iOS targets (aarch64-apple-ios / aarch64-apple-ios-sim) build with
# only the Command-Line-Tools. The xcframework assembly and the Android NDK
# build need tools NOT on this machine — they are flagged, never faked.
#
# Output archives land where the Slice 8 packages expect them:
#   clients/ios/libs/ios-arm64/libmotoview_client.a       (device)
#   clients/ios/libs/ios-arm64-sim/libmotoview_client.a   (simulator)
#   clients/ios/MotoView.xcframework                       (if xcodebuild present)
#   clients/android/motoview/src/main/jniLibs/<abi>/libmotoview_client.so
# ============================================================================
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
CLIENT_MANIFEST="$ROOT/client/Cargo.toml"
FLAGGED=0

flag() { echo "  [FLAGGED] $*" >&2; FLAGGED=1; }

build_ios() {
  echo "==> native-build: iOS core (cargo, --features ffi)"

  for t in aarch64-apple-ios aarch64-apple-ios-sim; do
    rustup target list --installed 2>/dev/null | grep -qx "$t" \
      || { echo "rust target $t not installed (rustup target add $t)" >&2; exit 1; }
  done

  echo "--> aarch64-apple-ios (device)"
  cargo build --release --target aarch64-apple-ios \
    --manifest-path "$CLIENT_MANIFEST" --features ffi
  echo "--> aarch64-apple-ios-sim (simulator)"
  cargo build --release --target aarch64-apple-ios-sim \
    --manifest-path "$CLIENT_MANIFEST" --features ffi

  DEV="$ROOT/client/target/aarch64-apple-ios/release/libmotoview_client.a"
  SIM="$ROOT/client/target/aarch64-apple-ios-sim/release/libmotoview_client.a"

  # Vendor the fresh archives into the Swift package's libs/ layout (Slice 8).
  mkdir -p "$ROOT/clients/ios/libs/ios-arm64" "$ROOT/clients/ios/libs/ios-arm64-sim"
  cp "$DEV" "$ROOT/clients/ios/libs/ios-arm64/libmotoview_client.a"
  cp "$SIM" "$ROOT/clients/ios/libs/ios-arm64-sim/libmotoview_client.a"
  echo "    device archive: $(wc -c < "$DEV" | tr -d ' ') bytes"
  echo "    sim archive:    $(wc -c < "$SIM" | tr -d ' ') bytes"

  # Sanity: confirm the device archive really is arm64 (not a host build).
  if command -v lipo >/dev/null 2>&1; then
    echo "    lipo device: $(lipo -info "$DEV" 2>/dev/null || echo '?')"
  fi

  # The header the .xcframework exposes to Swift.
  HDR="$ROOT/clients/ios/Sources/MotoViewFFI/include/motoview_ffi.h"
  XCF="$ROOT/clients/ios/MotoView.xcframework"

  if xcrun --find xcodebuild >/dev/null 2>&1 && xcodebuild -version >/dev/null 2>&1; then
    echo "==> assembling xcframework (full Xcode present)"
    rm -rf "$XCF"
    mkdir -p "$ROOT/clients/ios/.xcf/Headers"
    cp "$HDR" "$ROOT/clients/ios/.xcf/Headers/"
    cp "$ROOT/clients/ios/Sources/MotoViewFFI/include/module.modulemap" \
       "$ROOT/clients/ios/.xcf/Headers/" 2>/dev/null || true
    xcodebuild -create-xcframework \
      -library "$DEV" -headers "$ROOT/clients/ios/.xcf/Headers" \
      -library "$SIM" -headers "$ROOT/clients/ios/.xcf/Headers" \
      -output "$XCF"
    echo "    wrote $XCF"
  else
    echo "==> xcframework assembly SKIPPED (xcodebuild/full Xcode NOT present)"
    flag "Run on a full-Xcode machine to assemble the .xcframework:"
    cat >&2 <<EOF
            rm -rf "$XCF"
            mkdir -p clients/ios/.xcf/Headers
            cp "$HDR" clients/ios/.xcf/Headers/
            cp clients/ios/Sources/MotoViewFFI/include/module.modulemap clients/ios/.xcf/Headers/
            xcodebuild -create-xcframework \\
              -library "$DEV" -headers clients/ios/.xcf/Headers \\
              -library "$SIM" -headers clients/ios/.xcf/Headers \\
              -output "$XCF"
EOF
  fi

  echo "==> iOS core build done (cargo: OK; xcframework: $([ "$FLAGGED" -eq 1 ] && echo FLAGGED || echo built))"
}

build_android() {
  echo "==> native-build: Android core (cargo-ndk, --features ffi)"
  local abis="${ANDROID_ABIS:-arm64-v8a}"
  local out="$ROOT/clients/android/motoview/src/main/jniLibs"

  # Build the -t flags from the ABI list.
  local tflags=()
  for a in $abis; do tflags+=("-t" "$a"); done

  if command -v cargo-ndk >/dev/null 2>&1 && [ -n "${ANDROID_NDK_HOME:-}" ]; then
    echo "--> cargo ndk ${tflags[*]} -o $out build --release --features ffi"
    ( cd "$ROOT/client" && cargo ndk "${tflags[@]}" -o "$out" build --release --features ffi )
    echo "    wrote $out/<abi>/libmotoview_client.so"
  else
    echo "==> Android build SKIPPED (cargo-ndk and/or ANDROID_NDK_HOME NOT present)"
    flag "Install the Android toolchain, then run:"
    cat >&2 <<EOF
            rustup target add aarch64-linux-android armv7-linux-androideabi x86_64-linux-android i686-linux-android
            cargo install cargo-ndk
            export ANDROID_NDK_HOME=\$HOME/Library/Android/sdk/ndk/<version>
            cd client && cargo ndk ${tflags[*]} -o "$out" build --release --features ffi
EOF
    echo "    (see clients/android/CARGO_NDK.md for the full recipe)" >&2
  fi
}

case "${1:-}" in
  ios)     build_ios ;;
  android) build_android ;;
  both)    build_ios; build_android ;;
  *)
    echo "usage: tools/native-build.sh ios|android|both" >&2
    exit 1 ;;
esac

if [ "$FLAGGED" -eq 1 ]; then
  echo ""
  echo "==> NOTE: one or more steps were FLAGGED (a required tool is absent on"
  echo "    this machine). The cargo cross-compile that DOES run succeeded;"
  echo "    the flagged step prints the exact command to run elsewhere."
fi
exit 0
