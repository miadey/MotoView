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
| `diagnostics-server.js` | **The editor's inline-diagnostics bridge** (R6). A tiny, dependency-free localhost HTTP server. The studio editor POSTs the `.mview` buffer; this runs `motoview lint/check --json` over a throwaway copy and returns the R2 `{severity,rule,message,line,col,endLine,endCol}` array + the save-gate verdict, which the editor draws as squiggles/gutter markers. |
| `build-editor-bootstrap.js` | Derives the `editorBootstrap` Motoko `Text` constant in `apps/studio/src/Pages/Design.mview` from `apps/studio/assets/mview-editor.js` (base64-inlined `<script type="module">` via `@raw`). Re-run with `--inject` after editing the editor; `--check` verifies it is in sync. |
| `bridge-test.js` | Round-trips the bridge through the REAL compiler: a clean buffer is saveable; an unsecured `<form @submit>` yields a `secure-form` error with 1-based positions. |
| `run-tests.sh` | Runs all R6 editor/grammar tests (grammar tokenization, editor JS validity, bootstrap-in-sync, headless editor, bridge round-trip, studio build+lint). |
| `grammar-test/` | Headless `.mview` TextMate grammar tokenization test (vscode-textmate + vscode-oniguruma — the engine VS Code/Monaco use). |
| `editor-test/` | Headless jsdom + real-CodeMirror test: mounts the editor, asserts tokenization, and asserts an R2 diagnostic produces a lint marker decoration. |
| `README.md` | This file: the loop + honest dependencies. |

## The editor (R6)

The studio Design page (`apps/studio/src/Pages/Design.mview`) hosts a real
**CodeMirror 6** editor with `.mview` syntax highlighting and **inline
diagnostics**, replacing the old read-only `<pre>`:

- **Grammar:** `apps/studio/assets/mview.tmLanguage.json` — a portable TextMate
  grammar (template tags, `@directives`, `@expr`/`@raw`, `secure`/`bind`/`@event`
  attributes, embedded Motoko in `@code{…}`). Usable in VS Code / Monaco too.
- **Editor:** `apps/studio/assets/mview-editor.js` — CodeMirror 6 (pinned ESM
  CDN) + a `StreamLanguage` tokenizer that mirrors the grammar + an async linter
  that calls the bridge and renders `{line,col,endLine,endCol}` as squiggles.
- **Bridge:** `tools/studio/diagnostics-server.js` — run it for live diagnostics:

  ```sh
  node tools/studio/diagnostics-server.js          # 127.0.0.1:8731
  ```

> **DEV-TOOL exception to "no app JS".** Shipped MotoView apps run no app JS.
> The studio is a *design tool*, so it inlines the editor as a `<script
> type="module">` (base64, via `@raw`). This is deliberately scoped to the studio.
>
> **Framework gaps surfaced (real, not yet fixed here):**
> 1. **No clean app-static-asset mechanism** — there is no first-class way to ship
>    a `.js`/`.wasm` asset alongside a page, so the editor is base64-inlined. A
>    `@asset`/static-files directive would remove the workaround.
> 2. **HTML-comment parser bug** — a `<pre>`/`<script>`/`<style>` *tag name written
>    inside an HTML comment* (`<!-- … <pre> … -->`) flips the template parser into
>    raw-text mode and corrupts the parse downstream (manifested as `unbound
>    variable` errors). Worked around by not naming raw-text tags in comments.

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
