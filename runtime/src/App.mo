/// The MotoView runtime orchestrator.
///
/// A generated app actor instantiates `App(config, pages, layouts, assets)` and
/// forwards `http_request` (query) and `http_request_update` (update) to it.
///
///   * Rendering is a query  -> `http_request` / `/_motoview/render`
///   * Events are updates     -> `/_motoview/event` -> `http_request_update`
///   * The browser synchronizes through versioned UI batches (`batchId`).
import Text "mo:base/Text";
import Nat "mo:base/Nat";
import Nat16 "mo:base/Nat16";
import Nat32 "mo:base/Nat32";
import Int "mo:base/Int";
import Char "mo:base/Char";
import Principal "mo:base/Principal";
import Time "mo:base/Time";
import Buffer "mo:base/Buffer";
import HashMap "mo:base/HashMap";
import Iter "mo:base/Iter";
import Types "Types";
import Url "Url";
import Json "Json";
import Hash "Hash";
import Router "Router";
import Security "Security";
import Html "Html";
import Sha256 "Sha256";
import Hex "Hex";
import Roles "Roles";
import WalletAuth "WalletAuth";
import CertV2 "CertV2";
import CertifiedData "mo:base/CertifiedData";

module {

  type HttpRequest = Types.HttpRequest;
  type HttpResponse = Types.HttpResponse;
  type Ctx = Types.Ctx;
  type Batch = Types.Batch;
  type Head = Types.Head;
  type Page = Types.Page;
  type Layout = Types.Layout;

  public class App(
    config : Types.Config,
    pages : [Page],
    layouts : [Layout],
    assets : Types.Assets,
  ) {

    let expiryNs : Int = 15 * 60 * 1_000_000_000; // 15 minutes
    var counter : Nat = 0;
    // consumed nonce -> the time it stops mattering (token expiry). Pruned so
    // replay-protection state stays bounded. Persisted across upgrades by the
    // generated actor (stable var mvConsumed via dumpConsumed/setConsumed) so a
    // consumed nonce survives `dfx deploy --mode upgrade` and cannot be replayed
    // after an upgrade.
    let consumed = HashMap.HashMap<Text, Int>(256, Text.equal, Text.hash);

    func consumeNonce(nonce : Text) {
      consumed.put(nonce, Time.now() + expiryNs);
      if (consumed.size() > 4096) {
        // SOUNDNESS: only evict EXPIRED nonces. An expired nonce can never be
        // replayed (verify() already rejects it as "token expired"), so
        // forgetting it is safe. We must NEVER evict a still-live nonce: doing
        // so would re-open it to an eviction-replay. If every entry is unexpired
        // and we are over cap we keep them all and let the map grow — a bounded
        // DoS (an attacker can only inflate it with valid single-use tokens it
        // already spent, each self-expiring within expiryNs) traded for never
        // dropping live replay-protection state. Honest tradeoff.
        let now = Time.now();
        let dead = Buffer.Buffer<Text>(64);
        for ((k, exp) in consumed.entries()) { if (exp < now) { dead.add(k) } };
        for (k in dead.vals()) { consumed.delete(k) };
      };
    };

    // Snapshot / restore the consumed-nonce store for upgrade persistence,
    // mirroring dumpEpochs/setEpochs and dumpRoles/setRoles. On restore we drop
    // entries already expired at load time so the map does not carry dead weight
    // across an upgrade.
    public func dumpConsumed() : [(Text, Int)] { Iter.toArray(consumed.entries()) };
    public func setConsumed(es : [(Text, Int)]) {
      let now = Time.now();
      for ((k, exp) in es.vals()) { if (exp >= now) { consumed.put(k, exp) } };
    };

    // ---- Internet Identity session bridge --------------------------------
    // An authenticated `mvEstablish(nonce)` update call (caller verified by the
    // IC) records the principal under a client-chosen nonce. A normal
    // GET /mv-session?nonce=... then exchanges the nonce for an HMAC session
    // token (signed with the app secret) that the client stores in the
    // `mv_session` cookie; every later request carries it and we resolve the
    // real caller from it. No agent signing on the hot path.
    let pending = HashMap.HashMap<Text, (Principal, Int)>(64, Text.equal, Text.hash);
    let sessionTtlNs : Int = 24 * 60 * 60 * 1_000_000_000; // 24h

    // The HMAC secret for session + secure-form tokens. It is installed at
    // runtime from the canister's `raw_rand` (see the generated actor's
    // http_request_update), so it is cryptographically random, per-canister, and
    // never appears in source. `config.secret` is only an empty placeholder.
    var mvSecret : Blob = config.secret;
    public func setSecret(b : Blob) { if (b.size() == 32) { mvSecret := b } };
    public func needsSecret() : Bool { mvSecret.size() != 32 };

    // Static framework assets served as fast CERTIFIED QUERIES (response-cert v2,
    // BODY-BOUND) instead of upgrading to an update call. certified_data is set
    // on the first update (ensureCert); until then assets serve via the update
    // path (consensus-valid), so it self-bootstraps. The committed tree now
    // commits each asset's response BODY (status + certified headers + body
    // hash), so a boundary/MITM cannot swap a certified asset's body undetected.
    let certAssets : [Text] = [
      "/motoview.js", "/motoview.css", "/motoview.wasm", "/motoview-crypto.wasm", "/mv-auth.js",
      "/favicon.svg", "/favicon.ico", "/robots.txt", "/sitemap.xml",
      "/manifest.webmanifest", "/sw.js",
    ];

    // The certified header SET for a response (response_header_exclusions:[] →
    // ALL response headers are certified). It is exactly what we serve PLUS the
    // IC-CertificateExpression header (also certified) and MINUS IC-Certificate
    // (which carries the proof and is never certified). The response_hash is
    // computed over this set so it matches what the boundary recomputes from the
    // served response. Kept in one place so certify-time and serve-time agree.
    func certifiedHeaderSet(base : [(Text, Text)]) : [(Text, Text)] {
      let h = Buffer.Buffer<(Text, Text)>(base.size() + 1);
      for (p in base.vals()) { h.add(p) };
      h.add(("IC-CertificateExpression", CertV2.expression));
      Buffer.toArray(h);
    };
    // The body-bound response hash for a response with this status/base-headers/
    // body, over the certified header set above.
    func respHashFor(status : Nat, base : [(Text, Text)], body : Blob) : Blob {
      CertV2.responseHash(status, certifiedHeaderSet(base), body);
    };
    // The static prefix of a route, up to its first `{param}` segment.
    // "/u/{handle}" -> "/u" ; "/forum/t/{id:Nat}" -> "/forum/t".
    func wildcardPrefix(route : Text) : Text {
      let pre = Buffer.Buffer<Text>(4);
      for (s in Text.tokens(route, #char '/')) {
        if (Text.contains(s, #char '{')) { return "/" # Text.join("/", pre.vals()) };
        pre.add(s);
      };
      "/" # Text.join("/", pre.vals());
    };
    // Wildcard prefixes contributed by parameterized @cacheable routes.
    let wildPrefixes : [Text] = do {
      let b = Buffer.Buffer<Text>(4);
      for (p in pages.vals()) {
        if (p.cacheable and Text.contains(p.route, #char '{')) { b.add(wildcardPrefix(p.route)) };
      };
      Buffer.toArray(b);
    };
    func isWildPrefix(path : Text) : Bool {
      for (w in wildPrefixes.vals()) { if (w == path) { return true } };
      false;
    };
    // The certified entry for a @cacheable page route (or null = serve via the
    // always-correct update path). A page is certified as: a wildcard over its
    // static prefix if parameterized; otherwise an exact entry — EXCEPT the root
    // "/" and exact routes that collide with a wildcard prefix (e.g. /docs when
    // /docs/{slug} exists). Those two response shapes are rejected by the IC
    // boundary's response-verification today, so they fall back to update calls.
    func certEntryFor(route : Text, path : Text) : ?CertV2.Entry {
      if (Text.contains(route, #char '{')) { ?{ path = wildcardPrefix(route); wild = true } } else if (route == "/" or isWildPrefix(route)) {
        null;
      } else { ?{ path = path; wild = false } };
    };
    // The body-bound entry for a static asset: its served (status, base headers,
    // body) hashed per the v2 spec. Returns null if the path is not a real asset.
    func assetBoundEntry(path : Text) : ?CertV2.BoundEntry {
      switch (asset(path)) {
        case null { null };
        case (?r) { ?{ path = path; wild = false; respHash = respHashFor(Nat16.toNat(r.status_code), r.headers, r.body) } };
      };
    };

    // The set of body-bound page entries currently certified. @cacheable page
    // bodies are DYNAMIC (depend on canister state), so unlike static assets we
    // cannot hash them once at construction. They are registered on demand
    // (registerPage) and re-registered when their content changes; the root hash
    // is recomputed and re-installed via CertifiedData.set so the certificate
    // tracks the live body. Until a page is registered (or after a mutation, on
    // the first request that re-renders it) it falls back to the update path.
    let pageBound = HashMap.HashMap<Text, CertV2.BoundEntry>(16, Text.equal, Text.hash);

    // Build the full body-bound entry set: static assets (hashed now) + any
    // currently-registered @cacheable pages.
    func allBoundEntries() : [CertV2.BoundEntry] {
      let b = Buffer.Buffer<CertV2.BoundEntry>(certAssets.size() + pageBound.size());
      for (a in certAssets.vals()) {
        switch (assetBoundEntry(a)) { case (?e) { b.add(e) }; case null {} };
      };
      for ((_, e) in pageBound.entries()) { b.add(e) };
      Buffer.toArray(b);
    };

    var certReady : Bool = false;
    // Recompute and install the body-bound root hash over the current entry set.
    // Called on first update (bootstrap) and whenever a @cacheable page's body
    // changes (registerPage). Static-asset hashes are stable; only page entries
    // move the root.
    func recertify() {
      CertifiedData.set(CertV2.rootHash(allBoundEntries()));
      certReady := true;
    };
    public func ensureCert() { if (not certReady) { recertify() } };

    /// Register (or update) a @cacheable page's body-bound certificate entry and
    /// re-install certified_data so the live response is body-certified.
    ///
    /// IMPORTANT (dynamic @cacheable content): a static asset is hashed once and
    /// never changes, so it is certified at bootstrap. A @cacheable PAGE renders
    /// from canister state, so its certified body must be RE-CERTIFIED whenever
    /// that state mutates — otherwise the boundary would reject the stale-hash
    /// response and the request silently falls back to the (always-correct,
    /// consensus-validated) update path: safe, just no certified-query speedup.
    /// The runtime calls this from the certified-query render path so the entry
    /// self-heals to the current body on the next request after a mutation; an
    /// app that mutates @cacheable state in an update handler may also call it
    /// eagerly to keep the fast path warm. No-op if the rendered body is null.
    public func registerPage(target : CertV2.Entry, status : Nat, headers : [(Text, Text)], body : Blob) {
      let rh = respHashFor(status, headers, body);
      let entry : CertV2.BoundEntry = { path = target.path; wild = target.wild; respHash = rh };
      switch (pageBound.get(target.path)) {
        case (?prev) { if (prev.respHash == rh and prev.wild == target.wild) { return } }; // unchanged
        case null {};
      };
      pageBound.put(target.path, entry);
      recertify();
    };
    func isCertAsset(path : Text) : Bool {
      for (a in certAssets.vals()) { if (a == path) { return true } };
      false;
    };

    var loginCounter : Nat = 0;
    /// An unguessable single-use login nonce, bound to the browser via a
    /// short-lived httpOnly cookie (see /mv-login-begin + /mv-session).
    func freshNonce() : Text {
      loginCounter += 1;
      Hex.encode(Sha256.hmac(mvSecret, Text.encodeUtf8("mv-login." # Nat.toText(loginCounter) # "." # Int.toText(Time.now()))));
    };

    /// Called from the generated actor's authenticated `mvEstablish` method.
    public func establish(nonce : Text, who : Principal) {
      // Hard cap to bound memory against unauthenticated flooding of
      // mvEstablish: prune aggressively, and drop this entry if still full.
      if (pending.size() >= 4096) {
        let cutoff = Time.now() - 60 * 1_000_000_000; // 1 min
        let dead = Buffer.Buffer<Text>(256);
        for ((k, (_, t)) in pending.entries()) { if (t < cutoff) { dead.add(k) } };
        for (k in dead.vals()) { pending.delete(k) };
        if (pending.size() >= 4096) { return };
      };
      pending.put(nonce, (who, Time.now()));
    };

    // Per-principal session epoch. A token embeds the epoch it was minted at;
    // bumping a principal's epoch (on /mv-logout) invalidates ALL their existing
    // tokens server-side ("log out everywhere"), even un-expired ones. Synced to
    // a stable var by the generated actor so revocations survive upgrades.
    let epochs = HashMap.HashMap<Text, Nat>(64, Text.equal, Text.hash);
    func epochOf(pt : Text) : Nat { switch (epochs.get(pt)) { case (?e) e; case null 0 } };
    public func bumpEpoch(pt : Text) { epochs.put(pt, epochOf(pt) + 1) };
    public func dumpEpochs() : [(Text, Nat)] { Iter.toArray(epochs.entries()) };
    public func setEpochs(es : [(Text, Nat)]) { for ((k, v) in es.vals()) { epochs.put(k, v) } };

    // ---- role store: principal -> roles. Backs `@authorize role="..."` and the
    // ctx role API. Persisted by the generated actor (stable var mvRoles).
    let roleStore = Roles.Store();
    func hasRole(p : Principal, role : Text) : Bool { roleStore.has(p, role) };
    func grantRole(p : Principal, role : Text) { roleStore.grant(p, role) };
    func revokeRole(p : Principal, role : Text) { roleStore.revoke(p, role) };
    func claimRole(p : Principal, role : Text) : Bool { roleStore.claim(p, role) };
    public func dumpRoles() : [(Principal, [Text])] { roleStore.dump() };
    public func setRoles(rs : [(Principal, [Text])]) { roleStore.load(rs) };

    // ---- wallet spend-authorization gate (Slice 9B) -----------------------
    // Per-principal velocity limiter backing `ctx.authorizeSpend`. A leaked or
    // abused session cannot be drained in a burst: the rolling-window sum of
    // authorized spend "weight" is capped per principal. Persisted by the
    // generated actor via a stable var (dumpVelocity/setVelocity), like the
    // consumed-nonce store and the role store, so the limit survives upgrades.
    let velocity = WalletAuth.Velocity();
    public func dumpVelocity() : [(Text, [WalletAuth.Entry])] { velocity.dump() };
    public func setVelocity(es : [(Text, [WalletAuth.Entry])]) { velocity.load(es) };
    // Default policy: total authorized weight per principal within a rolling
    // window. An app can pick the weight unit (amount, normalized risk score);
    // these defaults bound a runaway/leaked session to a bursty-but-bounded loss.
    let spendWindowNs : Int = 60 * 60 * 1_000_000_000; // 1 hour
    let spendLimit : Nat = 1_000_000; // total weight per principal per window

    /// Authorize a wallet spend. A wallet confirm-step handler MUST call this and
    /// get `true` BEFORE it builds the sighash and calls
    /// `ChainKey.signWithEcdsa` / `signWithSchnorr`. (A future native
    /// `host_device_sign` hardware-assertion check MUST also gate signing once the
    /// native client lands — see WalletAuth.mo; it is NOT implemented here.)
    ///
    /// All four invariants are enforced atomically:
    ///   * valid session — the token is bound to `caller`'s principal;
    ///   * intent binding — the token's intentHash must equal the hash of THIS
    ///     `intent`, so a token minted for spend X cannot authorize spend Y;
    ///   * single-use — the token's nonce is consumed here (replay rejected);
    ///   * velocity — `weight` must keep the per-principal rolling-window sum
    ///     within `spendLimit`.
    /// Returns true only if every check passes. State (nonce consumption,
    /// velocity record) mutates ONLY on success, so a rejected attempt costs the
    /// caller nothing toward their limit and does not burn the token's nonce.
    func authorizeSpend(handler : Text, intent : [(Text, Text)], token : Text, weight : Nat, path : Text, caller : Principal) : Bool {
      switch (
        WalletAuth.authorizeSpend({
          secret = mvSecret;
          token;
          path;
          handler;
          caller;
          nowNs = Time.now();
          intent;
        })
      ) {
        case (#err(_)) { false }; // bad sig/route/handler/principal/intent/expired
        case (#ok({ nonce })) {
          // single-use: reject a token whose nonce was already consumed (replay).
          switch (consumed.get(nonce)) {
            case (?_) { return false };
            case null {};
          };
          // velocity: atomic check-and-record so the window sum can't be raced
          // past the cap within this message. Reject (without consuming the
          // nonce) if over limit, so the caller can retry after the window.
          if (not velocity.tryRecord(caller, weight, Time.now(), spendLimit, spendWindowNs)) {
            return false;
          };
          // all checks passed -> burn the nonce so this exact token can't sign again.
          consumeNonce(nonce);
          true;
        };
      };
    };

    func sessionMac(body : Text) : Text {
      Hex.encode(Sha256.hmac(mvSecret, Text.encodeUtf8(body)));
    };

    func mintSession(p : Principal) : Text {
      let pt = Principal.toText(p);
      let body = pt # "." # Int.toText(Time.now() + sessionTtlNs) # "." # Nat.toText(epochOf(pt));
      body # "." # sessionMac(body);
    };

    func verifySession(token : Text) : ?Principal {
      let parts = Iter.toArray(Text.split(token, #char '.'));
      if (parts.size() != 4) { return null };
      let body = parts[0] # "." # parts[1] # "." # parts[2];
      if (sessionMac(body) != parts[3]) { return null };
      switch (parseNat(parts[1])) {
        case (?exp) { if (Time.now() > exp) { return null } };
        case null { return null };
      };
      switch (parseNat(parts[2])) {
        case (?ep) { if (ep != epochOf(parts[0])) { return null } }; // revoked
        case null { return null };
      };
      ?Principal.fromText(parts[0]);
    };

    /// Exchange a nonce (set by an authenticated mvEstablish within 5 min) for a
    /// session. Returns (principalText, token) or null. Single-use.
    func sessionFor(nonce : Text) : ?(Text, Text) {
      switch (pending.get(nonce)) {
        case (?(p, t)) {
          if (nonce != "" and Time.now() - t < 5 * 60 * 1_000_000_000) {
            pending.delete(nonce);
            return ?(Principal.toText(p), mintSession(p));
          };
          null;
        };
        case null { null };
      };
    };

    /// Build an httpOnly, always-Secure cookie header. (The IC boundary is HTTPS
    /// in production; browsers also accept Secure cookies on localhost.) Secure
    /// is unconditional rather than gated on a client-forgeable header.
    func cookie(name : Text, value : Text, maxAge : Int, sameSite : Text) : Text {
      name # "=" # value # "; Path=/; HttpOnly; Secure; SameSite=" # sameSite # "; Max-Age=" # Int.toText(maxAge);
    };

    /// A text response that also emits one or more Set-Cookie headers. The
    /// session token is never exposed to JS (XSS-resistant).
    func respCookies(body : Text, cookies : [Text]) : HttpResponse {
      let hdrs = Buffer.Buffer<(Text, Text)>(cookies.size() + 2);
      hdrs.add(("content-type", "text/plain"));
      hdrs.add(("cache-control", "no-store"));
      for (c in cookies.vals()) { hdrs.add(("Set-Cookie", c)) };
      { status_code = 200; headers = Buffer.toArray(hdrs); body = Text.encodeUtf8(body); upgrade = null };
    };

    /// Read a named cookie from the request, "" if absent.
    func cookieFromReq(req : HttpRequest, name : Text) : Text {
      for ((k, v) in req.headers.vals()) {
        if (lower(k) == "cookie") {
          switch (cookieValue(v, name)) { case (?x) { return x }; case null {} };
        };
      };
      "";
    };

    /// The effective caller: the principal from a valid mv_session cookie, else
    /// the (anonymous) gateway caller.
    public func effectiveCaller(req : HttpRequest, fallback : Principal) : Principal {
      for ((k, v) in req.headers.vals()) {
        if (lower(k) == "cookie") {
          switch (cookieValue(v, "mv_session")) {
            case (?tok) { switch (verifySession(tok)) { case (?p) { return p }; case null {} } };
            case null {};
          };
        };
      };
      fallback;
    };

    func cookieValue(cookie : Text, name : Text) : ?Text {
      for (part in Text.split(cookie, #char ';')) {
        let pair = Iter.toArray(Text.split(Text.trim(part, #char ' '), #char '='));
        if (pair.size() == 2 and pair[0] == name) { return ?pair[1] };
      };
      null;
    };

    func lower(t : Text) : Text {
      Text.map(t, func(c : Char) : Char {
        if (c >= 'A' and c <= 'Z') { Char.fromNat32(Char.toNat32(c) + 32) } else { c };
      });
    };

    func parseNat(t : Text) : ?Nat {
      var n : Nat = 0;
      var any = false;
      for (c in t.chars()) {
        let d = Char.toNat32(c);
        if (d >= 48 and d <= 57) { n := n * 10 + Nat32.toNat(d - 48); any := true } else { return null };
      };
      if (any) { ?n } else { null };
    };

    // ---- public entry points ---------------------------------------------

    /// Query entry point. On the IC, query `http_request` responses must be
    /// certified or the boundary node rejects them with a response-verification
    /// error. MotoView pages are fully dynamic, so the MVP upgrades every
    /// request to an update call (validated by consensus, no certification
    /// needed). Certified *query* rendering for cacheable public pages is a
    /// roadmap optimization; the render/event protocol is unchanged.
    public func httpRequest(req : HttpRequest, caller : Principal) : HttpResponse {
      let (path, _) = Url.splitUrl(req.url);
      // Serve static framework assets and @cacheable pages as certified queries
      // (no upgrade). Until certified_data is set (first update), fall through.
      switch (certifiedAsset(path)) { case (?r) { return r }; case null {} };
      switch (certifiedPage(req, path, caller)) { case (?r) { return r }; case null {} };
      { status_code = 200; headers = jsonHeaders(); body = ""; upgrade = ?true };
    };

    // Emit the served headers for a body-bound certified response: the certified
    // header SET (base + IC-CertificateExpression) PLUS the IC-Certificate proof.
    // This is exactly the set responseHash was computed over (certifiedHeaderSet)
    // with IC-Certificate appended — keeping serve-time and certify-time aligned.
    func certHeaders(base : [(Text, Text)], target : CertV2.BoundEntry, entries : [CertV2.BoundEntry], cert : Blob) : [(Text, Text)] {
      let hdrs = Buffer.Buffer<(Text, Text)>(base.size() + 2);
      for (h in certifiedHeaderSet(base).vals()) { hdrs.add(h) };
      hdrs.add(("IC-Certificate", CertV2.headerValue(entries, target, cert)));
      Buffer.toArray(hdrs);
    };

    /// Build a body-bound certified-query response for a static asset, or null to
    /// fall back to the update path.
    func certifiedAsset(path : Text) : ?HttpResponse {
      if (not certReady or not isCertAsset(path)) { return null };
      switch (CertifiedData.getCertificate()) {
        case null { null };
        case (?cert) {
          switch (asset(path)) {
            case null { null };
            case (?r) {
              let target : CertV2.BoundEntry = { path = path; wild = false; respHash = respHashFor(Nat16.toNat(r.status_code), r.headers, r.body) };
              ?{ status_code = r.status_code; headers = certHeaders(r.headers, target, allBoundEntries(), cert); body = r.body; upgrade = null };
            };
          };
        };
      };
    };

    /// Render a @cacheable page in the query context and serve it as a BODY-BOUND
    /// certified query. The rendered body is hashed and (if changed) re-certified
    /// before the response is served, so the certificate commits THIS body. If
    /// the page body changed since the last certify and a fresh certificate is
    /// not yet available, we fall back to the update path (safe). Parameterized
    /// routes use a wildcard over their static prefix.
    func certifiedPage(req : HttpRequest, path : Text, caller : Principal) : ?HttpResponse {
      if (not certReady) { return null };
      switch (Router.find(pages, path)) {
        case null { null };
        case (?(page, params)) {
          if (not page.cacheable) { return null };
          // SECURITY (#42): never serve an `@authorize` page on the certified-
          // query fast path. This path runs in the QUERY context, where `caller`
          // is the anonymous gateway principal (the mv_session cookie is resolved
          // only in the update path) AND it does not call `authorized()` — so a
          // gated page with a caller-independent body would otherwise be served
          // to unauthenticated callers. Fall back to null -> the update path,
          // which resolves the session caller and enforces the gate. (`@cacheable`
          // is therefore a no-op on gated pages; the compiler warns about it.)
          if (page.authorize) { return null };
          switch (certEntryFor(page.route, path)) {
            case null { null }; // not safely certifiable -> update path
            case (?target) {
              let (_, q) = Url.splitUrl(req.url);
              let ctx = makeCtx("GET", path, Url.parsePairs(q), params, [], caller, "");
              page.onLoad(ctx);
              ignore page.takeRedirect();
              let head = page.head(ctx);
              let inner = page.render(ctx);
              let bid = batchIdFor(path, head.title, inner);
              let doc = renderDocument(page, ctx, head, inner, bid);
              let body = Text.encodeUtf8(doc);
              let base = [("content-type", "text/html"), ("cache-control", "no-store")];
              // Body-bind: ensure this exact body is the one committed in
              // certified_data. registerPage re-installs the root hash if the
              // body changed (and is a no-op if unchanged). The certificate we
              // then read must correspond to that root; if certified_data was
              // just bumped, getCertificate() may still reflect the PREVIOUS
              // root within this same message — in that case the witnessed body
              // hash would not match, so we fall back to the update path rather
              // than serve a mismatched (boundary-rejected) certificate.
              let entry : CertV2.BoundEntry = { path = target.path; wild = target.wild; respHash = respHashFor(200, base, body) };
              let changed = switch (pageBound.get(target.path)) {
                case (?prev) { prev.respHash != entry.respHash or prev.wild != entry.wild };
                case null { true };
              };
              if (changed) {
                // Cannot safely certify a freshly-changed body in a query (no
                // CertifiedData.set in query context); serve via the update path,
                // which re-renders, registers the body, and re-certifies.
                return null;
              };
              switch (CertifiedData.getCertificate()) {
                case null { null };
                case (?cert) {
                  ?{ status_code = 200; headers = certHeaders(base, entry, allBoundEntries(), cert); body = body; upgrade = null };
                };
              };
            };
          };
        };
      };
    };

    /// Update entry point. Serves assets, SSR pages, render polls and events.
    public func httpRequestUpdate(req : HttpRequest, caller : Principal) : HttpResponse {
      ensureCert();
      let (path, _) = Url.splitUrl(req.url);
      // Resolve the real signed-in principal from the mv_session cookie (set
      // after an Internet Identity login); falls back to the gateway caller.
      let who = effectiveCaller(req, caller);
      if (Text.startsWith(path, #text "/_motoview/event")) {
        serveEvent(req, who);
      } else {
        serveGet(req, who);
      };
    };

    // ---- request handling -------------------------------------------------

    /// Serve assets, SSR documents, and render polls.
    func serveGet(req : HttpRequest, caller : Principal) : HttpResponse {
      let (path, q) = Url.splitUrl(req.url);
      let qp = Url.parsePairs(q);

      // II login step 1: hand the browser an unguessable nonce, bound to it via
      // a short-lived httpOnly cookie. The client signs mvEstablish(nonce).
      if (path == "/mv-login-begin") {
        let n = freshNonce();
        return respCookies(n, [cookie("mv_login", n, 300, "Strict")]);
      };
      // II login step 2: redeem. The nonce comes from the httpOnly mv_login
      // cookie (NOT a URL param), so only the browser that began the login can
      // redeem it — preventing login-CSRF and nonce theft.
      if (path == "/mv-session") {
        switch (sessionFor(cookieFromReq(req, "mv_login"))) {
          case (?(who, tok)) {
            return respCookies(who, [
              cookie("mv_session", tok, 24 * 60 * 60, "Lax"),
              cookie("mv_login", "", 0, "Strict"),
            ]);
          };
          case null { return textResp("", "text/plain") };
        };
      };
      // Who is the caller right now (resolved from the cookie)? For login UI.
      if (path == "/mv-whoami") { return textResp(Principal.toText(caller), "text/plain") };
      // Log out everywhere: bump the caller's epoch (invalidates all their
      // outstanding tokens server-side) and clear this browser's cookie.
      if (path == "/mv-logout") {
        if (not Principal.isAnonymous(caller)) { bumpEpoch(Principal.toText(caller)) };
        return respCookies("ok", [cookie("mv_session", "", 0, "Lax")]);
      };
      // Internet Identity alternative origins (for a stable cross-domain
      // derivationOrigin). Configure via Config.altOrigins.
      if (path == "/.well-known/ii-alternative-origins") {
        let b = Buffer.Buffer<Text>(config.altOrigins.size());
        for (o in config.altOrigins.vals()) { b.add("\"" # o # "\"") };
        return textResp("{\"alternativeOrigins\":[" # Text.join(",", b.vals()) # "]}", "application/json");
      };

      // static assets
      switch (asset(path)) { case (?r) { return r }; case null {} };

      // render poll
      if (path == "/_motoview/render") {
        let target = Url.getOr(qp, "path", "/");
        let last = Url.getOr(qp, "lastBatchId", "");
        return jsonResp(Json.encodeBatch(renderBatch(target, last, caller, qp)));
      };

      // page route -> server-side render of the full document
      switch (Router.find(pages, path)) {
        case (?(page, params)) {
          if (not authorized(page, caller)) { return redirectDoc(authTarget(page)) };
          let ctx = makeCtx("GET", path, qp, params, [], caller, "");
          page.onLoad(ctx);
          ignore page.takeRedirect(); // ignore navigations during render
          let head = page.head(ctx);
          let inner = page.render(ctx);
          let bid = batchIdFor(path, head.title, inner);
          let doc = renderDocument(page, ctx, head, inner, bid);
          // Body-bind @cacheable pages: this update-context render is the moment
          // we CAN re-certify (CertifiedData.set is allowed in an update), so we
          // register the rendered body. The NEXT query for this page then serves
          // it as a fast, body-bound certified query. A no-op if the body is
          // unchanged. Only for routes that are safely certifiable (certEntryFor).
          if (page.cacheable) {
            switch (certEntryFor(page.route, path)) {
              case (?target) {
                let base = [("content-type", "text/html"), ("cache-control", "no-store")];
                registerPage(target, 200, base, Text.encodeUtf8(doc));
              };
              case null {};
            };
          };
          return htmlResp(200, doc);
        };
        case null { return htmlResp(404, notFoundDoc()) };
      };
    };

    /// Handle an event (mutation) and return a new batch.
    func serveEvent(req : HttpRequest, caller : Principal) : HttpResponse {
      let form = Url.parseForm(req.body);
      let handlerId = Url.getOr(form, "__mv_handler", "");
      let pagePath = Url.getOr(form, "__mv_path", "/");
      let last = Url.getOr(form, "__mv_batch", "");
      let args = collectArgs(form);

      switch (Router.find(pages, pagePath)) {
        case (?(page, params)) {
          if (not authorized(page, caller)) { return jsonResp(Json.encodeBatch(redirectBatch(authTarget(page)))) };

          // Secure-form verification + replay protection. Verification is forced
          // for any handler bound to a `secure` form (isSecureHandler) — NOT just
          // when the request says `__mv_secure=1` — so an attacker cannot bypass
          // the CSRF/replay/over-post checks by simply omitting the flag. A forced
          // request with no/invalid token fails verify() -> security error.
          if (isSecureHandler(page, handlerId) or Url.getOr(form, "__mv_secure", "") == "1") {
            let token = Url.getOr(form, "__mv_token", "");
            let schema = Url.getOr(form, "__mv_schema", "");
            switch (
              Security.verify(
                mvSecret,
                token,
                pagePath,
                handlerId,
                Principal.toText(caller),
                Time.now(),
                Security.schemaHash(schema),
                "", // ordinary forms carry no server-known intent
              )
            ) {
              case (#invalid(reason)) {
                return jsonResp(Json.encodeBatch(securityErrorBatch(reason)));
              };
              case (#ok({ nonce })) {
                switch (consumed.get(nonce)) {
                  case (?_) { return jsonResp(Json.encodeBatch(securityErrorBatch("replayed submission"))) };
                  case null { consumeNonce(nonce) };
                };
              };
            };
          };

          let ctx = makeCtx("POST", pagePath, [], params, form, caller, last);
          page.dispatch(ctx, handlerId, args);

          let redirect = page.takeRedirect();
          if (redirect != "") { return jsonResp(Json.encodeBatch(redirectBatch(redirect))) };

          // Re-run onLoad so the event response reflects the mutation the handler
          // just made (otherwise pages that load their display state in onLoad
          // wouldn't update until the next poll). Skip it when the handler set
          // validation errors, so the submitted form + errors are preserved.
          if (page.takeErrors().size() == 0) { page.onLoad(ctx) };

          // Render first so the page can show any validation errors it set,
          // then read those errors to decide the batch status.
          let head = page.head(ctx);
          let inner = page.render(ctx);
          let errors = page.takeErrors();
          let effects = page.takeEffects();
          let bid = batchIdFor(pagePath, head.title, inner);

          if (errors.size() > 0) {
            return jsonResp(Json.encodeBatch({
              status = #validationError;
              batchId = bid;
              html = inner;
              ui = null;
              head;
              effects;
              target = "mv-root";
              location = "";
              errors;
            }));
          };

          return jsonResp(Json.encodeBatch({
            status = #changed;
            batchId = bid;
            html = inner;
            ui = null;
            head;
            effects;
            target = "mv-root";
            location = "";
            errors = [];
          }));
        };
        case null { return jsonResp(Json.encodeBatch(redirectBatch("/"))) };
      };
    };

    // ---- render -----------------------------------------------------------

    func renderBatch(target : Text, last : Text, caller : Principal, qp : [(Text, Text)]) : Batch {
      switch (Router.find(pages, target)) {
        case (?(page, params)) {
          if (not authorized(page, caller)) { return redirectBatch(authTarget(page)) };
          let ctx = makeCtx("GET", target, qp, params, [], caller, last);
          page.onLoad(ctx);
          ignore page.takeRedirect();
          let head = page.head(ctx);
          let inner = page.render(ctx);
          let bid = batchIdFor(target, head.title, inner);
          if (bid == last) {
            return {
              status = #unchanged;
              batchId = bid;
              html = "";
              ui = null;
              head;
              effects = [];
              target = "mv-root";
              location = "";
              errors = [];
            };
          };
          {
            status = #changed;
            batchId = bid;
            html = inner;
            ui = null;
            head;
            effects = [];
            target = "mv-root";
            location = "";
            errors = [];
          };
        };
        case null { redirectBatch("/") };
      };
    };

    func batchIdFor(path : Text, title : Text, inner : Text) : Text {
      // Mask secure-form token values: they are minted fresh on every render but
      // do not represent UI state, so they must not affect the batchId (otherwise
      // every poll would look "changed" and disrupt typing).
      Hash.batchId(path # "\u{1f}" # title # "\u{1f}" # maskTokens(inner));
    };

    func maskTokens(t : Text) : Text {
      let parts = Iter.toArray(Text.split(t, #text "data-mv-token=\""));
      if (parts.size() <= 1) { return t };
      var out = parts[0];
      var i = 1;
      while (i < parts.size()) {
        out #= "data-mv-token=\"\"" # afterFirstQuote(parts[i]);
        i += 1;
      };
      out;
    };

    func afterFirstQuote(s : Text) : Text {
      let segs = Iter.toArray(Text.split(s, #char '\"'));
      var out = "";
      var i = 1;
      while (i < segs.size()) {
        if (i > 1) { out #= "\"" };
        out #= segs[i];
        i += 1;
      };
      out;
    };

    // ---- documents --------------------------------------------------------

    func renderDocument(page : Page, ctx : Ctx, head : Head, inner : Text, bid : Text) : Text {
      let wrapped = "<div id=\"mv-root\" data-mv-root data-mv-batch=\"" # bid # "\">" # inner # "</div>";
      let doc = switch (findLayout(page.layout)) {
        case (?layout) { layout.render(ctx, head, wrapped) };
        case null { defaultDocument(head, wrapped) };
      };
      injectAssets(doc);
    };

    func findLayout(name : Text) : ?Layout {
      if (name == "") { return null };
      for (l in layouts.vals()) { if (l.name == name) { return ?l } };
      null;
    };

    func injectAssets(doc : Text) : Text {
      var d = doc;
      // Guard on the actual tag (href=/src=), NOT the bare filename — otherwise a
      // page whose CONTENT mentions "motoview.js" (e.g. these docs) would be
      // mistaken for already having the script, and the client would never load.
      // Inject the base stylesheet at the START of <head>, so a layout's own
      // <style> and an @theme block (both later in <head>) override it.
      if (not Text.contains(d, #text "href=\"/motoview.css\"")) {
        d := Text.replace(d, #text "<head>", "<head><link rel=\"stylesheet\" href=\"/motoview.css\">");
      };
      // Theme: apply the user's saved light/dark choice from the mv_theme cookie
      // BEFORE first paint (no flash), client-side so it also works on certified
      // cacheable pages (whose cert can't vary by cookie). Framework glue, not app
      // JS — same category as the injected scripts below. The toggle is in
      // motoview.js ([data-mv-theme-toggle]); absent cookie -> prefers-color-scheme.
      if (not Text.contains(d, #text "mv_theme")) {
        d := Text.replace(
          d,
          #text "<head>",
          "<head><script>(function(){try{var m=document.cookie.match(/(?:^|; )mv_theme=(web-light|web-dark|teams-light|teams-dark|material-light|material-dark|hc|light|dark)/);if(m)document.documentElement.setAttribute('data-theme',m[1]);}catch(e){}})();</script>",
        );
      };
      // PWA: actually LINK the web manifest (it is served at /manifest.webmanifest
      // but a page must reference it for the browser to offer "Install") plus the
      // iOS/standalone meta. Without this the app is not installable.
      if (not Text.contains(d, #text "rel=\"manifest\"")) {
        d := Text.replace(
          d,
          #text "<head>",
          "<head><link rel=\"manifest\" href=\"/manifest.webmanifest\">"
          # "<link rel=\"apple-touch-icon\" href=\"/favicon.svg\">"
          # "<meta name=\"apple-mobile-web-app-capable\" content=\"yes\">"
          # "<meta name=\"mobile-web-app-capable\" content=\"yes\">"
          # "<meta name=\"apple-mobile-web-app-status-bar-style\" content=\"default\">",
        );
      };
      if (not Text.contains(d, #text "name=\"theme-color\"")) {
        d := Text.replace(d, #text "<head>", "<head><meta name=\"theme-color\" content=\"#6d28d9\">");
      };
      if (not Text.contains(d, #text "src=\"/motoview.js\"")) {
        d := Text.replace(d, #text "</body>", "<script src=\"/motoview.js\" defer></script></body>");
      };
      // Internet Identity login, available to every app at /mv-auth.js. It is a
      // no-op unless the page has a [data-mv-signin] element to wire.
      if (not Text.contains(d, #text "src=\"/mv-auth.js\"")) {
        d := Text.replace(d, #text "</body>", "<script src=\"/mv-auth.js\" defer></script></body>");
      };
      d;
    };

    func defaultDocument(head : Head, wrapped : Text) : Text {
      "<!DOCTYPE html><html lang=\"en\"><head><meta charset=\"utf-8\">"
      # "<meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">"
      # "<title>" # Html.escape(head.title) # "</title>"
      # (if (head.description != "") { "<meta name=\"description\" content=\"" # Html.escape(head.description) # "\">" } else { "" })
      # (if (head.canonical != "") { "<link rel=\"canonical\" href=\"" # Html.escape(head.canonical) # "\">" } else { "" })
      # head.extra
      # "<link rel=\"icon\" href=\"/favicon.svg\">"
      # "<link rel=\"stylesheet\" href=\"/motoview.css\">"
      # "</head><body>"
      # wrapped
      # "<script src=\"/motoview.js\" defer></script></body></html>";
    };

    func notFoundDoc() : Text {
      defaultDocument(
        { title = "Not found"; description = ""; canonical = ""; extra = "" },
        "<div id=\"mv-root\" data-mv-root><main class=\"mv-container\"><h1>404</h1><p>This page could not be found.</p><p><a href=\"/\">Go home</a></p></main></div>",
      );
    };

    func redirectDoc(location : Text) : HttpResponse {
      {
        status_code = 302;
        headers = [("location", location), ("content-type", "text/html")];
        body = Text.encodeUtf8("<a href=\"" # location # "\">" # location # "</a>");
        upgrade = null;
      };
    };

    // Where an unauthorized caller is sent for a given page: the page's
    // `@authorize redirect="..."` target, or "/" when unset. Letting a route
    // that IS the login target (or "/") gate itself without a redirect loop.
    func authTarget(page : Page) : Text {
      if (page.authRedirect == "") { "/" } else { page.authRedirect };
    };

    // True if `handler` is bound to a `secure` form on `page` — verification is
    // then mandatory (the request cannot opt out by omitting `__mv_secure`).
    func isSecureHandler(page : Page, handler : Text) : Bool {
      for (h in page.secureHandlers.vals()) { if (h == handler) { return true } };
      false;
    };

    // ---- batches ----------------------------------------------------------

    func redirectBatch(location : Text) : Batch {
      {
        status = #redirect;
        batchId = "";
        html = "";
        ui = null;
        head = Types.emptyHead();
        effects = [];
        target = "mv-root";
        location;
        errors = [];
      };
    };

    func securityErrorBatch(reason : Text) : Batch {
      {
        status = #validationError;
        batchId = "";
        html = "<div class=\"mv-alert mv-alert-danger\">Security check failed: " # Html.escape(reason) # ". Please reload the page and try again.</div>";
        ui = null;
        head = Types.emptyHead();
        effects = [{ kind = "toast"; target = "Security check failed"; value = "" }];
        target = "mv-root";
        location = "";
        errors = [];
      };
    };

    // ---- context ----------------------------------------------------------

    func makeCtx(
      method : Text,
      path : Text,
      qp : [(Text, Text)],
      params : [(Text, Text)],
      form : [(Text, Text)],
      caller : Principal,
      last : Text,
    ) : Ctx {
      // Ordinary secure-form token: no server-known intent, so intentHash="".
      // We honestly do NOT bind the not-yet-typed form values (impossible at
      // render time); the token binds route/handler/caller/schema/expiry/nonce.
      let mint = func(handler : Text, schema : Text) : Text {
        counter += 1;
        let nonce = Int.toText(Time.now()) # "-" # Nat.toText(counter);
        Security.mint(
          mvSecret,
          path,
          handler,
          Principal.toText(caller),
          Time.now() + expiryNs,
          nonce,
          Security.schemaHash(schema),
          "",
        );
      };
      // Intent-bound token for confirmation flows (e.g. a future wallet confirm
      // step, Slice 9): binds a value the server ALREADY KNOWS at mint time by
      // hashing its canonical intent into the token. verify() then rejects any
      // submission whose intent does not match.
      let mintIntent = func(handler : Text, schema : Text, intent : [(Text, Text)]) : Text {
        counter += 1;
        let nonce = Int.toText(Time.now()) # "-" # Nat.toText(counter);
        let ih = Security.intentHash(Security.canonicalIntent(intent));
        Security.mint(
          mvSecret,
          path,
          handler,
          Principal.toText(caller),
          Time.now() + expiryNs,
          nonce,
          Security.schemaHash(schema),
          ih,
        );
      };
      {
        method;
        path;
        queryParams = qp;
        params;
        form;
        caller;
        isAuthenticated = not Principal.isAnonymous(caller);
        lastBatchId = last;
        mintToken = mint;
        mintIntentToken = mintIntent;
        authorizeSpend = func(handler : Text, intent : [(Text, Text)], token : Text, weight : Nat) : Bool {
          authorizeSpend(handler, intent, token, weight, path, caller);
        };
        mintSpendToken = func(handler : Text, intent : [(Text, Text)]) : Text {
          counter += 1;
          let nonce = Int.toText(Time.now()) # "-" # Nat.toText(counter);
          WalletAuth.mintSpendToken(mvSecret, path, handler, caller, Time.now() + expiryNs, nonce, intent);
        };
        hasRole = hasRole;
        callerRoles = func() : [Text] { roleStore.rolesOf(caller) };
        grantRole = grantRole;
        revokeRole = revokeRole;
        claimRole = func(role : Text) : Bool { claimRole(caller, role) };
      };
    };

    func authorized(page : Page, caller : Principal) : Bool {
      if (not page.authorize) { return true };
      if (Principal.isAnonymous(caller)) { return false };
      // `@authorize role="X"` additionally requires the caller to hold role X.
      if (page.role == "") { return true };
      hasRole(caller, page.role);
    };

    func collectArgs(form : [(Text, Text)]) : [Text] {
      let out = Buffer.Buffer<Text>(2);
      var i = 0;
      var go = true;
      while (go) {
        switch (Url.get(form, "__mv_arg" # Nat.toText(i))) {
          case (?v) { out.add(v); i += 1 };
          case null { go := false };
        };
      };
      Buffer.toArray(out);
    };

    // ---- assets & SEO -----------------------------------------------------

    func asset(path : Text) : ?HttpResponse {
      if (path == "/motoview.js") { return ?textResp(assets.clientJs, "text/javascript") };
      if (path == "/mv-auth.js") { return ?textResp(assets.authJs, "text/javascript") };
      if (path == "/motoview.wasm") { return ?blobResp(assets.clientWasm, "application/wasm") };
      if (path == "/motoview-crypto.wasm") { return ?blobResp(assets.cryptoWasm, "application/wasm") };
      if (path == "/motoview.css") { return ?textResp(assets.css, "text/css") };
      if (path == "/favicon.svg" or path == "/favicon.ico") { return ?textResp(assets.favicon, "image/svg+xml") };
      if (path == "/robots.txt") { return ?textResp(robots(), "text/plain") };
      if (path == "/sitemap.xml") { return ?textResp(sitemap(), "application/xml") };
      // PWA: makes every MotoView app installable on desktop & mobile.
      if (path == "/manifest.webmanifest") { return ?textResp(manifest(), "application/manifest+json") };
      if (path == "/sw.js") { return ?textResp(serviceWorker(), "text/javascript") };
      null;
    };

    /// A web app manifest derived from the app name (PWA installability).
    func manifest() : Text {
      "{\"name\":\"" # config.appName # "\",\"short_name\":\"" # config.appName
      # "\",\"start_url\":\"/\",\"scope\":\"/\",\"display\":\"standalone\",\"orientation\":\"any\""
      # ",\"background_color\":\"#ffffff\",\"theme_color\":\"#6d28d9\""
      # ",\"icons\":[{\"src\":\"/favicon.svg\",\"sizes\":\"any\",\"type\":\"image/svg+xml\",\"purpose\":\"any maskable\"}]}";
    };

    /// An offline-first service worker. On install it precaches the app shell
    /// (the WASM client + CSS + auth glue + icon), so the app loads with no
    /// network. At runtime: framework assets use stale-while-revalidate (cache + background
    /// refresh, so redeploys propagate); page
    /// navigations are network-first with a cache fallback (so a page you've
    /// visited still opens offline, showing its last-seen content); the live
    /// protocol/session endpoints (`/_motoview/*`, `/mv-session`, …) are never
    /// cached. A bumped cache name + activate cleanup retires old versions.
    func serviceWorker() : Text {
      "var C='motoview-v4';\n"
      # "var SHELL=['/motoview.js','/motoview.wasm','/motoview.css','/mv-auth.js','/favicon.svg','/manifest.webmanifest'];\n"
      # "function dyn(p){return p.indexOf('/_motoview/')===0||p==='/mv-session'||p==='/mv-login-begin'||p==='/mv-whoami'||p==='/mv-logout';}\n"
      # "self.addEventListener('install',function(e){self.skipWaiting();e.waitUntil(caches.open(C).then(function(c){return c.addAll(SHELL).catch(function(){});}));});\n"
      # "self.addEventListener('activate',function(e){e.waitUntil(caches.keys().then(function(ks){return Promise.all(ks.map(function(k){if(k!==C){return caches.delete(k);}}));}).then(function(){return self.clients.claim();}));});\n"
      // Stale-while-revalidate for shell assets: serve cache instantly (fast +
      // offline) AND refresh the cache from the network in the background, so a
      // redeploy's new CSS/JS/wasm propagates on the next load (no manual cache bump).
      # "function cacheFirst(req){return caches.open(C).then(function(c){return c.match(req).then(function(h){var f=fetch(req).then(function(r){if(r&&r.ok){c.put(req,r.clone());}return r;}).catch(function(){return h;});return h||f;});});}\n"
      # "function netFirst(req){return caches.open(C).then(function(c){return fetch(req).then(function(r){if(r&&r.ok){c.put(req,r.clone());}return r;}).catch(function(){return c.match(req).then(function(h){return h||c.match('/');});});});}\n"
      # "self.addEventListener('fetch',function(e){var req=e.request;if(req.method!=='GET'){return;}var u=new URL(req.url);if(u.origin!==self.location.origin){return;}var p=u.pathname;if(dyn(p)){return;}"
      # "if(SHELL.indexOf(p)!==-1){e.respondWith(cacheFirst(req));return;}"
      # "if(req.mode==='navigate'||(req.headers.get('accept')||'').indexOf('text/html')!==-1){e.respondWith(netFirst(req));return;}"
      # "e.respondWith(cacheFirst(req));});\n";
    };

    func robots() : Text {
      "User-agent: *\nAllow: /\nSitemap: /sitemap.xml\n";
    };

    func sitemap() : Text {
      let b = Buffer.Buffer<Text>(pages.size());
      b.add("<?xml version=\"1.0\" encoding=\"UTF-8\"?><urlset xmlns=\"http://www.sitemaps.org/schemas/sitemap/0.9\">");
      for (p in pages.vals()) {
        if (not Text.contains(p.route, #char '{') and not p.authorize) {
          b.add("<url><loc>" # p.route # "</loc></url>");
        };
      };
      b.add("</urlset>");
      Text.join("", b.vals());
    };

    // ---- HTTP response helpers --------------------------------------------

    func headers(contentType : Text) : [(Text, Text)] {
      [("content-type", contentType), ("cache-control", "no-store")];
    };

    func jsonHeaders() : [(Text, Text)] {
      [("content-type", "application/json"), ("cache-control", "no-store")];
    };

    func textResp(body : Text, contentType : Text) : HttpResponse {
      { status_code = 200; headers = headers(contentType); body = Text.encodeUtf8(body); upgrade = null };
    };

    func blobResp(body : Blob, contentType : Text) : HttpResponse {
      { status_code = 200; headers = headers(contentType); body; upgrade = null };
    };

    func htmlResp(status : Nat16, body : Text) : HttpResponse {
      { status_code = status; headers = headers("text/html; charset=utf-8"); body = Text.encodeUtf8(body); upgrade = null };
    };

    func jsonResp(body : Text) : HttpResponse {
      { status_code = 200; headers = jsonHeaders(); body = Text.encodeUtf8(body); upgrade = null };
    };
  };
};
