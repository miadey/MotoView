#!/usr/bin/env bash
#
# bundle.sh — package the native MotokoStudio as a distributable macOS
#             .app + .dmg, using ONLY the Command-Line-Tools (no full Xcode,
#             no cargo-bundle / third-party bundler, no Apple account).
#
# What it does (in order):
#   1. cargo build --release            (the wgpu/Metal native studio)
#   2. assemble dist/MotokoStudio.app/Contents/{Info.plist,MacOS/,Resources/}
#   3. codesign --sign -  (AD-HOC)       — required so it launches on Apple
#                                          Silicon without a "damaged" kill.
#      ...or, with --sign "Developer ID Application: ...", a REAL signature.
#   4. hdiutil create ... UDZO           — the distributable .dmg
#   5. print where the artifacts are + the HONEST signing/notarization note.
#
# Usage:
#   bash bundle.sh                       # default: AD-HOC signature (no account)
#   bash bundle.sh --sign "Developer ID Application: Your Name (TEAMID)"
#   bash bundle.sh --sign "..." --notarize-profile "<notarytool-keychain-profile>"
#
# This script is deliberately transparent and dependency-free: every tool it
# calls (cargo, codesign, hdiutil, plutil, file, lipo) is the stock toolchain.
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

# ---- parse args -------------------------------------------------------------
SIGN_IDENTITY="-"                   # "-" == ad-hoc (no cert / no account)
NOTARIZE_PROFILE=""
while [ $# -gt 0 ]; do
  case "$1" in
    --sign)
      SIGN_IDENTITY="${2:?--sign needs an identity, e.g. \"Developer ID Application: Name (TEAMID)\"}"
      shift 2 ;;
    --notarize-profile)
      NOTARIZE_PROFILE="${2:?--notarize-profile needs a notarytool keychain profile name}"
      shift 2 ;;
    -h|--help)
      sed -n '2,30p' "$SELF"; exit 0 ;;
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
for tool in cargo codesign hdiutil plutil file; do
  command -v "$tool" >/dev/null 2>&1 || { echo "FATAL: '$tool' not found on PATH" >&2; exit 1; }
done

# ---- 1. release build -------------------------------------------------------
echo "==> [1/5] cargo build --release"
cargo build --release
SRC_BIN="$SCRIPT_DIR/target/release/$BIN_NAME"
[ -x "$SRC_BIN" ] || { echo "FATAL: release binary not found at $SRC_BIN" >&2; exit 1; }

# ---- 2. assemble the .app bundle --------------------------------------------
echo "==> [2/5] assembling $APP_NAME.app"
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

# 2c. OPTIONAL icon: only if a committed/working .icns exists.
if [ -f "$SCRIPT_DIR/$APP_NAME.icns" ]; then
  cp "$SCRIPT_DIR/$APP_NAME.icns" "$APP_DIR/Contents/Resources/$APP_NAME.icns"
  echo "    bundled icon $APP_NAME.icns"
else
  # No icon committed: drop the CFBundleIconFile key so we don't dangle-point
  # at a missing resource. macOS then uses the generic app icon.
  plutil -remove CFBundleIconFile "$APP_DIR/Contents/Info.plist" 2>/dev/null || true
  echo "    no $APP_NAME.icns — using the generic app icon (CFBundleIconFile removed)"
fi

# 2d. PkgInfo (cosmetic but conventional): APPL + 4-char signature.
printf 'APPL????' > "$APP_DIR/Contents/PkgInfo"

# 2e. validate the plist before we sign anything.
plutil -lint "$APP_DIR/Contents/Info.plist"

# ---- 3. codesign (AD-HOC by default) ----------------------------------------
echo "==> [3/5] codesign ($SIGN_LABEL)"
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

# ---- 4. make the .dmg -------------------------------------------------------
echo "==> [4/5] hdiutil create $APP_NAME.dmg"
rm -f "$DMG_PATH"
hdiutil create -volname "$APP_NAME" -srcfolder "$APP_DIR" \
               -ov -format UDZO "$DMG_PATH" >/dev/null
# A signed (non-ad-hoc) build can/should also sign the .dmg itself.
if [ "$SIGN_IDENTITY" != "-" ]; then
  codesign --force --sign "$SIGN_IDENTITY" "$DMG_PATH"
fi

# ---- 4b. OPTIONAL notarization (real cert + notarytool only) ----------------
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

# ---- 5. report + honest signing note ----------------------------------------
echo
echo "==> [5/5] done."
echo "    app : $APP_DIR"
echo "    dmg : $DMG_PATH"
echo
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
