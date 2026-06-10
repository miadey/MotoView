# Tier-3 leapfrog features (R10) — implemented

The three differentiators from the roadmap's "leapfrog thesis" are now real, built,
and tested. This doc records what landed, how to drive it, and what is honestly
visual-flagged vs. fully tested.

## 1. Backend-bound completion (no client/server drift)

The completion palette inside a `@code { … }` block **is the project's real Motoko
service surface**. When you request completion inside `@code`, the language server
(`motoview lsp`) — in addition to the Motoko delegation marker — offers an item for
every `public func` / `public type` / `public let` of every stateful service
(`src/Services/*.mo`, the `public class <Name>()` convention).

- Each item: `label` = the decl name, `detail` = `<Service>.<kw> <signature>`,
  `kind` = Function / Struct / Constant.
- The project root is resolved from the document URI (walks up to `dfx.json` /
  `src/Services`).
- Implementation: `compiler/src/services.rs` (a Rust-side scanner mirroring
  `tools/studio/signatures.js`), wired into `compiler/src/lsp.rs`
  (`completion_at_uri` → `backend_surface`).

Why it matters: a generated/typed `@code` handler can only reference functions and
types that actually exist with the types they have — the "always linked to the
backend" requirement, expressed as the editor palette itself.

**Tested:** `compiler/src/services.rs` unit tests (6) + an LSP completion test
(`lsp::tests::completion_inside_code_is_backend_bound_to_project_services`) that
stages a project with a `Services/Notes.mo`, drives the REAL completion handler, and
asserts `add` (func) + `Note` (type) appear with the right signature/kind and that a
private func never leaks.

## 2. Deterministic record/replay (time-travel)

A recorded **session** is an ordered list of events; replaying it re-runs that exact
sequence through the page's `mvDispatch` (the same dispatch the live actor uses),
then renders. Because the IC's dispatch+render is deterministic, replaying the same
session yields a **byte-identical** forest — time-travel for near-free.

```
motoview preview <dir> --replay <session.json>
```

Session format (object with an `events` array, or a bare array):

```json
{ "events": [
  { "handler": "increment", "args": ["1"] },
  { "handler": "increment", "args": ["1"], "caller": "2vxsx-fae" },
  { "handler": "reset" }
] }
```

`args` and `caller` are optional (default: no args / anonymous principal). Each
event maps directly to `mvDispatch(ctx, handler, args)`.

Implementation: `project::build_preview_with_events` + `assemble_preview` emit one
`mvDispatch(…)` per event before the final `mvRender`; `main.rs`
`cmd_preview_replay` + `parse_session` load and drive it.

**Tested:** `tools/studio/replay-test.js` drives the real compiler against
`examples/counter` and asserts: same session twice → byte-identical forest; a
2-event `increment(1)` session → the counter reads `2`; the replayed forest differs
from the initial render; a mixed `+5, +1, reset` session → `0`; `+5, -1` → `4`.
Plus `parse_session` is unit-tested in `compiler/src/main.rs`.

## 3. The 3-up canvas (one edit → web + iOS + Android)

`motoview preview <dir> --serve` now shows a **three-column canvas**: web (live DOM),
iOS (SwiftUI `NativeView`), Android (Compose `NativeView`) — all driven by the ONE IR
forest, live over SSE on every `.mview` change.

- The fan-out (`tools/studio/fanout.js`) is a pure function of the forest mapping it
  to each pane's data, faithful to the REAL native renderers' tag classification
  (block→container, text→Text, button→Button, raw→WebView-fallback leaf). iOS and
  Android share the identical native descriptor (their renderers are 1:1).
- The studio `--serve` panel inlines a byte-faithful copy of that core (between the
  `MV-FANOUT-BEGIN` / `MV-FANOUT-END` markers in `compiler/src/main.rs`).

**Honest scope:** the web column renders live. The iOS/Android columns are a
**forest-faithful preview** (the same native element/text/button classification,
styled per platform), NOT a running simulator — true native panes need
Xcode/Android Studio simulator processes. The VISUAL three columns are
browser-flagged; the DATA each column consumes is headless-tested.

**Tested:** `tools/studio/fanout-test.js` drives the real counter forest and asserts
the fan-out node-for-node (web 4 buttons; native 4 Buttons carrying the real
`@click` handlers + labels; counter value reaches a native Text; native panes drop
whitespace glue) AND panel-parity (the panel's inlined fan-out matches the tested
module exactly, so the browser-flagged visual can't silently drift).

## Verify

```
cargo build --release --manifest-path compiler/Cargo.toml
cargo test --manifest-path compiler/Cargo.toml          # 130 green (was 122)
node tools/studio/replay-test.js                        # determinism + counter -> 2
node tools/studio/fanout-test.js                        # one forest -> 3 panes
bash tools/studio/run-tests.sh                          # full studio suite
```
