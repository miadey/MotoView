# __NAME__

A MotoView app — server-driven UI written in Motoko, served from an ICP
canister. No Node, no npm, no JavaScript build step.

## Layout

```
__NAME__/
├── motoview.json        # project config (name, src/out, seo)
├── dfx.json             # DFINITY SDK manifest (canister + runtime package arg)
├── mops.toml            # Motoko deps (the `motoview` runtime)
└── src/
    ├── Pages/           # routable .mview templates (start at Home.mview)
    └── Layouts/         # shared page shell (MainLayout.mview)
```

Add `src/Components/*.mview` (reusable UI), `src/Services/*.mo` (business logic
+ upgrade-stable state) and `src/Models/*.mo` (types) as your app grows.

## Run it

```bash
motoview build        # compile .mview -> .mvbuild/main.mo
motoview check        # build, then type-check (errors map back to your .mview)
motoview lint         # security lint (secure forms, @authorize, @raw)
motoview dev          # build, then `dfx deploy` to the local replica
```

Open the printed canister URL: the page is served straight from the canister.
Clicking the button runs the `event -> update -> batch -> DOM swap` cycle.

> The generated actor lives in `.mvbuild/main.mo` — a build artifact (like
> Blazor's `obj/`), gitignored and regenerated on every build. You never edit
> it; you edit the `.mview` files in `src/`.
