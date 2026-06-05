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

## Level 3: `@theme` — design tokens & theme packages

`@theme` sets the CSS variables (design tokens) that power semantic components,
the base stylesheet, and your own `@style` blocks. Put it in a [layout](layouts.md)
once and every page using that layout inherits it. It compiles to a single
`<style>:root{ … }</style>` injected after the base stylesheet, so your values
win.

### Brand your whole app from one color

MotoView's design foundation is the authentic **Microsoft Fluent 2** design-token
system (the real `--colorBrandBackground`, `--colorNeutralForeground1`, type ramp,
spacing, radii, shadows, motion). To rebrand the entire app, give `@theme` a single
brand color and the compiler generates the full Fluent brand ramp (16 shades) and
wires every brand alias token — for **light and dark**:

```mview
@theme brand="#d13438"
```

That one line themes every component (buttons, links, accents, focus rings) to your
brand, in both light and dark mode, the same way Fluent's `createLightTheme` /
`createDarkTheme` do. Dark mode is automatic via `prefers-color-scheme` (or force it
with `<html data-theme="dark">` / `data-theme="light">`).

### Apply a built-in theme package

The framework ships five ready-made, accessibility-checked themes (WCAG-AA
contrast). Apply one by name:

```razor
@theme "midnight"
```

| Name | Look |
|---|---|
| `midnight` | dark — deep indigo-black, violet primary |
| `ocean` | light — cool blue/teal |
| `forest` | light — natural green, warm neutrals |
| `sunset` | light — warm coral on cream |
| `slate` | light — minimal near-grayscale, slate primary |

### Override individual tokens

Set any tokens directly, with or without a preset. Overrides win over the preset:

```razor
@theme "ocean" {
  --mv-primary: #0d9488;
  --mv-radius: 4px;
}
```

### The tokens

| Token | Purpose |
|---|---|
| `--mv-primary`, `--mv-primary-600`, `--mv-primary-fg` | brand color, its hover shade, and text on a primary button |
| `--mv-bg`, `--mv-surface`, `--mv-muted` | page / card / subtle-fill backgrounds |
| `--mv-border` | hairline borders |
| `--mv-text`, `--mv-text-soft` | primary and secondary text |
| `--mv-success`, `--mv-danger`, `--mv-warning` | status colors |
| `--mv-radius`, `--mv-radius-sm` | corner radii |
| `--mv-font` | base font stack |
| `--mv-shadow` | elevation shadow |

Because tokens are ordinary CSS custom properties, your own `@style` and
components read them with `var(--mv-primary)` — so changing a brand color is a
one-line edit, not a search-and-replace through your markup.

### Dark mode

Dark mode is just the `midnight` package (or your own dark token values):

```razor
@theme "midnight"
```

Every semantic component and any `@style` that reads tokens flips automatically —
no JavaScript, no extra wiring. The compiled base stylesheet is served by the
canister at `/motoview.css` alongside `/motoview.wasm` and `/motoview.js`.

### Let the user choose: `<ThemeToggle />`

Light and dark are both baked into the theme, and you can let the user pick. Drop
the built-in toggle anywhere (an app bar, a nav, a settings page):

```razor
<ThemeToggle />
```

It's a framework primitive, so there's still **no application JavaScript**. Clicking
it flips `<html data-theme="light|dark">` instantly and saves the choice in an
`mv_theme` cookie; the runtime injects a tiny inline script that re-applies the
saved theme **before first paint on every load** (so there's no flash, and it
works on certified cacheable pages too, whose certificate can't vary by cookie).
With no saved choice it follows `prefers-color-scheme`. The sun/moon icon is pure
CSS driven by `[data-theme]`.

For the full Fluent theme set, drop in `<ThemePicker />` — a dropdown of the
official themes (**Web Light**, **Web Dark**, **Teams Light**, **Teams Dark**,
**High Contrast**), exactly like the Fluent UI docs. It uses the same `mv_theme`
cookie + no-flash mechanism; the glue marks the active option and updates the
label on load. The named themes are layered on the light/dark neutral bases:
`teams-*` swap the brand to Teams purple, `hc` is a black/white/yellow
high-contrast token set (a user's picked theme overrides the app's `@theme brand`).

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

When you're ready to make pages interactive, head to [Events](events.md) and [Forms & Validation](forms.md).
