# Bzzz on MotoView — Build Spec (authoritative)

This is the contract for building the **Bzzz** social super-app natively on
**MotoView** (Motoko + `.mview`, server-driven UI on the Internet Computer).
Every generated file MUST follow this exactly so the app compiles. The pipeline
that verifies your work is:

```
compiler/target/release/motoview build apps/bzzz --name bzzz
moc --check --package base $BASE --package motoview <repo>/runtime/src apps/bzzz/src/main.mo
```

Both must succeed with **no errors** (warnings about unused identifiers are OK).

---

## PART 1 — MotoView authoring contract (VERIFIED — obey precisely)

### Project layout
```
apps/bzzz/
  dfx.json, motoview.json
  src/Pages/*.mview       routable pages (one @page route each)
  src/Layouts/*.mview      shared shells (@yield)
  src/Components/*.mview    optional reusable components (param X : T)
  src/Services/*.mo         plain Motoko — business logic + STATE
  src/main.mo               GENERATED — never edit
```

### Services = stateful Motoko `public class` (THIS is how state is shared)
A Motoko `module` CANNOT hold mutable state. So each service file
`src/Services/Name.mo` exports **`public class Name()`** (zero-arg constructor,
class name === file name). The compiler instantiates ONE shared instance at
actor scope; every page calls `Name.method(...)` against the SAME instance →
shared, cross-page, canister-lifetime state. Pattern:

```motoko
import HashMap "mo:base/HashMap";
import Principal "mo:base/Principal";
import Text "mo:base/Text";
import Time "mo:base/Time";
import Iter "mo:base/Iter";
import Array "mo:base/Array";
module {
  public type Item = { id : Nat; owner : Principal; body : Text; at : Int };
  public class Name() {
    var nextId : Nat = 1;
    let items = HashMap.HashMap<Nat, Item>(64, Nat.equal, Hash.hash); // see note on Nat hash
    public func add(owner : Principal, body : Text) : Nat {
      let id = nextId; nextId += 1;
      items.put(id, { id; owner; body; at = Time.now() }); id
    };
    public func list() : [Item] { Iter.toArray(items.vals()) };
  };
}
```
- **Services are self-contained**: define your own `public type`s **inside the
  `public class`** (NOT at module level) so the shared instance exposes them —
  pages then reference them as `Name.Item` through the instance. Do NOT split
  types into `src/Models`.
- **Cross-service coordination happens in PAGES**, not service→service calls
  (a service instance cannot see another service instance). Pages can call
  `Chat.x(...)` and `Identity.y(...)` together.
- For `HashMap<Nat,_>` use `import Nat "mo:base/Nat"` + `Nat.equal` and a hash:
  `import Hash "mo:base/Hash"; Hash.hash` (takes Nat). For `HashMap<Text,_>` use
  `Text.equal, Text.hash`. For `HashMap<Principal,_>` use `Principal.equal, Principal.hash`.
- Every service MUST `moc --check` standalone (it only imports `mo:base/*`).

### Pages (`.mview`)
A page is template markup + a `@code { }` Motoko block. Directives:
- `@page "/path"` / `@page "/x/{id:Nat}"` / `@page "/u/{handle}"` (typed/untyped route params, in scope by name inside @code).
- `@layout AppLayout`  `@title EXPR`  `@description EXPR`
- `@code { ... }` — state vars + handlers + `onLoad`.

**Inside `@code` (CRITICAL RULES):**
- Use **`func name(...)`** — NEVER `public func` (breaks the parser).
- State: `var x : T = ...;` (page-local, persists for canister life). Initialize to empty; load real data in `onLoad`.
- **`func onLoad(ctx : Context) { ... }`** runs before every render. Use it to
  read shared services into page vars: `items := Chat.list(roomId);`,
  `me := Identity.handleOf(ctx.caller);`. `Context` is provided (alias for the request ctx).
- Handlers: **`func h(ctx : Context) : async () { ... }`** — if the FIRST param is
  `ctx : Context`, the live request context is injected (read `ctx.caller`,
  `ctx.form`, route params). Extra params after ctx bind to event args.
  Handler without caller needs: `func h(id : Nat) : async () {...}` (arg from `@click="h(x.id)"`).
- `ctx.caller : Principal`, `ctx.isAuthenticated : Bool`, `ctx.query("k") : Text` (if available), route params in scope by name.
- The flow on an event: handler runs (mutates a service) → `onLoad` re-runs → render. So handlers usually just call the service; `onLoad` refreshes page vars.
- Effects available in handlers: `toast("msg")`, `animate("#sel","pulse")`, `focusOn("#sel")`, `scrollTo("#sel")`.
- **Page @code can only use base modules the actor imports:** `Nat, Nat32, Nat64, Int, Float, Char, Text, Buffer, Principal, Array, Iter, Option, Time, Bool`, plus every Service by name. Do NOT `import` inside @code. Keep page logic THIN — push anything else into a service.

**Template directives:**
- Output: `@var`, `@obj.field`, `@(expr)` (use parens for calls/operators), `@helperFunc(arg)`. HTML-escaped automatically.
- `@if cond { } else if cond { } else { }`  (verified working).
- `@for x in arrayExpr { }` — give an ARRAY (compiler appends `.vals()`); do NOT write `.vals()` yourself.
- `@switch expr { case #Variant { } case #Variant b { } }` — cover all cases.
- Events: `@click="h"`, `@click="h(item.id)"`, `<form @submit="h" secure>`.
- Forms: inputs `<InputText name="x" label="X" bind="@x" required />` (also `InputEmail/InputNumber/TextArea`), `<ValidationSummary />`, `<Button kind="primary" type="submit">`. Bound `@x` page vars are auto-populated from the submitted form before the handler runs.
- Validation in a handler: `validate { x required "msg"; x minLength 2 "msg"; n min 1 "msg"; }`.
- Built-in components compile to HTML: `Button, Card, Alert, Badge, PageHeader, Table, Grid, InputText, InputEmail, InputNumber, TextArea, ValidationSummary`. You can also just write raw HTML with `class="..."`.
- **CSS/JS:** `<style>` and `<script>` are raw text — write normal CSS incl. `@media`, `@keyframes`. Bare `@` is literal there.
- Attribute interpolation works: `id="row-@item.id"`, `style="--c:@color"`.

### Reserved words / gotchas
- Do NOT use `query` or `label` as identifiers. Unary `^` is bitwise-not.
- `Time.now() : Int` (nanoseconds). Store timestamps as `Int`.
- Render display of record fields on loop vars uses a `debug_show` fallback — fine for Text/Nat. For best output, expose Text-returning helpers from services.

---

## PART 2 — Architecture & route map

Single canister `bzzz`. Shell layout `AppLayout` (top nav: Servers, Forum, Feed,
Messages, Status, Me) already exists. Forum gets its own Discourse-styled look
(can reuse AppLayout + page CSS, or a ForumLayout).

**Services (self-contained stateful classes):**
| Service | Responsibility |
|---|---|
| `Identity` | principal → handle/display/avatar/bio (EXISTS) |
| `Chat` | rooms/channels, messages (post/edit/delete/react), threads, typing/presence |
| `Servers` | servers/guilds, their channel lists, server kind (Discussion/Forum/Feed), membership, roles (Owner/Admin/Mod), moderation (pin/lock/mute) |
| `Forum` | Discourse-style: categories (name+color), topics (title, category, tags, pinned/closed, accepted-answer), posts/replies, likes |
| `Feed` | X-style: posts, user-follows, reposts/likes, home timeline |
| `Messenger` | WhatsApp: 1:1 + group conversations, messages (ciphertext envelopes), read receipts, typing |
| `Status` | 24h ephemeral statuses per user |
| `Keys` | E2EE public key directory: per-device IK/SPK/SPKsig + OTPK pool; fetch bundle |
| `Admin` | admin allowlist, whoami |

**Pages / routes:**
| Page | Route | Shows |
|---|---|---|
| Home (EXISTS) | `/` | hub/landing + handle claim |
| Me | `/me` | own profile: set handle/display/bio; show counts |
| Servers | `/servers` | list + create servers (guilds) |
| Server | `/servers/{id:Nat}` | a server's channels + create channel + member rail + role/mod controls |
| Channel | `/channel/{rid:Nat}` | chat room: message list, composer, reactions, edit/delete, threads, typing |
| Forum | `/forum` | Discourse home: category cards + latest topics table |
| ForumCategory | `/forum/c/{id:Nat}` | topics in a category + New Topic |
| ForumTopic | `/forum/t/{id:Nat}` | OP + replies, like, reply composer, accept-answer, pin/close |
| ForumNew | `/forum/new` | new-topic composer (title, category, tags, body) |
| Feed | `/feed` | home timeline + compose + follow/repost/like |
| Profile | `/u/{handle}` | a user's posts + follow button |
| Messages | `/messages` | conversation list + start 1:1/group |
| Thread | `/messages/{id:Nat}` | a conversation: messages + composer (E2EE envelope) |
| Status | `/status` | post a status + view recent statuses |
| Admin | `/admin` | whoami, admin list (gated) |

Keep `@title`/`@description` SEO-friendly on every page.

---

## PART 3 — bzzz data model (per domain, condensed; adapt to Motoko)

### Chat / Servers
- **Room/Channel**: `id:Nat, name:Text (lowercase a-z0-9-_, ≤20), createdAt:Int, serverId:Nat (0 = default)`.
- **Message**: `id:Nat, roomId:Nat, at:Int, author:Principal, authorHandle:Text, text:Text (≤280), replyTo:Nat (0=none), edited:Int (0=no), deleted:Int (0=live)`. FIFO cap ~500/room. Delete = tombstone (blank text, keep row).
- **Reaction**: per message, `emoji:Text → count:Nat` (aggregate). Allowed set ~ 👍 ❤️ 😀 🎉 🔥 🌯.
- **Server/Guild**: `id:Nat, name:Text, createdAt:Int, channelIds:[Nat], owner:Principal, kind: #Discussion|#Forum|#Feed (immutable), private:Bool`.
- **Role**: per (serverId, principal) → `#None|#Moderator|#Admin|#Owner` (ordered). Owner set at create. Global super-admin via Admin service.
- **Moderation**: pinned message ids (set), locked room ids (set, blocks new posts from non-mods), mutes (serverId, principal, untilMs).
- **Membership**: per server, set of principals; presence (lastSeen, online via recent ping); typing (roomId → who, recent).
- **Thread**: per parent message id → list of `{author:Principal, handle:Text, at:Int, text:Text}`.

### Forum (Discourse-style — see PART 4 for look)
- **Category**: `id:Nat, name:Text, slug:Text, description:Text, color:Text (#hex), topicCount:Nat, postCount:Nat`.
- **Topic**: `id:Nat, categoryId:Nat, title:Text, slug:Text, author:Principal, authorHandle:Text, createdAt:Int, bumpedAt:Int, tags:[Text], pinned:Bool, closed:Bool, acceptedPostId:Nat (0=none), views:Nat`.
- **Post (reply)**: `id:Nat, topicId:Nat, author:Principal, authorHandle:Text, at:Int, body:Text (markdown-ish), replyToPost:Nat, likeCount:Nat`. First post = OP.
- **Likes**: per post, set of principals (likeCount derived). Bookmarks optional.
- Sorting tabs: latest (by bumpedAt), top (by likeCount/posts), new, categories.

### Feed (X-style)
- **FeedPost**: `id:Nat, author:Principal, authorHandle:Text, at:Int, text:Text, likes:Nat, reposts:Nat, replyCount:Nat`.
- **UserFollow**: follower:Principal → set of followee principals.
- **Repost/Like**: per (principal, postId). Home timeline = posts by self + followed, reverse-chron.

### Messenger (WhatsApp) + Keys (E2EE)
- **Conversation**: `id:Nat, kind: #Direct|#Group, members:[Principal], name:Text (groups), createdAt:Int`. Direct convo keyed by the sorted principal pair.
- **DmMessage**: `id:Nat, convoId:Nat, sender:Principal, at:Int, ciphertext:Text (E2EE ENVELOPE, base64 — server stores ciphertext, never plaintext), readBy:[Principal]`.
- **Key directory (Keys)**: per principal → list of devices `{ deviceId:Text, ikPub:Text (b64 X25519 identity key), spkPub:Text, spkSig:Text, spkAt:Int, otpks:[Text] (one-time prekey pool) }`. `fetchBundle(peer)` returns ik/spk/spksig + pops one otpk. Real E2EE crypto runs CLIENT-side; the canister is a key directory + ciphertext relay. Implement the directory + envelope storage honestly; document that browser-side X25519/AEAD is the client's job (key directory is the on-chain part).
- **Status**: `{ id:Nat, author:Principal, at:Int, text:Text, kind:#Text|#Image, expiresAt:Int (at + 24h) }`. List = non-expired, grouped by author.

### Admin
- allowlist of principals; first caller can self-bootstrap if empty; `whoami` returns caller + isAdmin; list/add/remove (admin-gated).

---

## PART 4 — Discourse / forum.dfinity.org look & feel (match this)

Stock Discourse skinned with DFINITY branding. Reproduce the default theme.

**Palette:** near-black text `#1a1a1a`; white cards on light-grey page `#f8f9fa`;
link/accent blue `#0088CC`; solved/success green `#009900`; like-heart `#fa6c8d`;
danger `#c80001`; DFINITY orange `#F15A24`. Category color squares (exact):
Developers `#F15A24`, Getting Started `#0088CC`, Motoko `#c22d7f`, Rust `#f74c00`,
JavaScript `#f0db4f`, Showcase `#F7941D`, Internet Identity `#0088CC`,
Uncategorized `#FBB03B`, Education `#12A89D`, Community `#BF1E2E`. Seed ~8 categories using these.

**Layout:** centered content column ~1100px. Forum sub-nav tab strip:
Latest · New · Unread · Top · Categories. 

**Components:**
- **Category badge** = small COLORED SQUARE + label (color from category). Avatars are always CIRCLES; categories always SQUARES.
- **Topic row** (table): bold title link (blue on hover) with status glyphs (📌 pinned, 🔒 closed, ✅ solved), a line of tag pills + the category badge under the title; then small circular participant avatars; right-aligned numeric **Replies** and **Views** columns; relative **Activity** time (from bumpedAt).
- **Tag** = small low-contrast rounded pill.
- **Topic view** (`/forum/t/{id}`): title + category badge + tags; stream of posts each = circular avatar (left) + bold username + relative date, markdown-ish body, action bar (♥ like+count, Reply, Share, Bookmark); accepted answer shows a green solution block; closed disables the composer; reply composer docked at the bottom.
- **New Topic composer**: title input, category chooser (colored badges), tag input, body textarea, Create button.
- **Buttons**: solid accent (blue) primary `New Topic`/`Reply`; ghost secondary for cancel.

URLs mirror Discourse: `/forum`, `/forum/c/{id}`, `/forum/t/{id}`, `/forum/new`, `/u/{handle}`.

---

## Build order
1. Services first (self-contained, each moc-checks standalone).
2. Pages + Layouts referencing services.
3. Integration: `motoview build` + `moc --check` the whole actor; fix errors.

When in doubt about syntax, mirror the WORKING examples:
`apps/bzzz/src/Pages/Home.mview`, `apps/bzzz/src/Services/Identity.mo`,
`examples/crm/src/Pages/Board.mview`, `examples/crm/src/Services/Crm.mo`.
