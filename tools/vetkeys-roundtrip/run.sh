#!/usr/bin/env bash
# Verify a deployed MotoView app's vetKeys endpoints end-to-end:
#   client transport key -> mvVetkdDeriveKey -> unwrap the vetKey -> IBE round-trip.
#
# Usage:  tools/vetkeys-roundtrip/run.sh <canister-name> [project-dir]
# Prints: ROUND_TRIP_OK ...   on success.
set -euo pipefail
CANISTER="${1:?usage: run.sh <canister-name> [project-dir]}"
DIR="${2:-.}"
HERE="$(cd "$(dirname "$0")" && pwd)"

# Note: clear CC/CFLAGS if your shell points them at a non-host toolchain.
( cd "$HERE" && env -u CC -u CFLAGS cargo build --release >/dev/null )
VKH="$HERE/target/release/vkh"

cd "$DIR"
TPK="$("$VKH" gen)"                                                  # 48-byte transport public key
dfx canister call "$CANISTER" mvVetkdPublicKey            > /tmp/mv_master.txt 2>/dev/null
dfx canister call "$CANISTER" mvVetkdDeriveKey "(blob \"$TPK\")" > /tmp/mv_ek.txt 2>/dev/null
MASTER="$(python3 "$HERE/decode.py" /tmp/mv_master.txt)"            # 96-byte master public key
EK="$(python3 "$HERE/decode.py" /tmp/mv_ek.txt)"                    # 192-byte encrypted vetKey
PRIN="$(dfx identity get-principal)"                               # the derivation input (caller)
exec "$VKH" verify "$MASTER" "$EK" "$PRIN"
