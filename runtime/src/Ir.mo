/// Portable UI-IR: a JSON node tree the compiler can emit as a SECOND backend
/// (alongside HTML), so the SAME `.mview` source can later drive native
/// SwiftUI / Compose renderers without re-authoring the views.
///
/// The compiler emits Motoko that builds a `UINode` tree via `Ir.Builder`,
/// mirroring the ergonomics of `Html.Builder`:
///
///   let ir = Ir.Builder();
///   ir.open("h1");              // begin an element
///   ir.attr("class", "title"); // add an attribute (value escaped on serialize)
///   ir.event("click", "save"); // add an event (eventName -> handlerId/marker)
///   ir.key("row-7");           // mark a keyed region
///   ir.text(userName);         // add escaped dynamic text
///   ir.close();                // close the current element
///   let json = Ir.toJson(ir.build());
///
/// Builtins/charts that are not yet IR-modeled fall back to `ir.raw(html)`,
/// which wraps the EXACT HTML the HTML backend would emit as a `#raw` leaf —
/// so the output is never wrong, only not-yet-native.
import Text "mo:base/Text";
import Buffer "mo:base/Buffer";

module {

  /// A portable UI node. `#raw` is the honest fallback: literal HTML a native
  /// renderer can show in a webview slot until the node kind is modeled.
  public type UINode = {
    #element : {
      tag : Text;
      attrs : [(Text, Text)];
      events : [(Text, Text)]; // (eventName, handlerId)
      key : ?Text; // keyed-region key, or null
      children : [UINode];
    };
    #text : Text; // dynamic text (escaped on serialize)
    #raw : Text; // fallback: literal HTML
  };

  // ---- JSON serialization -----------------------------------------------

  /// Escape a Text as a JSON string body (without surrounding quotes). Matches
  /// `Json.escape` so the IR rides the same wire conventions as the batch.
  public func escape(t : Text) : Text {
    var s = t;
    s := Text.replace(s, #char '\\', "\\\\");
    s := Text.replace(s, #char '\"', "\\\"");
    s := Text.replace(s, #char '\n', "\\n");
    s := Text.replace(s, #char '\r', "\\r");
    s := Text.replace(s, #char '\t', "\\t");
    s;
  };

  func str(t : Text) : Text { "\"" # escape(t) # "\"" };

  func pairsJson(pairs : [(Text, Text)]) : Text {
    let parts = Buffer.Buffer<Text>(pairs.size());
    for ((k, v) in pairs.vals()) { parts.add(str(k) # ":" # str(v)) };
    "{" # Text.join(",", parts.vals()) # "}";
  };

  /// Serialize a `UINode` to compact JSON.
  ///   element -> {"t":"el","tag":..,"attrs":{..},"events":{..},"key":..,"children":[..]}
  ///   text    -> {"t":"text","value":".."}
  ///   raw     -> {"t":"raw","html":".."}
  /// `key` is omitted when null.
  public func toJson(node : UINode) : Text {
    switch (node) {
      case (#text t) { "{\"t\":\"text\",\"value\":" # str(t) # "}" };
      case (#raw h) { "{\"t\":\"raw\",\"html\":" # str(h) # "}" };
      case (#element e) {
        let kids = Buffer.Buffer<Text>(e.children.size());
        for (c in e.children.vals()) { kids.add(toJson(c)) };
        let keyField = switch (e.key) {
          case (?k) { ",\"key\":" # str(k) };
          case null { "" };
        };
        "{\"t\":\"el\",\"tag\":" # str(e.tag)
        # ",\"attrs\":" # pairsJson(e.attrs)
        # ",\"events\":" # pairsJson(e.events)
        # keyField
        # ",\"children\":[" # Text.join(",", kids.vals()) # "]}";
      };
    };
  };

  /// Serialize a forest (the page body is a list of top-level nodes) to a JSON
  /// array. This is what the runtime stores in `Batch.ui`.
  public func toJsonForest(nodes : [UINode]) : Text {
    let parts = Buffer.Buffer<Text>(nodes.size());
    for (n in nodes.vals()) { parts.add(toJson(n)) };
    "[" # Text.join(",", parts.vals()) # "]";
  };

  // ---- builder ----------------------------------------------------------

  // A mutable element under construction: its parts accumulate as the
  // generated code calls attr/event/key/text/raw, and a child list grows as
  // nested elements open and close.
  type Frame = {
    tag : Text;
    attrs : Buffer.Buffer<(Text, Text)>;
    events : Buffer.Buffer<(Text, Text)>;
    var key : ?Text;
    children : Buffer.Buffer<UINode>;
  };

  func newFrame(tag : Text) : Frame {
    {
      tag = tag;
      attrs = Buffer.Buffer<(Text, Text)>(8);
      events = Buffer.Buffer<(Text, Text)>(2);
      var key = null;
      children = Buffer.Buffer<UINode>(8);
    };
  };

  func freeze(f : Frame) : UINode {
    #element {
      tag = f.tag;
      attrs = Buffer.toArray(f.attrs);
      events = Buffer.toArray(f.events);
      key = f.key;
      children = Buffer.toArray(f.children);
    };
  };

  /// A tree builder mirroring `Html.Builder`. Maintains a stack of open
  /// elements; leaves (text/raw) and closed elements attach to the parent on
  /// top of the stack, or to the root forest when the stack is empty.
  public class Builder() {
    let roots = Buffer.Buffer<UINode>(16);
    let stack = Buffer.Buffer<Frame>(16);

    func emit(node : UINode) {
      if (stack.size() == 0) { roots.add(node) } else {
        stack.get(stack.size() - 1 : Nat).children.add(node);
      };
    };

    /// Begin an element; subsequent attr/event/key/text/raw apply to it until
    /// the matching `close()`.
    public func open(tag : Text) { stack.add(newFrame(tag)) };

    /// Close the current element and attach it to its parent (or the root).
    public func close() {
      if (stack.size() == 0) { return };
      let f = stack.remove(stack.size() - 1 : Nat);
      emit(freeze(f));
    };

    /// Add an attribute to the current element (value escaped on serialize).
    public func attr(name : Text, value : Text) {
      if (stack.size() == 0) { return };
      stack.get(stack.size() - 1 : Nat).attrs.add((name, value));
    };

    /// Add an event (eventName -> handlerId) to the current element.
    public func event(name : Text, handler : Text) {
      if (stack.size() == 0) { return };
      stack.get(stack.size() - 1 : Nat).events.add((name, handler));
    };

    /// Mark the current element as a keyed region.
    public func key(k : Text) {
      if (stack.size() == 0) { return };
      stack.get(stack.size() - 1 : Nat).key := ?k;
    };

    /// Add escaped dynamic text as a child of the current element (or a root).
    public func text(t : Text) { emit(#text t) };

    /// Add a raw-HTML fallback leaf (used for not-yet-IR-modeled builtins).
    public func raw(t : Text) { emit(#raw t) };

    /// Return the built forest (closing any still-open elements defensively).
    public func build() : [UINode] {
      while (stack.size() > 0) { close() };
      Buffer.toArray(roots);
    };

    /// Convenience: build + serialize the forest to JSON.
    public func toJson() : Text { toJsonForest(build()) };
  };
};
