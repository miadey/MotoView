/// Lowercase hex encoding helpers.
import Blob "mo:base/Blob";
import Text "mo:base/Text";
import Nat8 "mo:base/Nat8";
import Nat32 "mo:base/Nat32";

module {
  let digits = "0123456789abcdef";

  func nibble(n : Nat32) : Text {
    let chars = Text.toArray(digits);
    Text.fromChar(chars[Nat32.toNat(n & 0xF)]);
  };

  /// Encode a byte as two hex chars.
  public func byte(b : Nat8) : Text {
    let n = Nat32.fromNat(Nat8.toNat(b));
    nibble(n >> 4) # nibble(n);
  };

  /// Encode a blob as a lowercase hex string.
  public func encode(blob : Blob) : Text {
    var out = "";
    for (b in blob.vals()) { out #= byte(b) };
    out;
  };

  /// Encode a Nat32 as 8 hex chars.
  public func nat32(n : Nat32) : Text {
    byte(Nat8.fromNat(Nat32.toNat((n >> 24) & 0xFF)))
    # byte(Nat8.fromNat(Nat32.toNat((n >> 16) & 0xFF)))
    # byte(Nat8.fromNat(Nat32.toNat((n >> 8) & 0xFF)))
    # byte(Nat8.fromNat(Nat32.toNat(n & 0xFF)));
  };
};
