#!/usr/bin/env bash
#
# run-tests.sh — run all MotokoStudio design-time tests (R5 loop + R6 editor).
#
# R6 (editor + grammar):
#  1. grammar tokenization (vscode-textmate over apps/studio/assets/mview.tmLanguage.json)
#  2. editor JS syntax validity (node --check, ESM)
#  3. bootstrap-in-sync check (the inlined editor matches mview-editor.js)
#  4. headless editor test (jsdom + real CodeMirror: mount, tokenize, diagnostics)
#  5. diagnostics-bridge round-trip over the REAL compiler (lint/check --json)
#  6. studio still builds + lints clean
#
# R5 (AI generate/validate/repair loop + binding palette):
#  7. palette extraction (signatures.js over the fixture's Services/*.mo + a real project)
#  8. loop CONVERGENCE (mock: broken -> fed-back diagnostic -> fixed; saveable in <=2 rounds)
#  9. loop GATE (mock: always-broken -> budget exhausted -> FAILURE, never saveable)
#
# Requires: node, and the repo-root compiler at compiler/target/release/motoview
# (or $MOTOVIEW). Run from anywhere; paths are resolved from this script.
set -u

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" >/dev/null 2>&1 && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../.." >/dev/null 2>&1 && pwd)"
BIN="${MOTOVIEW:-$REPO_ROOT/compiler/target/release/motoview}"
export MOTOVIEW="$BIN"

fail=0
step() { echo; echo "=== $* ==="; }

step "1/6 grammar tokenization"
( cd "$SCRIPT_DIR/grammar-test" && node tokenize-test.js ) || fail=1

step "2/6 editor JS node --check (ESM)"
if node --check --input-type=module < "$REPO_ROOT/apps/studio/assets/mview-editor.js"; then
  echo "editor JS: VALID"
else
  echo "editor JS: INVALID"; fail=1
fi

step "3/6 editorBootstrap in sync with mview-editor.js"
if node "$SCRIPT_DIR/build-editor-bootstrap.js" --check; then
  echo "bootstrap: in sync"
else
  echo "bootstrap: STALE — run: node tools/studio/build-editor-bootstrap.js --inject"; fail=1
fi

step "4/6 headless editor test (jsdom + CodeMirror)"
( cd "$SCRIPT_DIR/editor-test" && node editor-test.mjs 2>/dev/null ) || fail=1

step "5/6 diagnostics bridge round-trip (real compiler)"
MOTOVIEW="$BIN" node "$SCRIPT_DIR/bridge-test.js" || fail=1

step "6/9 studio builds + lints clean"
"$BIN" build "$REPO_ROOT/apps/studio" >/dev/null 2>&1 && echo "build: OK" || { echo "build: FAIL"; fail=1; }
"$BIN" lint "$REPO_ROOT/apps/studio" >/dev/null 2>&1 && echo "lint: OK (0 errors)" || { echo "lint: FAIL (errors)"; fail=1; }

step "7/9 R5 palette extraction (signatures.js)"
node "$SCRIPT_DIR/gen-test/palette-test.js" || fail=1

step "8/9 R5 loop CONVERGENCE (mock: broken -> fed-back diag -> fixed)"
node "$SCRIPT_DIR/gen-test/convergence-test.js" || fail=1

step "9/9 R5 loop GATE (mock: always-broken -> never saveable)"
node "$SCRIPT_DIR/gen-test/gate-test.js" || fail=1

# R7 (debug/observability): structured log parser + IR view-tree builder.
step "R7 log parser (structured Debug.print stream -> events)"
node "$SCRIPT_DIR/log-parser-test.js" || fail=1

step "R7 view-tree (motoview preview IR forest -> collapsible tree)"
MOTOVIEW="$BIN" node "$SCRIPT_DIR/view-tree-test.js" || fail=1

# R10 (Tier-3 leapfrog): deterministic record/replay + 3-up canvas forest fan-out.
step "R10 replay-determinism (same session -> identical forest; counter -> 2)"
MOTOVIEW="$BIN" node "$SCRIPT_DIR/replay-test.js" || fail=1

step "R10 3-up canvas fan-out (one forest -> web + iOS + Android pane data)"
MOTOVIEW="$BIN" node "$SCRIPT_DIR/fanout-test.js" || fail=1

echo
if [ "$fail" -eq 0 ]; then
  echo "ALL STUDIO TESTS PASSED (R5 loop + R6 editor + R7 observability)"
  exit 0
else
  echo "SOME STUDIO TESTS FAILED"
  exit 1
fi
