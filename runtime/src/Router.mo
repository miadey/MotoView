/// Route matching: `/products/{id}` and typed constraints `/orders/{id:Nat}`.
import Text "mo:base/Text";
import Iter "mo:base/Iter";
import Char "mo:base/Char";
import Buffer "mo:base/Buffer";
import Types "Types";

module {

  func segments(p : Text) : [Text] {
    // drop leading/trailing slash noise, split on '/'
    let raw = Iter.toArray(Text.split(p, #char '/'));
    let out = Buffer.Buffer<Text>(raw.size());
    for (s in raw.vals()) { if (s != "") { out.add(s) } };
    Buffer.toArray(out);
  };

  func isDigits(t : Text) : Bool {
    if (t == "") { return false };
    for (c in t.chars()) {
      let n = Char.toNat32(c);
      if (n < 48 or n > 57) { return false };
    };
    true;
  };

  // "{id}" -> ?("id", ""), "{id:Nat}" -> ?("id","Nat"), literal -> null
  func paramSpec(seg : Text) : ?(Text, Text) {
    if (Text.startsWith(seg, #char '{') and Text.endsWith(seg, #char '}')) {
      let inner = trimBraces(seg);
      let parts = Iter.toArray(Text.split(inner, #char ':'));
      if (parts.size() == 2) { ?(parts[0], parts[1]) } else { ?(inner, "") };
    } else { null };
  };

  func trimBraces(seg : Text) : Text {
    var s = Text.replace(seg, #char '{', "");
    Text.replace(s, #char '}', "");
  };

  /// Match `path` against route `pattern`; returns captured params on success.
  public func match(pattern : Text, path : Text) : ?[(Text, Text)] {
    let ps = segments(pattern);
    let xs = segments(path);
    if (ps.size() != xs.size()) { return null };
    let params = Buffer.Buffer<(Text, Text)>(2);
    var i = 0;
    while (i < ps.size()) {
      switch (paramSpec(ps[i])) {
        case (?(name, kind)) {
          if (kind == "Nat" and not isDigits(xs[i])) { return null };
          params.add((name, xs[i]));
        };
        case null { if (ps[i] != xs[i]) { return null } };
      };
      i += 1;
    };
    ?Buffer.toArray(params);
  };

  /// Find the first page whose route matches `path`.
  public func find(pages : [Types.Page], path : Text) : ?(Types.Page, [(Text, Text)]) {
    for (page in pages.vals()) {
      switch (match(page.route, path)) {
        case (?params) { return ?(page, params) };
        case null {};
      };
    };
    null;
  };
};
