#!/usr/bin/env bash
#
# bundle.sh — package the native MotokoStudio as a distributable macOS
#             .app + .dmg, using ONLY the Command-Line-Tools (no full Xcode,
#             no cargo-bundle / third-party bundler, no Apple account).
#
# What it does (in order):
#   1. gen-icon                          (pure-Rust SVG -> AppIcon.iconset ->
#                                          iconutil -> AppIcon.icns)
#   2. cargo build --release for BOTH    (the wgpu/Metal native studio)
#      aarch64 + x86_64, then `lipo` them into ONE UNIVERSAL binary
#      (Apple Silicon AND Intel). `--arch host` opts out to host-only.
#   3. assemble dist/MotokoStudio.app/Contents/{Info.plist,MacOS/,Resources/}
#      — including Resources/AppIcon.icns, with CFBundleIconFile pointed at it.
#   4. codesign --sign -  (AD-HOC)       — required so it launches on Apple
#                                          Silicon without a "damaged" kill.
#      ...or, with --sign "Developer ID Application: ...", a REAL signature.
#   5. hdiutil create ... UDZO           — the distributable .dmg
#   6. print where the artifacts are + the HONEST signing/notarization note.
#
# Usage:
#   bash bundle.sh                       # default: UNIVERSAL + AD-HOC (no account)
#   bash bundle.sh --arch host           # host-arch only (skip the cross-build)
#   bash bundle.sh --sign "Developer ID Application: Your Name (TEAMID)"
#   bash bundle.sh --sign "..." --notarize-profile "<notarytool-keychain-profile>"
#
# This script is deliberately transparent and dependency-free: every tool it
# calls (cargo, codesign, hdiutil, plutil, file, lipo, iconutil) is the stock
# toolchain. The icon renderer is a pure-Rust crate (icongen/) — NO system SVG
# renderer (rsvg/resvg/inkscape) is required.
set -euo pipefail

# ---- locate ourselves; all paths are relative to this script's dir ----------
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
SELF="$SCRIPT_DIR/$(basename "${BASH_SOURCE[0]}")"   # absolute path to this file
cd "$SCRIPT_DIR"

APP_NAME="MotokoStudio"
BIN_NAME="motokostudio"             # [[bin]] name in Cargo.toml
DIST_DIR="$SCRIPT_DIR/dist"
APP_DIR="$DIST_DIR/$APP_NAME.app"
DMG_PATH="$DIST_DIR/$APP_NAME.dmg"
PLIST_TEMPLATE="$SCRIPT_DIR/Info.plist"

# Icon pipeline paths.
ICON_SVG="$SCRIPT_DIR/assets/icon.svg"
ICONGEN_MANIFEST="$SCRIPT_DIR/icongen/Cargo.toml"
ICONSET_DIR="$DIST_DIR/AppIcon.iconset"     # build output (gitignored)
ICNS_PATH="$DIST_DIR/AppIcon.icns"          # build output (gitignored)
ICON_NAME="AppIcon"                         # CFBundleIconFile value (no ext)

# ---- parse args -------------------------------------------------------------
SIGN_IDENTITY="-"                   # "-" == ad-hoc (no cert / no account)
NOTARIZE_PROFILE=""
ARCH_MODE="universal"               # "universal" (default) | "host"
while [ $# -gt 0 ]; do
  case "$1" in
    --sign)
      SIGN_IDENTITY="${2:?--sign needs an identity, e.g. \"Developer ID Application: Name (TEAMID)\"}"
      shift 2 ;;
    --notarize-profile)
      NOTARIZE_PROFILE="${2:?--notarize-profile needs a notarytool keychain profile name}"
      shift 2 ;;
    --arch)
      case "${2:?--arch needs a value: universal|host}" in
        universal|host) ARCH_MODE="$2" ;;
        *) echo "bundle.sh: --arch must be 'universal' or 'host' (got: $2)" >&2; exit 2 ;;
      esac
      shift 2 ;;
    -h|--help)
      sed -n '2,38p' "$SELF"; exit 0 ;;
    *)
      echo "bundle.sh: unknown arg: $1" >&2; exit 2 ;;
  esac
done

# Fail fast: notarization needs a real cert (ad-hoc can never be notarized).
if [ -n "$NOTARIZE_PROFILE" ] && [ "$SIGN_IDENTITY" = "-" ]; then
  echo "FATAL: --notarize-profile requires a real --sign identity (ad-hoc can't be notarized)" >&2
  exit 2
fi

if [ "$SIGN_IDENTITY" = "-" ]; then
  SIGN_LABEL="ad-hoc (no Apple account)"
else
  SIGN_LABEL="\"$SIGN_IDENTITY\""
fi

echo "==> MotokoStudio packager (CLT-only, no third-party bundler)"
echo "    signing identity: $SIGN_LABEL"
echo

# ---- 0. sanity: required CLT tools present ----------------------------------
for tool in cargo codesign hdiutil plutil file lipo iconutil; do
  command -v "$tool" >/dev/null 2>&1 || { echo "FATAL: '$tool' not found on PATH" >&2; exit 1; }
done

mkdir -p "$DIST_DIR"

# ---- 1. gen-icon: pure-Rust SVG -> AppIcon.iconset -> AppIcon.icns ----------
# The icongen/ crate (its OWN standalone workspace) reads assets/icon.svg and
# renders every iconset PNG (16..1024). iconutil then folds the .iconset/ dir
# into a single AppIcon.icns. Both the .iconset/ and .icns are build outputs
# under dist/ (gitignored); the only tracked icon source is assets/icon.svg.
echo "==> [1/6] gen-icon (icongen -> $ICON_NAME.iconset -> $ICON_NAME.icns)"
ICON_OK=0
if [ -f "$ICON_SVG" ] && [ -f "$ICONGEN_MANIFEST" ]; then
  cargo build --release --manifest-path "$ICONGEN_MANIFEST"
  ICONGEN_BIN="$SCRIPT_DIR/icongen/target/release/icongen"
  if [ -x "$ICONGEN_BIN" ]; then
    rm -rf "$ICONSET_DIR" "$ICNS_PATH"
    "$ICONGEN_BIN" "$ICON_SVG" "$ICONSET_DIR"
    iconutil -c icns -o "$ICNS_PATH" "$ICONSET_DIR"
    if [ -f "$ICNS_PATH" ]; then
      ICON_OK=1
      echo "    built $ICNS_PATH"
    fi
  fi
fi
if [ "$ICON_OK" != "1" ]; then
  echo "    WARNING: icon generation skipped/failed — bundling without an icon." >&2
fi

# ---- 2. release build (UNIVERSAL by default; --arch host opts out) ----------
echo "==> [2/6] cargo build --release (arch mode: $ARCH_MODE)"
ARM_TARGET="aarch64-apple-darwin"
X86_TARGET="x86_64-apple-darwin"
UNIVERSAL_BUILT=0                   # reported truthfully in the summary

if [ "$ARCH_MODE" = "host" ]; then
  # Host-arch only: a plain `cargo build --release` lands in target/release/.
  cargo build --release
  SRC_BIN="$SCRIPT_DIR/target/release/$BIN_NAME"
  [ -x "$SRC_BIN" ] || { echo "FATAL: release binary not found at $SRC_BIN" >&2; exit 1; }
else
  # Universal: build BOTH explicit targets, then lipo them together.
  echo "    building $ARM_TARGET ..."
  cargo build --release --target "$ARM_TARGET"
  ARM_BIN="$SCRIPT_DIR/target/$ARM_TARGET/release/$BIN_NAME"
  [ -x "$ARM_BIN" ] || { echo "FATAL: arm64 binary not found at $ARM_BIN" >&2; exit 1; }

  echo "    building $X86_TARGET ..."
  # The x86_64 cross-build CAN fail (a dep that won't cross-compile). If it
  # does, we DO NOT fake a universal binary: we fall back to the arm64 build
  # alone and say so loudly + in the final summary.
  if cargo build --release --target "$X86_TARGET"; then
    X86_BIN="$SCRIPT_DIR/target/$X86_TARGET/release/$BIN_NAME"
  else
    X86_BIN=""
  fi

  SRC_BIN="$DIST_DIR/$BIN_NAME.universal"
  if [ -n "$X86_BIN" ] && [ -x "$X86_BIN" ]; then
    lipo -create -output "$SRC_BIN" "$ARM_BIN" "$X86_BIN"
    UNIVERSAL_BUILT=1
    echo "    lipo -> universal: $(lipo -info "$SRC_BIN")"
  else
    # Honest fallback: arm64 only.
    cp "$ARM_BIN" "$SRC_BIN"
    UNIVERSAL_BUILT=0
    echo "    WARNING: x86_64 cross-build failed — falling back to arm64-only." >&2
    echo "             (NOT a faked universal binary.)" >&2
  fi
fi

# ---- 3. assemble the .app bundle --------------------------------------------
echo "==> [3/6] assembling $APP_NAME.app"
rm -rf "$APP_DIR"
mkdir -p "$APP_DIR/Contents/MacOS" "$APP_DIR/Contents/Resources"

# 2a. binary -> Contents/MacOS/<binary>
cp "$SRC_BIN" "$APP_DIR/Contents/MacOS/$BIN_NAME"
chmod +x "$APP_DIR/Contents/MacOS/$BIN_NAME"

# 2b. Info.plist template -> Contents/Info.plist, then stamp the real version.
cp "$PLIST_TEMPLATE" "$APP_DIR/Contents/Info.plist"
APP_VERSION="$(
  awk -F'"' '/^version[[:space:]]*=/{print $2; exit}' "$SCRIPT_DIR/Cargo.toml"
)"
APP_VERSION="${APP_VERSION:-0.1.0}"
plutil -replace CFBundleVersion            -string "$APP_VERSION" "$APP_DIR/Contents/Info.plist"
plutil -replace CFBundleShortVersionString -string "$APP_VERSION" "$APP_DIR/Contents/Info.plist"
echo "    stamped version $APP_VERSION"

# 2c. icon: copy the freshly-generated AppIcon.icns into Resources/ and keep
#     CFBundleIconFile pointed at it. If gen-icon failed (ICON_OK!=1) drop the
#     key so we don't dangle-point at a missing resource (generic icon then).
if [ "$ICON_OK" = "1" ] && [ -f "$ICNS_PATH" ]; then
  cp "$ICNS_PATH" "$APP_DIR/Contents/Resources/$ICON_NAME.icns"
  plutil -replace CFBundleIconFile -string "$ICON_NAME" "$APP_DIR/Contents/Info.plist"
  echo "    bundled icon Resources/$ICON_NAME.icns (CFBundleIconFile=$ICON_NAME)"
else
  plutil -remove CFBundleIconFile "$APP_DIR/Contents/Info.plist" 2>/dev/null || true
  echo "    no icon — using the generic app icon (CFBundleIconFile removed)"
fi

# 2d. PkgInfo (cosmetic but conventional): APPL + 4-char signature.
printf 'APPL????' > "$APP_DIR/Contents/PkgInfo"

# 2e. validate the plist before we sign anything.
plutil -lint "$APP_DIR/Contents/Info.plist"

# ---- 4. codesign (AD-HOC by default) ----------------------------------------
echo "==> [4/6] codesign ($SIGN_LABEL)"
# --force: re-sign if already signed. --deep: also sign nested code (we have
# none, but it is correct/harmless). --timestamp=none for ad-hoc (a trusted
# timestamp needs a real cert + network); a real Developer ID build below adds
# --options runtime (hardened runtime) which notarization requires.
if [ "$SIGN_IDENTITY" = "-" ]; then
  codesign --force --deep --sign - "$APP_DIR"
else
  codesign --force --deep --options runtime --timestamp \
           --sign "$SIGN_IDENTITY" "$APP_DIR"
fi
codesign --verify --deep --strict --verbose=2 "$APP_DIR" 2>&1 | sed 's/^/    /'

# ---- 5. make the .dmg -------------------------------------------------------
echo "==> [5/6] hdiutil create $APP_NAME.dmg"
rm -f "$DMG_PATH"
hdiutil create -volname "$APP_NAME" -srcfolder "$APP_DIR" \
               -ov -format UDZO "$DMG_PATH" >/dev/null
# A signed (non-ad-hoc) build can/should also sign the .dmg itself.
if [ "$SIGN_IDENTITY" != "-" ]; then
  codesign --force --sign "$SIGN_IDENTITY" "$DMG_PATH"
fi

# ---- 5b. OPTIONAL notarization (real cert + notarytool only) ----------------
if [ -n "$NOTARIZE_PROFILE" ]; then
  if command -v xcrun >/dev/null 2>&1 && xcrun --find notarytool >/dev/null 2>&1; then
    echo "==> [4b] notarytool submit + staple"
    xcrun notarytool submit "$DMG_PATH" --keychain-profile "$NOTARIZE_PROFILE" --wait
    xcrun stapler staple "$DMG_PATH"
    xcrun stapler staple "$APP_DIR"
  else
    echo "WARNING: notarytool not found (needs Xcode). Skipping notarization." >&2
    echo "         The .dmg is signed but NOT notarized." >&2
  fi
fi

# ---- 6. report + honest signing note ----------------------------------------
echo
echo "==> [6/6] done."
echo "    app : $APP_DIR"
echo "    dmg : $DMG_PATH"
echo
echo "    ----------------------------------------------------------------"
# Honest architecture report straight off the final, bundled binary.
FINAL_BIN="$APP_DIR/Contents/MacOS/$BIN_NAME"
echo "    ARCH: $(lipo -info "$FINAL_BIN" 2>/dev/null || file "$FINAL_BIN")"
if [ "$ARCH_MODE" = "universal" ]; then
  if [ "$UNIVERSAL_BUILT" = "1" ]; then
    echo "      • UNIVERSAL (arm64 + x86_64) — runs on Apple Silicon AND Intel."
  else
    echo "      • NOT universal: the x86_64 cross-build failed; fell back to"
    echo "        arm64-only. (No faked fat binary — see WARNING above.)"
  fi
else
  echo "      • host-arch only (--arch host)."
fi
echo "    ICON: $([ "$ICON_OK" = "1" ] && echo "AppIcon.icns bundled (from assets/icon.svg)" || echo "none (generation failed/skipped)")"
echo "    ----------------------------------------------------------------"
if [ "$SIGN_IDENTITY" = "-" ]; then
  cat <<'NOTE'
    SIGNING: AD-HOC (codesign --sign -). No Apple account, no cert.
      • Runs on THIS Mac, and on others via right-click -> Open (one-time
        Gatekeeper override). Plain double-click on another Mac will warn.
      • To distribute WITHOUT warnings on any Mac you must NOTARIZE, which
        needs a paid Apple Developer ID cert ($99/yr) + Xcode's notarytool:
            bash bundle.sh --sign "Developer ID Application: Name (TEAMID)" \
                           --notarize-profile <notarytool-keychain-profile>
        That path is NOT taken by default and is NOT faked here.
NOTE
else
  echo "    SIGNING: Developer ID ($SIGN_IDENTITY)."
  if [ -n "$NOTARIZE_PROFILE" ]; then
    echo "      • Signed AND submitted to notarytool + stapled (above)."
  else
    echo "      • Signed but NOT notarized. Add --notarize-profile <profile>"
    echo "        (needs Xcode's notarytool) for warning-free distribution."
  fi
fi
echo "    ----------------------------------------------------------------"
