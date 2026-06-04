---
title: The Protocol
section: Architecture
slug: protocol
---

# The Protocol

MotoView speaks a small, deliberate wire protocol called `motoview/1`. It rests on one idea that keeps the whole framework simple:

> Rendering is a query, events are updates, and the browser synchronizes through versioned UI batches.

Once you internalize that sentence, the rest of the protocol falls out naturally. Let's walk through it.

## Render-as-query

The first time a visitor hits a route, the canister renders the page server-side and returns plain HTML. The interactive content lives inside a single mount point:

```html
<div id="mv-root">
  <!-- server-rendered page content -->
</div>
```

That's the SEO-friendly, no-JavaScript-required baseline. A crawler, a `curl`, or a browser with scripting disabled all see real HTML.

After the page loads, the WASM client keeps the view fresh by *asking* for it. Rendering a page is a read, so synchronization is modeled as a query:

```bash
GET /_motoview/render?path=/products/42&lastBatchId=ab12cd34
```

The server re-renders the route for the current caller and compares the result to the `lastBatchId` the client already has. Nothing about the UI changes unless the underlying state changed.

## Event-as-update

When the user does something — `@click="save"`, `@submit="send"`, `@input="search"` — the client posts the handler identity plus its baked-in arguments:

```bash
POST /_motoview/event
Content-Type: application/x-www-form-urlencoded
```

Events are state transitions, so they are served by `http_request_update`. The handler runs, mutates state, and the server immediately renders and returns the **new batch** in the same response. There is no waiting for the next poll tick after an interaction — the round trip carries the fresh UI back with it. See [Events](events.md) for how handler arguments are evaluated server-side and dispatched to typed Motoko functions.

## The batch JSON

Every sync and every event resolves to a *batch*: a versioned snapshot of the UI with a status that tells the client what to do.

Every batch carries a `"protocol": "motoview/1"` field. The fields below are exactly what the runtime emits (`runtime/src/Json.mo`).

**`changed`** — state moved; here is the new markup to swap into `target` (always `"mv-root"` in the MVP full-container-replace mode). Includes the document `head` and any client `effects`.

```json
{
  "protocol": "motoview/1",
  "status": "changed",
  "batchId": "9f3c1a07",
  "mode": "replace",
  "target": "mv-root",
  "html": "<section>…</section>",
  "head": { "title": "Dashboard", "description": "", "canonical": "" },
  "effects": []
}
```

**`unchanged`** — the rendered state hashes to the `lastBatchId` the client sent, so `html` is omitted entirely. This is the common case while polling and it keeps the wire quiet.

```json
{ "protocol": "motoview/1", "status": "unchanged", "batchId": "9f3c1a07" }
```

**`redirect`** — the handler or guard wants the client somewhere else.

```json
{ "protocol": "motoview/1", "status": "redirect", "location": "/login" }
```

**`validation-error`** — a `validate model { … }` block failed; the batch carries the re-rendered form (with `<ValidationSummary />` and per-field errors populated) plus the `errors` map, so the user sees exactly what to fix.

```json
{
  "protocol": "motoview/1",
  "status": "validation-error",
  "batchId": "44ed90b2",
  "target": "mv-root",
  "html": "<form>…</form>",
  "errors": { "email": "That email address looks invalid." },
  "effects": []
}
```

> The `mode` field on a `changed` batch is `"replace"` in the MVP (full-container replace). Keyed-region and patch modes are on the [roadmap](roadmap.md).

## batchId

The `batchId` is a hash of the rendered state. It is the version stamp that makes the protocol idempotent and cheap:

- The client sends its current `batchId` as `lastBatchId` on every render query.
- If the freshly rendered output hashes to the same value, the server answers `unchanged` and skips sending markup.
- Only a genuine state change produces a new `batchId`, and only then does the DOM get swapped.

This is what lets MotoView poll frequently without thrashing the page.

## Adaptive polling cadence

The WASM client runs a small state machine that dials its sync frequency up and down based on what the user is doing:

| State   | Cadence  | When                                   |
| ------- | -------- | -------------------------------------- |
| Hot     | ~350ms   | for ~3s right after an interaction     |
| Warm    | ~2.5s    | tab visible, no recent interaction     |
| Cold    | ~15s     | visible but idle                       |
| Hidden  | ~45s     | tab backgrounded                       |
| Offline | backoff  | exponential backoff while disconnected |

Because the event response already returns the new batch, "hot" mode is mostly about catching *other* callers' changes immediately after you act, then relaxing as activity dies down.

## Why polling on ICP

ICP canisters cannot hold open server-push connections, so MotoView synchronizes by polling — but cheaply, thanks to `batchId` and adaptive cadence.

There is one transport detail worth knowing. In the MVP, `http_request` returns `upgrade = true`, so every request — including renders — is promoted to `http_request_update`. This sidesteps query response-certification while the protocol stabilizes.

> **Roadmap:** Certified query rendering for cacheable public pages, so reads can be served as true (certified) queries without the upgrade.
