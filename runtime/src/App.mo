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
import Int "mo:base/Int";
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
      if (Text.startsWith(path, #text "/_motoview/event")) {
        serveEvent(req, caller);
      } else {
        serveGet(req, caller);
      };
    };

    // ---- request handling -------------------------------------------------

    /// Serve assets, SSR documents, and render polls.
    func serveGet(req : HttpRequest, caller : Principal) : HttpResponse {
      let (path, q) = Url.splitUrl(req.url);
      let qp = Url.parsePairs(q);

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
                config.secret,
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
          config.secret,
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
      if (path == "/motoview.wasm") { return ?blobResp(assets.clientWasm, "application/wasm") };
      if (path == "/motoview.css") { return ?textResp(assets.css, "text/css") };
      if (path == "/favicon.svg" or path == "/favicon.ico") { return ?textResp(assets.favicon, "image/svg+xml") };
      if (path == "/robots.txt") { return ?textResp(robots(), "text/plain") };
      if (path == "/sitemap.xml") { return ?textResp(sitemap(), "application/xml") };
      null;
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
