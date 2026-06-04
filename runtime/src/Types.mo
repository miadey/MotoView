/// MotoView runtime — core types.
///
/// These types form the contract between:
///   * the `motoview` compiler (which generates Motoko `Page` values), and
///   * the runtime (which routes HTTP, renders pages, and dispatches events).
///
/// Rendering is a query. Events are updates. The browser synchronizes through
/// versioned UI batches identified by `batchId`.

module {

  // ---- IC HTTP gateway interface ----------------------------------------

  public type HeaderField = (Text, Text);

  /// Incoming HTTP request, as delivered by the IC HTTP gateway.
  public type HttpRequest = {
    method : Text;
    url : Text;
    headers : [HeaderField];
    body : Blob;
    certificate_version : ?Nat16;
  };

  /// HTTP response. When `upgrade = ?true`, the gateway re-issues the request
  /// as an update call to `http_request_update` (used for events / mutations).
  public type HttpResponse = {
    status_code : Nat16;
    headers : [HeaderField];
    body : Blob;
    upgrade : ?Bool;
  };

  // ---- Render context ----------------------------------------------------

  /// Everything a page needs to render or handle an event.
  public type Ctx = {
    method : Text; // "GET" | "POST"
    path : Text; // clean path, e.g. "/products/42"
    queryParams : [(Text, Text)]; // parsed query string
    params : [(Text, Text)]; // route params, e.g. ("id", "42")
    form : [(Text, Text)]; // submitted form fields (events only)
    caller : Principal; // msg.caller (anonymous for query renders)
    isAuthenticated : Bool; // caller is not the anonymous principal
    lastBatchId : Text; // batchId the client currently displays
    // Mint a signed secure-form token bound to (path, handler, caller, schema).
    // Generated render code calls this for `<form secure>`. No-op ("") outside
    // a render that needs it.
    mintToken : (handler : Text, schema : Text) -> Text;
  };

  // ---- View model returned by generated render code ----------------------

  /// SEO / document head data produced by a page.
  public type Head = {
    title : Text;
    description : Text;
    canonical : Text;
    extra : Text; // raw HTML for <head> (og tags, @section "head", ...)
  };

  public func emptyHead() : Head {
    { title = ""; description = ""; canonical = ""; extra = "" };
  };

  /// A declarative client effect (focus, scroll, toast, animate, ...).
  public type Effect = {
    kind : Text; // "focus" | "scrollTo" | "toast" | "animate"
    target : Text; // selector or message
    value : Text; // optional extra (animation name, etc.)
  };

  // ---- The render batch (wire protocol: motoview/1) ----------------------

  public type BatchStatus = {
    #changed;
    #unchanged;
    #redirect;
    #validationError;
  };

  /// A versioned UI batch. The full-container-replace strategy (MVP) carries
  /// the rendered `html` plus head + effects. Unchanged batches omit html.
  public type Batch = {
    status : BatchStatus;
    batchId : Text;
    html : Text;
    head : Head;
    effects : [Effect];
    target : Text; // DOM id to replace; "app" by default
    location : Text; // redirect destination
    errors : [(Text, Text)]; // validation errors: field -> message
  };

  // ---- The Page interface implemented by generated modules ---------------

  /// A compiled `.mview` page. The compiler emits one `Page` value per page.
  /// All functions are synchronous: in the MVP, handler logic runs locally in
  /// the canister (no inter-canister awaits), which keeps `render` query-safe.
  public type Page = {
    route : Text; // route pattern, e.g. "/products/{id}" or "/products/{id:Nat}"
    layout : Text; // layout name, or "" for none
    authorize : Bool; // requires an authenticated caller
    role : Text; // required role, or "" for any authenticated caller
    onLoad : (Ctx) -> (); // data-loading lifecycle (runs on GET renders)
    render : (Ctx) -> Text; // inner HTML of the page body
    title : (Ctx) -> Text; // document title
    head : (Ctx) -> Head; // full head data
    dispatch : (Ctx, Text, [Text]) -> (); // (ctx, handlerId, args) -> mutate state
    // Validation errors produced during the last dispatch (field -> message);
    // returns and clears the page's error buffer.
    takeErrors : () -> [(Text, Text)];
    // Pending client-side navigation requested via `Navigation.go(path)`;
    // returns and clears it ("" if none).
    takeRedirect : () -> Text;
    // Declarative effects (toast/animate/focus/scrollTo) emitted by the last
    // dispatch; returns and clears the page's effect buffer.
    takeEffects : () -> [Effect];
  };

  /// A compiled layout. `render` receives the page's inner HTML (`body`) plus
  /// the page head, and returns the full document HTML.
  public type Layout = {
    name : Text;
    render : (Ctx, Head, Text) -> Text;
  };

  // ---- Runtime configuration ---------------------------------------------

  public type Config = {
    appName : Text;
    secret : Blob; // server secret for signing secure-form tokens
    seo : Bool;
    // Extra origins trusted as an Internet Identity derivationOrigin, served at
    // /.well-known/ii-alternative-origins (for a stable principal across domains).
    altOrigins : [Text];
  };

  /// Static client assets served by the canister. Populated by the runtime's
  /// `ClientAssets` module (the Rust→WASM bridge + its JS bootstrap + CSS).
  public type Assets = {
    clientJs : Text; // JS bootstrap/glue that loads the wasm bridge
    authJs : Text; // hand-written Internet Identity login (served at /mv-auth.js)
    clientWasm : Blob; // the compiled Rust→WASM client ("the brain")
    css : Text; // bridge + base theme CSS
    favicon : Text; // SVG favicon markup
  };

};
