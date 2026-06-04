#!/usr/bin/env bash
# Type-checks one or more Motoko files against the dfx-bundled base library.
# Usage: tools/check.sh path/to/File.mo [more.mo ...]
set -euo pipefail
CACHE="$HOME/.cache/dfinity/versions/0.28.0"
MOC="$CACHE/moc"
BASE="$CACHE/base"
for f in "$@"; do
  echo "checking: $f"
  "$MOC" --check --package base "$BASE" "$f"
done
echo "OK"
