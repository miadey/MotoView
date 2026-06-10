// Unit test for the portable UI-IR (Ir.mo). Run:
//   moc -r --package base <base> runtime/test/IrTest.mo
import Ir "../src/Ir";
import Debug "mo:base/Debug";

func check(name : Text, got : Text, want : Text) {
  if (got != want) {
    Debug.print("FAIL " # name);
    Debug.print("  want: " # want);
    Debug.print("  got:  " # got);
    assert false;
  } else {
    Debug.print("ok   " # name);
  };
};

// ---- 1. leaf serialization + escaping -------------------------------------
// A #text leaf is escaped on serialize (quotes, backslash, newline, tab).
check(
  "text-escape",
  Ir.toJson(#text("a \"q\" & <b>\n\tend")),
  "{\"t\":\"text\",\"value\":\"a \\\"q\\\" & <b>\\n\\tend\"}",
);

// A #raw leaf carries literal HTML verbatim (only JSON-escaped for the wire).
check(
  "raw-leaf",
  Ir.toJson(#raw("<button class=\"x\">Go</button>")),
  "{\"t\":\"raw\",\"html\":\"<button class=\\\"x\\\">Go</button>\"}",
);

// ---- 2. a hand-built element with attrs/events/key ------------------------
let li : Ir.UINode = #element({
  tag = "li";
  attrs = [("class", "row")];
  events = [];
  key = ?"row-7";
  children = [
    #element({
      tag = "button";
      attrs = [("data-mv-arg0", "7")];
      events = [("click", "pick")];
      key = null;
      children = [#text("Pick")];
    }),
  ];
});

check(
  "element-tree",
  Ir.toJson(li),
  "{\"t\":\"el\",\"tag\":\"li\",\"attrs\":{\"class\":\"row\"},\"events\":{},\"key\":\"row-7\",\"children\":["
  # "{\"t\":\"el\",\"tag\":\"button\",\"attrs\":{\"data-mv-arg0\":\"7\"},\"events\":{\"click\":\"pick\"},\"children\":["
  # "{\"t\":\"text\",\"value\":\"Pick\"}]}]}",
);

// `key` is omitted entirely when null (the button above has no "key" field).
check(
  "key-omitted-when-null",
  Ir.toJson(#element({ tag = "p"; attrs = []; events = []; key = null; children = [] })),
  "{\"t\":\"el\",\"tag\":\"p\",\"attrs\":{},\"events\":{},\"children\":[]}",
);

// ---- 3. the Builder produces the same tree the compiler would -------------
// Mirrors what generated IR codegen emits for:
//   <section><h1>Hi @name</h1><li key="@it"><button @click="pick(it)">@it</button></li></section>
let b = Ir.Builder();
b.open("section");
b.open("h1");
b.raw("Hi ");           // static template text -> #raw leaf
b.text("Ada & Co");     // dynamic text -> escaped #text leaf
b.close();              // </h1>
b.open("li");
b.key("it-1");
b.open("button");
b.event("click", "pick");
b.attr("data-mv-arg0", "it-1");
b.text("it-1");
b.close();              // </button>
b.close();              // </li>
b.close();              // </section>

let forest = b.build();
check("forest-size", debug_show (forest.size()), "1");

let json = Ir.toJsonForest(forest);
let expected =
  "[{\"t\":\"el\",\"tag\":\"section\",\"attrs\":{},\"events\":{},\"children\":["
  # "{\"t\":\"el\",\"tag\":\"h1\",\"attrs\":{},\"events\":{},\"children\":["
  # "{\"t\":\"raw\",\"html\":\"Hi \"},"
  # "{\"t\":\"text\",\"value\":\"Ada & Co\"}]},"
  # "{\"t\":\"el\",\"tag\":\"li\",\"attrs\":{},\"events\":{},\"key\":\"it-1\",\"children\":["
  # "{\"t\":\"el\",\"tag\":\"button\",\"attrs\":{\"data-mv-arg0\":\"it-1\"},\"events\":{\"click\":\"pick\"},\"children\":["
  # "{\"t\":\"text\",\"value\":\"it-1\"}]}]}]}]";
check("builder-forest-json", json, expected);

// The Builder's toJson() convenience equals toJsonForest(build()).
let b2 = Ir.Builder();
b2.open("div");
b2.text("hi");
b2.close();
check("builder-tojson", b2.toJson(), "[{\"t\":\"el\",\"tag\":\"div\",\"attrs\":{},\"events\":{},\"children\":[{\"t\":\"text\",\"value\":\"hi\"}]}]");

Debug.print("ALL IR TESTS PASSED");
