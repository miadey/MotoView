# Pulse — Master Phased Execution Plan (Bzzz → Pulse rebrand/redesign)

> Grounded against the live codebase at `apps/bzzz` (compiler verified: `motoview build apps/bzzz --name bzzz` → exit 0 in ~0.07s; `moc --check … apps/bzzz/.mvbuild/main.mo` → 0 errors, warnings only). Canister stays `bzzz` (`fi4qb-naaaa-aaaab-qhaxq-cai`). Routes confirmed: 19 pages across 3 layouts.

---

## 1. Decisions

### 1.1 Theme-token strategy — **remap Fluent tokens in the APP layer, force Pulse dark**
- **Keep `@theme brand="#8b5cf6"`** in each layout header (`AppLayout.mview:1`, `ForumLayout.mview:1`). Verified: the compiler's `color.rs::brand_theme_css` (compiler/src/color.rs:98-112) emits an **inline `<style>:root{…}[data-theme="dark"]{…}@media (prefers-color-scheme: dark){…}</style>`** — it does **not** touch `runtime/src/ClientAssets.mo`. So swapping the brand hex re-ramps every Fluent `--colorBrand*` with **zero ClientAssets regen** (honors constraint 3).
- **Do NOT edit `client/glue/motoview.css` / `ClientAssets.mo`.** All Pulse theming lives in a `<style>` block in the layouts (compiles into `.mvbuild/main.mo`). This is the explicit constraint-3 win.
- **Define Pulse tokens once, alias Fluent to them.** In each layout `<style>` add, after the `body{…}` rule (AppLayout.mview:12), a `:root` block:
  - Raw Pulse tokens from spec §2: `--pulse-bg:#0a0a0f; --pulse-bg-sidebar:#0d0d15; --pulse-surface:#15151e; --pulse-surface-2:#1c1c27; --pulse-border:#232331; --pulse-primary:#8b5cf6; --pulse-primary-hover:#7c3aed; --pulse-primary-soft:rgba(139,92,246,.14); --pulse-text:#f3f4f6; --pulse-text-2:#9ca3af; --pulse-text-3:#6b7280; --pulse-success:#22c55e; --pulse-warning:#f59e0b; --pulse-danger:#ef4444; --pulse-verified:#3b82f6;`
  - **Then remap the Fluent surface/text tokens to Pulse** so the existing `.bz-card`, `.mv-appbar`, `.mv-nav`, `.bz-banner` etc. inherit dark with no per-rule edits: `--colorNeutralBackground2:var(--pulse-bg); --colorNeutralBackground1:var(--pulse-surface); --colorNeutralBackground3:var(--pulse-surface-2); --colorNeutralForeground1:var(--pulse-text); --colorNeutralForeground3:var(--pulse-text-2); --colorNeutralForeground4:var(--pulse-text-3); --colorNeutralStroke2:var(--pulse-border); --colorNeutralStroke3:var(--pulse-border); --colorBrandBackground:var(--pulse-primary); --colorBrandForeground1:var(--pulse-primary);`
- **Force dark default:** the brand ramp dark block only applies under `[data-theme="dark"]` or prefers-dark. Since Pulse ships dark-first, set the `:root` remap above unconditionally (not behind a media query). Keep a `[data-theme="light"]` escape hatch block for the future light variant (spec §2 says light is secondary).
- **Replace `--bz-accent`** (AppLayout.mview:82) `--bz-accent: var(--pulse-primary); --bz-accent-2: var(--pulse-primary-hover);` so banners/section titles/chips go violet instead of the rainbow per-page hues.
- **Card radius:** Fluent `--borderRadiusXLarge` is already used by `.bz-card`; override it to `16px` in the `:root` remap to hit spec §4 globally without per-card edits.
- **Drop the `material-*` theme overrides** (AppLayout.mview:121-141) in P9 (dead weight under Pulse) — non-blocking.

### 1.2 Inter font — **self-host woff2, app-layer @font-face**
- The Roboto path is framework-embedded: `Types.mo:190 robotoWoff2:Blob`, served at `App.mo:1075 /fonts/roboto.woff2`, embedded in `ClientAssets.mo`. Touching that = ClientAssets regen (avoid).
- **Pulse approach (no framework edit):** add `@font-face { font-family:'Inter'; src:url('/fonts/inter.woff2') }` in the layout `<style>` and set `--fontFamilyBase: 'Inter', -apple-system, 'Segoe UI', Roboto, sans-serif;`. **But** `/fonts/inter.woff2` must be served. Two options, pick per P1:
  - **(A — MVP, zero framework edit):** ship Inter as a **base64 data: URL** inside the `@font-face src` in the layout `<style>` (subset latin, weights 400/500/700; ~30-60KB). Compiles into `main.mo`, served as part of the page, no new route. **Recommended for MVP.**
  - **(B — clean, framework edit, defer):** add `interWoff2:Blob` to `Types.mo:183 Assets`, embed in `ClientAssets.mo`, add `App.mo` route `/fonts/inter.woff2`. This is a ClientAssets regen — defer to post-MVP. Falls back gracefully to the existing system/Roboto chain until then.

### 1.3 MainLayout approach — **rename AppLayout → MainLayout by re-skinning in place, not a rewrite**
- AppLayout already **is** the shell the spec wants: sticky `<AppBar>` top bar (AppLayout.mview:152), 248px sticky `<Nav>` rail (AppLayout.mview:29, 172), content + footer, and `<BzShell>` mobile bottom tab bar (AppLayout.mview:192). The spec §5 MainLayout = this shell, reskinned + French + 9 nav items + user card.
- **Decision: keep the file `AppLayout.mview`** (renaming forces editing `@layout AppLayout` in 14 pages and risks the build). Re-skin it in place to the Pulse MainLayout spec; treat "MainLayout" as the conceptual name in docs/comments. The 9-item nav replaces the current 7 NavItems (AppLayout.mview:174-181). Add the user card (avatar + online dot + handle) pinned at the bottom of `<Nav>`.
- **ForumLayout** stays a separate layout (it has the Discourse 2-col sidebar the Forums pages need) — reskin to Pulse, share the same token `:root` block (copy-paste the Pulse tokens into ForumLayout.mview:12).
- **WelcomeLayout** is already minimal dark — light reskin to Pulse login (spec §6 Connexion).
- **BzShell** → reskin to the 5 French mobile tabs + French Cmd-K palette (BzShell.mview:82-89, tab section). Keep the filename (referenced as `<BzShell>` in both AppLayout and ForumLayout); "PulseShell" is conceptual.
- **ChatLayout / CommunityLayout** (arch §6.2): not separate layouts for MVP — `/chat` and `/communautes/{id}` render their own 3-col / tabbed bodies inside AppLayout, like Channel.mview/Server.mview do today.

### 1.4 Constraints honored
- **Canister stays `bzzz`:** no edit to `dfx.json:4`, `canister_ids.json`, `motoview.json` `"name":"bzzz"`. Only product/brand/UI strings change. Service **file names** stay (`Servers.mo`, `Status.mo`) — Pulse "Communautés"/"Annonces" are UI relabels + thin field additions.
- **No JS logic:** all interaction stays server-driven (`@code` + framework glue attributes like `data-mv-authed`, `data-mv-signout`, `data-mv-cmdk`). New components are pure `.mview` views; no new client JS. Tabs/active-state computed in `@code` (e.g. `var tab:Text` from query param), not JS.
- **App-layer styling only:** every Pulse style lives in layout/page/component `<style>`; framework CSS untouched.

---

## 2. Route map (current → Pulse)

| Current route | File | Pulse label | Pulse route | Layout (Pulse) | Notes |
|---|---|---|---|---|---|
| `/` | Home.mview | Accueil | `/` | AppLayout | **Folds** Home dashboard + `/feed` into the social feed (spec §6). |
| `/feed` | Feed.mview | Fil (alias of Accueil) | `/feed` (keep) | AppLayout | Keep route as deep-link; feed logic moves to `/`. Optionally redirect. |
| `/feed/p/{id:Nat}` | FeedPost.mview | Message | `/feed/p/{id:Nat}` (keep) | AppLayout | Post detail; reskin only. |
| `/servers` | Servers.mview | Communautés | `/communautes` **(+ keep `/servers` alias)** | AppLayout | Relabel + CommunityCard. Add `/communautes` as a new `@page` rendering same body; keep `/servers` for live bookmarks. |
| `/servers/{id:Nat}` | Server.mview | Communauté | `/communautes/{id:Nat}` (+ keep `/servers/{id}`) | AppLayout | Add tabs Aperçu/Fil/Forum/Chat/Annonces/Docs/Membres. |
| `/channel/{rid:Nat}` | Channel.mview | Chat en direct | `/chat/{rid:Nat}` **(+ keep `/channel/{rid}`)** | AppLayout | Add `/chat` index + `/chat/{rid}`; reskin Discord-dark → Pulse-dark. |
| — (new) | — | Chat en direct (index) | `/chat` | AppLayout | New room-picker landing (lists rooms). |
| `/messages` | Messages.mview | Messages | `/messages` (keep) | AppLayout | Add tabs Tous/Non lus/Groupes + FAB + MessageCard. |
| `/messages/{id:Nat}` | Conversation.mview | Conversation | `/messages/{id:Nat}` (keep) | AppLayout | Reskin. |
| `/forum` | Forum.mview | Forums | `/forum` (keep) | ForumLayout | Catégories panel + Sujets récents (TopicCard). |
| `/forum/c/{id:Nat}` | ForumCategory.mview | Catégorie | `/forum/c/{id:Nat}` (keep) | ForumLayout | Reskin. |
| `/forum/t/{id:Nat}` | ForumTopic.mview | Sujet | `/forum/t/{id:Nat}` (keep) | ForumLayout | Card posts + Meilleure réponse + **Suivre** (new). |
| `/forum/new` | ForumNew.mview | Nouveau sujet | `/forum/new` (keep) | ForumLayout | French form. |
| `/status` | Status.mview | Annonces | `/annonces` **(+ keep `/status`)** | AppLayout | Add `/annonces` page (AnnouncementCard). Keep `/status` as the live 24h ephemeral feature, or relabel. |
| — (new) | — | Événements | `/evenements` | AppLayout | New (P8). |
| — (new) | — | Paramètres | `/parametres` | AppLayout | New (P7). |
| `/me` | Me.mview | Profil | `/profil` **(+ keep `/me` alias)** | AppLayout | Banner + tabs + stats. |
| `/u/{handle}` | Profile.mview | Profil public | `/u/{handle}` (keep) | AppLayout | Same redesign read-only. |
| `/admin` | Admin.mview | Console d'administration | `/admin` (keep) | AppLayout | French strings. |
| `/welcome` | Welcome.mview | Connexion | `/welcome` (keep) | WelcomeLayout | II login reskin. |
| `/search` | Search.mview | Recherche | `/search` (keep) | AppLayout | Wire top-bar search here. |
| `/about` | About.mview | À propos | `/about` (keep) | AppLayout | French. |
| `/greet/{name}` | Greet.mview | (demo) | delete in P9 | AppLayout | Demo page — remove. |

**New-route mechanism:** MotoView routes are `@page` directives in `.mview` files (verified by the compiler route table). Adding `/communautes`, `/chat`, `/annonces`, `/evenements`, `/parametres`, `/profil` = new `.mview` files (or a second `@page` line where the framework allows). For the **aliases** (`/communautes`→Servers body, `/profil`→Me body, etc.), the lowest-risk path is a new thin page that includes the same `@code` + body, or a `<meta http-equiv="refresh">` redirect page (no-JS, same pattern as the auth gate at AppLayout.mview:147). Verify multi-`@page` support during P1; if unsupported, use redirect pages.

---

## 3. Phased plan

Each phase is independently shippable: it builds clean (`motoview build`), typechecks (`moc --check`), and deploys to the `bzzz` canister. **Standard test gate per phase (run all):**
1. `compiler/target/release/motoview build apps/bzzz --name bzzz` → exit 0.
2. `moc --check --package base $BASE --package motoview runtime/src apps/bzzz/.mvbuild/main.mo` → 0 errors (warnings OK).
3. **Render at scale:** `dfx deploy` to local replica; hit every touched route; for list pages, seed N≈200 items via the service handlers and confirm pagination + render time hold.
4. **Playwright authed screenshot vs mockup:** sign in via II, screenshot the touched route at desktop (≥1024px) and mobile (375px), diff against the spec mockup (`webapp-testing` / `agent-browser` skill).
5. **Visual a11y:** contrast check `--pulse-text` (#f3f4f6) on `--pulse-bg` (#0a0a0f) and primary on dark (WCAG AA).

`MOC=~/.cache/dfinity/versions/0.28.0/moc`, `BASE=~/.cache/dfinity/versions/0.28.0/base`.

---

### Phase 1 — Foundation (task #12)
**Goal:** Pulse dark theme tokens + Inter + reskinned MainLayout/ForumLayout/WelcomeLayout/BzShell shell + logo + full Bzzz→Pulse + EN→FR rebrand of chrome. Everything inherits this.

**Files to edit:**
- `src/Layouts/AppLayout.mview`
  - `:1` `@theme brand="#0f6cbd"` → `@theme brand="#8b5cf6"`.
  - `:8` `apple-mobile-web-app-title content="Bzzz"` → `"Pulse"`.
  - After `:14` (the `body{}` rule) insert the Pulse `:root` token block + Fluent→Pulse remap (§1.1) + `@font-face Inter` data-URL (§1.2-A) + `--fontFamilyBase` override + `--borderRadiusXLarge:16px`.
  - `:82` `--bz-accent`/`--bz-accent-2` → Pulse primary/hover.
  - `:153` brand `🐝 Bzzz` → Pulse logo chip (inline ECG-waveform SVG, gradient `#fb3b5c→#a855f7`) + `PULSE` wordmark.
  - `:160-167` account menu → FR (`Accueil / Mon profil / Mon statut / Messages / Console d'administration / À propos de Pulse / Déconnexion`).
  - `:173-181` Nav → **9 FR items** (Accueil `/`, Communautés `/communautes`, Forums `/forum`, Chat en direct `/chat`, Messages `/messages`, Annonces `/annonces`, Événements `/evenements`, Profil `/profil`, Paramètres `/parametres`) with the spec icon set; active = `--pulse-primary-soft` bg.
  - Add **user card** at bottom of `<Nav>` (avatar via `Identity.avatarOf(ctx.caller)` + online dot + `Identity.atHandleOf`).
  - `:188` footer `🐝 Bzzz` → Pulse mark + wordmark.
- `src/Layouts/ForumLayout.mview` — `:1` brand `#8b5cf6`; `:8` title `Pulse`; copy Pulse `:root` token block into `:12`; `:142` logo chip; `:154,:244,:273` Bzzz→Pulse; app-switcher labels → FR.
- `src/Layouts/WelcomeLayout.mview` — title→`Pulse`, gradient→Pulse brand, logo mark.
- `src/Components/BzShell.mview` — `:21` active color already `--colorBrandForeground1` (auto-violet via remap, no edit needed); `:82` placeholder→`"Rechercher dans Pulse…"`; `:85-89` palette label `Aller à` + FR rows; consolidate bottom tabs to **5 FR** (Accueil/Communautés/Forums/Messages/Profil).
- `src/Pages/Welcome.mview` — `:3-4` title/desc FR; `:9` logo; `:14` `Se connecter avec Internet Identity`; `:16` FR note; `:30-32,:42-48` FR onboarding copy + tagline "Sécurisé, privé et décentralisé."
- **All 19 pages** `@title`/`@description` → Pulse + FR (one-line edits; safe, see §5).

**Pulse design applied:** dark default everywhere; Inter; 16px cards; violet primary; 9-item sidebar + user card; 5 mobile tabs; ECG logo; French chrome.

**Done-criteria:** every route renders dark + French chrome; logo present; sidebar shows 9 items, mobile shows 5; build + typecheck green; screenshots of `/`, `/forum`, `/welcome` match the shell mockup. **No page-body redesign yet** — bodies still old but now dark/French.

---

### Phase 2 — Accueil (task #13)
**Goal:** the social feed at `/` (folds Home + Feed).
**Files:** rewrite `src/Pages/Home.mview` body to: `PostComposer` ("Quoi de neuf dans votre communauté ?" + Image/Sondage/Fichier/Mention + **Publier**) → `Tabs` (Pour vous/Abonnements/Tendances/Annonces) → `PostCard` stream → right rail **Tendances** + **Membres actifs**. Reuse `Feed.*` service calls already in `Feed.mview`'s `@code`. `Feed.mview` keeps its route but its composer/timeline become the shared components.
**New components:** `src/Components/PostComposer.mview`, `src/Components/PostCard.mview`, `src/Components/Tabs.mview`, `src/Components/Avatar.mview`.
**Pulse design:** PostCard anatomy per arch §33 (Auteur·rôle·temps | contenu | média | Répondre·Reposter·Citer·Aimer·Sauvegarder·Plus); dark surfaces; pill tabs; rail cards 16px.
**Test:** standard gate; seed ~200 posts via `Feed` handlers, confirm timeline pagination + render; screenshot `/` desktop+mobile vs Accueil mockup.
**Done:** `/` is the live feed (real `Feed` data), composer posts persist, tabs switch via `@code` query state, mobile shows filter tabs.

---

### Phase 3 — Forums reskin (task #14)
**Goal:** Catégories panel + Sujets récents + Sujet + Nouveau sujet, Pulse-dark.
**Files:** `src/Pages/Forum.mview` (Catégories left panel from `Forum.categories()`:192 + Sujets récents using new TopicCard from `Forum.topicsLatest()`:276; `+ Nouveau sujet` button); `src/Pages/ForumCategory.mview` (reskin, `Forum.topics(id)`:269); `src/Pages/ForumTopic.mview` (card posts, **Meilleure réponse** chip via `Forum.acceptedPostId`:590/`isSolved`:594, FR composer "Répondre"); `src/Pages/ForumNew.mview` (FR form). `ForumLayout.mview` already reskinned in P1.
**New component:** `src/Components/TopicCard.mview` (title + category badge via `Forum.categoryColor`:206 + tag chips via `Forum.tagsText`:628 + replies `Forum.replyCount`:490 + `Forum.relativeTime`:601 + author avatar).
**Gap flagged:** **Suivre** has no backend — `grep watch|follow|suivre src/Services/Forum.mo` = 0 hits. Either add minimal `watchTopic/isWatching` to Forum.mo (P3) **or** defer Suivre to Beta and ship the button disabled. Recommend deferring per arch §31 (Beta).
**Test:** seed ~200 topics; confirm category panel + topic list pagination (`topicsPage`:314); screenshots vs Forums + Sujet mockups.
**Done:** Forums dark/French/card-based; best-answer chip renders for solved topics.

---

### Phase 4 — Chat en direct reskin (task #15)
**Goal:** `/chat` + `/chat/{rid}`; Salons (+ unread), Salons vocaux (stub), ChatMessage, composer.
**Files:** add `/chat` index + reskin `src/Pages/Channel.mview` (rename concept to Chat; add `/chat/{rid}` route, keep `/channel/{rid}`). Convert `--dc-*` Discord palette → `--pulse-*`. Composer placeholder → "Envoyer un message…". Left rail: search + **Salons** (text rooms via `Chat`) + **Salons vocaux** (stub list, e.g. "Développeurs 03/10") + connected-voice footer (stub).
**New component:** `src/Components/ChatMessage.mview` (reskin of existing `.dc-msg`; avatar/name/time/text + reaction chips — reactions already exist at Channel.mview `.dc-reactions`).
**Gaps flagged:** voice channels + file-attachment card have **no backend** (Chat.mo has no voice/attachment fields). Ship Salons vocaux + attachment card as **visual stubs** (clearly non-functional, no fake data) per arch §25.2 (voice is out-of-MVP); wire later.
**Test:** seed a room with ~300 messages; confirm grouping + scroll perf; screenshot vs Chat mockup.
**Done:** `/chat` lists rooms; `/chat/{rid}` dark Pulse chat works (send/react persist via `Chat`); voice section visibly stubbed.

---

### Phase 5 — Messages reskin (task #16)
**Goal:** tabs Tous/Non lus/Groupes + MessageCard list + FAB compose + Conversation reskin.
**Files:** `src/Pages/Messages.mview` (replace the New chat/New group dialogs with a FAB + tabs; conversation list → MessageCard via `Messenger` list calls); `src/Pages/Conversation.mview` (reskin to Pulse dark, keep E2E chip, FR encryption note).
**New component:** `src/Components/MessageCard.mview` (avatar/name/last-msg/time/unread badge per arch §33).
**Keep:** E2EE envelope behavior unchanged (Messenger.mo + Keys.mo untouched).
**Test:** seed ~150 conversations; confirm inbox pagination; screenshot vs Messages mockup desktop+mobile.
**Done:** Messages dark/French with tabs + FAB; conversation thread reskinned; E2E intact.

---

### Phase 6 — Communautés (task #17)
**Goal:** browse (`/communautes`) + community page tabs (`/communautes/{id}`).
**Files:** new `/communautes` (CommunityCard list from `Servers.servers()`:97 + "Créer une communauté" using `Servers.createServer`:77) keeping `/servers` alias; new `/communautes/{id}` with tabs Aperçu/Fil/Forum/Chat/Annonces/Docs/Membres (reuse `Server.mview` channels/members bodies). Reskin `src/Pages/Servers.mview` + `src/Pages/Server.mview`.
**New component:** `src/Components/CommunityCard.mview` (avatar/name/short desc/"N membres"/Rejoindre|Ouvrir).
**Gap flagged:** `Servers.Server` type (Servers.mo:36) lacks description/avatar/banner/visibility. Add **optional fields** (default empty) to the `Server` record + a setter — additive, non-breaking. Member-count from existing role membership.
**Test:** seed ~100 communities; confirm grid; screenshots vs Communautés (desktop+mobile) mockups.
**Done:** `/communautes` lists real servers as CommunityCards; community page tabs render; create persists.

---

### Phase 7 — Profil reskin + Paramètres (task #18)
**Goal:** gradient banner + circular avatar + stats + badges + tabs; new `/parametres`.
**Files:** `src/Pages/Me.mview` → `/profil` (keep `/me` alias): gradient banner (`#7c3aed→#3b82f6`, in page `<style>` not framework), circular avatar, "Michel ✓ @handle" via `Identity`, stats row, tabs Publications/Sujets/Réponses/Badges/Communautés/Sauvegardes; inline-edit bio with save (existing `Identity` bind). `src/Pages/Profile.mview` (`/u/{handle}`) → same redesign read-only. New `src/Pages/Parametres.mview` (`/parametres`): profil/notifications/confidentialité/communautés.
**New components:** `src/Components/UserBadge.mview`, `src/Components/Chip.mview`.
**Gaps flagged:** reputation/status/preferences fields not in Identity.mo. For MVP: render stats from existing counts (`Feed`/`Forum` post/topic counts); Badges + preferences = **scaffold** (real "Premier post" style badges deferred to Beta per arch §16/§31). Paramètres notification/privacy toggles can persist via a minimal `Preferences` add to Identity.mo (`mvStableSave` already exists for the stable pattern — see Status.mo:58) **or** scaffold-only for MVP.
**Test:** screenshots vs Profil mockup (banner gradient, verified check, tabs) desktop+mobile.
**Done:** `/profil` + `/u/{handle}` redesigned; `/parametres` reachable with at least profil edit working.

---

### Phase 8 — Annonces + Événements (task #19)
**Goal:** new `/annonces` + `/evenements`.
**Files:** new `src/Pages/Annonces.mview` (`/annonces`, keep `/status` alias) — AnnouncementCard list. New `src/Pages/Evenements.mview` (`/evenements`).
**New component:** `src/Components/AnnouncementCard.mview` (badge "Officiel" + titre + résumé + date + Lire/Suivre/Commenter per arch §33).
**Backend decision:** Status.mo already has a persistent-with-TTL model + `mvStableSave`/`mvStableLoad` (Status.mo:58-65). **Reuse Status data** for Annonces (arch §67) by making expiry optional, **or** add a thin `Announcement.mo` service (id/communityId/authorId/title/content/status/publishedAt/commentsTopicId per arch §18.11). For MVP: reuse Status (relabel) for Annonces; **Événements = scaffold** (event card with date/titre/lieu/RSVP, no service) per arch §31 (Could-have/Phase 5). No fake data — empty states say "Aucune annonce" / "Aucun événement".
**Test:** seed announcements via Status handlers; screenshot `/annonces`; `/evenements` shows honest empty/stub.
**Done:** `/annonces` lists real announcement data as cards; `/evenements` scaffolded.

---

### Phase 9 — Polish + verify + deploy (task #20)
**Goal:** mobile+desktop polish vs all mockups, a11y, perf at scale, mainnet deploy.
**Work:** remove `Greet.mview`; drop dead `material-*` overrides (AppLayout.mview:121-141); audit all `--bz-*`→`--pulse-*`; verify 5-tab labels don't overflow ≤375px; touch targets ≥44px; tab-bar/rail handoff at the 839px breakpoint (AppLayout.mview:115); contrast pass; full Playwright sweep of all 19+ routes desktop+mobile.
**Test:** full gate on every route; final authed screenshot diff; confirm canister id unchanged.
**Done:** all routes match mockups; deploy to `bzzz` mainnet (`fi4qb-naaaa-aaaab-qhaxq-cai`).

---

## 4. Gap / new-work list (net-new, with minimal MVP build)

**New pages**
- `/communautes`, `/communautes/{id}` (P6) — CommunityCard list + tabbed community page. MVP: reuse `Servers` service, additive optional fields.
- `/chat`, `/chat/{rid}` (P4) — room index + reskinned channel. MVP: alias of Channel logic.
- `/annonces` (P8) — AnnouncementCard list. MVP: reuse `Status` data.
- `/evenements` (P8) — event list. MVP: **scaffold**, honest empty state.
- `/parametres` (P7) — settings. MVP: profil edit live, notifications/privacy scaffold.
- `/profil` (P7) — alias/rename of Me with redesign.

**New components** (`src/Components/`): `PostComposer`, `PostCard`, `TopicCard`, `ChatMessage`, `MessageCard`, `CommunityCard`, `AnnouncementCard`, `Tabs` (pill/underline), `Chip`, `Avatar` (online dot + verified), `UserBadge`. MVP build = pure view components, data passed from page `@code`, no client JS, dark/16px/Inter.

**New service work (minimal, additive)**
- Forum.mo: **Suivre** (`watchTopic/isWatching`) — *defer to Beta* (button disabled in MVP).
- Servers.mo: optional `description/avatar/banner/visibility` on `Server` + setter (P6, additive).
- Status.mo→Annonces: optional expiry / status field (P8) — reuse, don't fork, for MVP.
- *Deferred (Beta/Prod, arch §31):* `Notification.mo`, `Preferences.mo`, `ModerationReport.mo`, reputation/badges, voice, attachments, post→topic / chat→topic conversion. **Not in MVP.**

**Assets**
- **Logo:** inline ECG-waveform SVG (gradient `#fb3b5c→#e23b7a→#a855f7`) in layouts (P1). Favicon already a violet waveform SVG in `ClientAssets.mo:19` — acceptable for MVP; recolor to gradient = ClientAssets regen, defer.
- **Inter font:** base64 data-URL `@font-face` in layout `<style>` (P1, option A); framework-embedded route deferred.

---

## 5. Rebrand checklist

**Safe-to-relabel (UI strings only — high volume, ~80+ strings):**
- **Brand:** `🐝 Bzzz`/`Bzzz` → Pulse logo + `PULSE`/`Pulse` in AppLayout.mview:153,188; ForumLayout.mview:142,244,273; Welcome.mview:9,29,41; BzShell; all `@title`/`@description` (19 pages); `apple-mobile-web-app-title` (AppLayout:8, ForumLayout:8).
- **Nav EN→FR:** Home→Accueil, Feed→Fil, Servers→Communautés, Forum→Forums, Messages→Messages, Status→Annonces, Me→Profil (AppLayout:174-181, BzShell tabs+palette:85-89).
- **Actions EN→FR:** Post→Publier, Reply→Répondre, Follow→Suivre, Send→Envoyer, Search→Rechercher, Sign out→Déconnexion, Accept answer→Marquer comme solution, Read/Comment→Lire/Commenter, Create Topic→Nouveau sujet, etc. (across page bodies, done per-phase).
- **Account menu / footer / about / welcome copy** → FR (P1).
- **Composer placeholders / empty states / breadcrumbs** → FR (per-phase).

**Must-keep (do NOT change):**
- Canister name `bzzz` in `dfx.json:4`, `canister_ids.json`, `motoview.json` `"name"`.
- Mainnet id `fi4qb-naaaa-aaaab-qhaxq-cai`.
- Service **file names** (`Servers.mo`, `Status.mo`, …) and their Motoko type/function names (internal, not user-facing) — relabel only in UI.
- Internal CSS class prefixes `bz-*` / `fl-*` (infrastructure; only retheme via tokens).
- Existing `@page` routes (keep as aliases; add FR routes alongside) to preserve live deep links.
- The auth-gate meta-redirect pattern (AppLayout.mview:146-148) and all `data-mv-*` glue attributes (no-JS-logic contract).
- E2EE envelope behavior (Messenger.mo, Keys.mo).

---

## 6. Risks

- **ClientAssets gotcha (primary):** the favicon (`ClientAssets.mo:19`) and Roboto woff2 (`Types.mo:190`/`App.mo:1075`) live in the **framework** layer. Recoloring the favicon to the Pulse gradient or self-hosting Inter via a `/fonts/inter.woff2` route both require **ClientAssets.mo regen** — explicitly out-of-scope per constraint 3. **Mitigation:** keep the existing violet waveform favicon for MVP; ship Inter as a base64 data-URL in the layout `<style>` (compiles into `main.mo`, no framework edit).
- **`@theme` re-ramp blast radius:** changing brand to `#8b5cf6` re-ramps **every** Fluent `--colorBrand*` app-wide and the Fluent→Pulse `:root` remap recolors every `.bz-*`/`.mv-*` surface in all 19 pages at once. **Mitigation:** P1 is a global reskin by design; verify status-token usage (success/danger) survives — spec provides explicit `--pulse-success/-warning/-danger/-verified` to map.
- **Renaming AppLayout = 14 `@layout` edits + build break risk.** **Mitigation:** keep the filename, reskin in place ("MainLayout" is conceptual).
- **New-route mechanism unverified:** whether one `.mview` can declare multiple `@page` routes is unconfirmed. **Mitigation:** P1 spike; fall back to thin redirect pages (meta-refresh, same as the auth gate) for `/communautes`, `/profil`, `/chat`, `/annonces` aliases.
- **Backend gaps presented honestly (no fakes — per global rules):** Suivre (no Forum follow API), voice channels + file attachments (no Chat fields), badges/reputation (no Identity fields), Événements (no service). **Mitigation:** ship as clearly-labeled visual scaffolds with honest empty states, defer real backend to Beta per arch §31. Do **not** seed fake data.
- **Scale:** list pages (feed, forum topics, communities, inbox, messages) must hold at N≈100-300. Existing services have paginated readers (`Forum.topicsPage`:314, `Forum.postsPage`:426); reuse them — don't render unbounded lists in cards.
- **MotoView SSR limits:** no client state — tabs/active-states must be `@code`/query-param driven (no JS); each page load re-fetches. Keep `onLoad` reads cheap.
- **Mobile 5-tab vs 9-nav mismatch:** sidebar has 9 items, mobile bar fits 5 (Chat/Annonces/Événements/Paramètres reachable via Communautés/Profil/menu) per spec §5/§23. **Mitigation:** follow the spec's exact 5 and route the other 4 through the account menu + community tabs.
- **French consistency:** ~80 strings, FR pluralization/tone (arch §4.4). **Mitigation:** single reviewer pass in P9; keep service/role identifiers English internally.
- **Live data:** the `bzzz` canister holds real state. **Mitigation:** all service changes additive (optional fields, defaults); never rename/remove existing types or the canister.