---
title: What's New
section: Overview
slug: whats-new
---

# What's New

A changelog of the major recent additions to MotoView. MotoView is a young
framework, so this is a record of what landed and what it does — not a
marketing sheet. Where a feature is opt-in or has a caveat, it says so.

---

## Zero-trust end-to-end encryption (in-browser vetKeys)

The headline addition: a MotoView app can now hold **only ciphertext** on-chain
and decrypt **in the browser**. The canister never sees plaintext and has no
decrypt path. Encryption keys are vetKeys — threshold-derived across the subnet,
so no single node ever holds the key — and the BLS unwrap + IBE encrypt/decrypt
run client-side in a Rust→WASM crypto module. This closes the last pillar of the
zero-trust story (see [Zero-Trust](zero-trust.md) for the full architecture).

It is **opt-in**, and honestly so: the default brain is ~76 KB of WASM; bundling
the `ic-vetkeys` BLS12-381 + IBE machinery adds ~300 KB. Apps that don't need
client-side encryption keep the lean default and pay nothing.

The moving parts:

- **`motoview-crypto` WASM module** — the client crypto (`client-crypto/`,
  built with `tools/build-client.sh`) compiled to `wasm32-unknown-unknown` and
  served by the canister at `/motoview-crypto.wasm`. All BLS/IBE logic lives
  here; there is no crypto in JavaScript.
- **`/_motoview/vetkd/*` endpoints** — every generated actor auto-exposes
  `GET /_motoview/vetkd/public-key` (the master public key) and
  `POST /_motoview/vetkd/derive` (a vetKey for the session-authenticated
  caller, bound to their principal via the II session cookie). The canister
  mediates the threshold derivation but only forwards a blob already encrypted
  to the client's transport key.
- **`window.mvCrypto` glue** — the dumb-hands bridge. It lazy-loads
  `/motoview-crypto.wasm` on first use, gets randomness from `window.crypto`,
  fetches the two endpoints, marshals bytes in and out of the crypto WASM, and
  caches the unwrapped vetKey for the session. No decision logic.
- **Declarative `data-mv-encrypt` / `data-mv-decrypt`** — the entire client
  surface is two attributes, no JavaScript. On submit, every `[data-mv-encrypt]`
  field is IBE-encrypted before the (secure) form is sent. On render, every
  `[data-mv-decrypt]` element is decrypted in place.

```mview
@* encrypt a field in the browser before the secure form is sent *@
<form @submit="addNote" secure>
  <textarea name="note" data-mv-encrypt required></textarea>
  <Button appearance="primary" type="submit">Encrypt &amp; save</Button>
</form>

@* render stored ciphertext; the browser decrypts it in place on load *@
@for n in notes {
  <span data-mv-decrypt="@n.ciphertext">🔒 decrypting…</span>
}
```

- **`EncStore` + `Audit`** — the server-side half. `EncStore` is ciphertext-only
  storage keyed by principal, with **no read-plaintext method** by design.
  `Audit` is an append-only, upgrade-stable log: every sensitive transition
  (store, fetch, key derive) records principal, action, timestamp, and a content
  hash — never the plaintext. The canister can't read your data but can still
  prove who did what, when.
- **`examples/vault`** — a worked per-user encrypted notes vault: the browser
  encrypts, the canister stores ciphertext via the vault store, every access is
  audited, and the browser decrypts on read. Your Motoko handler reads
  `ctx.form` (already ciphertext) and never sees plaintext.

> **Caveat:** the local replica's `dfx_test_key` charges ~26.2 billion cycles per
> derive (deriving is an update call, not a query); mainnet uses `key_1`. Budget
> cycles and cache the unwrapped vetKey for the session. See [Zero-Trust](zero-trust.md#the-dfx_test_key-vs-key_1--cycle-caveat).

---

## Theming

### `<ThemePicker />` — the Fluent theme menu

Drop in `<ThemePicker />` for a dropdown of the official Fluent themes, exactly
like the Fluent UI docs: **Web Light**, **Web Dark**, **Teams Light**,
**Teams Dark**, and **High Contrast**. The `teams-*` themes swap the brand to
Teams purple; `hc` is a black/white/yellow high-contrast token set. It uses the
same `mv_theme` cookie + no-flash mechanism as the toggle (a tiny inline head
script re-applies the saved theme before first paint, so it works even on
certified cacheable pages), and a user's picked theme overrides the app's
`@theme brand`.

```razor
<ThemePicker />
```

### `<ThemeToggle />` — light/dark switch

A simpler primitive for just light vs dark. Clicking it flips
`<html data-theme="light|dark">` instantly and saves the choice in the same
`mv_theme` cookie; with no saved choice it follows `prefers-color-scheme`. Like
the picker, it's a framework primitive — **no application JavaScript**.

```razor
<ThemeToggle />
```

See [Styling & Themes](styling-and-themes.md) for the full theming model
(`@theme brand="#hex"`, the built-in theme packages, and the design tokens).

---

## Bzzz: Discourse-style forum clone

The Bzzz forum (`apps/bzzz`) was rebuilt as a faithful clone of
**forum.dfinity.org** (stock Discourse skinned with DFINITY branding). It
reproduces the real layout: the left sidebar, the latest-topics table with
Topic / Replies / Views / Activity columns, category badges with the actual
forum.dfinity.org colors, pinned/closed/solved glyphs, the Latest / Categories /
New / Unread / Popular / Top tabs, and the topic thread + post timeline. Whole
rows are clickable links (Discourse's `/forum/t/{id}` URL scheme is mirrored),
not just the title. All rows are real service results — no fabricated topics.

---

## Bzzz: complete role-scope system + Admin Console

Bzzz now has a full two-scope role model:

- **Global (app-wide):** `SuperAdmin > Admin > Moderator > None`
  (`src/Services/Admin.mo`). Bootstrap-safe — the first caller to claim becomes
  the founding Super Admin, the last Super Admin can never be demoted.
- **Per-server:** `Owner > Admin > Moderator > Member`
  (`src/Services/Servers.mo`), with moderation (pin / lock / mute) gated by rank.
- **Cross-scope override:** a global staffer can act in **any** server or forum
  even without a server role — pages gate on
  `Servers.canModerate(sid, caller) or Admin.isMod(caller)`.

Every role transition is enforced server-side in the canister (a client can't
grant itself a role), and roles persist across upgrades. The **Admin Console**
(`/admin`) gives Super Admins a console for global staff management plus app-wide
stats, with bootstrap and no-access states.

---

## Vibrant Fluent UI 2 redesign + left-aligned layout

Bzzz was redesigned around the Fluent UI 2 token foundation — a more vibrant
color treatment, consistent spacing/radii/shadows from the real Fluent tokens
(`--colorBrandBackground`, `--colorNeutralForeground1`, the type ramp, etc.), and
a left-aligned layout that reads like a real app shell rather than a centered
demo.

---

## Expanded Fluent component coverage

A large batch of built-in Fluent components landed this round. Capitalized tags
in any `.mview` resolve to these — see [Components](components.md). New/expanded
this round:

**Buttons**

- **`Button`** — `appearance` (`primary` | `secondary` | `outline` | `subtle` |
  `transparent` | `danger`), `kind` (alias of `appearance`), `size`
  (`small` | `large`), `shape` (`rounded` [default] | `circular` | `square`),
  `icon` (literal emoji/text), `iconPosition` (`before` [default] | `after`),
  `disabled`, `type` (`button` [default] | `submit` | `reset`), plus event
  handlers (`on:click` etc., unchanged).
- **`CompoundButton`** — `appearance`, `kind` (alias), `size`, `secondary` (the
  description line — literal or `@expr`), `icon`, `disabled`, `type`, `on:click`.
- **`ToggleButton`** — `appearance`, `kind` (alias), `size`, `shape`, `name`
  (checkbox name, submitted with forms), `checked` (initial pressed state),
  `icon`, `disabled`.
- **`MenuButton`** — `appearance`, `kind` (alias), `size`, `label` (button text,
  literal or `@expr`), `icon` (before the label), `children` (MenuItem nodes).
- **`SplitButton`** — `appearance` (default `primary`), `kind` (alias), `label`
  (primary action text), `icon` (on the action half), `on:click` (fires on the
  **primary** action button), `children` (MenuItem nodes for the chevron menu).

```razor
<Button appearance="primary" icon="💾" @click="save">Save</Button>
<CompoundButton icon="📥" secondary="Sync from server" @click="pull">Import</CompoundButton>
<SplitButton label="Publish" @click="publish">
  <MenuItem>Save draft</MenuItem>
</SplitButton>
```

**Forms & labels**

- **`Field`** — `label`, `required`, `hint`, `validationMessage`,
  `validationState` (`success` | `warning` | `error` | `none`).
- **`Label`** — `size` (`small` | `medium` | `large`), `weight`
  (`regular` | `semibold`), `required`, `htmlFor`, `for`, `text`.
- **`InfoLabel`** — `label`, `info`, `weight` (`regular` | `semibold`).
- **`SpinButton`** — `name`, `label`, `value`, `min`, `max`, `step`, `disabled`.
- **`Slider`** — `name`, `value`, `min`, `max`, `step`, `vertical`, `disabled`.
- **`Input`** — `name`, `type`, `placeholder`, `value`, `size`
  (`small` | `large`), `appearance` (`outline` | `underline` |
  `filled-lighter` | `filled-darker`), `disabled`, `required`.

**Badges & status**

- **`CounterBadge`** — `count`, `appearance` (`filled` | `ghost` | `outline`),
  `color` (`brand` | `danger` | `important` | `informative` | `severe` |
  `subtle` | `success` | `warning`), `size` (`tiny` → `extra-large`), `showZero`,
  `dot`.
- **`PresenceBadge`** — `status` (`available` | `away` | `busy` | `dnd` |
  `offline` | `out-of-office`), `size`, `outOfOffice`.
- **`Avatar`** — `name`, `size`, `shape`, `presence`, `text`, `color`, `active`,
  `badge`.
- **`Badge`** — `title`, `type`, `appearance`, `shape`, `size`, `color`.
- **`Spinner`** — `size`, `label`, `labelPosition`.
- **`ProgressBar`** — `value`, `max`, `color`, `validationState`, `thickness`,
  `shape`.

**Typography (Fluent type ramp)**

- **`Display`**, **`Title1`** / **`Title2`** / **`Title3`**,
  **`Subtitle1`** / **`Subtitle2`**, **`Body1`** / **`Body2`** (`strong`),
  **`Caption1`** / **`Caption2`** (`strong`).

**Cards, navigation & containers**

- **`CardHeader`** — `header`, `description`, children = leading image/avatar slot.
- **`CardPreview`** — `src`, `alt`, children = custom media.
- **`CardFooter`** — children = action buttons / controls.
- **`Tree`** / **`TreeItem`** (`label`, `open`).
- **`Toolbar`** (`size`), **`Carousel`**.
- **`TagGroup`** (`size`), **`Tag`** (`value`, `dismissible`, `interactive`,
  `appearance`, `size`, `shape`), **`InteractionTag`** (`value`, `dismissible`,
  `size`, `@click`).
- **`MenuItemCheckbox`** / **`MenuItemRadio`** (`name`, `value`, `checked`).

```razor
<Card>
  <CardHeader header="Release 0.4" description="Zero-trust + Fluent batch" />
  <CardPreview src="/shot.png" alt="screenshot" />
  <CardFooter>
    <Button appearance="primary">Open</Button>
  </CardFooter>
</Card>
```

---

## Compiler robustness

- **Keyword-safe component params.** A component `param` named after a Motoko
  reserved word (e.g. `type`, `label`, `for`, `class`) is now **auto-mangled** to
  a guaranteed-safe `mvP_<name>` identifier in the generated Motoko, while the
  prop name you write stays the same. Any param name works — you find out at
  build time that nothing breaks, instead of hitting an `moc` error. (See
  `compiler/src/codegen.rs`, `mangle_param`.)
- **Arbitrary component nesting** with props + `@children` is verified — deeply
  nested capitalized components passing typed props and yielding child content
  compile and render correctly.

---

## Where to go next

- [Zero-Trust](zero-trust.md) — the full encrypted-storage architecture and
  threat model.
- [Styling & Themes](styling-and-themes.md) — `@theme`, the theme packages,
  `<ThemePicker />` / `<ThemeToggle />`.
- [Components](components.md) — authoring components and the built-in catalog.
