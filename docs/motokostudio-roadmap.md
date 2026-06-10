# MotokoStudio: Roadmap to a World-Class Dev Experience

**Audience:** the MotoView creator.
**Question this answers:** what is missing for MotokoStudio to be as usable and performant as Xcode / Android Studio (and the best modern AI app builders) — a world-class experience that makes developing Motoko apps very easy?
**Tone:** honest. Where MotoView is far behind, this says so; where it can leapfrog, the advantage is justified from the actual code, not spin. Verified against the tree 2026-06-09.

> **Honesty header — read first.** MotokoStudio today is **not an IDE**. `apps/studio` is three MotoView pages: a Design page whose "editor" is a **read-only `<pre>`** over a static starter string (it self-labels *"visual editor: placeholder"*), a Preview page that is an **`<iframe>` to an already-deployed canister** (*"no mock preview — deploy first, then preview"*), and a Security posture page. There is **no text editor** (you cannot type a character), **no language server**, **no live preview**, **no debugger/profiler**, and **no working AI** (only `tools/studio/generate.md`, a contract that ships no model). The one working thing is a batch CLI gate: `motoview check` (full moc type-check, ~0.34s) + `motoview lint` (deny-by-default security rules, ~0.01s), chained by `tools/studio/validate.sh`. Everything below treats that as the starting point, not as a finished studio.

---

## 1. The blunt truth

The gap versus Xcode / Android Studio is **essentially the entire IDE** — every interactive surface those tools are *made of* is unbuilt here. But the gap is **asymmetric**: MotoView is behind on everything *except* the one thing the whole industry is currently chasing — a correctness/security oracle a model cannot bypass — which it already has and which already runs fast.

So the job is not to invent an IDE from zero. It is to **wrap interactive surfaces around a compiler, an IR, and a gate that already exist** (`compiler/`, `runtime/src/Ir.mo`, `tools/studio/validate.sh`). That reframing is what makes this roadmap a quarter of integration work for "usable," not a multi-year greenfield IDE project.

---

## 2. The gap, by dimension

| Dimension | World-class bar (2025–26) | MotoView today | Key missing piece(s) |
|---|---|---|---|
| **Editor & language intelligence** | SourceKit-LSP / Kotlin K2: sub-100ms completion, diagnostics, go-to-def, rename, fix-its | Read-only `<pre>`; no highlighting; no editor | A real editor (Monaco/CodeMirror 6) + a `.mview` language server. **Blocker: the AST carries zero spans** — diagnostics can name only a *file*, not a line/col |
| **Live preview & hot reload** | SwiftUI/Compose canvas, <1s, stateful, multi-device | `<iframe>` to a deployed canister; `dev` = build + full `dfx deploy`; no watcher | A no-deploy IR preview harness + watch/push. `EmitMode::Ir` exists but is not wired into the pipeline |
| **Debug, profile, observe** | LLDB step-debug, Instruments, Layout Inspector, Logcat | Nothing. Generated runtime emits **zero `Debug.print`** | Log stream, event/batch timeline, view-tree inspector, per-event cost — all tappable from the existing protocol |
| **AI codegen** | Agentic generate→build→repair loop, multi-file, design-to-code | A markdown contract; no LLM call, no loop | A real generation service + a closed validate/repair loop + `motoview check --json` + a signature-palette extractor |
| **Lifecycle / build / distribute** | Templates, build variants, one-click cloud build, signed store ship | `new` = one hardcoded scaffold; native pipeline wired but **never run** (nothing signed/uploaded) | Template selection, a canister/env registry, a cycles on-ramp, and **one** proven native ship |
| **Onboarding / learnability** | Zero-install playground, in-IDE lessons, prompt-to-app | CLI-only; heavy local toolchain; docs↔scaffold drift | Zero-install playground, in-studio example gallery, guided first-app flow, inline teaching errors |

---

## 3. The leapfrog thesis — where MotoView can match-or-beat the native IDEs

These are **structural advantages that already exist in the repo**, not aspirations.

**(1) Multi-platform live preview from one source — web + iOS + Android simultaneously.**
The compiler already emits a portable `UINode` forest (`runtime/src/Ir.mo`) that **three renderers consume from the same bytes**: the Rust brain (`client/src/ir.rs` `render_forest`), iOS SwiftUI (`clients/ios/.../NativeView.swift`), and Android Compose (`clients/android/.../NativeView.kt`). Xcode previews only Apple; Android Studio only Android. A 3-up canvas updated on every save is **unique *and* mostly already built** — the renderers exist; only the harness wiring `source → forest → panes` is missing. This is integration, not invention.

**(2) The preview *is* the real app, and the web target needs no native compile.**
Because a `.mview` is ONE type-checked artifact with no client/server contract to drift, a green preview is *provably* the code that ships — SwiftUI/Compose previews are best-effort renders of a view in isolation that can diverge from runtime. And the web forest is a ~0.02s Rust transpile rendered by the brain, so the web pane is **instant ground truth** while native panes catch up. The compile-gate that pushes iOS developers to hot-reload hacks simply does not exist on the web target.

**(3) Compiler-as-oracle makes AI codegen reliable AND secure-by-construction.**
`validate.sh` is unbypassable and deny-by-default: a state-mutating `<form @submit>` without `secure` is a *hard error* (`compiler/src/lint.rs`), `@raw` on dynamic data is flagged, wallet spends must route through `authorizeSpend`. So an AI **literally cannot save an app that fails type-check or the security lint** — a guarantee Cursor / v0 / Xcode-AI structurally cannot make. The research frontier the field is chasing (type-constrained, execution-grounded generation) is *native* here: the moc type system **is** the constraint oracle, for free.

**(4) Server-driven UI turns "attach a debugger" into "tap a protocol."**
Every event round-trips through `serveEvent` producing a deterministic, versioned `Batch` (status / batchId / effects / handler / caller), and `Ir.mo` serializes the rendered tree as JSON on every render. So the view-hierarchy inspector, the event timeline, and **deterministic record/replay time-travel** that Xcode/Android approximate only with heavyweight instruments are near-free taps on data MotoView already emits. The IC's deterministic replay — the very property that makes live step-debugging hard — is exactly what makes **record/replay trivial**.

---

## 4. Prioritized roadmap

Effort: **S** = days–1wk · **M** = weeks · **L** = month+. "Reuse" is the story — leverage what already runs.

### Tier 1 — "Usable" (the minimum a developer would choose) · ~a quarter

| Deliverable | Effort | Reuse |
|---|---|---|
| **Real editor in the studio**: embed Monaco/CM6, wire to the session buffer, replace the read-only `<pre>`; add a TextMate/Monarch `.mview` grammar (template + `@code` Motoko + `@directives`) | **S** | New front-end; no compiler change |
| **File-level inline diagnostics**: re-run the existing `check`/`lint` on idle, surface results inline | **S** | Reuses the whole compiler + the existing moc→`.mview` mapping (`// mv:src` markers, `compiler/src/project.rs`) |
| **AST/parser span rewrite** *(the universal unlock)*: add line/col/offset to `ast.rs` nodes and carry them through codegen without breaking the golden tests | **L** | Touches `parser.rs`/`ast.rs`/`project.rs`; gates everything line-accurate |
| **`.mview` language server**: a thin outer server that delegates `@code` spans to the **WASM Motoko LSP** (`dfinity/vscode-motoko`, runs in-browser) and owns template/directive intelligence | **L** | **Embeds an existing mature LSP** — avoids writing a Motoko analyzer |
| **No-deploy web preview**: wire `EmitMode::Ir` through `project.rs` to emit a forest; `motoview preview` + watch pushes it over SSE; render with `client/src/ir.rs` `render_forest` | **M** | Reuses the IR backend + keyed `ir_diff` for state-preserving swaps |
| **Real AI codegen loop**: make the generate seam call an LLM, run `validate.sh`, feed compiler errors back, retry to green | **M** | Needs `motoview check --json` (new) + a signature-palette extractor; rests safety on the existing gate |

### Tier 2 — "Competitive" (parity on table-stakes) · ~1–2 quarters

- **Debug/observability suite** *(M, very high reuse)*: log stream (auto-instrument generated handlers with structured `Debug.print` in `codegen.rs` + tail `dfx canister logs`); event/batch timeline (the `Batch` is already serialized — just tap it); view-hierarchy inspector with click-to-source (`Ir.mo` already emits the tree); per-event instruction cost via one `ic0.performance_counter` wrap; read-only live state viewer over existing `dump*` accessors. **Explicitly do NOT** attempt source-step debugging or memory-graph tooling here — both are research-grade on the IC.
- **Multi-device preview panes** *(M–L)*: feed the same forest into a SwiftUI preview process and a Compose preview — integration/IPC, not new rendering logic; lands the unique 3-up canvas.
- **Project + canister management** *(M)*: wire `apps/studio/templates` (SecureForm/Identity/Wallet) into `motoview new --template` + `git init`; a canister-id/env registry in `motoview.json`; a cycles on-ramp (balance/top-up/pre-deploy funding check); one-click web deploy surfaced in the studio.
- **Formatting + code-actions** *(M, once spans exist)*: a `.mview` formatter; lint rules re-expressed as quick-fixes — an **"add `secure` + ValidationSummary"** fix-it is a *category Xcode can't offer*.
- **Onboarding / playground** *(M–L)*: fix the docs↔scaffold drift first *(S, hours — `new` should write the promised AI rule files + `mops.toml`, align `var` vs `stable var`)*; an in-studio gallery over the 8 real `apps/`; a guided first-app stepper; a hosted zero-install playground (the expensive, highest-leverage onboarding lever).

### Tier 3 — "World-class / differentiated" (the leapfrog) · multi-quarter

- **Deterministic record/replay time-travel debugging** *(M–L)*: capture `(handler, args, caller, batch)` and step the app forward/back; PocketIC backs the deterministic replay. The genuine differentiator no native IDE has cheaply.
- **The 3-up live canvas as the default surface**: web + iOS + Android from one edit, state-preserved via the existing `ir_diff`.
- **Backend-bound completion as a default, not a feature**: the completion palette *is* the project's real Motoko service surface (one artifact, no drift). Builds on spans + the type info the compiler already computes for `check`.
- **Multi-file / whole-app agentic generation** grounded in the signature extractor, with conversational refine + checkpoints (grow `Studio.mo` a history list, not one slot).
- **Design-from-image / Figma → Fluent tokens** *(L, net-new vision step; lower priority than the loop)*.
- **One proven native store ship** *(high effort, gated on money/machines not code)*: a real `.xcodeproj` app target Archived to TestFlight once — moving the wired-but-never-run pipeline from "complete and flagged" to "demonstrated."

---

## 5. The hardest unknowns (and how to de-risk)

1. **The composite `.mview` language server.** No off-the-shelf server exists for "template + embedded Motoko + custom directives"; region-projection (hand the Motoko LSP a valid Motoko doc for `@code`, translate positions back) is the classic embedded-language problem that HTML+JS, Vue, and Svelte each needed a dedicated team for. **Gated on the span rewrite** — today `@code` is copied almost verbatim with no offsets to translate. *De-risk:* land the span rewrite first behind the golden tests; reuse the WASM Motoko LSP rather than building an analyzer; ship file-level diagnostics (works today) before line-level.
2. **Motoko LSP maturity ceiling.** Even with reuse, completion/refactor quality is capped by the Motoko toolchain (a smaller ecosystem than SourceKit/K2). *De-risk:* lean on `vscode-motoko` for the Motoko layer and invest MotoView-specific value in the *template / directive / backend-binding* layer, where you are not competing with Swift tooling.
3. **Canister debugging is genuinely impossible in the classic sense.** Replicated, deterministic, message-at-a-time execution means there is no live process to attach LLDB to, and Motoko has no step-debugger. *De-risk:* **don't fake it** — offer deterministic record/replay + before/after-state inspection (covers most real needs) and never claim "pause on line 42."
4. **Browser-hosted IDE + build-speed limits.** MotoView's own steps are fast (build ~0.02s, check ~0.34s measured); the wall is `dfx deploy` (seconds warm, minutes cold), and anything touching real identity / vetKeys / inter-canister calls cannot run without a replica. *De-risk:* split the UI honestly into **"design-time preview"** (no replica, instant, IR-rendered) vs **"run against real canister"** (deploy required) — and never let the studio lie about which one the developer is looking at.
5. **The off-canister LLM dependency.** There is no on-canister model; generation is an external HTTP call with a key the deployer holds — a real dent in the "pure ICP" narrative, and model competence on a niche DSL (`.mview` + Motoko, both sparse in training data) will be markedly worse than for React/Swift. *De-risk:* rest the *safety* guarantee entirely on the unbypassable gate (the generator need not be trusted); invest in few-shot exemplars from `templates/` and possibly grammar-constrained decoding to lift convergence inside the repair budget. State honestly: the gate keeps output **safe**, but it does not make a weak model **productive**.

---

## 6. The single highest-leverage first build

**Build the `.mview` editor + language server with live inline diagnostics first — and put the AST/parser span rewrite on its critical path.**

- **It's the #1 blocker.** You cannot type a character into MotokoStudio today. Every other feature — AI refine, fix-its, go-to-def, hover, even a meaningful preview loop — is moot until there is an editable buffer with feedback. An IDE is, first, an editor.
- **It maximally reuses what exists.** File-level inline diagnostics ship in **days** by re-running the existing `check`/`lint` and surfacing the moc→`.mview` mapping — credible value immediately, no compiler changes. The Motoko intelligence inside `@code` is *embedded*, not built, via the WASM `vscode-motoko` LSP in the same browser.
- **The span rewrite is the universal unlock.** Today the AST carries no positions, so diagnostics can only name a *file*. Line/col spans are the shared prerequisite for line-accurate diagnostics, go-to-def, hover, rename, and fix-its — every later editor feature depends on it.

Sequence: editor + `.mview` grammar (S) → file-level diagnostics from `check`/`lint` (S) → `check --json` (S) → **AST span rewrite (L)** → line-accurate diagnostics + `vscode-motoko` delegation (L) → no-deploy IR preview (M) → AI repair loop (M).

---

## Appendix — verified facts this roadmap rests on

Checked against the tree on 2026-06-09:

- **AST has zero spans** — `grep -cE 'span|line|col|offset' compiler/src/ast.rs` → `0`. (The single biggest blocker for line-accurate tooling.)
- **moc→`.mview` mapping already exists** — `// mv:src` markers are emitted in `compiler/src/project.rs` (e.g. `:217`), so file-level diagnostics need no new mapping work.
- **Generated runtime emits zero `Debug.print`** — no observability is instrumented today; the debug suite is greenfield but cheap (the protocol is already structured).
- **No `motoview check --json`** — diagnostics are human-text only; a machine-readable mode is a prerequisite for the AI repair loop and inline editor diagnostics.
- **`EmitMode::Ir` exists but is not wired** into the build/preview pipeline — the no-deploy preview is integration, not invention.
- **Three IR renderers already consume the same `UINode` forest** — `client/src/ir.rs` (web), `clients/ios/.../NativeView.swift` (SwiftUI), `clients/android/.../NativeView.kt` (Compose) — which is what makes the simultaneous 3-up preview a near-term integration rather than a research project.
- **The gate is real and fast** — `motoview lint` (~0.01s, deny-by-default) + `motoview check` (~0.34s, full moc type-check), chained by `tools/studio/validate.sh`. This is the asset the whole studio is built around.

See also: [`docs/native-vision-and-plan.md`](native-vision-and-plan.md) (the native client + security build-out) and [`RELEASE.md`](../RELEASE.md) (the store pipeline).
