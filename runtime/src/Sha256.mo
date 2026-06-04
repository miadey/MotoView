/// A compact, dependency-free SHA-256 implementation.
///
/// Used by `Security` to produce real HMAC-SHA256 MACs for secure-form tokens
/// (principal/route/handler/nonce/expiry binding). Operates on `Blob`/`[Nat8]`.
import Blob "mo:base/Blob";
import Nat8 "mo:base/Nat8";
import Nat32 "mo:base/Nat32";
import Nat64 "mo:base/Nat64";
import Array "mo:base/Array";
import Buffer "mo:base/Buffer";

module {

  let K : [Nat32] = [
    0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5,
    0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174,
    0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da,
    0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7, 0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967,
    0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85,
    0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070,
    0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
    0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2,
  ];

  func rotr(x : Nat32, n : Nat32) : Nat32 {
    (x >> n) | (x << (32 - n));
  };

  /// SHA-256 over a blob, returning the 32-byte digest.
  public func hash(msg : Blob) : Blob {
    var h0 : Nat32 = 0x6a09e667;
    var h1 : Nat32 = 0xbb67ae85;
    var h2 : Nat32 = 0x3c6ef372;
    var h3 : Nat32 = 0xa54ff53a;
    var h4 : Nat32 = 0x510e527f;
    var h5 : Nat32 = 0x9b05688c;
    var h6 : Nat32 = 0x1f83d9ab;
    var h7 : Nat32 = 0x5be0cd19;

    // --- padding ---
    let data = Buffer.Buffer<Nat8>(msg.size() + 72);
    for (b in msg.vals()) { data.add(b) };
    let bitLen : Nat64 = Nat64.fromNat(msg.size()) * 8;
    data.add(0x80);
    while (data.size() % 64 != 56) { data.add(0x00) };
    var i : Nat32 = 0;
    while (i < 8) {
      let shift : Nat64 = Nat64.fromNat(Nat32.toNat((7 - i) * 8));
      data.add(Nat8.fromNat(Nat64.toNat((bitLen >> shift) & 0xFF)));
      i += 1;
    };

    let bytes = Buffer.toArray(data);
    let w = Array.init<Nat32>(64, 0);

    let nBlocks = bytes.size() / 64;
    var blk = 0;
    while (blk < nBlocks) {
      let base = blk * 64;
      var t = 0;
      while (t < 16) {
        let o = base + t * 4;
        w[t] :=
          (Nat32.fromNat(Nat8.toNat(bytes[o])) << 24)
          | (Nat32.fromNat(Nat8.toNat(bytes[o + 1])) << 16)
          | (Nat32.fromNat(Nat8.toNat(bytes[o + 2])) << 8)
          | Nat32.fromNat(Nat8.toNat(bytes[o + 3]));
        t += 1;
      };
      while (t < 64) {
        let s0 = rotr(w[t - 15], 7) ^ rotr(w[t - 15], 18) ^ (w[t - 15] >> 3);
        let s1 = rotr(w[t - 2], 17) ^ rotr(w[t - 2], 19) ^ (w[t - 2] >> 10);
        w[t] := w[t - 16] +% s0 +% w[t - 7] +% s1;
        t += 1;
      };

      var a = h0;
      var b = h1;
      var c = h2;
      var d = h3;
      var e = h4;
      var f = h5;
      var g = h6;
      var h = h7;

      t := 0;
      while (t < 64) {
        let S1 = rotr(e, 6) ^ rotr(e, 11) ^ rotr(e, 25);
        let ch = (e & f) ^ ((^e) & g);
        let temp1 = h +% S1 +% ch +% K[t] +% w[t];
        let S0 = rotr(a, 2) ^ rotr(a, 13) ^ rotr(a, 22);
        let maj = (a & b) ^ (a & c) ^ (b & c);
        let temp2 = S0 +% maj;
        h := g;
        g := f;
        f := e;
        e := d +% temp1;
        d := c;
        c := b;
        b := a;
        a := temp1 +% temp2;
        t += 1;
      };

      h0 +%= a;
      h1 +%= b;
      h2 +%= c;
      h3 +%= d;
      h4 +%= e;
      h5 +%= f;
      h6 +%= g;
      h7 +%= h;
      blk += 1;
    };

    func be(n : Nat32) : [Nat8] {
      [
        Nat8.fromNat(Nat32.toNat((n >> 24) & 0xFF)),
        Nat8.fromNat(Nat32.toNat((n >> 16) & 0xFF)),
        Nat8.fromNat(Nat32.toNat((n >> 8) & 0xFF)),
        Nat8.fromNat(Nat32.toNat(n & 0xFF)),
      ];
    };

    let out = Buffer.Buffer<Nat8>(32);
    for (x in [h0, h1, h2, h3, h4, h5, h6, h7].vals()) {
      for (byte in be(x).vals()) { out.add(byte) };
    };
    Blob.fromArray(Buffer.toArray(out));
  };

  /// HMAC-SHA256(key, message) -> 32-byte digest.
  public func hmac(key : Blob, message : Blob) : Blob {
    let blockSize = 64;
    var k = Blob.toArray(key);
    if (k.size() > blockSize) { k := Blob.toArray(hash(key)) };

    let ipad = Buffer.Buffer<Nat8>(blockSize + message.size());
    let opad = Buffer.Buffer<Nat8>(blockSize + 32);
    var i = 0;
    while (i < blockSize) {
      let kb : Nat8 = if (i < k.size()) { k[i] } else { 0 };
      ipad.add(kb ^ 0x36);
      opad.add(kb ^ 0x5c);
      i += 1;
    };
    for (b in message.vals()) { ipad.add(b) };
    let inner = hash(Blob.fromArray(Buffer.toArray(ipad)));
    for (b in inner.vals()) { opad.add(b) };
    hash(Blob.fromArray(Buffer.toArray(opad)));
  };
};
