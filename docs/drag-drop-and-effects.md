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
- `animate("#panel", "pulse")` — add an animation class (`pulse`, `fadein`) briefly.
- `focusOn("#email")` — move focus to a selector.
- `scrollTo("#errors")` — smooth-scroll an element into view.

Effects ride back in the render batch's `effects` array, so they fire exactly once per event — never on a poll.

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
# open http://<canister-id>.localhost:4943/  and drag a card between columns
```
