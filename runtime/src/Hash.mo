/// Fast non-cryptographic hashing for `batchId` change-detection (FNV-1a 32).
import Text "mo:base/Text";
import Nat8 "mo:base/Nat8";
import Nat32 "mo:base/Nat32";
import Hex "Hex";

module {
  /// FNV-1a 32-bit over the UTF-8 bytes of `t`.
  public func fnv1a(t : Text) : Nat32 {
    var h : Nat32 = 0x811c9dc5;
    for (b in Text.encodeUtf8(t).vals()) {
      h := h ^ Nat32.fromNat(Nat8.toNat(b));
      h := h *% 0x01000193;
    };
    h;
  };

  /// Short stable id for a render state: "b_" + 8 hex chars.
  public func batchId(parts : Text) : Text {
    "b_" # Hex.nat32(fnv1a(parts));
  };
};
