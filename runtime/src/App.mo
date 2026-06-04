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
    // replay-protection state stays bounded.
    let consumed = HashMap.HashMap<Text, Int>(256, Text.equal, Text.hash);

    func consumeNonce(nonce : Text) {
      consumed.put(nonce, Time.now() + expiryNs);
      if (consumed.size() > 4096) {
        let now = Time.now();
        let dead = Buffer.Buffer<Text>(64);
        for ((k, exp) in consumed.entries()) { if (exp < now) { dead.add(k) } };
        for (k in dead.vals()) { consumed.delete(k) };
      };
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

    func sessionMac(body : Text) : Text {
      Hex.encode(Sha256.hmac(mvSecret, Text.encodeUtf8(body)));
    };

    func mintSession(p : Principal) : Text {
      let body = Principal.toText(p) # "." # Int.toText(Time.now() + sessionTtlNs);
      body # "." # sessionMac(body);
    };

    func verifySession(token : Text) : ?Principal {
      let parts = Iter.toArray(Text.split(token, #char '.'));
      if (parts.size() != 3) { return null };
      let body = parts[0] # "." # parts[1];
      if (sessionMac(body) != parts[2]) { return null };
      switch (parseNat(parts[1])) {
        case (?exp) { if (Time.now() > exp) { return null } };
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
    func effectiveCaller(req : HttpRequest, fallback : Principal) : Principal {
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
    public func httpRequest(_ : HttpRequest, _ : Principal) : HttpResponse {
      { status_code = 200; headers = jsonHeaders(); body = ""; upgrade = ?true };
    };

    /// Update entry point. Serves assets, SSR pages, render polls and events.
    public func httpRequestUpdate(req : HttpRequest, caller : Principal) : HttpResponse {
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
      // Clear the session cookie.
      if (path == "/mv-logout") { return respCookies("ok", [cookie("mv_session", "", 0, "Lax")]) };

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
          if (not authorized(page, caller)) { return redirectDoc() };
          let ctx = makeCtx("GET", path, qp, params, [], caller, "");
          page.onLoad(ctx);
          ignore page.takeRedirect(); // ignore navigations during render
          let head = page.head(ctx);
          let inner = page.render(ctx);
          let bid = batchIdFor(path, head.title, inner);
          return htmlResp(200, renderDocument(page, ctx, head, inner, bid));
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
          if (not authorized(page, caller)) { return jsonResp(Json.encodeBatch(redirectBatch("/"))) };

          // secure form verification + replay protection
          if (Url.getOr(form, "__mv_secure", "") == "1") {
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
          if (not authorized(page, caller)) { return redirectBatch("/") };
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
      if (not Text.contains(d, #text "motoview.css")) {
        d := Text.replace(d, #text "</head>", "<link rel=\"stylesheet\" href=\"/motoview.css\"></head>");
      };
      if (not Text.contains(d, #text "motoview.js")) {
        d := Text.replace(d, #text "</body>", "<script src=\"/motoview.js\" defer></script></body>");
      };
      // Internet Identity login, available to every app at /mv-auth.js. It is a
      // no-op unless the page has a [data-mv-signin] element to wire.
      if (not Text.contains(d, #text "mv-auth.js")) {
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

    func redirectDoc() : HttpResponse {
      {
        status_code = 302;
        headers = [("location", "/"), ("content-type", "text/html")];
        body = Text.encodeUtf8("<a href=\"/\">/</a>");
        upgrade = null;
      };
    };

    // ---- batches ----------------------------------------------------------

    func redirectBatch(location : Text) : Batch {
      {
        status = #redirect;
        batchId = "";
        html = "";
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
      };
    };

    func authorized(page : Page, caller : Principal) : Bool {
      if (not page.authorize) { return true };
      not Principal.isAnonymous(caller);
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

    /// A minimal service worker: cache-first for the static framework assets
    /// (offline shell + fast loads); everything else falls through to network.
    func serviceWorker() : Text {
      "self.addEventListener('install',function(e){self.skipWaiting();});\n"
      # "self.addEventListener('activate',function(e){e.waitUntil(self.clients.claim());});\n"
      # "self.addEventListener('fetch',function(e){var p=new URL(e.request.url).pathname;"
      # "if(p==='/motoview.js'||p==='/motoview.wasm'||p==='/motoview.css'||p==='/favicon.svg'||p==='/manifest.webmanifest'){"
      # "e.respondWith(caches.open('motoview-v1').then(function(c){return c.match(e.request).then(function(r){"
      # "return r||fetch(e.request).then(function(resp){c.put(e.request,resp.clone());return resp;});});}));}});\n";
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
