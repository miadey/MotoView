# Debug & observability (R7)

MotoView is server-driven: rendering is a query, every interaction is an update
that returns a versioned `Batch`. R7 makes that loop **observable** without
changing the default build at all.

There are four things you can see:

1. a **log stream** of every dispatched event,
2. an **event / batch timeline**,
3. a **view-hierarchy inspector** (the IR forest as a collapsible tree), and
4. a **per-event instruction cost**.

> **Out of scope (honest):** source-step debugging / attaching a debugger.
> The Internet Computer executes your canister on a *replicated* set of nodes —
> there is no single deterministic host to break into and step. That is a hard
> property of the platform, not a missing feature. Faithful **record/replay** is
> the R10 roadmap item; it is not what this slice does.

## Opt-in instrumentation

The default build emits **zero** `Debug.print` — the generated `main.mo` is
byte-identical whether or not you ask for observability. Instrumentation is opt-in:

```sh
motoview build <dir> --instrument     # generate the instrumented actor
motoview check <dir> --instrument     # type-check the instrumented actor
```

When `--instrument` is on, the generated event dispatch (`mvDispatch`) wraps each
handler with:

* an instruction-cost delta around the dispatch, via
  `ExperimentalInternetComputer.performanceCounter(0)`, and
* one structured `Debug.print` line per event.

`Debug` and `ExperimentalInternetComputer` are imported **only** in the
instrumented build, so the default header is unchanged.

## The log line format

Each dispatched event prints exactly one stable, pipe-delimited line:

```
MV|dispatch|page=<Page>|handler=<h>|event=<h>|caller=<principal>|lastBatch=<batchId>|costInstr=<n>
```

| field       | meaning                                                          |
|-------------|------------------------------------------------------------------|
| `MV`        | tag namespace (literal)                                          |
| `dispatch`  | record kind (literal)                                            |
| `page`      | the page object that owns the handler                           |
| `handler`   | the dispatched handler id (e.g. `increment`)                    |
| `event`     | mirrors `handler` — on the server the dispatched event *is* the handler |
| `caller`    | `Principal.toText(ctx.caller)` (anonymous renders show `2vxsx-fae`) |
| `lastBatch` | `ctx.lastBatchId` — the batch the client currently displays      |
| `costInstr` | instruction-count delta across the handler body                  |

The new batch id is computed *after* the handler runs (it hashes the rendered
output), so the line carries the **last** batch id the client had — the timeline
chains events by it.

Read the stream off a deployed canister with:

```sh
dfx canister logs <canister-id>
```

`dfx` prefixes each line with a sequence/timestamp; the parser scans for the
`MV|` marker anywhere in the line, so those prefixes are tolerated.

## The studio tooling

`tools/studio/observability.js` is a dependency-free, **headless-tested** module:

* `parseLogStream(text)` → structured events
  `{ tag, page, handler, event, caller, lastBatch, costInstr, prefix, fields, raw }`.
* `summarizeEvents(events)` → per-handler `{ count, totalCost, avgCost, maxCost }`.
* `buildViewTree(forest)` → the collapsible view hierarchy from a
  `motoview preview` IR forest (`{t:"el"|"text"|"raw",…}`; whitespace-only `raw`
  nodes dropped), with each element's `events` lifted to a `handlers` list.
* `nodesForHandler(roots, handler)` → the source node(s) a logged handler fires,
  for **click-to-source**.
* `renderTimelineHtml` / `renderTreeHtml` → headless HTML the panel embeds.

Tests (run by `tools/studio/run-tests.sh`):

* `tools/studio/log-parser-test.js` — a sample structured stream → the right
  events (handler, caller, batchId, numeric cost, null cost when absent).
* `tools/studio/view-tree-test.js` — the **real** forest from
  `motoview preview examples/counter` → the expected tree (a `<section>`, the
  live count `text` node, four `<button>`s each carrying a `@click`).

`tools/studio/observability-panel.html` is the live studio panel: paste the log
stream on the left (timeline, hot handlers in red), paste the preview forest on
the right (collapsible tree); clicking a timeline row highlights the firing
node(s). The panel **UI** (collapse interactions, click-to-source highlighting)
is browser-pending — the parser and tree-builder it embeds are the same
headless-tested core.
