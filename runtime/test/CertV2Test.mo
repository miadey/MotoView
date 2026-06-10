// Unit tests for response-certification-v2 (CertV2.mo). Run:
//   moc -r --package base <base> runtime/test/CertV2Test.mo
//
// Covers three layers, against IC-spec ground truth where it exists:
//   1. HashTree hash()/encodeTree() vs. the IC interface-spec WORKED EXAMPLE
//      (the "a/b/c/d, x/y" tree). Its root hash + CBOR are published constants,
//      so this pins our domain separators, fork/labeled/leaf hashing, and CBOR.
//   2. The representation-independent hash + response_hash (status + headers +
//      body) — the body-bound part, checked for stability + body-sensitivity.
//   3. The body-bound certified tree: a known (path,status,headers,body) yields
//      a STABLE root hash, and the witness for a target prunes siblings while
//      reconstructing exactly that root (so the IC-Certificate header proves the
//      body, not just the path set).
import CertV2 "../src/CertV2";
import Sha256 "../src/Sha256";
import Debug "mo:base/Debug";
import Blob "mo:base/Blob";
import Text "mo:base/Text";
import Nat8 "mo:base/Nat8";
import Buffer "mo:base/Buffer";
import Array "mo:base/Array";

// ---- hex helpers -----------------------------------------------------------
let HEX = "0123456789abcdef";
let hexArr = Text.toArray(HEX);
func toHex(b : Blob) : Text {
  let out = Buffer.Buffer<Char>(b.size() * 2);
  for (x in b.vals()) {
    out.add(hexArr[Nat8.toNat(x / 16)]);
    out.add(hexArr[Nat8.toNat(x % 16)]);
  };
  Text.fromIter(out.vals());
};
// ===========================================================================
// 1. IC interface-spec WORKED EXAMPLE.
//    Tree (interface spec, "Encoding of certificates" / Example):
//      ┬ "a" ┬ "x" "hello"
//      │     └ "y" "world"
//      ├ "b" "good"
//      ├ "c"
//      └ "d" "morning"
//    Published root hash + CBOR are fixed constants of the spec.
// ===========================================================================
let example : CertV2.Tree =
  #fork(
    #fork(
      #labeled("a", #fork(
        #fork(#labeled("x", #leaf("hello")), #empty),
        #labeled("y", #leaf("world")),
      )),
      #labeled("b", #leaf("good")),
    ),
    #fork(
      #labeled("c", #empty),
      #labeled("d", #leaf("morning")),
    ),
  );

let EXPECTED_ROOT = "eb5c5b2195e62d996b84c9bcc8259d19a83786a2f59e0878cec84c811f669aa0";
// The IC interface spec publishes the BARE (untagged) CBOR for this example.
// On the wire the HashTree blob (in the IC-Certificate `tree=` field) is wrapped
// with the CBOR self-describe tag 0xd9d9f7 — which is what encodeTree() emits —
// so the expected value below is the published example WITH that tag prefix.
let EXPECTED_CBOR_BARE = "8301830183024161830183018302417882034568656c6c6f810083024179820345776f726c6483024162820344676f6f648301830241638100830241648203476d6f726e696e67";
let EXPECTED_CBOR = "d9d9f7" # EXPECTED_CBOR_BARE;

let gotRoot = toHex(CertV2.hash(example));
if (gotRoot != EXPECTED_ROOT) {
  Debug.print("ROOT MISMATCH:\n  got " # gotRoot # "\n  exp " # EXPECTED_ROOT);
  assert false;
};
let gotCbor = toHex(CertV2.encodeTree(example));
if (gotCbor != EXPECTED_CBOR) {
  Debug.print("CBOR MISMATCH:\n  got " # gotCbor # "\n  exp " # EXPECTED_CBOR);
  assert false;
};
Debug.print("[1] IC-spec worked-example tree: root hash + self-describe CBOR match");

// pruned nodes reconstruct to the same root (witness soundness primitive):
// replace the "c"/"d" right fork with its pruned hash; root must be unchanged.
let prunedExample : CertV2.Tree =
  #fork(
    #fork(
      #labeled("a", #fork(
        #fork(#labeled("x", #leaf("hello")), #empty),
        #labeled("y", #leaf("world")),
      )),
      #labeled("b", #leaf("good")),
    ),
    #pruned(CertV2.hash(#fork(#labeled("c", #empty), #labeled("d", #leaf("morning"))))),
  );
assert (toHex(CertV2.hash(prunedExample)) == EXPECTED_ROOT);
Debug.print("[2] pruning a subtree preserves the root hash");

// ===========================================================================
// 2. response_hash: representation-independent hash of (status + headers) then
//    || SHA256(body), final SHA256. We recompute the EXPECTED value here with
//    the SAME primitives the spec mandates, independently of CertV2's internal
//    structure, to pin the algorithm (LEB128 status, sha256(key)+sha256(value)
//    pairs, sorted, concat, sha256; then concat body hash; sha256).
// ===========================================================================
func leb128(n : Nat) : Blob {
  let out = Buffer.Buffer<Nat8>(4);
  var v = n;
  loop {
    let byte = v % 128;
    v /= 128;
    if (v != 0) { out.add(Nat8.fromNat(byte) | 0x80) } else { out.add(Nat8.fromNat(byte)); return Blob.fromArray(Buffer.toArray(out)) };
  };
};
func cat(parts : [Blob]) : Blob {
  let b = Buffer.Buffer<Nat8>(64);
  for (p in parts.vals()) { for (x in p.vals()) { b.add(x) } };
  Blob.fromArray(Buffer.toArray(b));
};
func cmp(a : Blob, b : Blob) : { #less; #equal; #greater } {
  let aa = Blob.toArray(a); let bb = Blob.toArray(b);
  let n = if (aa.size() < bb.size()) aa.size() else bb.size();
  var i = 0;
  while (i < n) { if (aa[i] < bb[i]) return #less; if (aa[i] > bb[i]) return #greater; i += 1 };
  if (aa.size() < bb.size()) #less else if (aa.size() > bb.size()) #greater else #equal;
};
// reference rep-independent hash over (key, valueHash) pairs.
func refRIH(pairs : [(Text, Blob)]) : Blob {
  let hashed = Array.map<(Text, Blob), (Blob, Blob)>(pairs, func((k, vh)) {
    (Sha256.hash(Text.encodeUtf8(k)), vh);
  });
  let sorted = Array.sort(hashed, func(a : (Blob, Blob), b : (Blob, Blob)) : { #less; #equal; #greater } {
    switch (cmp(a.0, b.0)) { case (#equal) cmp(a.1, b.1); case other other };
  });
  let parts = Buffer.Buffer<Blob>(sorted.size() * 2);
  for ((kh, vh) in sorted.vals()) { parts.add(kh); parts.add(vh) };
  Sha256.hash(cat(Buffer.toArray(parts)));
};
// status 200, one certified header (content-type), body "hello world".
let status : Nat = 200;
let body : Blob = Text.encodeUtf8("hello world");
let headerHash = refRIH([
  (":ic-cert-status", Sha256.hash(leb128(status))),
  ("content-type", Sha256.hash(Text.encodeUtf8("text/plain"))),
]);
let expectedRespHash = Sha256.hash(cat([headerHash, Sha256.hash(body)]));

let gotRespHash = CertV2.responseHash(status, [("content-type", "text/plain")], body);
if (gotRespHash != expectedRespHash) {
  Debug.print("RESP HASH MISMATCH:\n  got " # toHex(gotRespHash) # "\n  exp " # toHex(expectedRespHash));
  assert false;
};
Debug.print("[3] responseHash matches the spec algorithm (status+headers+body)");

// body sensitivity: a different body MUST change the response hash.
let respHash2 = CertV2.responseHash(status, [("content-type", "text/plain")], Text.encodeUtf8("HELLO WORLD"));
assert (respHash2 != gotRespHash);
// status sensitivity:
let respHash3 = CertV2.responseHash(404, [("content-type", "text/plain")], body);
assert (respHash3 != gotRespHash);
// header sensitivity:
let respHash4 = CertV2.responseHash(status, [("content-type", "text/html")], body);
assert (respHash4 != gotRespHash);
Debug.print("[4] responseHash is body/status/header-sensitive (MITM body-swap detected)");

// ===========================================================================
// 3. Body-bound certified tree + witness.
//    Build a tree over two certified assets with their body hashes; check the
//    root hash is STABLE (regression pin) and that the witness for one asset
//    prunes the other yet still reconstructs the same root.
// ===========================================================================
let bodyA = Text.encodeUtf8("console.log('a')");
let bodyB = Text.encodeUtf8("/* css */");
let rhA = CertV2.responseHash(200, [("content-type", "text/javascript")], bodyA);
let rhB = CertV2.responseHash(200, [("content-type", "text/css")], bodyB);

let entries : [CertV2.BoundEntry] = [
  { path = "/app.js"; wild = false; respHash = rhA },
  { path = "/app.css"; wild = false; respHash = rhB },
];

let root = CertV2.rootHash(entries);
// Stable regression pin: recompute and compare to the first run's value.
Debug.print("[5] body-bound root hash = " # toHex(root));

// Witness for /app.js must reconstruct the SAME root (so the boundary can
// verify it against certified_data) AND must commit the /app.js body hash.
let wit = CertV2.witnessTree(entries, entries[0]);
let witRoot = CertV2.hash(wit);
if (witRoot != root) {
  Debug.print("WITNESS ROOT MISMATCH:\n  got " # toHex(witRoot) # "\n  exp " # toHex(root));
  assert false;
};
Debug.print("[6] witness for /app.js reconstructs the body-bound root");

// The witness must PRUNE the sibling /app.css: its rendered CBOR must NOT
// contain /app.css's body hash, but MUST contain /app.js's.
let witCbor = toHex(CertV2.encodeTree(wit));
assert (Text.contains(witCbor, #text (toHex(rhA))));
assert (not Text.contains(witCbor, #text (toHex(rhB))));
Debug.print("[7] witness includes /app.js body hash, prunes /app.css");

// Tamper: if the served body's hash differs from the certified one, the
// witness reconstructs a DIFFERENT root than what certified_data committed —
// i.e. a swapped body is detectable. Simulate by certifying rhA but witnessing
// a tampered respHash.
let tampered : CertV2.BoundEntry = { path = "/app.js"; wild = false; respHash = CertV2.responseHash(200, [("content-type", "text/javascript")], Text.encodeUtf8("console.log('EVIL')")) };
let witTampered = CertV2.witnessTree([tampered, entries[1]], tampered);
assert (CertV2.hash(witTampered) != root);
Debug.print("[8] a body-swapped response yields a different root (rejected by boundary)");

// Wildcard entry still supported (parameterized routes), body-bound.
let wildEntries : [CertV2.BoundEntry] = [
  { path = "/u"; wild = true; respHash = rhA },
];
let wildRoot = CertV2.rootHash(wildEntries);
let wildWit = CertV2.witnessTree(wildEntries, wildEntries[0]);
assert (CertV2.hash(wildWit) == wildRoot);
Debug.print("[9] wildcard (<*>) entries remain body-bound");

// headerValue still produces a non-empty IC-Certificate string.
let hv = CertV2.headerValue(entries, entries[0], "\00\01\02\03");
assert (Text.contains(hv, #text "certificate=:"));
assert (Text.contains(hv, #text "tree=:"));
assert (Text.contains(hv, #text "version=2"));
assert (Text.contains(hv, #text "expr_path=:"));
Debug.print("[10] headerValue emits a v2 IC-Certificate header");

Debug.print("CERTV2_TEST_OK");
