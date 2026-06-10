/// Secure-form tokens.
///
/// A secure form embeds a signed token that binds the submission to the
/// route, handler, caller principal, an expiry and a single-use nonce, plus a
/// hash of the expected field schema. On submit the server re-derives the MAC
/// (HMAC-SHA256) and rejects mismatches, expired tokens and replays.
///
/// Token wire format:  payload "." hmacHex
/// payload =  path "|" handler "|" principalText "|" expiryNs "|" nonce "|" schemaHash "|" intentHash
///
/// `intentHash` SOUNDLY binds a token to a value the server already knows at
/// mint time (a confirmation-flow intent — see `canonicalIntent`/`intentHash`).
/// For an ordinary `<form secure>` the server cannot know the not-yet-typed
/// field values at render time, so it MUST pass intentHash="" (the honest,
/// unbound case). A confirmation page that re-renders a server-known intent
/// (e.g. a wallet confirm step) passes a real hash; verify then rejects any
/// token whose intent does not match. Building a fake binding would be worse
/// than none, so we never pretend to bind unknown values.
import Text "mo:base/Text";
import Order "mo:base/Order";
import Array "mo:base/Array";
import Nat "mo:base/Nat";
import Iter "mo:base/Iter";
import Int "mo:base/Int";
import Nat32 "mo:base/Nat32";
import Char "mo:base/Char";
import Sha256 "Sha256";
import Hex "Hex";
import Hash "Hash";

module {

  public type Verdict = {
    #ok : { nonce : Text };
    #invalid : Text;
  };

  /// Hash of a form's declared field names (order-independent-ish: caller
  /// passes a canonical, comma-joined, sorted list).
  public func schemaHash(fields : Text) : Text {
    Hex.nat32(Hash.fnv1a(fields));
  };

  func payload(path : Text, handler : Text, principal : Text, expiryNs : Int, nonce : Text, schema : Text, intent : Text) : Text {
    path # "|" # handler # "|" # principal # "|" # Int.toText(expiryNs) # "|" # nonce # "|" # schema # "|" # intent;
  };

  func sign(secret : Blob, msg : Text) : Text {
    Hex.encode(Sha256.hmac(secret, Text.encodeUtf8(msg)));
  };

  /// Mint a token for a form rendered now. `intent` is "" for ordinary forms
  /// and a server-known intent hash (from `intentHash(canonicalIntent(...))`)
  /// for confirmation flows that bind a value the server already knows.
  public func mint(
    secret : Blob,
    path : Text,
    handler : Text,
    principal : Text,
    expiryNs : Int,
    nonce : Text,
    schema : Text,
    intent : Text,
  ) : Text {
    let p = payload(path, handler, principal, expiryNs, nonce, schema, intent);
    p # "." # sign(secret, p);
  };

  // constant-time-ish string compare
  func eq(a : Text, b : Text) : Bool {
    if (a.size() != b.size()) { return false };
    let ca = Text.toArray(a);
    let cb = Text.toArray(b);
    var diff : Nat32 = 0;
    var i = 0;
    while (i < ca.size()) {
      if (ca[i] != cb[i]) { diff |= 1 };
      i += 1;
    };
    diff == 0;
  };

  /// Verify a submitted token. Replay protection (consuming the nonce) is the
  /// caller's responsibility, using the returned nonce.
  public func verify(
    secret : Blob,
    token : Text,
    path : Text,
    handler : Text,
    principal : Text,
    nowNs : Int,
    schema : Text,
    intent : Text,
  ) : Verdict {
    let parts = Iter.toArray(Text.split(token, #char '.'));
    if (parts.size() != 2) { return #invalid("malformed token") };
    let p = parts[0];
    let mac = parts[1];
    if (not eq(mac, sign(secret, p))) { return #invalid("bad signature") };

    let f = Iter.toArray(Text.split(p, #char '|'));
    if (f.size() != 7) { return #invalid("malformed payload") };
    if (f[0] != path) { return #invalid("route mismatch") };
    if (f[1] != handler) { return #invalid("handler mismatch") };
    if (f[2] != principal) { return #invalid("principal mismatch") };
    if (f[5] != schema) { return #invalid("field schema mismatch") };
    if (f[6] != intent) { return #invalid("intent mismatch") };

    let expiry = textToInt(f[3]);
    if (nowNs > expiry) { return #invalid("token expired") };

    #ok({ nonce = f[4] });
  };

  /// Deterministic, order-INDEPENDENT serialization of (key, value) pairs into
  /// a single canonical string. Used by a confirmation page to derive a stable
  /// `intentHash` over its SERVER-KNOWN intent (e.g. the amount + recipient it
  /// is about to ask the user to confirm).
  ///
  /// Pairs are sorted by key, then by value (so duplicate keys are still
  /// ordered deterministically). Each key and value is length-prefixed
  /// ("<byteLen>:<text>") so that no choice of separator characters in the
  /// inputs can produce a collision: ("a","12"),("b","3") and
  /// ("a","1"),("b","23") serialize to distinct strings. The length prefix
  /// makes the encoding prefix-free, so concatenation is injective.
  public func canonicalIntent(fields : [(Text, Text)]) : Text {
    func lp(s : Text) : Text {
      Nat.toText(Text.encodeUtf8(s).size()) # ":" # s;
    };
    func cmp(a : (Text, Text), b : (Text, Text)) : Order.Order {
      switch (Text.compare(a.0, b.0)) {
        case (#equal) { Text.compare(a.1, b.1) };
        case (o) { o };
      };
    };
    let sorted = Array.sort(fields, cmp);
    let parts = Array.map<(Text, Text), Text>(sorted, func((k, v)) { lp(k) # lp(v) });
    Text.join("|", parts.vals());
  };

  /// Hash of a canonical intent string (SHA-256, hex). This is the value a
  /// confirmation page binds into its token via `mint(..., intentHash(...))`.
  public func intentHash(canonical : Text) : Text {
    Hex.encode(Sha256.hash(Text.encodeUtf8(canonical)));
  };

  func textToInt(t : Text) : Int {
    var n : Int = 0;
    var neg = false;
    for (c in t.chars()) {
      if (c == '-') { neg := true } else {
        let code = Char.toNat32(c);
        if (code >= 48 and code <= 57) {
          n := n * 10 + Nat32.toNat(code - 48);
        };
      };
    };
    if (neg) { -n } else { n };
  };
};
