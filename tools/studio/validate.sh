#!/usr/bin/env bash
#
# validate.sh — the MotokoStudio SAVE GATE (security-by-construction).
#
# The studio MUST NOT save or deploy a .mview artifact that the unbypassable
# compiler rejects. This script runs the compiler over a project dir and EXITS
# NONZERO if EITHER gate fails:
#
#   1. motoview lint  <dir>   — deny-by-default security lint. A state-mutating
#                               <form @submit> without `secure` is a hard Error
#                               (CSRF + over-posting hole), among other rules.
#                               This gate needs NO replica / moc.
#   2. motoview check <dir>   — full Motoko type-check of the generated actor
#                               (catches type errors, bad service wiring, etc).
#                               Needs `moc` (from dfx) + the runtime `--package`
#                               in the project's dfx.json. If moc is unavailable,
#                               `check` is a no-op (best-effort) — lint still runs.
#
# The AI cannot talk its way past this: generation is OFF-canister and design-time,
# but the artifact only becomes saveable once the COMPILER accepts it. See
# tools/studio/generate.md (the generation contract) and tools/studio/README.md
# (the studio loop).
#
# Usage:
#   tools/studio/validate.sh <project-dir>
#   MOTOVIEW=/path/to/motoview tools/studio/validate.sh <project-dir>
#
# Exit codes:
#   0  — both lint and check passed; the artifact is saveable.
#   1  — lint failed (insecure / invalid) — REFUSE to save.
#   2  — check (type-check) failed — REFUSE to save.
#   3  — usage / environment error (no project dir, no compiler).

set -u

PROJECT_DIR="${1:-}"

# Resolve this script's dir so we can find the repo-root compiler by default.
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" >/dev/null 2>&1 && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../.." >/dev/null 2>&1 && pwd)"

# The compiler binary: explicit $MOTOVIEW, else the repo release build, else PATH.
if [ -n "${MOTOVIEW:-}" ]; then
  BIN="$MOTOVIEW"
elif [ -x "$REPO_ROOT/compiler/target/release/motoview" ]; then
  BIN="$REPO_ROOT/compiler/target/release/motoview"
elif command -v motoview >/dev/null 2>&1; then
  BIN="$(command -v motoview)"
else
  echo "validate: FAIL — could not find the 'motoview' compiler binary." >&2
  echo "  set MOTOVIEW=/path/to/motoview, or build compiler/target/release/motoview." >&2
  exit 3
fi

if [ -z "$PROJECT_DIR" ]; then
  echo "usage: validate.sh <project-dir>" >&2
  exit 3
fi
if [ ! -f "$PROJECT_DIR/motoview.json" ]; then
  echo "validate: FAIL — '$PROJECT_DIR' is not a MotoView project (no motoview.json)." >&2
  exit 3
fi

echo "== MotokoStudio save gate =="
echo "compiler: $BIN"
echo "project:  $PROJECT_DIR"
echo

# ── Gate 1: lint (deny-by-default security) ───────────────────────────────────
echo "--- gate 1/2: motoview lint ---"
if ! "$BIN" lint "$PROJECT_DIR"; then
  echo
  echo "validate: REFUSED — lint failed. The artifact is insecure or invalid and"
  echo "                    will NOT be saved. (e.g. an unsecured mutating form.)"
  exit 1
fi
echo "lint: OK"
echo

# ── Gate 2: check (full Motoko type-check) ────────────────────────────────────
echo "--- gate 2/2: motoview check ---"
if ! "$BIN" check "$PROJECT_DIR"; then
  echo
  echo "validate: REFUSED — type-check failed. The generated actor does not type"
  echo "                    against the runtime; the artifact will NOT be saved."
  exit 2
fi
echo "check: OK"
echo

echo "validate: PASS — both gates clean. Artifact is saveable/deployable."
exit 0
