<!-- Phased implementation plan (2026-06-13). Grounded in real file:line hooks via a 6-area code map. Security-first: keeps the II auth-gate; read-only public browsing explicitly deferred. -->

# Bzzz World-Best-UX Implementation Plan — Persistent Shell, Forum Wayfinding, Cmd+K, Value-First Onboarding

Built the MotoView way: server-driven, full-page SSR, no client logic in the glue. Every step is grounded in verified file:line hooks.

---

## 0. Verified ground truth (what the code actually is today)

- **Layouts are document-level functions**, not persistent shells. `renderDocument()` calls `findLayout(page.layout)` then `layout.render(ctx, head, wrapped)` on **every** page GET — there is no cross-navigation persistence (`runtime/src/App.mo:834-847`). The `Layout` type is `{ name; render : (Ctx, Head, Text) -> Text }` (`runtime/src/Types.mo:165-168`). The compiler emits one `mvLayout_<Name>(ctx, mvHead, mvBody)` per `.mview` in `src/Layouts/` (`compiler/src/codegen.rs:651-675`).
- **`ctx.path` is available** to every layout and page (`runtime/src/Types.mo:39`, "clean path, e.g. `/products/42`") and is already used inside `ForumLayout` for active-state (`ForumLayout.mview:150,192`). This is our server-side router signal for active nav + breadcrumb assembly.
- **The auth gate is correctly page-and-layout level.** Both layouts wrap their *entire* chrome in `@if (ctx.isAuthenticated and Identity.isBound(ctx.caller)) { … } else { redirect }` (`AppLayout.mview:144-190`, `ForumLayout.mview:118-262`). The gated chrome (AppBar, Nav, forum sidebar, data) is **only emitted inside the authed branch** — anonymous users get a meta-refresh stub, never the shell HTML. This is the property we must preserve.
- **Reusable components already exist as codegen built-ins:** `AppBar` (`codegen.rs:1673`), `Nav` (`:1667`), `NavItem`/`Tab` (`:1689`), `Breadcrumb` (`:1979`), `BreadcrumbItem` (`:1987`). `Breadcrumb` is already used on `/me`, `/u/{handle}`, `/servers/{id}`, `/forum/new` (`Me.mview:7`, `Profile.mview:8-11`, `Server.mview:15-19`, `ForumNew.mview:9-13`) — so the forum-breadcrumb work is *reusing a working component*, not building one.
- **The CSS-only modal pattern is real:** `.mv-dialog-toggle:checked ~ .mv-dialog-overlay` (`motoview.css:2919`) and the forum off-canvas `.fl-burger:checked ~ .fl-shell .fl-sidebar` (`ForumLayout.mview:108`). This is the basis for a no-JS Cmd+K open state and mobile toggles.
- **There are NO keyboard listeners in the glue.** `start()` wires click/submit/input/change/drag*/visibilitychange only (`motoview.js:534-545`). A Cmd+K *keyboard shortcut* therefore requires either (a) a one-line logic-free keydown dispatcher added to the glue, or (b) no keyboard binding at all (a visible trigger + `:target`/checkbox). This is the single honest "needs minimal glue" item.
- **AppBar markup is duplicated** verbatim across `AppLayout.mview:150-166` and `ForumLayout.mview:127-144`. The forum mini-app-switcher (`ForumLayout.mview:230-235`) is missing Home and Forum — the documented "breaks the loop" bug.

---

## 1. Architecture decisions

### 1.1 Shell model — one `BzShell` component, included into both layouts (not a forced single layout)

**Decision: do NOT collapse AppLayout + ForumLayout into one RootShell.** The forum legitimately needs a category sidebar driven by `Forum.categories()` (`ForumLayout.mview:191-205`); the app rail does not. Forcing one layout creates the documented "kitchen-sink" risk. Instead:

- Create **`apps/bzzz/src/Components/BzShell.mview`** — a component (not a layout) holding the responsive global nav (bottom-bar / rail / sidebar) **and** the Cmd+K overlay. Components compile to reusable render functions and can be invoked from any template.
- **Include `<BzShell active="…"/>` inside the authed branch of BOTH layouts**, replacing `AppLayout`'s inline `<Nav>` block (`AppLayout.mview:168-178`) and supplementing `ForumLayout`'s sidebar. This installs the global shell **once per layout file** (2 call sites, identical component), so the nav model is consistent everywhere and every page inherits it through its existing `@layout` — **no page `@layout` changes required.**
- This wraps "every page once" in the operational sense: all 18 gated pages go through one of two layouts, each of which renders the *same* `BzShell`. Pinned Home, the four pillars, and Cmd+K are now present at every depth, including `/forum/t/{id}`.

**Why a component, not editing each layout inline:** kills the AppBar/nav duplication (the documented duplication risk), gives one source of truth for breakpoints and the pinned-Home affordance, and keeps blast radius to 1 new file + 2 small edits.

### 1.2 Responsive strategy — pure CSS media queries, unified 600/840 breakpoints

All three responsive states are **pure CSS, zero JS**, placed in `client/glue/motoview.css` (global, themable via tokens) so both layouts share them:

- **`< 600px` → bottom tab bar.** `.bz-shellnav { position: fixed; bottom: 0; left:0; right:0; height: 56px; display:flex; justify-content: space-around; }`, icons-only, `env(safe-area-inset-bottom)` padding. Content area gets `padding-bottom: 64px`. **Fixes the documented "nav just vanishes under 860px" defect** (`AppLayout.mview:114` `display:none`).
- **`600–839px` → icon nav-rail.** `width: 72px`, vertical flex, icon + tiny label, sticky `top:48px`.
- **`≥ 840px` → labeled sidebar.** `width: 248px` (token `--bz-shell-w`), current desktop behavior.
- **Unify breakpoints to 600/840.** Today AppLayout uses 860px (`:113`), ForumLayout 900px (`:103`) and 560px (`:113`). Define `--bp-mobile: 600px` / `--bp-rail: 840px` as documentation constants and rewrite both layouts' nav media queries to 600/840. The forum *category* sidebar keeps its own off-canvas behavior but its breakpoint moves to 840 for consistency.

No `window.matchMedia`, no JS breakpoint detection — the layout reflows by CSS alone.

### 1.3 How each shortlist item maps to the no-JS rule (honest ledger)

| Shortlist item | Realization | No-JS classification |
|---|---|---|
| **Persistent shell + pinned Home** | `BzShell` rendered by both layouts; Home is the first, visually-distinct, always-on nav item with `match="/"` active state computed from `ctx.path` server-side | **Fully no-JS.** Pure SSR HTML + CSS media queries. |
| **Forum breadcrumbs** | Reuse existing `<Breadcrumb>`/`<BreadcrumbItem>` (`codegen.rs:1979`) in `Forum.mview`, `ForumCategory.mview`, `ForumTopic.mview`; trail built in page `@code` from `categoryId`/`catName` already in scope (`ForumTopic.mview:311,314,370`) | **Fully no-JS.** Server-rendered `<a>` links, sticky via CSS. |
| **Labeled Home affordance** | Keep logo→`/` link; add a *labeled* "🏠 Home" item in `BzShell` and a "🏠 Go to Home" row in the account menu (`AppLayout.mview:156-164`) | **Fully no-JS.** |
| **Back vs. Up controls** | **Up** = breadcrumb parent crumb (fixed parent, always correct, server-rendered). **Back** = a `← Back` link rendered only when a `?from=<safe-path>` query param is present; the server validates/whitelists `from` and renders it as the href. **Seeded back-stack** = on deep entry the server synthesizes the parent chain into the breadcrumb (Up always works) and links deep CTAs with `?from=`. | **Up: fully no-JS.** **Back via `?from=`: fully no-JS but needs brain logic** (whitelist validation in Motoko). **We do NOT touch `history.pushState`** (that would be client logic). Browser Back still works natively; we add visible Up + optional Back affordances on top. |
| **Cmd+K palette** | Overlay markup + result list rendered server-side in `BzShell`; opened via a visible 🔍 trigger using the `.mv-dialog-toggle:checked` CSS pattern (`motoview.css:2919`). Search results: the input is a normal `<form>` GET to a `/search` page (server does the matching) — **not** per-keystroke. | **Open/close: fully no-JS** (checkbox/`:target`). **Cmd+K *keyboard binding*: needs ONE logic-free glue line** (see 1.4). **Search ranking: brain logic** (Motoko `Text.contains`), exactly where logic belongs. |
| **Value-first onboarding** | Server-rendered micro-tour cards on first authed Home (gated by `showPersonalize`, `Home.mview:190`) + improved II microcopy in `Welcome.mview:15-16`. **Read-only public browsing is explicitly OUT of default scope** (see Security §2). | **Fully no-JS.** SSR cards, skippable via links. |

### 1.4 The one honest "minimal glue" decision: Cmd+**K** binding

The glue has no keydown listener (`motoview.js:534-545`). To bind the literal Cmd/Ctrl+K **keyboard shortcut**, the only options are:

- **(A) Recommended for Phase 1:** ship the palette with a **visible 🔍 trigger** (in AppBar) that toggles the overlay via the existing checkbox/`:target` CSS — **zero glue change**, fully within the rule. The shortcut is deferred.
- **(B) Phase 4 (opt-in):** add exactly one logic-free dispatcher mirroring the existing `visibilitychange` precedent (`motoview.js:543-545`):
  ```js
  document.addEventListener("keydown", function (e) {
    if ((e.metaKey || e.ctrlKey) && (e.key === "k" || e.key === "K")) {
      e.preventDefault();
      var t = document.getElementById("bz-cmdk-toggle"); // the checkbox
      if (t) { t.checked = true; }                        // mechanical: flip a checkbox
    }
  }, true);
  ```
  This is **mechanical DOM only** (flip one checkbox the server rendered) — it contains **no app logic, no filtering, no routing, no state machine**. It is the same category as the existing drag/visibility wiring. We will gate this behind explicit owner approval and document it as the single keyboard-affordance exception. **It is not required for the palette to function.**

This is the only place the plan touches `motoview.js`. Everything else is SSR + CSS + Motoko.

---

## 2. Security review (per change)

**Secure default, stated up front:** Bzzz today gates **100%** of content behind II (`About.mview:13`: "all of Bzzz requires II sign-in"). **This plan KEEPS that default.** Writes stay gated behind II + `@authorize` (enforced at all three entrypoints — document GET, `/_motoview/render`, `/_motoview/event` per the map). **Read-only public browsing is NOT enabled by this plan**; it is called out only as a separately-approvable product decision (§2.6) and is explicitly deferred. No change in this plan weakens the auth gate.

| Change | Security implication | Mitigation / property preserved |
|---|---|---|
| **2.1 `BzShell` component inside both layouts** | The shell renders nav links, the account handle (`Identity.atHandleOf`), and Cmd+K results — all gated data. Must never reach anonymous users. | `BzShell` is included **only inside the existing `@if (ctx.isAuthenticated and Identity.isBound) {…}` branch** (`AppLayout.mview:149`, `ForumLayout.mview:123`). The anonymous branch is untouched and still emits only the redirect stub. **Verify post-edit**: anonymous GET of `/`, `/forum`, `/forum/t/1` returns zero shell/nav/handle HTML (see test §5). |
| **2.2 Forum breadcrumbs** | Breadcrumb shows `catName` and topic title — both already gated page content. A deleted-category link could 404. | No new data exposure (already rendered on the page). Render breadcrumb **inside the topic-exists `@if`**, not the 404 branch (`ForumTopic.mview:164-172`). Fall back the category crumb to `/forum` if `categoryId` no longer resolves. |
| **2.3 Back via `?from=` query param** | **Open-redirect / reflected-link risk** if `from` is rendered into an `href` unvalidated (`?from=https://evil.com` or `javascript:`). | **Brain-side whitelist**: in the page `@code`, accept `from` **only** if it `Text.startsWith(#text "/")`, does **not** start with `//` or `/\`, and contains no `:`. Otherwise drop the Back link entirely. Never reflect raw `from` into HTML without this guard. This is the single most important security gate in the plan. |
| **2.4 Cmd+K → `/search`** | Search runs server-side over gated data; results must respect the caller's authz. Query string is reflected into the results page. | `/search` carries `@authorize redirect="/welcome"` like every other page (so the gate runs at all three entrypoints). Escape the query when echoing it (use the framework's auto-escaped `@(expr)`, never `@raw`). Cap results (e.g. 20) to bound cost at the 5001-user scale. |
| **2.5 Onboarding micro-tour / microcopy** | Tour cards render only for authed users (gated by `showPersonalize`, `Home.mview:190`). II microcopy is static copy. | No data exposure. Do **not** auto-bind on a read in new code paths; reuse the existing acknowledged auto-bind in `Welcome.onLoad` only. |
| **2.6 Read-only public browsing (DEFERRED, not in scope)** | Would require `authorized()` to distinguish read vs write and would expose feed/forum to anonymous principals; also interacts with `@cacheable` certified queries and E2EE scoping. | **Out of scope.** If the owner later approves it, it is a *separate* PR that (a) splits read/write authz, (b) updates `About.mview:13`, (c) audits every page for accidental gated-data leak, (d) decides certified-response strategy. Flagged here so it is a conscious decision, not an accident. |

**Cross-cutting security invariants for every phase:**
- **No inline scripts / CSP-friendly.** All new behavior is SSR HTML + CSS + the existing glue. The optional Cmd+K keydown (1.4B) lives in `motoview.js` (already a served, CSP-allowed asset) — **not** an inline `<script>`. No `onclick=` attributes.
- **Certified responses unaffected.** Bzzz uses the update path (not `@cacheable`), so cookie-derived authz and the gate are evaluated per request; the shell adds no certified-cacheable surface that could leak.
- **Persistent shell must not leak gated data to anonymous users** — guaranteed by placing it strictly inside the authed `@if` branch (2.1).
- **Regression guard for the auth gate** is a first-class test (see §5).

---

## 3. Phased plan (ordered by leverage; each phase independently shippable)

### Phase 1 — Persistent shell + pinned Home + responsive nav (highest leverage; fixes "can't get home from the forum")

**Goal:** A global nav present at every depth in both layouts, with Home pinned/visually-distinct and a labeled Home affordance; bottom-bar/rail/sidebar reflow by CSS; the forum mini-switcher loop fixed.

**Files to create:**
- `apps/bzzz/src/Components/BzShell.mview` — the shell component:
  - Renders an `active` prop (the current pillar) for active-state styling, computed by the caller from `ctx.path`.
  - Nav items, Home first and visually distinct: `🏠 Home`(`/`), `📰 Feed`(`/feed`), `💬 Servers`(`/servers`), `🗨️ Forum`(`/forum`), `✉️ Messages`(`/messages`), `🟢 Status`(`/status`), `👤 Me`(`/me`) — mirroring the existing item set/icons (`AppLayout.mview:170-177`) so users aren't disoriented.
  - One semantic `<nav class="bz-shellnav" aria-label="Primary">` reflowed by CSS (single DOM, not duplicated per breakpoint — avoids the HTML-doubling risk).

**Files to edit:**
- `client/glue/motoview.css` (append after the token block ~`:312`): `.bz-shellnav` + the three media-query variants (1.2), all using `--color*`/`--spacing*` tokens (themable, dark-mode-safe), motion via `--durationNormal`/`--curveEasyEase`.
- `apps/bzzz/src/Layouts/AppLayout.mview`: replace the inline `<Nav>…</Nav>` block (`:168-178`) with `<BzShell active="@bzActive(ctx.path)"/>`; remove the `@media (max-width:860px){ .mv-nav{display:none} }` rule (`:113-114`).
- `apps/bzzz/src/Layouts/ForumLayout.mview`: add `<BzShell active="forum"/>` inside the authed branch (the global pillars), keep the forum category sidebar as forum-specific chrome; **fix the mini-switcher** (`:230-235`) to include `🏠 Home`(`/`) first and re-add `🗨️ Forum`(`/forum`) — closes the documented broken loop. Add the labeled "🏠 Go to Home" row to the account menu (`:135-141`).
- `apps/bzzz/src/Layouts/AppLayout.mview` account menu (`:156-164`): add `<a class="mv-acct-item" href="/">🏠 Go to Home</a>` (the documented quick-win escape hatch).
- Add a tiny helper `bzActive(path:Text):Text` — either in a shared `@code` include or inlined per layout — mapping `ctx.path` → pillar via `Text.startsWith` (same idiom as `ForumLayout.mview:150`).

**Test plan:**
- `motoview check apps/bzzz` (type-check) passes.
- Compiler build of `apps/bzzz/.mvbuild/main.mo` succeeds; run the compiler test suite (`compiler/tests/`) — golden/codegen tests green.
- Render-walk at the seeded 5001-user scale: GET `/`, `/feed`, `/servers`, `/forum`, `/forum/t/{id}`, `/messages` — confirm `bz-shellnav` present in every response and active pillar correct.
- Responsive: load each at 375px / 768px / 1280px (webapp-testing/agent-browser) — confirm bottom-bar at 375, rail at 768, sidebar at 1280, no duplicate nav, content not occluded.
- **Auth-gate regression**: anonymous GET `/forum/t/1` contains **no** `bz-shellnav`/handle HTML, only the redirect stub.

**Done criteria:** From any page including a deep forum thread, Home is reachable in **one tap** on all three breakpoints; nav never disappears on mobile; forum mini-switcher reaches Home + Forum; auth gate intact.

---

### Phase 2 — Forum breadcrumbs (the owner's literal pain point)

**Goal:** Sticky `Home › Forum › Category › Topic` breadcrumb on `/forum`, `/forum/c/{id}`, `/forum/t/{id}`, first crumb always Home→`/`.

**Files to edit (reusing the existing `<Breadcrumb>` component, `codegen.rs:1979`):**
- `apps/bzzz/src/Pages/ForumTopic.mview`: insert after the category block (~`:31`), **inside** the topic-exists branch:
  ```
  <Breadcrumb>
    <BreadcrumbItem href="/">Home</BreadcrumbItem>
    <BreadcrumbItem href="/forum">Forum</BreadcrumbItem>
    <BreadcrumbItem href="/forum/c/@categoryId">@catName</BreadcrumbItem>
    <BreadcrumbItem current="true">@topicTitle</BreadcrumbItem>
  </Breadcrumb>
  ```
  Data already in scope: `categoryId` (`:311`), `catName` (`:314`, set from `Forum.categoryName` at `:370`), `topicTitle` (`:312`).
- `apps/bzzz/src/Pages/ForumCategory.mview`: add `Home › Forum › {category}` after the head (~`:20`).
- `apps/bzzz/src/Pages/Forum.mview`: add `Home › Forum` above the controls (~`:10`).
- `client/glue/motoview.css`: add sticky styling to the breadcrumb class — `position: sticky; top: 48px; z-index: 4; backdrop-filter: blur(8px)` (mirrors the `FeedPost.fp-head` pattern), with a `<600px` compact variant (smaller padding/font, optionally collapse to immediate parent only).

**Test plan:**
- `motoview check` + compiler suite green; golden snapshots for the three forum pages updated and reviewed.
- Manual walkthrough: `/forum/t/{id}?page=3` → Home crumb lands on `/` (not `/forum`); Category crumb lands on `/forum/c/{id}`; breadcrumb stays visible on scroll.
- Edge: deleted-category topic → category crumb degrades gracefully (link to `/forum`, name still shown).
- Confirm breadcrumb **absent** in the 404 branch (`:164-172`).

**Done criteria:** Every forum list/category/thread page shows a sticky, clickable breadcrumb; from a thread, Home is one tap and Forum-home is one tap; no breadcrumb on the 404 state.

---

### Phase 3 — Back vs. Up + deep-link safety

**Goal:** Separate, visible **Up** (fixed parent, always present via breadcrumb) and **Back** (history, present only when a validated `?from=` is supplied); deep entries land with a correct Up chain.

**Files to edit:**
- Forum/feed/server detail pages (`ForumTopic.mview`, `FeedPost.mview`, `Server.mview`, `Channel.mview`): in `@code`, read `ctx.query`'s `from`, run the **whitelist guard** (§2.3), and render a `← Back` link only when valid. Keep the existing explicit back links (e.g. `FeedPost.mview:10`, `Conversation.mview:10,56`).
- Deep-link CTAs (notification/share targets, e.g. a topic row linking to `/forum/t/{id}`): append `?from=/forum/c/{categoryId}` so Back resolves to the parent. **Up** needs no seeding — the breadcrumb (Phase 2) already synthesizes the full parent chain server-side from `categoryId`, which **is** the documented "seed back-stack on deep entry" done safely without `history` APIs.

**Test plan:**
- Visit `/forum/t/{id}` with no `from` → no Back link, Up (breadcrumb) works.
- With `?from=/forum/c/3` → Back link to `/forum/c/3`.
- **Security**: `?from=https://evil.com`, `?from=//evil.com`, `?from=javascript:alert(1)`, `?from=/\evil` → **no** Back link rendered (guard rejects). This is a required passing security test.
- `motoview check` + suite green.

**Done criteria:** Up always present and correct on detail pages; Back appears only for validated internal `from`; open-redirect attempts produce no link.

---

### Phase 4 — Cmd+K command palette

**Goal:** A jump-to-anywhere palette: pinned destinations (Home/Feed/Servers/Forum/Messages/Status) + a server-side search.

**Files to create/edit:**
- `apps/bzzz/src/Pages/Search.mview` (new): `@page "/search"`, `@authorize redirect="/welcome"`; a `<form method="get">` with `<input name="q" type="search">`; `@code` runs `Text.contains` over topics/channels/handles, **capped at 20 results**, query echoed via auto-escaped `@(q)`.
- `BzShell.mview`: render the palette overlay using the `.mv-dialog-toggle:checked ~ .mv-dialog-overlay` pattern (`motoview.css:2919`) — a checkbox `#bz-cmdk-toggle` + a 🔍 trigger `<label>` in the AppBar area; overlay contains the pinned links + a search `<form action="/search">`. **Open/close is pure CSS.**
- `client/glue/motoview.css`: palette overlay styling (`--shadow16`, `--durationNormal`, tokens only).
- **(Opt-in, owner-approved)** `client/glue/motoview.js`: add the one logic-free keydown→checkbox dispatcher (1.4B) so Cmd/Ctrl+K opens it. Escape-to-close = a backdrop `<label>` (CSS) per the dialog precedent.

**Test plan:**
- Trigger opens/closes palette with **no glue** (checkbox path); Cmd+K opens it if 1.4B shipped.
- `/search?q=…` returns ≤20 escaped results at 5001-user scale; latency measured and acceptable.
- **Security**: `/search` enforces the gate (anonymous → redirect); `q=<script>` is escaped, not executed.
- `motoview check` + suite green.

**Done criteria:** Palette reachable from every page, pinned links jump correctly, search returns bounded escaped results, gate enforced, no app logic in glue.

---

### Phase 5 — Value-first onboarding (teach the nav model)

**Goal:** New, freshly-bound users learn the persistent shell + breadcrumb model; II microcopy is benefit-first.

**Files to edit:**
- `apps/bzzz/src/Pages/Home.mview`: behind the existing `showPersonalize` flag (`:190`), render a 3-card, skippable, server-rendered micro-tour teaching only the model: "This is your Home (🏠, one tap from anywhere)", "These four pillars switch apps", "Breadcrumbs (🏠 › Section › Post) get you back". Skip = a link that sets a dismiss state. No JS modal.
- `apps/bzzz/src/Pages/Welcome.mview` (`:15-16`): tighten II copy to passkey/biometric, benefit-first; label the create-identity link explicitly.
- Empty states (first feed/forum/DM view): replace "No posts yet" with a "Start here" CTA that names the first action.

**Test plan:** Fresh-bind user sees the tour once, can skip, doesn't see it again; copy renders; `motoview check` + suite green. **Read-only public browsing is NOT added** (gate unchanged).

**Done criteria:** New users encounter a skippable nav-model tour; II copy is benefit-first; default gate unchanged.

---

## 4. Component inventory

| Component | Location | Responsibility |
|---|---|---|
| **`BzShell`** | `apps/bzzz/src/Components/BzShell.mview` (new) | Responsive global nav (bottom-bar/rail/sidebar) + pinned Home + Cmd+K overlay; included in both layouts' authed branch. |
| **`.bz-shellnav` + variants** | `client/glue/motoview.css` (new rules) | The three CSS breakpoint states (600/840), token-based, dark/Material-safe. |
| **`Breadcrumb` / `BreadcrumbItem`** | Existing built-ins (`codegen.rs:1979,1987`); new usages in `Forum.mview`, `ForumCategory.mview`, `ForumTopic.mview` | Forum wayfinding (Phase 2). **Reused, not created.** |
| **HomeButton affordances** | Account menus (`AppLayout.mview:156-164`, `ForumLayout.mview:135-141`) + `BzShell` first item | Labeled, redundant Home escape hatches. |
| **CommandPalette overlay** | `BzShell.mview` + `apps/bzzz/src/Pages/Search.mview` (new) | CSS-toggled overlay + server-side `/search`. |
| **Back/Up controls** | Page `@code` in `ForumTopic`/`FeedPost`/`Server`/`Channel` | Whitelisted `?from=` Back link; Up via breadcrumb. |
| **Onboarding microcopy/tour** | `Home.mview` (`showPersonalize`), `Welcome.mview:15-16`, empty states | Teaches the nav model; benefit-first II copy. |
| **Optional keydown dispatcher** | `client/glue/motoview.js` (one block, opt-in) | Mechanical Cmd+K → flip checkbox. The sole glue change. |

---

## 5. Testing & verification strategy

1. **Type-check:** `motoview check apps/bzzz` after every phase (it's a real subcommand — `compiler/src/main.rs:39`).
2. **Compiler/codegen suite:** run `compiler/tests/`; update and human-review golden snapshots for every edited `.mview` (layouts, forum pages, Home, Welcome, new Search/BzShell).
3. **Build:** confirm `apps/bzzz/.mvbuild/main.mo` regenerates and the canister builds.
4. **Render/size at 5001-user scale (already seeded):** GET each pillar + a deep `/forum/t/{id}` + `/search?q=…`; assert shell present, active pillar correct, breadcrumb present on forum pages, response sizes sane (palette/breadcrumb don't bloat HTML), `/search` ≤20 results.
5. **Manual nav walkthroughs (every pillar → Home in one tap):** Servers→channel→Home; Feed→post→Home; **Forum→thread→Forum-home→app-Home** (the headline fix); Messages→thread→Home. Verify on 375/768/1280px (webapp-testing or agent-browser skill).
6. **Accessibility:** keyboard-tab through nav + palette; visible focus (tokens already define `:focus-visible`); `aria-label="Primary"` on nav, `aria-label="Breadcrumb"` on breadcrumb, labeled Home not icon-only on desktop; Escape closes palette.
7. **Auth-gate regression guard (run every phase):** anonymous GET `/`, `/forum`, `/forum/t/1`, `/search` returns only the redirect stub — **zero** shell/nav/handle/data HTML. Confirm the gate still fires at `/_motoview/render` and `/_motoview/event` (not just document GET). Optionally wire the `layout-auth-gate` lint to fail CI.
8. **Security tests (Phase 3/4):** the `?from=` open-redirect rejection matrix and the `/search` escape/gate tests above must pass.

---

## 6. Risks & watch-outs

- **Anti-patterns to NOT reintroduce:** no hamburger-hiding the nav (we *replace* `display:none` at `:114` with a bottom-bar); Home stays sticky/pinned & visually distinct; Up (breadcrumb) is seeded server-side so deep entries aren't dead-ends; Back ≠ Home (separate controls, Phase 3).
- **MotoView-specific:**
  - **Codegen/golden tests:** new component + breadcrumb arms will shift snapshots; review each diff (don't blind-accept) to catch escaping/structure regressions.
  - **Layout blast radius:** editing both layouts touches all 18 gated pages at once — Phase 1's render-walk + auth-gate guard must run before merge.
  - **AppBar duplication:** `BzShell` must not re-duplicate AppBar; AppBar stays in each layout, `BzShell` owns only the nav + palette. (Future cleanup: promote AppBar to a single shared include.)
  - **Forum three-column crowding (≥840px):** category sidebar + content + optional right rail + global shell — verify with grid/flex that columns don't collide; if tight, the global rail can defer to the forum's own switcher on `ForumLayout` (now fixed).
  - **Breadcrumb cost on `/forum/t/{id}`:** reuse already-fetched `catName`/`categoryId` (`:314,370`) — no extra service calls.
  - **`from` open-redirect** (Phase 3) — the single highest-severity item; the whitelist test is mandatory.
  - **Glue temptation creep:** keep the keydown dispatcher strictly to flipping one checkbox; any filtering/routing in JS violates the rule and must be rejected in review.

---

## 7. Definition of done

- [ ] **Navigable:** from any page (esp. a deep `/forum/t/{id}`), Home is one tap on mobile/tablet/desktop; forum thread → forum-home → app-home each one tap; mini-switcher reaches Home + Forum.
- [ ] **Forum wayfinding:** sticky `Home › Forum › Category › Topic` breadcrumb on `/forum`, `/forum/c/{id}`, `/forum/t/{id}`; first crumb → `/`; absent on 404.
- [ ] **Back vs. Up:** Up (breadcrumb) always correct; Back appears only for whitelisted internal `?from=`; all open-redirect attempts rejected.
- [ ] **Cmd+K:** palette reachable everywhere (visible trigger always; keyboard shortcut if 1.4B approved), pinned links + bounded escaped `/search`.
- [ ] **Onboarding:** skippable nav-model tour for fresh-bind users; benefit-first II copy; empty states guide first action.
- [ ] **No-JS rule honored:** all logic in Motoko/SSR + CSS; the only glue change is the optional, logic-free, mechanical keydown→checkbox dispatcher.
- [ ] **Secure:** auth gate unchanged and verified (anonymous sees no shell/data at document/render/event entrypoints); `/search` gated + escaped; `?from=` whitelisted; CSP/no-inline-script preserved; read-only public browsing explicitly deferred.
- [ ] **Tested & bug-free:** `motoview check` clean; compiler suite + reviewed goldens green; render/size + nav + a11y walkthroughs pass at 5001-user scale; auth-gate regression guard green every phase.
- [ ] **Responsive:** 600/840 breakpoints unified across both layouts; bottom-bar/rail/sidebar correct, no duplicate nav, no content occlusion, dark/Material themes intact.