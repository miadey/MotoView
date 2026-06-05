#!/usr/bin/env bash
# Assemble the combined MotoView site + Bzzz app into apps/launch and build it.
# Routes: the site serves /, /docs, /components, /variants; Bzzz serves /app + its
# own routes (/feed, /forum, /servers, /messages, /me, /status, /about, /admin, ...).
# One canister, one MotoView actor. Deploy target: dvl2u-oaaaa-aaaap-quswq-cai.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"; cd "$ROOT"
rm -rf apps/launch/src apps/launch/.mvbuild
mkdir -p apps/launch/src/{Pages,Layouts,Services}
cp apps/site/src/Pages/*.mview apps/launch/src/Pages/
for f in apps/bzzz/src/Pages/*.mview; do
  b=$(basename "$f")
  if [ "$b" = "Home.mview" ]; then sed 's#@page "/"#@page "/app"#' "$f" > apps/launch/src/Pages/AppHome.mview
  else cp "$f" apps/launch/src/Pages/; fi
done
cp apps/site/src/Layouts/*.mview apps/bzzz/src/Layouts/*.mview apps/launch/src/Layouts/
cp apps/site/src/Services/*.mo apps/bzzz/src/Services/*.mo apps/launch/src/Services/
[ -d apps/bzzz/src/Components ] && cp -r apps/bzzz/src/Components apps/launch/src/ || true
# the marketing site owns "/", so point Bzzz's brand + Home nav at /app
sed -i.bak 's#<a class="mv-appbar-brand" href="/">#<a class="mv-appbar-brand" href="/app">#' apps/launch/src/Layouts/AppLayout.mview
sed -i.bak 's#<NavItem href="/" #<NavItem href="/app" #' apps/launch/src/Layouts/AppLayout.mview
rm -f apps/launch/src/Layouts/*.bak
compiler/target/release/motoview build apps/launch --name motoview
echo "Built apps/launch. Deploy to mainnet:"
echo "  cd apps/launch && DFX_WARNING=-mainnet_plaintext_identity dfx deploy motoview --network ic --mode reinstall --yes"
