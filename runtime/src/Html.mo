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

  /// Generic display fallback for values whose type the compiler could not
  /// resolve. Uses `debug_show` and strips one layer of wrapping quotes so a
  /// `Text` value renders without the surrounding quotes.
  public func show(t : Text) : Text {
    // The compiler passes already-stringified values here; kept for symmetry.
    t;
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
