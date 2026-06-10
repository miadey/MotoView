# tools/studio — MotokoStudio design-time orchestration (off-canister)

MotokoStudio lets you design MotoView apps with AI, **always bound to the
backend**, and **secure by construction**. The studio *app* (the shell you look
at) lives in `apps/studio/` and is itself a real MotoView app. This directory is
the **design-time orchestration** that runs **off-canister** around it.

> **Honesty first.** Read this before anything else:
> - **There is NO on-canister LLM.** AI generation is an external, off-canister,
>   design-time HTTP call. The studio canister never runs a model.
> - **Live preview needs a running `dfx` replica.** There is no mock preview; if
>   nothing is deployed, the preview frame is blank, on purpose.
> - **The visual drag-and-drop editor is a scaffold/placeholder.** Today the
>   `.mview` source pane is authoritative — it is what the gate validates and what
>   deploys.
> - The thing that *is* real and verifiable: the **save gate** (`validate.sh`).
>   The compiler is unbypassable, so an insecure or ill-typed artifact cannot be
>   saved or deployed — not even if the AI generates one.

## The studio loop

```
   ┌──────────┐   ┌────────────┐   ┌───────────────────┐   ┌──────────┐   ┌──────────┐
   │  prompt  │ → │  generate  │ → │   validate GATE   │ → │  deploy  │ → │ preview  │
   │ (human)  │   │ (off-chain │   │ check + lint      │   │ dfx →    │   │ iframe → │
   │          │   │  LLM call) │   │ REFUSE if fails   │   │ replica  │   │ replica  │
   └──────────┘   └────────────┘   └───────────────────┘   └──────────┘   └──────────┘
                       ▲                    │ nonzero
                       └────── retry ───────┘  (artifact discarded; error shown)
```

1. **Prompt** — describe the page you want.
2. **Generate** — an off-canister LLM emits ONE complete `.mview` (template +
   `@code` + typed state), bound to the project's REAL Motoko service signatures
   (the "binding palette"). See [`generate.md`](./generate.md). This step is an
   interface/contract — no model is faked here.
3. **Validate gate** — [`validate.sh`](./validate.sh) runs `motoview lint` and
   `motoview check`. **If either fails, it exits nonzero and the artifact is
   refused.** Security-by-construction: an unsecured mutating `<form @submit>` is
   a hard build Error (Slice 1), so the studio cannot save it.
4. **Deploy** — `dfx deploy` builds the generated Motoko actor onto a local
   replica.
5. **Preview** — the studio's `/preview` page embeds the running canister in an
   iframe. Real canister, no simulation.

## Files here

| File | What it is |
|------|------------|
| `validate.sh` | **The save gate.** Runs `lint` + `check`; exits nonzero if either fails. This is the unbypassable, security-by-construction enforcement. Real and tested. |
| `generate.md` | The AI-generation **contract**: input (prompt + real service signatures) → output (one complete `.mview`) → MUST pass `validate.sh`. The LLM call is marked off-canister/design-time and is **not** faked. |
| `README.md` | This file: the loop + honest dependencies. |

## Using the gate

```sh
# From the repo root. Exits 0 only if BOTH gates pass.
tools/studio/validate.sh apps/studio              # a clean project → exit 0
tools/studio/validate.sh path/to/your/project     # your app

# Point at a different compiler build if needed:
MOTOVIEW=/abs/path/to/motoview tools/studio/validate.sh <project>
```

Exit codes: `0` saveable · `1` lint failed (insecure/invalid) · `2` type-check
failed · `3` usage/environment error.

## Dependencies (be honest about these)

- **The compiler** `compiler/target/release/motoview` (for `lint` + `check`).
- **`moc` via `dfx`** for the `check` type-check. If `moc` is unavailable, `check`
  is a no-op (best-effort) and only `lint` gates — so build it (`dfx` installed)
  for the full type-check gate. The project's `dfx.json` must declare the runtime
  package (`--package motoview <path>`), exactly like `apps/studio/dfx.json`.
- **A `dfx` replica** for deploy + live preview (`dfx start` then `dfx deploy`).
- **An external LLM endpoint** for the generation step — supplied at deploy time
  via env (`$MOTOVIEW_STUDIO_LLM_ENDPOINT`, `$MOTOVIEW_STUDIO_LLM_KEY`). Not
  bundled, not faked.

## Templates

Ready-made, lint-clean patterns to seed a generation or start by hand live in
[`apps/studio/templates/`](../../apps/studio/templates/):

- `SecureForm.mview` — a `secure` form with `validate { }` and `@authorize`.
- `Identity.mview` — Internet Identity sign-in + a role-gated view.
- `Wallet.mview` — a spend flow that confirms the intent and runs the intent-bound
  authorization gate (`ctx.authorizeSpend`) before any signing, referencing
  `ChainKey` + `EncStore`.

Each illustrates wiring to the verified runtime modules and passes `motoview
lint`.
