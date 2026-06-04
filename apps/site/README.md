# MotoView site — built on MotoView

The MotoView marketing + documentation site, **served by a MotoView canister**.
We eat our own dog food: the landing page and all 27 documentation pages are
`.mview` pages compiled to a Motoko actor — no static host, no JS framework.

## What's here

```
src/
  Layouts/
    HomeLayout.mview   landing-page shell (its own CSS)
    DocLayout.mview    docs shell — topbar + sidebar + content; calls Docs.sidebarForPath(ctx.path)
  Pages/
    Home.mview         @page "/"            — landing (hero, features, code window, CTA)
    DocsIndex.mview    @page "/docs"         — docs overview
    Doc.mview          @page "/docs/{slug}"  — one doc, rendered from the Docs service via @raw
  Services/
    Docs.mo            GENERATED — every doc's HTML + the sidebar nav (see tools/gen-docs.py)
tools/
  gen-docs.py          converts /docs/*.md  →  src/Services/Docs.mo
```

## Content pipeline

The docs live as markdown in the repo's top-level `/docs`. `tools/gen-docs.py`
converts them to HTML and bakes them into `src/Services/Docs.mo` (one record per
page + a `sidebar(active)` builder). The `Doc.mview` page looks a page up by its
route slug and emits the HTML with the `@raw(...)` directive (trusted,
unescaped). Re-run it whenever the docs change:

```bash
python3 apps/site/tools/gen-docs.py
```

## Certified queries

Every `/docs/<slug>` page is marked `@cacheable`, so it's served as a fast
**certified query** over a single wildcard certificate (`/docs/<*>`) — no
consensus round-trip. The root `/` and the exact `/docs` index fall back to the
(always-correct) update path automatically. The static framework assets
(`/motoview.js`, `/motoview.css`, …) are certified too.

## Build & run

```bash
python3 apps/site/tools/gen-docs.py          # regenerate content (after editing /docs)
motoview build apps/site --name site         # compile .mview -> src/main.mo
cd apps/site && dfx deploy                    # local replica
# or: dfx deploy --playground                 # public, on the IC mainnet boundary
```

Open the printed `http://<canister-id>.localhost:4955/`.
