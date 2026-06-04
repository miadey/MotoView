/// Secure-form tokens.
///
/// A secure form embeds a signed token that binds the submission to the
/// route, handler, caller principal, an expiry and a single-use nonce, plus a
/// hash of the expected field schema. On submit the server re-derives the MAC
/// (HMAC-SHA256) and rejects mismatches, expired tokens and replays.
///
/// Token wire format:  payload "." hmacHex
/// payload =  path "|" handler "|" principalText "|" expiryNs "|" nonce "|" schemaHash
import Text "mo:base/Text";
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

  func payload(path : Text, handler : Text, principal : Text, expiryNs : Int, nonce : Text, schema : Text) : Text {
    path # "|" # handler # "|" # principal # "|" # Int.toText(expiryNs) # "|" # nonce # "|" # schema;
  };

  func sign(secret : Blob, msg : Text) : Text {
    Hex.encode(Sha256.hmac(secret, Text.encodeUtf8(msg)));
  };

  /// Mint a token for a form rendered now.
  public func mint(
    secret : Blob,
    path : Text,
    handler : Text,
    principal : Text,
    expiryNs : Int,
    nonce : Text,
    schema : Text,
  ) : Text {
    let p = payload(path, handler, principal, expiryNs, nonce, schema);
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
  ) : Verdict {
    let parts = Iter.toArray(Text.split(token, #char '.'));
    if (parts.size() != 2) { return #invalid("malformed token") };
    let p = parts[0];
    let mac = parts[1];
    if (not eq(mac, sign(secret, p))) { return #invalid("bad signature") };

    let f = Iter.toArray(Text.split(p, #char '|'));
    if (f.size() != 6) { return #invalid("malformed payload") };
    if (f[0] != path) { return #invalid("route mismatch") };
    if (f[1] != handler) { return #invalid("handler mismatch") };
    if (f[2] != principal) { return #invalid("principal mismatch") };
    if (f[5] != schema) { return #invalid("field schema mismatch") };

    let expiry = textToInt(f[3]);
    if (nowNs > expiry) { return #invalid("token expired") };

    #ok({ nonce = f[4] });
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
