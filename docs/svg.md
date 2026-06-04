---
title: SVG Interfaces
section: Styling
slug: svg
---

# SVG Interfaces

SVG is just markup, and MotoView treats it that way. Because a `.mview` template is rendered server-side into HTML, you can write `<svg>` directly in your page, loop over Motoko data to draw nodes and edges, and wire `@click` onto individual shapes. The result is a fully server-driven diagram: no charting library, no client JavaScript, and the same event-update-batch cycle that powers the rest of the framework.

This makes MotoView a natural fit for dashboards, flow diagrams, and network maps — anything where the picture *is* the data living in your canister.

## SVG is first-class markup

There is nothing special to enable. Drop an `<svg>` element into a template and use the directives you already know — [`@for`](control-flow.md), `@if`, inline output `@(expr)`, and [event handlers](events.md) — on its children.

```razor
@page "/dashboard"
@title "Live metrics"

@code {
  type Bar = { label : Text; value : Nat };
  var bars : [Bar] = [
    { label = "Reads"; value = 80 },
    { label = "Writes"; value = 45 },
    { label = "Errors"; value = 12 },
  ];
}

<svg viewBox="0 0 320 160" role="img" aria-label="Request volume">
  @for bar in bars {
    <rect x="@(bar.value)" y="10" width="@(bar.value)" height="24" rx="3" />
    <text x="6" y="28">@bar.label — @bar.value</text>
  }
</svg>
```

Each value is evaluated on the server at render time and baked into the attribute. Because rendering is a query, the browser sees finished SVG — which means search engines and screen readers do too. Add `role` and `aria-label` so your diagram stays accessible.

## Looping over nodes and links

A diagram is usually two collections: the things, and the connections between them. Model both in your [`@code`](code-blocks.md) block and draw them with two loops — links first so they sit behind the nodes.

```razor
@code {
  type Node = { id : Text; x : Nat; y : Nat; label : Text };
  type Link = { x1 : Nat; y1 : Nat; x2 : Nat; y2 : Nat };

  var nodes : [Node] = [
    { id = "api"; x = 40;  y = 60; label = "API" },
    { id = "db";  x = 200; y = 60; label = "DB" },
  ];
  var links : [Link] = [
    { x1 = 40; y1 = 60; x2 = 200; y2 = 60 },
  ];

  var selected : ?Text = null;

  public func select(id : Text) : async () {
    selected := ?id;
  };
}

<svg viewBox="0 0 280 120" class="map">
  @for link in links {
    <line x1="@link.x1" y1="@link.y1" x2="@link.x2" y2="@link.y2" class="edge" />
  }

  @for node in nodes {
    <g class="node" @click="select(node.id)">
      <circle cx="@node.x" cy="@node.y" r="18" />
      <text x="@node.x" y="@node.y" text-anchor="middle">@node.label</text>
    </g>
  }
</svg>
```

## Clicking shapes

`@click` works on any SVG element — a `<circle>`, a `<rect>`, or a wrapping `<g>`. Handler arguments are evaluated server-side and baked into `data-mv-arg*` attributes, then forwarded to your typed Motoko function. So `@click="select(node.id)"` dispatches to `select` with the node's id; you never touch the wire format.

Use the resulting server state to drive appearance with `@if` or [`@switch`](control-flow.md):

```razor
@for node in nodes {
  <g @click="select(node.id)"
     class="@(selected == ?node.id ? "node selected" else "node")">
    <circle cx="@node.x" cy="@node.y" r="18" />
  </g>
}
```

After a click the server returns a new batch immediately, the WASM client swaps `#mv-root`, and the selected node redraws. No round-trip you have to manage by hand.

## Styling the diagram

Co-locate styles with the page using a [`@style`](styling.md) block, or pull from your [theme tokens](theming.md):

```razor
@style {
  .map .edge   { stroke: var(--mv-border); stroke-width: 2; }
  .map .node   { cursor: pointer; }
  .map circle  { fill: var(--mv-surface); stroke: var(--mv-primary); }
  .map .selected circle { fill: var(--mv-primary); }
}
```

## Live network maps

Combine SVG with [adaptive polling](protocol.md): when a node's status changes in your canister, the next sync produces a different `batchId`, the client fetches the new batch, and the map re-renders. Recolor by status with `@switch`:

```razor
@for node in nodes {
  @switch node.status {
    case (#healthy) { <circle class="ok"   cx="@node.x" cy="@node.y" r="18" /> }
    case (#degraded){ <circle class="warn" cx="@node.x" cy="@node.y" r="18" /> }
    case (#down)    { <circle class="down" cx="@node.x" cy="@node.y" r="18" /> }
  }
}
```

That is the whole model: state lives in Motoko, the shapes are a projection of that state, and clicks are [updates](events.md) that change it.

## See also

- [Control Flow](control-flow.md) — `@for`, `@if`, and `@switch`
- [Events](events.md) — how `@click` arguments reach your handlers
- [Styling](styling.md) and [Theming](theming.md) — `@style` blocks and tokens
- [Protocol](protocol.md) — adaptive polling that keeps live maps current
