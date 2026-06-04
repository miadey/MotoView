/// HTML building helpers used by generated render code.
///
/// The compiler emits calls like:
///   let b = Html.Builder();
///   b.raw("<h1>");
///   b.text(userName);              // escaped
///   b.raw("</h1>");
///   b.build()
import Text "mo:base/Text";
import Buffer "mo:base/Buffer";

module {

  /// Escape text for use in element content / attribute values.
  public func escape(t : Text) : Text {
    var s = t;
    s := Text.replace(s, #char '&', "&amp;");
    s := Text.replace(s, #char '<', "&lt;");
    s := Text.replace(s, #char '>', "&gt;");
    s := Text.replace(s, #char '\"', "&quot;");
    s := Text.replace(s, #char '\'', "&#39;");
    s;
  };

  /// Strip one layer of surrounding double-quotes. The compiler wraps
  /// type-unknown expressions as `Html.unquote(debug_show(expr))`, so a `Text`
  /// field (which `debug_show` renders as `"value"`) displays as `value`, while
  /// numbers/variants (`5`, `#Won`) pass through unchanged.
  public func unquote(t : Text) : Text {
    let n = t.size();
    if (n >= 2 and Text.startsWith(t, #char '\"') and Text.endsWith(t, #char '\"')) {
      let chars = Text.toArray(t);
      var out = "";
      var i = 1;
      while (i + 1 < n) { out #= Text.fromChar(chars[i]); i += 1 };
      out;
    } else { t };
  };

  /// An append-only HTML buffer. Joins in O(n) at `build()`.
  public class Builder() {
    let parts = Buffer.Buffer<Text>(64);

    /// Append raw, already-safe HTML (literal template chunks).
    public func raw(t : Text) { parts.add(t) };

    /// Append escaped dynamic text.
    public func text(t : Text) { parts.add(escape(t)) };

    /// Append an attribute (name="value") with the value escaped.
    public func attr(name : Text, value : Text) {
      parts.add(" " # name # "=\"" # escape(value) # "\"");
    };

    public func build() : Text { Text.join("", parts.vals()) };
  };
};
