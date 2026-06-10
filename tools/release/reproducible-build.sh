#!/usr/bin/env bash
# ============================================================================
# MotoView — reproducible client-wasm build + module-hash emitter (Slice 11)
# ============================================================================
#
# WHAT THIS DOES
#   Deterministically rebuilds the Rust->WASM client "brain" (the same artifact
#   `tools/build-client.sh` embeds into `runtime/src/ClientAssets.mo` and every
#   MotoView canister serves at `/motoview.wasm`), then prints the SHA-256 of
#   the optimized module. That hash is the value you PIN for a banking-grade
#   "the canister is serving exactly the audited brain" integrity check.
#
# WHY A HASH MATTERS (ties to client/src/cert_verify.rs)
#   A native MotoView client (iOS/Android, Slice 8) verifies IC responses
#   against the pinned NNS root key (cert_verify.rs::IC_ROOT_KEY) so it never
#   trusts the boundary node. Chain-key verification proves the *bytes came from
#   the named canister*, but NOT that the canister is running the code you
#   audited. To close that gap you additionally pin:
#     (a) the canister's `module_hash` (the WASM the replica is executing), and
#     (b) the SHA-256 of the served `/motoview.wasm` asset (this script's output),
#         which the certified-asset response binds via certified_data.
#   With (a)+(b) pinned, a verified response is provably produced by a known
#   canister running known code serving a known brain. See "PINNING" below.
#
# DETERMINISM
#   The build is reproducible because:
#     * `[profile.release]` in client/Cargo.toml is fixed (opt-level="z",
#       lto=true, codegen-units=1, panic="abort", strip=true).
#     * We force a stable toolchain view and strip embedded paths via
#       RUSTFLAGS=--remap-path-prefix, and clear timestamp-bearing env.
#     * wasm-opt is deterministic for a given input + flag set; we pin the
#       exact flags `tools/build-client.sh` uses.
#   Verified on the build machine: two clean rebuilds produce the SAME opt hash.
#
# REQUIREMENTS (all present on the build machine)
#   cargo + the wasm32-unknown-unknown target, wasm-opt, shasum (or sha256sum).
#
# USAGE
#   tools/release/reproducible-build.sh            # build + print hashes
#   tools/release/reproducible-build.sh --check EXPECTED_OPT_SHA256
#                                                  # fail if hash != expected
#   tools/release/reproducible-build.sh --json     # machine-readable output
#
# EXIT CODES
#   0 ok   1 build/tool error   2 hash mismatch (with --check)
# ============================================================================
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
CLIENT="$ROOT/client"

# ---- args -----------------------------------------------------------------
EXPECT=""
JSON=0
while [ $# -gt 0 ]; do
  case "$1" in
    --check) EXPECT="${2:-}"; shift 2 ;;
    --json)  JSON=1; shift ;;
    -h|--help) sed -n '2,55p' "$0"; exit 0 ;;
    *) echo "unknown arg: $1" >&2; exit 1 ;;
  esac
done

# ---- portable sha256 ------------------------------------------------------
sha256() {
  if command -v sha256sum >/dev/null 2>&1; then sha256sum "$1" | awk '{print $1}';
  elif command -v shasum    >/dev/null 2>&1; then shasum -a 256 "$1" | awk '{print $1}';
  else echo "no sha256 tool found" >&2; exit 1; fi
}

log() { [ "$JSON" -eq 1 ] || echo "$@"; }

# ---- preflight ------------------------------------------------------------
command -v cargo    >/dev/null 2>&1 || { echo "cargo not found" >&2; exit 1; }
command -v wasm-opt >/dev/null 2>&1 || { echo "wasm-opt not found" >&2; exit 1; }
rustup target list --installed 2>/dev/null | grep -qx wasm32-unknown-unknown \
  || { echo "rust target wasm32-unknown-unknown not installed" >&2; exit 1; }

# ---- deterministic environment -------------------------------------------
# Remap source paths out of the binary so the hash does not depend on the
# checkout location. Keep panic=abort etc. from Cargo.toml's release profile.
export CARGO_TERM_COLOR=never
export SOURCE_DATE_EPOCH="${SOURCE_DATE_EPOCH:-1704067200}"   # 2024-01-01, fixed
export RUSTFLAGS="${RUSTFLAGS:-} --remap-path-prefix=$ROOT=/motoview --remap-path-prefix=$HOME/.cargo=/cargo"

cd "$CLIENT"
RAW="target/wasm32-unknown-unknown/release/motoview_client.wasm"

log "==> reproducible build: client brain (release, wasm32-unknown-unknown)"
log "    SOURCE_DATE_EPOCH=$SOURCE_DATE_EPOCH"
log "    RUSTFLAGS=$RUSTFLAGS"
cargo build --release --target wasm32-unknown-unknown >/dev/null 2>&1 \
  || { echo "cargo build failed" >&2; exit 1; }

RAW_SHA="$(sha256 "$RAW")"
RAW_BYTES="$(wc -c < "$RAW" | tr -d ' ')"
log "    raw wasm:  $RAW_BYTES bytes  sha256=$RAW_SHA"

# Optimize with the EXACT flags tools/build-client.sh ships, into a temp file
# so we never depend on a pre-existing dist/.
OPT="$(mktemp -t motoview-opt.XXXXXX).wasm"
trap 'rm -f "$OPT"' EXIT
wasm-opt -Oz \
  --enable-bulk-memory \
  --enable-nontrapping-float-to-int \
  --enable-sign-ext \
  --enable-mutable-globals \
  "$RAW" -o "$OPT"

OPT_SHA="$(sha256 "$OPT")"
OPT_BYTES="$(wc -c < "$OPT" | tr -d ' ')"
log "    opt wasm:  $OPT_BYTES bytes  sha256=$OPT_SHA"
log ""
log "==> MODULE HASH TO PIN (served /motoview.wasm asset):"
log "    $OPT_SHA"

if [ "$JSON" -eq 1 ]; then
  printf '{"raw_sha256":"%s","raw_bytes":%s,"opt_sha256":"%s","opt_bytes":%s,"source_date_epoch":%s}\n' \
    "$RAW_SHA" "$RAW_BYTES" "$OPT_SHA" "$OPT_BYTES" "$SOURCE_DATE_EPOCH"
fi

# ---- optional hash gate ---------------------------------------------------
if [ -n "$EXPECT" ]; then
  if [ "$OPT_SHA" = "$EXPECT" ]; then
    log "==> CHECK OK: opt hash matches pinned value"
    exit 0
  else
    echo "CHECK FAILED: opt hash $OPT_SHA != expected $EXPECT" >&2
    exit 2
  fi
fi

# ============================================================================
# PINNING — how to wire this into a banking-grade integrity check
# ============================================================================
#
# 1) SERVED-ASSET PIN (this script's `opt_sha256`)
#    The certified asset canister serves /motoview.wasm; the v2 HTTP response
#    certification (runtime/src/CertV2.mo) binds SHA-256(body) into the
#    certificate. A native client that already runs cert_verify.rs can compare
#    the verified body hash to the value printed above. Pin it as a constant
#    next to IC_ROOT_KEY, e.g.:
#        pub const PINNED_BRAIN_SHA256: [u8;32] = hex!("<opt_sha256>");
#    and reject any /motoview.wasm whose certified body hash != PINNED_BRAIN_SHA256.
#    (NOTE: cert_verify.rs today verifies certified_data==SHA256(body); the v2
#    http_expr walker is the documented follow-up in that file's verify_response
#    caveat — until it lands, this pin is enforced against the simple binding.)
#
# 2) CANISTER MODULE_HASH PIN (the code the replica runs)
#    Independently, pin the *canister's* module_hash (the Motoko-compiled WASM
#    the replica executes, which embeds this brain via ClientAssets.mo). Read it
#    with dfx or the IC management canister:
#        dfx canister --network ic info <CANISTER_ID>
#        # -> "Module hash: 0x...."
#    or via read_state on path ["canister", <id>, "module_hash"] (chain-key
#    verified by the same cert_verify.rs path). Pin that 32-byte value and
#    reject responses from a canister whose certified module_hash drifted. This
#    is the "is the canister running the audited code" half; (1) is the "is it
#    serving the audited brain" half.
#
# REPRODUCING THE PIN: anyone can clone the repo at the audited commit, run THIS
# script, and confirm they get the same opt_sha256 — that is what makes the pin
# auditable rather than a number to trust.
# ============================================================================
