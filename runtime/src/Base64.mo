/// Base64 decoder, used to embed the compiled WASM client as a compact string
/// literal in `ClientAssets` (so the canister can serve `/motoview.wasm`).
import Text "mo:base/Text";
import Blob "mo:base/Blob";
import Nat8 "mo:base/Nat8";
import Nat32 "mo:base/Nat32";
import Char "mo:base/Char";
import Buffer "mo:base/Buffer";

module {
  func val(c : Char) : ?Nat32 {
    let n = Char.toNat32(c);
    if (n >= 65 and n <= 90) { ?(n - 65) } // A-Z -> 0-25
    else if (n >= 97 and n <= 122) { ?(n - 71) } // a-z -> 26-51
    else if (n >= 48 and n <= 57) { ?(n + 4) } // 0-9 -> 52-61
    else if (n == 43) { ?62 } // +
    else if (n == 47) { ?63 } // /
    else { null }; // '=' padding and whitespace -> null
  };

  /// Decode a standard base64 string to a Blob.
  public func decode(s : Text) : Blob {
    let out = Buffer.Buffer<Nat8>(s.size() * 3 / 4 + 3);
    var acc : Nat32 = 0;
    var bits : Nat32 = 0;
    for (c in s.chars()) {
      switch (val(c)) {
        case (?v) {
          acc := (acc << 6) | v;
          bits += 6;
          if (bits >= 8) {
            bits -= 8;
            out.add(Nat8.fromNat(Nat32.toNat((acc >> bits) & 0xFF)));
          };
        };
        case null {}; // skip '=' and whitespace
      };
    };
    Blob.fromArray(Buffer.toArray(out));
  };
};
