/// URL, query-string and form-body parsing (percent + form-url decoding).
import Text "mo:base/Text";
import Char "mo:base/Char";
import Blob "mo:base/Blob";
import Nat8 "mo:base/Nat8";
import Nat32 "mo:base/Nat32";
import Buffer "mo:base/Buffer";
import Iter "mo:base/Iter";

module {

  func hexVal(c : Char) : ?Nat8 {
    let n = Char.toNat32(c);
    if (n >= 48 and n <= 57) { ?Nat8.fromNat(Nat32.toNat(n - 48)) } // 0-9
    else if (n >= 97 and n <= 102) { ?Nat8.fromNat(Nat32.toNat(n - 87)) } // a-f
    else if (n >= 65 and n <= 70) { ?Nat8.fromNat(Nat32.toNat(n - 55)) } // A-F
    else { null };
  };

  /// Decode an application/x-www-form-urlencoded component
  /// ('+' -> space, %XX -> byte), preserving UTF-8.
  public func decode(s : Text) : Text {
    let bytes = Buffer.Buffer<Nat8>(s.size());
    let chars = Text.toArray(s);
    var i = 0;
    let n = chars.size();
    while (i < n) {
      let c = chars[i];
      if (c == '+') {
        bytes.add(0x20);
        i += 1;
      } else if (c == '%' and i + 2 < n) {
        switch (hexVal(chars[i + 1]), hexVal(chars[i + 2])) {
          case (?hi, ?lo) { bytes.add(hi * 16 + lo); i += 3 };
          case _ { appendChar(bytes, c); i += 1 };
        };
      } else {
        appendChar(bytes, c);
        i += 1;
      };
    };
    switch (Text.decodeUtf8(Blob.fromArray(Buffer.toArray(bytes)))) {
      case (?t) { t };
      case null { s };
    };
  };

  func appendChar(buf : Buffer.Buffer<Nat8>, c : Char) {
    for (b in Blob.toArray(Text.encodeUtf8(Text.fromChar(c))).vals()) {
      buf.add(b);
    };
  };

  /// Split a request URL into (path, raw-query).
  public func splitUrl(url : Text) : (Text, Text) {
    let parts = Iter.toArray(Text.split(url, #char '?'));
    if (parts.size() >= 2) { (parts[0], parts[1]) } else { (url, "") };
  };

  /// Parse "a=1&b=2" into decoded key/value pairs.
  public func parsePairs(raw : Text) : [(Text, Text)] {
    if (raw == "") { return [] };
    let out = Buffer.Buffer<(Text, Text)>(8);
    for (seg in Text.split(raw, #char '&')) {
      if (seg != "") {
        let kv = Iter.toArray(Text.split(seg, #char '='));
        if (kv.size() >= 2) {
          // value may itself contain '=' (rare); re-join the tail
          var v = kv[1];
          var j = 2;
          while (j < kv.size()) { v #= "=" # kv[j]; j += 1 };
          out.add((decode(kv[0]), decode(v)));
        } else {
          out.add((decode(kv[0]), ""));
        };
      };
    };
    Buffer.toArray(out);
  };

  /// Parse a form-encoded request body blob.
  public func parseForm(body : Blob) : [(Text, Text)] {
    switch (Text.decodeUtf8(body)) {
      case (?t) { parsePairs(t) };
      case null { [] };
    };
  };

  /// Look up the first value for `key` in a pair list.
  public func get(pairs : [(Text, Text)], key : Text) : ?Text {
    for ((k, v) in pairs.vals()) { if (k == key) { return ?v } };
    null;
  };

  /// Look up with a default.
  public func getOr(pairs : [(Text, Text)], key : Text, def : Text) : Text {
    switch (get(pairs, key)) { case (?v) v; case null def };
  };
};
