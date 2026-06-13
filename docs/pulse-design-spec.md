# Pulse — Design System & Build Spec

Visual source of truth, read from the reference mockup (desktop Accueil + Forums + Chat, mobile Accueil/Communautés/Messages/Profil, II login). Pair with `architecture_pulse_forum_motoview.md` (the functional spec, in French). UI language is **French**. Default theme is the **dark "Pulse"** look below.

> Canister identity stays `bzzz` (mainnet `fi4qb-naaaa-aaaab-qhaxq-cai`) to preserve live data — only the product/brand becomes Pulse.

## 1. Brand
- **Wordmark:** `PULSE` (uppercase, bold, slight letter-spacing, white).
- **Logo mark:** an ECG/heartbeat **pulse waveform** (a jagged spike line) in white, on a rounded-square chip filled with a vibrant **red→pink→violet** gradient (`#fb3b5c → #e23b7a → #a855f7`). Used top of sidebar (mark + wordmark) and as favicon/app icon.
- **Tagline (login):** "Connexion avec Internet Identity — Sécurisé, privé et décentralisé."
- Handles look like `@michel.icp`; names can carry a blue ✓ verified check.

## 2. Color tokens (dark — default)
| Token | Value | Use |
|---|---|---|
| `--pulse-bg` | `#0a0a0f` | app background (deepest) |
| `--pulse-bg-sidebar` | `#0d0d15` | left sidebar |
| `--pulse-surface` | `#15151e` | cards / panels |
| `--pulse-surface-2` | `#1c1c27` | elevated/hover, inputs, composer |
| `--pulse-border` | `#232331` | hairline borders (`rgba(255,255,255,.06)`) |
| `--pulse-primary` | `#8b5cf6` | primary buttons, active nav, links |
| `--pulse-primary-hover` | `#7c3aed` | hover |
| `--pulse-primary-soft` | `rgba(139,92,246,.14)` | active-nav bg, focus rings |
| `--pulse-text` | `#f3f4f6` | primary text |
| `--pulse-text-2` | `#9ca3af` | secondary text |
| `--pulse-text-3` | `#6b7280` | muted/meta |
| `--pulse-success` | `#22c55e` | online dot, success |
| `--pulse-warning` | `#f59e0b` | warning tags |
| `--pulse-danger` | `#ef4444` | danger/urgent |
| `--pulse-verified` | `#3b82f6` | verified ✓ |
| gradient (logo) | `#fb3b5c → #a855f7` | brand mark |
| gradient (profile banner) | `#7c3aed → #3b82f6` | profile header |

Light theme is secondary (doc §4.3 "interface claire par défaut" but the mockup is dark — ship dark first, keep a light variant later). Category/tag chip hues: blue `#3b82f6`, gray `#6b7280`, violet `#8b5cf6`, orange `#f59e0b`, red `#ef4444`, green `#22c55e`.

## 3. Typography
- Family: **Inter** (self-host woff2; fallback `-apple-system, Segoe UI, Roboto, sans-serif`).
- Page title (Accueil/Forums/Messages): 24–28px / 700.
- Section heading (Tendances, Catégories): 16–18px / 700.
- Body: 14–15px / 400–500. Meta: 12–13px / `--pulse-text-3`.

## 4. Shape & elevation
- Card radius **16px**; buttons **10px**; inputs **10–12px**; chips/pills **999px**; avatars **circular**.
- Shadows soft and low; rely on surface contrast + 1px borders more than shadow. Subtle gradients only (banner, logo, hover).

## 5. App shell (`MainLayout`)
**Desktop (≥1024px): fixed left sidebar (~248px) + top bar + content.**
- **Sidebar** (`--pulse-bg-sidebar`): logo (mark+wordmark) at top; nav list below; user card pinned at bottom.
  - Nav items (icon + label, ~44px rows, rounded; active = `--pulse-primary-soft` bg + `--pulse-primary` icon/text):
    `Accueil` (home) · `Communautés` (people) · `Forums` (chat-bubbles) · `Chat en direct` (message-square) · `Messages` (envelope) · `Annonces` (megaphone) · `Événements` (calendar) · `Profil` (user) · `Paramètres` (gear).
  - User card (bottom): rounded surface, avatar + online dot, `Michel` + `@michel.icp`.
- **Top bar** (sticky, transparent over `--pulse-bg`): page title (left, bold) · centered search input (`Rechercher…`, rounded, surface bg) · right cluster: notification bell (badge dot), a couple of round icon buttons, user avatar (online dot).
- **Content:** page-specific; max comfortable width, generous padding.

**Mobile (<768px): bottom tab bar + compact header.**
- Bottom tabs (5, icon+label, active=primary): `Accueil · Communautés · Forums · Messages · Profil`. Chat en direct reachable from Communautés / a FAB.
- Header: title + search + bell. Horizontal filter tabs where needed (Pour vous / Abonnements / Tendances).

## 6. Pages (route → spec)
Keep canister routes; relabel/redesign. New routes added where noted.
- **Accueil `/`** — the social feed. Composer card (avatar + "Quoi de neuf dans votre communauté ?" + row: Image · Sondage · Fichier · Mention + **Publier** purple) → tabs (Pour vous/Abonnements/Tendances/Annonces) → feed of **PostCard**. Right rail: **Tendances** (hashtags + counts + "Voir plus") and **Membres actifs** (avatars + En ligne). (Folds today's `/` dashboard + `/feed`.)
- **Communautés `/communautes`** (was `/servers`) — "Mes communautés" + `+`; list of **CommunityCard** (avatar, name, "12.4k membres", chevron). Community page `/communautes/{id}` with tabs: Aperçu/Fil/Forum/Chat/Annonces/Docs/Membres.
- **Forums `/forum`** — title + "Rechercher dans les forums…" + **+ Nouveau sujet**. Left **Catégories** panel (colored icon + name + desc + count; "Toutes les catégories"). Right **Sujets récents** (TopicCard: icon + title + tag chips + replies + time + author avatar).
- **Sujet `/forum/t/{id}`** — title, category, tags, status, OP, replies, meilleure réponse, suivre.
- **Chat en direct `/chat`** (was `/servers/{id}` channels) — left rail: search + **Salons** (`# général`…unread badges) + **Salons vocaux** (Général, Développeurs 03/10) + connected-voice footer. Main: channel header + **ChatMessage** list (avatar/name/time/text + reaction chips + file-attachment card) + composer "Envoyer un message…".
- **Messages `/messages`** — search + tabs **Tous/Non lus/Groupes** + conversation list (**MessageCard**: avatar, name, last msg, time, unread badge) + FAB compose. Conversation `/messages/{id}`.
- **Annonces `/annonces`** — NEW. **AnnouncementCard** (badge officiel, titre, résumé, date, actions Lire/Suivre/Commenter). (Reuse Status/announcement data where possible.)
- **Événements `/evenements`** — NEW. event list/cards (date, titre, lieu, RSVP).
- **Profil `/profil` + `/u/{handle}`** — gradient banner (violet→blue) + back/settings; circular avatar; `Michel ✓ @michel.icp`; role line; 📍 lieu · 🔗 lien; stats (Publications/Abonnés/Abonnements); **Badges** row (+N); À propos; tabs Publications/Sujets/Réponses/Badges/Communautés/Sauvegardes.
- **Paramètres `/parametres`** — NEW. profile, notifications, confidentialité, communautés.
- **Connexion `/welcome`** — II login card: "Connexion avec Internet Identity" + "Sécurisé, privé et décentralisé" + **Se connecter** (dark, ∞ logo) + "Qu'est-ce qu'Internet Identity ?".

## 7. Components (new/reskinned)
`PostCard` · `PostComposer` · `TopicCard` · `ChatMessage` · `MessageCard` · `CommunityCard` · `AnnouncementCard` · `UserBadge` · `NotificationBell` · `CommunitySwitcher` · `InternetIdentityButton` · `Tabs` (pill/underline) · `Chip` (tag) · `Avatar` (with online dot + verified).

## 8. Build order (phases)
1. **Foundation:** Pulse dark theme tokens + Inter + the `MainLayout` shell (sidebar + top bar + mobile tabs) + logo. Rebrand Bzzz→Pulse + French strings. *(Identity-defining; everything inherits it.)*
2. **Accueil:** feed page (composer + tabs + PostCard + Tendances + Membres actifs rails).
3. **Forums** reskin (Catégories + Sujets récents + Sujet + Nouveau sujet).
4. **Chat en direct** reskin (salons text+voice + ChatMessage + composer).
5. **Messages** reskin (tabs + MessageCard + FAB + Conversation).
6. **Communautés** (browse + community page tabs).
7. **Profil** reskin (banner + stats + badges) + **Paramètres**.
8. **Annonces** + **Événements** (new sections).
9. Polish pass vs the mockup (mobile + desktop), a11y, perf at scale, verify, deploy.
