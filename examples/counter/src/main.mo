// GENERATED-STYLE actor for the MotoView "counter" example.
//
// This is hand-written to match exactly what the `motoview` compiler emits
// from `src/Pages/Counter.mview` + `src/Layouts/MainLayout.mview`. It exists so
// the runtime + client + dfx pipeline can be exercised before/independently of
// the compiler. `motoview build` regenerates an equivalent file.
import App "mo:motoview/App";
import Html "mo:motoview/Html";
import Types "mo:motoview/Types";
import MV "mo:motoview";
import Nat "mo:base/Nat";
import Nat32 "mo:base/Nat32";
import Char "mo:base/Char";
import Text "mo:base/Text";
import Buffer "mo:base/Buffer";

actor {
  type Ctx = Types.Ctx;
  type Head = Types.Head;

  // ===== Page: Counter ("/") ================================================
  let CounterPage = object page {
    var count : Nat = 0;
    let errors = Buffer.Buffer<(Text, Text)>(0);
    var redirect : Text = "";

    public func onLoad(_ : Ctx) {};

    public func render(_ : Ctx) : Text {
      let b = Html.Builder();
      b.raw("<section class=\"mv-container\"><h1>Counter</h1>");
      b.raw("<p class=\"counter-value\">Current value: <strong>");
      b.text(Nat.toText(count));
      b.raw("</strong></p>");
      b.raw("<div class=\"counter-actions\">");
      b.raw("<button class=\"mv-btn mv-btn-primary\" data-mv-handler=\"increment\" data-mv-event=\"click\" data-mv-arg0=\"1\">+1</button>");
      b.raw("<button class=\"mv-btn mv-btn-primary\" data-mv-handler=\"increment\" data-mv-event=\"click\" data-mv-arg0=\"5\">+5</button>");
      b.raw("<button class=\"mv-btn mv-btn-secondary\" data-mv-handler=\"decrement\" data-mv-event=\"click\">-1</button>");
      b.raw("<button class=\"mv-btn mv-btn-ghost\" data-mv-handler=\"reset\" data-mv-event=\"click\">Reset</button>");
      b.raw("</div></section>");
      b.build();
    };

    public func title(_ : Ctx) : Text { "Counter" };

    public func dispatch(_ : Ctx, handler : Text, args : [Text]) {
      switch handler {
        case "increment" {
          let by = if (args.size() > 0) { natOf(args[0]) } else { 1 };
          count += by;
        };
        case "decrement" { if (count > 0) { count -= 1 } };
        case "reset" { count := 0 };
        case _ {};
      };
    };

    public func takeErrors() : [(Text, Text)] {
      let e = Buffer.toArray(errors);
      errors.clear();
      e;
    };

    public func takeRedirect() : Text {
      let r = redirect;
      redirect := "";
      r;
    };
  };

  func natOf(t : Text) : Nat {
    var n : Nat = 0;
    for (c in t.chars()) {
      let code = Char.toNat32(c);
      if (code >= 48 and code <= 57) {
        n := n * 10 + Nat32.toNat(code - 48);
      };
    };
    n;
  };

  func counterHead(ctx : Ctx) : Head {
    { title = CounterPage.title(ctx); description = "A live counter built with MotoView — rendering is a query, the +/- clicks are updates."; canonical = ""; extra = "" };
  };

  let counter : Types.Page = {
    route = "/";
    layout = "MainLayout";
    authorize = false;
    role = "";
    onLoad = CounterPage.onLoad;
    render = CounterPage.render;
    title = CounterPage.title;
    head = counterHead;
    dispatch = CounterPage.dispatch;
    takeErrors = CounterPage.takeErrors;
    takeRedirect = CounterPage.takeRedirect;
  };

  // ===== Layout: MainLayout =================================================
  func mainLayout(_ : Ctx, head : Head, body : Text) : Text {
    "<!DOCTYPE html><html lang=\"en\"><head><meta charset=\"utf-8\">"
    # "<meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">"
    # "<title>" # Html.escape(head.title) # "</title>"
    # (if (head.description != "") { "<meta name=\"description\" content=\"" # Html.escape(head.description) # "\">" } else { "" })
    # head.extra
    # "<style>.counter-value{font-size:1.4rem}.counter-actions{display:flex;gap:.5rem;flex-wrap:wrap}header.mv-nav{display:flex;gap:1rem;align-items:center;border-bottom:1px solid var(--mv-border)}</style>"
    # "</head><body>"
    # "<header class=\"mv-nav mv-container\" style=\"padding-top:1rem;padding-bottom:1rem\"><strong>▼ MotoView</strong><span style=\"color:var(--mv-text-soft)\">counter example</span></header>"
    # "<main>" # body # "</main>"
    # "<footer class=\"mv-container\" style=\"color:var(--mv-text-soft);font-size:.9rem\">Powered by MotoView on the Internet Computer · rendering is a query, events are updates.</footer>"
    # "</body></html>";
  };

  // ===== App wiring =========================================================
  let pages : [Types.Page] = [counter];
  let layouts : [Types.Layout] = [{ name = "MainLayout"; render = mainLayout }];

  let config : Types.Config = {
    appName = "Counter";
    secret = "\6d\6f\74\6f\76\69\65\77\2d\63\6f\75\6e\74\65\72\2d\64\65\76\2d\73\65\63\72\65\74\2d\30\31\32\33" : Blob;
    seo = true;
  };

  let app = App.App(config, pages, layouts, MV.defaultAssets());

  public shared query (msg) func http_request(req : Types.HttpRequest) : async Types.HttpResponse {
    app.httpRequest(req, msg.caller);
  };

  public shared (msg) func http_request_update(req : Types.HttpRequest) : async Types.HttpResponse {
    app.httpRequestUpdate(req, msg.caller);
  };
};
