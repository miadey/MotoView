---
title: Drag & Drop, Effects & Services
section: Interactivity
slug: drag-drop-and-effects
---

# Drag & Drop, Effects & Services

The [CRM example](https://github.com/miadey/MotoView/tree/main/examples/crm) — a drag-and-drop sales-pipeline Kanban — exercises three capabilities that make MotoView apps feel alive without a line of frontend JavaScript: **drag-and-drop**, **handler-triggered effects**, and **services**.

## Drag & drop

Drag-and-drop is server-driven. Mark a draggable item with `data-mv-drag` (its value is the argument passed to your handler) and a drop zone with `data-mv-drop` (the handler name) plus `data-mv-dropval` (a second argument). When the user drops, the WASM client dispatches `handler(dragValue, dropValue)` as a normal event.

```razor
<section class="kanban-col" id="col-@stage" data-mv-drop="dropDeal" data-mv-dropval="@stage">
    @for deal in dealsIn(stage) {
        <article class="deal-card" draggable="true" data-mv-drag="@(deal.id)">
            <span class="deal-title">@deal.title</span>
        </article>
    }
</section>
```

```motoko
@code {
    func dropDeal(id : Nat, stage : Text) : async () {
        deals := Crm.move(deals, id, stage);
        toast("Moved to " # stage);
        animate("#col-" # stage, "pulse");
    };
}
```

The glue handles `dragstart`/`dragover`/`drop`, highlights the active drop zone (`.mv-drop-over`), and tilts the dragged card (`.mv-dragging`). You only write the handler.

## Effects from handlers

Inside any `@code` function you can queue declarative effects that run on the client after the batch is applied:

- `toast("Deal added")` — a transient notification.
- `animate("#panel", "shake")` — play one of the built-in animations on a selector (see below).
- `focusOn("#email")` — move focus to a selector.
- `scrollTo("#errors")` — smooth-scroll an element into view.

Effects ride back in the render batch's `effects` array, so they fire exactly once per event — never on a poll.

## Animations

MotoView ships a CSS animation library (transform/opacity only, GPU-friendly,
with a `prefers-reduced-motion` guard). Play any of them from a handler with
`animate("#sel", "name")`, or declaratively on **keyed list items** as they're
inserted and removed:

```mview
<ul>
@for item in items {
    <li key="@item.id" enter="fade-up" exit="slide-out-right">@item.title</li>
}
</ul>
```

When the [keyed diff](directives-reference.md) inserts an item it plays its
`enter` animation; when it removes one it plays `exit` and removes the node only
after the animation ends. All of this is the WASM client toggling CSS classes —
no application JavaScript.

The library:

- **Entrances** — `fade-in`, `fade-up`, `fade-down`, `fade-left`, `fade-right`, `scale-in`, `zoom-in`, `pop`, `slide-in-left`, `slide-in-right`.
- **Exits** — `fade-out`, `fade-up-out`, `fade-down-out`, `scale-out`, `zoom-out`, `slide-out-left`, `slide-out-right`, `collapse`.
- **Attention** — `pulse`, `shake`, `bounce`, `wobble`, `flash`, `tada`, `spin` (loops), `pop`.

## Services & models

A `.mview` file is markup plus Motoko, but heavier logic belongs in plain Motoko modules. Drop them in `src/Services/*.mo` or `src/Models/*.mo` and the compiler imports them into the generated actor automatically — page `@code` can then call them.

```motoko
// src/Services/Crm.mo  — a stateless service (a Motoko module is a library)
module {
    public type Deal = { id : Nat; title : Text; company : Text; value : Nat; stage : Text };
    public func byStage(deals : [Deal], s : Text) : [Deal] {
        Array.filter<Deal>(deals, func(d) { d.stage == s });
    };
    public func move(deals : [Deal], id : Nat, stage : Text) : [Deal] { /* ... */ };
}
```

```razor
@code {
    var deals : [Crm.Deal] = Crm.seed();           // state lives in the page
    func dealsIn(s : Text) : [Crm.Deal] { Crm.byStage(deals, s) };
}
```

> A Motoko `module` is stateless, so keep mutable state (the `var deals`) in the page `@code` and let the service provide pure transforms. See [State](state.md).

## Run the CRM

```bash
cd examples/crm
dfx start --background && dfx deploy
# open http://<canister-id>.localhost:4955/  and drag a card between columns
```
