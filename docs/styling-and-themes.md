---
title: Styling & Themes
section: Styling
slug: styling-and-themes
---

# Styling & Themes

MotoView gives you three levels of styling, and the trick to a clean codebase is
knowing which one to reach for. Start with **semantic components** for everyday UI,
drop into **`@style`** when a page needs something specific, and use **`@theme`
tokens** to set the colors, spacing, and fonts that everything else inherits.

The goal is simple: write expressive markup, not utility-class soup.

Under the hood, the design foundation is the **authentic Microsoft Fluent UI 2**
design-token system — the real `--colorNeutral*` / `--colorBrand*` ramps, type
ramp, spacing scale, radii, shadows, and motion curves. Components read those
tokens, so changing one token (or one brand color) re-themes the whole app, in
light and dark, with no JavaScript.

## Level 1: Semantic components

Most of your UI is buttons, cards, alerts, inputs, and tables. MotoView ships
these as built-in components so you describe *what* a thing is, not *how* to draw it.

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

Compare that to a wall of class names. The component carries the styling, and the
styling stays consistent everywhere it's used. Built-in components include `Button`
(`kind`, `size`), `Card` (`title`), `Alert` and `Badge` (`type`), the inputs
`InputText` / `InputEmail` / `InputNumber` / `TextArea`, plus `ValidationSummary`,
`Table`, `PageHeader`, and `Grid` (`columns`).

When you need your own reusable piece, build a component in `src/Components/` — see
[Components](components.md).

## Level 2: `@style` local CSS

When a page needs styling that doesn't belong in a shared component, use a `@style`
block. The CSS is scoped to that page, so you can write plain selectors without
worrying about collisions.

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
    gap: var(--spacingHorizontalL);
  }
  .price {
    font-size: var(--fontSizeBase600);
    font-weight: var(--fontWeightBold);
    color: var(--colorBrandForeground1);
  }
}
```

Notice the `var(--colorBrandForeground1)` and `var(--spacingHorizontalL)`
references — local CSS should still read from **theme tokens** so a single theme
change updates the whole app, and so dark mode and the theme picker keep working
automatically.

## The Fluent 2 token foundation

The base stylesheet (`/motoview.css`, served by the canister) opens with the full
Fluent 2 token set, resolved to literal values. **Light** is
`createLightTheme(brandWeb)`; **dark** is `createDarkTheme(brandWeb)`; the brand is
Fluent's *Communication blue* (`brandWeb`, ramp anchor `#0f6cbd` at shade 80).

These are real CSS custom properties — read them anywhere with `var(--token)`.

### Color tokens

The brand ramp is sixteen shades, `--colorBrandBackground10` … `160`:

```css
--colorBrandBackground10:  #061724;   /* darkest  */
--colorBrandBackground80:  #0f6cbd;   /* the anchor / primary brand */
--colorBrandBackground160: #ebf3fc;   /* lightest */
```

On top of the ramp sit the **semantic color aliases**. A representative slice:

| Group | Example tokens |
|---|---|
| Neutral foreground | `--colorNeutralForeground1` … `4`, `…OnBrand`, `…Disabled`, `…Inverted` |
| Neutral background | `--colorNeutralBackground1` … `6`, `…1Hover/Pressed/Selected`, `…Disabled` |
| Subtle background | `--colorSubtleBackground`, `…Hover`, `…Pressed`, `…Selected` |
| Neutral stroke | `--colorNeutralStroke1` … `3`, `…Accessible`, `…StrokeSubtle`, `…Disabled` |
| Brand foreground | `--colorBrandForeground1` / `2`, `--colorBrandForegroundLink`, `…Hover/Pressed` |
| Brand background | `--colorBrandBackground`, `…Hover`, `…Pressed`, `…Selected`, `--colorBrandBackground2` |
| Compound brand | `--colorCompoundBrandForeground1`, `…Background`, `…Stroke` (+ hover/pressed) |
| Brand stroke | `--colorBrandStroke1` / `2`, `…2Hover`, `…2Pressed` |
| Focus | `--colorStrokeFocus1` (inner), `--colorStrokeFocus2` (outer) |
| Status: danger | `--colorStatusDangerForeground1..3`, `…Background1..3`, `…Border1..2` |
| Status: success | `--colorStatusSuccessForeground1..3`, `…Background1..3`, `…Border1..2` |
| Status: warning | `--colorStatusWarningForeground1..3`, `…Background1..3`, `…Border1..2` |
| Status: info | `--colorStatusInfoForeground1..2`, `…Background1..2`, `…Border1` |
| Palettes | `--colorPaletteRed*`, `…Green*`, `…DarkOrange*`, `…Yellow*` |

### Type ramp

```css
--fontFamilyBase:      'Segoe UI', -apple-system, BlinkMacSystemFont, Roboto, sans-serif;
--fontFamilyMonospace: Consolas, 'Courier New', Courier, monospace;
--fontFamilyNumeric:   Bahnschrift, 'Segoe UI', …;

--fontSizeBase100..600:  10/12/14/16/20/24px
--fontSizeHero700..1000: 28/32/40/68px
--lineHeightBase100..600 + --lineHeightHero700..1000   /* matched line heights */

--fontWeightRegular: 400;  --fontWeightMedium: 500;
--fontWeightSemibold: 600; --fontWeightBold: 700;
```

Ready-made type styles are exposed as classes: `.fui-text-caption1`,
`.fui-text-body1`, `.fui-text-body1-strong`, `.fui-text-subtitle2/1`,
`.fui-text-title3/2/1`, `.fui-text-largetitle`.

### Spacing, radii, strokes

```css
--spacingHorizontalNone/XXS/XS/SNudge/S/MNudge/M/L/XL/XXL/XXXL   /* 0,2,4,6,8,10,12,16,20,24,32px */
--spacingVerticalNone/XXS/…/XXXL                                  /* same ladder, vertical */
--borderRadiusNone/Small/Medium/Large/XLarge/Circular            /* 0,2,4,6,8,10000px */
--strokeWidthThin/Thick/Thicker/Thickest                         /* 1,2,3,4px */
```

### Shadows (elevation)

```css
--shadow2 / 4 / 8 / 16 / 28 / 64           /* ambient + key, depth ladder */
--shadow2Brand / …64Brand                  /* brand-tinted variants */
```

Shadows are the one set of *colored* tokens that differs by theme — they deepen in
dark mode for the same perceived elevation.

### Motion

```css
--durationUltraFast/Faster/Fast/Normal/Gentle/Slow/Slower/UltraSlow  /* 50…500ms */
--curveAccelerate(Max/Mid/Min)  --curveDecelerate(Max/Mid/Min)
--curveEasyEase  --curveEasyEaseMax  --curveLinear
```

The type ramp, spacing, radii, stroke widths, durations and curves are
theme-independent — only the *color* tokens (and shadows) change between light and
dark.

### Legacy `--mv-*` aliases

Earlier MotoView used a small `--mv-*` palette. Those tokens still exist as **thin
aliases onto Fluent**, so older code and `@theme { … }` overrides keep working:

```css
--mv-primary:    var(--colorBrandBackground);
--mv-primary-600:var(--colorBrandBackgroundHover);
--mv-primary-fg: var(--colorNeutralForegroundOnBrand);
--mv-bg:         var(--colorNeutralBackground2);
--mv-surface:    var(--colorNeutralBackground1);
--mv-muted:      var(--colorNeutralBackground3);
--mv-border:     var(--colorNeutralStroke2);
--mv-text:       var(--colorNeutralForeground1);
--mv-text-soft:  var(--colorNeutralForeground3);
--mv-success:    var(--colorStatusSuccessForeground3);
--mv-danger:     var(--colorStatusDangerForeground3);
--mv-warning:    var(--colorStatusWarningForeground3);
--mv-radius:     var(--borderRadiusXLarge);
--mv-radius-sm:  var(--borderRadiusMedium);
--mv-font:       var(--fontFamilyBase);
--mv-shadow:     var(--shadow4);
```

Prefer the Fluent tokens in new code; reach for `--mv-*` only when you're matching
existing markup. Both flip correctly in dark mode and under the theme picker.

## Level 3: `@theme` — design tokens & theme packages

`@theme` sets the CSS variables (design tokens) that power semantic components, the
base stylesheet, and your own `@style` blocks. Put it in a [layout](layouts.md)
once and every page using that layout inherits it. It compiles to a
`<style>:root{ … }</style>` injected into `<head>` *after* the base stylesheet, so
your values win.

The three forms can be combined on one directive, and they apply in this order
(later wins): **brand ramp → preset → token overrides**.

```razor
@theme brand="#d13438" "ocean" { --borderRadiusMedium: 8px; }
```

### Brand your whole app from one color

Give `@theme` a single brand color and the compiler generates the full Fluent brand
ramp (16 shades) and wires every brand alias token — for **light AND dark** — the
same way Fluent's `createLightTheme` / `createDarkTheme(brandRamp)` do:

```razor
@theme brand="#d13438"
```

That one line themes every component (buttons, links, accents, focus rings) to your
brand, in both light and dark mode.

**How it works** (`compiler/src/color.rs`): the input color is converted to HSL and
anchored at **shade 80**; the other 15 shades follow the reference ramp's lightness
ladder with brand-matched saturation, so any color yields a cohesive, Fluent-shaped
ramp. The compiler then emits:

- the 16 `--colorBrandBackground10..160` variables, plus
- every brand **alias** (`--colorBrandBackground`, `…Hover`, `--colorBrandForeground1`,
  `--colorBrandForegroundLink`, `--colorBrandStroke1`, the compound-brand tokens, …)
  picked from the ramp at the *light* shade in `:root` and the *dark* shade under
  `[data-theme="dark"]` and `@media (prefers-color-scheme: dark)`.

The light/dark shade mapping for each alias lives in
`compiler/src/brand_aliases.rs` (derived from Fluent's `createLightTheme` /
`createDarkTheme(brandWeb)`). For example `--colorBrandBackground` uses ramp shade
**80** in light and **70** in dark; `--colorBrandForeground1` uses **80** in light
and **100** in dark.

```css
/* what @theme brand="#d13438" compiles to (abridged) */
:root{
  --colorBrandBackground10:#…; … --colorBrandBackground160:#…;
  --colorBrandBackground:#d13438; --colorBrandForeground1:#d13438; …
}
[data-theme="dark"]{ --colorBrandBackground:#…; --colorBrandForeground1:#…; … }
@media (prefers-color-scheme: dark){
  :root:not([data-theme="light"]){ /* same dark brand aliases */ }
}
```

### Apply a built-in theme package

The framework ships five ready-made, accessibility-checked presets (WCAG-AA
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

Presets are sets of `--mv-*` overrides (see `theme_preset` in
`compiler/src/codegen.rs`), so they layer cleanly on top of the Fluent base.

### Override individual tokens

Set any tokens directly — Fluent or `--mv-*` — with or without a preset. Overrides
win over the preset:

```razor
@theme "ocean" {
  --colorBrandBackground: #0d9488;
  --borderRadiusMedium: 4px;
}
```

Because tokens are ordinary CSS custom properties, your own `@style` and components
read them with `var(--colorBrandBackground)` — so changing a brand color is a
one-line edit, not a search-and-replace through your markup.

## Light & dark

Dark mode is built in and requires **no JavaScript and no extra wiring**:

- **Automatic** — `@media (prefers-color-scheme: dark)` flips every color token to
  the dark theme, following the user's OS setting.
- **Forced** — set `data-theme` on the root to override the OS:
  `<html data-theme="dark">` or `<html data-theme="light">`. The dark media query is
  scoped `:root:not([data-theme])`, so an explicit choice always wins.

Every semantic component and any `@style` that reads tokens flips automatically. The
base stylesheet is served by the canister at `/motoview.css`, alongside
`/motoview.wasm` and `/motoview.js`.

### Let the user choose: `<ThemeToggle />`

Drop the built-in toggle anywhere (an app bar, a nav, a settings page):

```razor
<ThemeToggle />
```

It's a framework primitive, so there's still **no application JavaScript**. It
compiles to a single button with `data-mv-theme-toggle`. Clicking it:

1. flips `<html data-theme="light|dark">` instantly, and
2. saves the choice in an `mv_theme` cookie (`path=/; max-age=1y; samesite=lax`).

On every subsequent load the runtime injects a tiny **inline head script** that
re-applies the saved theme **before first paint** — so there's no flash of the wrong
theme, and it works on certified, cacheable pages too (whose certificate can't vary
by cookie, so the theme has to be applied client-side from the cookie):

```html
<script>(function(){try{var m=document.cookie.match(
  /(?:^|; )mv_theme=(web-light|web-dark|teams-light|teams-dark|hc|light|dark)/);
  if(m)document.documentElement.setAttribute('data-theme',m[1]);}catch(e){}})();</script>
```

With no saved choice it follows `prefers-color-scheme`. The sun/moon icon is **pure
CSS** driven by `[data-theme]` — it shows the theme you'll switch *to* (moon in
light, sun in dark).

### The full Fluent theme set: `<ThemePicker />`

For the complete official Fluent theme lineup, drop in the picker — a dropdown just
like the Fluent UI docs:

```razor
<ThemePicker />
```

It offers the five official themes:

| Picker option | `data-theme` key | Brand |
|---|---|---|
| **Web Light** | `web-light` | Communication blue (the `:root` base) |
| **Web Dark** | `web-dark` | Communication blue on the dark base |
| **Teams Light** | `teams-light` | Teams purple (`#5b5fc7`) on the light base |
| **Teams Dark** | `teams-dark` | Teams purple on the dark base |
| **High Contrast** | `hc` | black / white / yellow (cyan hover) |

**How it works end to end:**

1. **The markup** — `ThemePicker` compiles to a CSS-only `<details>` dropdown
   (`.mv-theme-picker`). Each option is a `<button data-mv-theme-set="<key>">` with a
   color swatch and a label. No app JS in the markup at all.

2. **The CSS layers** (`client/glue/motoview.css`) — the named themes are *layered
   on the light/dark neutral bases*, not redefined from scratch:
   - `web-light` is the plain `:root` light base; `web-dark` shares the same block as
     `[data-theme="dark"]` and `[data-theme="teams-dark"]`, so it inherits the full
     dark neutral palette.
   - `[data-theme="teams-light"]` / `[data-theme="teams-dark"]` override only the
     **brand** tokens to Teams purple, sitting on the light/dark neutrals respectively.
   - `[data-theme="hc"]` is a complete high-contrast token set: black backgrounds,
     white text/strokes, yellow brand, cyan hover.

3. **`!important` on named-theme brand tokens** — the Teams and HC brand overrides
   use `!important`. This is deliberate: it lets a user's picked theme **override the
   app's own `@theme brand`**. An app may set its brand in a layout; if the user
   then picks Teams or High Contrast from the picker, their choice wins.

   ```css
   [data-theme="teams-light"] {
     --colorBrandBackground: #5b5fc7 !important;
     --colorBrandForeground1: #5b5fc7 !important;
     /* … */
   }
   ```

4. **The glue** (`client/glue/motoview.js`) — a single delegated click handler:
   - `[data-mv-theme-set]` → `mvSetTheme(key)` sets `<html data-theme=key>` and
     writes the `mv_theme` cookie, closes the dropdown, then calls
     `mvPaintThemePicker()`.
   - `mvPaintThemePicker()` reads the current `data-theme`, marks the matching option
     with `aria-current="true"` (the CSS shows a check + semibold label), and updates
     the summary's `.mv-theme-picker-label` to the active theme's name. It runs on
     load too, so the picker always reflects the saved choice. (It normalizes the
     toggle's `light`/`dark` keys to `web-light`/`web-dark`, and falls back to
     `prefers-color-scheme` when no `data-theme` is set.)

5. **No-flash + persistence** — exactly the same `mv_theme` cookie and inline head
   script as the toggle. The head regex explicitly allows the picker's keys
   (`web-light|web-dark|teams-light|teams-dark|hc|light|dark`), so the picked theme
   is applied before first paint on every load.

6. **Works on certified/cacheable pages** — because the theme is applied **entirely
   client-side from the cookie** (head script + glue), it never changes the server's
   HTML or its certificate. A `@cacheable` certified page and a dynamic page behave
   identically: the cached HTML ships neutral, then the head script paints the user's
   theme instantly.

`ThemeToggle` and `ThemePicker` can coexist — they share the cookie, and
`mvPaintThemePicker` understands both key spaces.

## The vibrant per-surface accent system (Bzzz)

Fluent's neutral palette is the right default for *most* of an app, but a
multi-surface product often wants each area to carry its own hue (Discord blurple,
WhatsApp green, X blue, the forum's violet). Bzzz layers a small **per-surface
accent system** on top of the Fluent tokens — vivid where it counts, neutral
everywhere else.

A surface declares two custom properties and the shared accent classes do the rest:

```css
/* the layout sets sensible defaults… */
.bz-content { --bz-accent: var(--colorBrandBackground); --bz-accent-2: #7c3aed; }
```

```razor
@* …and a page overrides them for its own hue *@
<div class="fd-wrap" style="--bz-accent: #1d9bf0; --bz-accent-2: #0ea5e9;">
  <div class="bz-banner bz-banner-row">
    <span class="bz-banner-ico">📰</span>
    <div><h1>Feed</h1><p>What everyone's buzzing about.</p></div>
  </div>

  <h2 class="bz-sec-title">Trending</h2>
  <span class="bz-accent-chip">#motoview</span>

  <article class="bz-card bz-card-accent"> … </article>
</div>
```

| Class / token | What it does |
|---|---|
| `--bz-accent` / `--bz-accent-2` | the surface's two-stop accent (used as a gradient). Set per surface; inherits to children |
| `.bz-banner` | a full-bleed gradient hero (`linear-gradient(120deg, accent → accent-2)`), white text, soft accent-tinted shadow, a radial sheen via `::after` |
| `.bz-banner-row` / `.bz-banner-ico` / `.bz-banner-actions` | banner layout helpers (icon + heading row, action buttons) |
| `.bz-sec-title` | a section heading with a small gradient bar before it (`::before`) |
| `.bz-accent-chip` | a pill in the accent color over a `color-mix` accent tint of the surface background |
| `.bz-card-accent` | a `.bz-card` with a 3px gradient accent edge across the top (`::before`) |

Because the gradients and tints are built from `--bz-accent` /
`color-mix(... var(--bz-accent) ...)`, a single surface can re-theme its banners,
chips, titles and card edges by setting just those two variables — and because the
neutrals underneath are still Fluent tokens, the surface remains correct in dark
mode.

**High Contrast still wins.** When the user picks High Contrast, dedicated
`[data-theme="hc"]` rules flatten the vibrant gradients to solid HC colors — banners
become solid black with a yellow border, accent bars and chips become yellow on
black — so accessibility is never sacrificed for flair:

```css
[data-theme="hc"] .bz-banner { background:#000 !important; border:2px solid #ff0;
  box-shadow:none; color:#fff; }
[data-theme="hc"] .bz-card-accent::before,
[data-theme="hc"] .bz-sec-title::before { background:#ff0 !important; }
[data-theme="hc"] .bz-accent-chip { background:#000 !important; color:#ff0 !important;
  box-shadow: inset 0 0 0 1px #ff0; }
```

## Avoiding utility-class soup

The pattern across all three levels is the same: push styling decisions *down* into
tokens and components, and keep your markup describing intent.

```razor
@* Reach for this *@
<Button kind="primary" size="lg">Save</Button>

@* Not this *@
<button class="inline-flex items-center px-4 py-2 rounded-md
  bg-blue-600 text-white font-medium hover:bg-blue-700">Save</button>
```

A rule of thumb:

- **Semantic component** if it's a common UI element — let the component own the look.
- **`@style`** if it's genuinely page-specific layout or one-off styling — and read
  Fluent tokens from it so dark mode and the theme picker keep working.
- **`@theme`** for anything visual you'd want to change in one place: brand color,
  spacing scale, radius, fonts.

Build and deploy as usual:

```bash
motoview build
motoview dev
```

When you're ready to make pages interactive, head to [Events](events.md) and
[Forms & Validation](forms.md).
