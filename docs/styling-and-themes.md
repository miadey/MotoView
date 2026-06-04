---
title: Styling & Themes
section: Styling
slug: styling-and-themes
---

# Styling & Themes

MotoView gives you three levels of styling, and the trick to a clean codebase is knowing which one to reach for. Start with semantic components for everyday UI, drop into `@style` when a page needs something specific, and use `@theme` tokens to set the colors, spacing, and fonts that everything else inherits.

The goal is simple: write expressive markup, not utility-class soup.

## Level 1: Semantic components

Most of your UI is buttons, cards, alerts, inputs, and tables. MotoView ships these as built-in components so you describe *what* a thing is, not *how* to draw it.

```razor
@page "/dashboard"

<PageHeader>Dashboard</PageHeader>

<Card title="Account">
  <Badge type="success">Active</Badge>
  <p>You have @invoices.size() open invoices.</p>
  <Button kind="primary" size="lg" @click="renew">Renew plan</Button>
</Card>

<Alert type="warning">Your trial ends in 3 days.</Alert>
```

Compare that to a wall of class names. The component carries the styling, and the styling stays consistent everywhere it's used. Built-in components include `Button` (`kind`, `size`), `Card` (`title`), `Alert` and `Badge` (`type`), the inputs `InputText` / `InputEmail` / `InputNumber` / `TextArea`, plus `ValidationSummary`, `Table`, `PageHeader`, and `Grid` (`columns`).

When you need your own reusable piece, build a component in `src/Components/` — see [Components](components.md).

## Level 2: `@style` local CSS

When a page needs styling that doesn't belong in a shared component, use a `@style` block. The CSS is scoped to that page, so you can write plain selectors without worrying about collisions.

```razor
@page "/pricing"

<div class="plans">
  @for plan in plans {
    <Card title="@plan.name">
      <span class="price">@plan.price</span>
    </Card>
  }
</div>

@style {
  .plans {
    display: grid;
    grid-template-columns: repeat(3, 1fr);
    gap: 1.5rem;
  }
  .price {
    font-size: 2rem;
    font-weight: 700;
    color: var(--mv-color-primary);
  }
}
```

Notice the `var(--mv-color-primary)` reference — local CSS should still read from theme tokens so a single theme change updates the whole app.

## Level 3: `@theme` tokens

`@theme` defines the CSS variables that power semantic components and your `@style` blocks. Set these once — usually in a [layout](layouts.md) — and everything inherits.

```razor
@theme {
  --mv-color-primary: #2563eb;
  --mv-color-success: #16a34a;
  --mv-color-warning: #d97706;
  --mv-color-bg:      #ffffff;
  --mv-color-text:    #1f2937;
  --mv-radius:        0.5rem;
  --mv-font:          system-ui, sans-serif;
}
```

Because tokens are ordinary CSS custom properties, changing a brand color is a one-line edit, not a search-and-replace through your markup.

## The default theme and dark mode

A new project from `motoview new` comes with a default theme already wired up, so the counter and other starter examples render cleanly before you change a thing. The compiled assets — including the framework stylesheet — are served by the canister at `/motoview.css` alongside `/motoview.wasm` and `/motoview.js`.

Dark mode is just a second set of token values under a media query in your `@theme` block:

```razor
@theme {
  --mv-color-bg:   #ffffff;
  --mv-color-text: #1f2937;

  @media (prefers-color-scheme: dark) {
    --mv-color-bg:   #0f172a;
    --mv-color-text: #e2e8f0;
  }
}
```

Semantic components and any `@style` that reads tokens flip automatically — no JavaScript, no extra wiring.

## Avoiding utility-class soup

The pattern across all three levels is the same: push styling decisions *down* into tokens and components, and keep your markup describing intent.

```razor
@* Reach for this *@
<Button kind="primary" size="lg">Save</Button>

@* Not this *@
<button class="inline-flex items-center px-4 py-2 rounded-md
  bg-blue-600 text-white font-medium hover:bg-blue-700">Save</button>
```

A rule of thumb:

- **Semantic component** if it's a common UI element — let the component own the look.
- **`@style`** if it's genuinely page-specific layout or one-off styling.
- **`@theme`** for anything visual you'd want to change in one place: colors, spacing scale, radius, fonts.

Build and deploy as usual:

```bash
motoview build
motoview dev
```

When you're ready to make pages interactive, head to [Events](events.md) and [Forms & Validation](forms-and-validation.md).
