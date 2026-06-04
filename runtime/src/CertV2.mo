/// IC HTTP response-certification v2 — "pass-through" (no-certification) form.
///
/// Lets the canister serve responses as fast *certified queries* (no consensus
/// round-trip) instead of upgrading every request to an update call. We use the
/// pass-through expression `default_certification(ValidationArgs{no_certification:Empty{}})`,
/// which tells the boundary "this path's responses are not body-certified" — so
/// we only have to commit the SET of certified paths into `certified_data`, not
/// hash every body. Ideal for static assets (js/css/wasm/favicon/…).
///
/// Ported from the proven C# implementation in the bzzz/wasp repo (live on
/// mainnet). Spec: internetcomputer.org/docs/references/http-gateway-protocol-spec
/// (response certification v2). Tree shape, per registered single-segment path:
///   "http_expr" → <segment> → "<$>" → <expr_hash> → leaf("")
import Sha256 "Sha256";
import Blob "mo:base/Blob";
import Text "mo:base/Text";
import Buffer "mo:base/Buffer";
import Array "mo:base/Array";
import Nat8 "mo:base/Nat8";
import Nat "mo:base/Nat";

module {

  // ---- HashTree (IC certified-tree) ----
  public type Tree = {
    #empty;
    #fork : (Tree, Tree);
    #labeled : (Blob, Tree);
    #leaf : Blob;
    #pruned : Blob;
  };

  func concat(parts : [Blob]) : Blob {
    let b = Buffer.Buffer<Nat8>(64);
    for (p in parts.vals()) { for (x in p.vals()) { b.add(x) } };
    Blob.fromArray(Buffer.toArray(b));
  };

  func domainSep(name : Text) : Blob {
    let nb = Blob.toArray(Text.encodeUtf8(name));
    concat([Blob.fromArray([Nat8.fromNat(nb.size())]), Blob.fromArray(nb)]);
  };
  func dsEmpty() : Blob { domainSep("ic-hashtree-empty") };
  func dsFork() : Blob { domainSep("ic-hashtree-fork") };
  func dsLabeled() : Blob { domainSep("ic-hashtree-labeled") };
  func dsLeaf() : Blob { domainSep("ic-hashtree-leaf") };

  public func hash(t : Tree) : Blob {
    switch t {
      case (#empty) { Sha256.hash(dsEmpty()) };
      case (#leaf c) { Sha256.hash(concat([dsLeaf(), c])) };
      case (#labeled(l, st)) { Sha256.hash(concat([dsLabeled(), l, hash(st)])) };
      case (#fork(a, b)) { Sha256.hash(concat([dsFork(), hash(a), hash(b)])) };
      case (#pruned h) { h };
    };
  };

  // ---- CBOR ----
  func cborHeader(b : Buffer.Buffer<Nat8>, major : Nat8, value : Nat) {
    let mt : Nat8 = major << 5;
    if (value < 24) { b.add(mt | Nat8.fromNat(value)) }
    else if (value < 256) { b.add(mt | 24); b.add(Nat8.fromNat(value)) }
    else if (value < 65536) { b.add(mt | 25); b.add(Nat8.fromNat(value / 256)); b.add(Nat8.fromNat(value % 256)) }
    else {
      b.add(mt | 26);
      var i : Nat = 4;
      while (i > 0) { i -= 1; b.add(Nat8.fromNat((value / (256 ** i)) % 256)) };
    };
  };
  func cborBytes(b : Buffer.Buffer<Nat8>, data : Blob) {
    cborHeader(b, 2, data.size());
    for (x in data.vals()) { b.add(x) };
  };
  func encodeInto(b : Buffer.Buffer<Nat8>, t : Tree) {
    switch t {
      case (#empty) { cborHeader(b, 4, 1); cborHeader(b, 0, 0) };
      case (#fork(l, r)) { cborHeader(b, 4, 3); cborHeader(b, 0, 1); encodeInto(b, l); encodeInto(b, r) };
      case (#labeled(l, st)) { cborHeader(b, 4, 3); cborHeader(b, 0, 2); cborBytes(b, l); encodeInto(b, st) };
      case (#leaf c) { cborHeader(b, 4, 2); cborHeader(b, 0, 3); cborBytes(b, c) };
      case (#pruned h) { cborHeader(b, 4, 2); cborHeader(b, 0, 4); cborBytes(b, h) };
    };
  };
  func selfDescribe(b : Buffer.Buffer<Nat8>) { b.add(0xd9); b.add(0xd9); b.add(0xf7) };
  func encodeTree(t : Tree) : Blob {
    let b = Buffer.Buffer<Nat8>(256);
    selfDescribe(b);
    encodeInto(b, t);
    Blob.fromArray(Buffer.toArray(b));
  };

  func buildBalanced(trees : [Tree]) : Tree { buildRange(trees, 0, trees.size()) };
  func buildRange(trees : [Tree], lo : Nat, hi : Nat) : Tree {
    if (hi - lo == 0) { return #empty };
    if (hi - lo == 1) { return trees[lo] };
    let mid = lo + (hi - lo) / 2;
    #fork(buildRange(trees, lo, mid), buildRange(trees, mid, hi));
  };

  // ---- base64 (RFC4648, standard) ----
  let B64 = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
  func base64(data : Blob) : Text {
    let alpha = Text.toArray(B64);
    let bytes = Blob.toArray(data);
    let n = bytes.size();
    let out = Buffer.Buffer<Char>(n * 4 / 3 + 4);
    var i = 0;
    while (i + 3 <= n) {
      let x = Nat8.toNat(bytes[i]) * 65536 + Nat8.toNat(bytes[i + 1]) * 256 + Nat8.toNat(bytes[i + 2]);
      out.add(alpha[x / 262144 % 64]); out.add(alpha[x / 4096 % 64]); out.add(alpha[x / 64 % 64]); out.add(alpha[x % 64]);
      i += 3;
    };
    let rem : Nat = n - i;
    if (rem == 1) {
      let x = Nat8.toNat(bytes[i]) * 65536;
      out.add(alpha[x / 262144 % 64]); out.add(alpha[x / 4096 % 64]); out.add('='); out.add('=');
    } else if (rem == 2) {
      let x = Nat8.toNat(bytes[i]) * 65536 + Nat8.toNat(bytes[i + 1]) * 256;
      out.add(alpha[x / 262144 % 64]); out.add(alpha[x / 4096 % 64]); out.add(alpha[x / 64 % 64]); out.add('=');
    };
    Text.fromIter(out.vals());
  };

  // ---- v2 certification ----
  public let expression : Text = "default_certification(ValidationArgs{no_certification:Empty{}})";
  func exprHash() : Blob { Sha256.hash(Text.encodeUtf8(expression)) };
  func terminalMarker() : Blob { Text.encodeUtf8("<$>") };
  func httpExpr() : Blob { Text.encodeUtf8("http_expr") };
  // terminal = labeled("<$>", labeled(expr_hash, leaf("")))
  func terminal() : Tree { #labeled(terminalMarker(), #labeled(exprHash(), #leaf(""))) };

  func seg(path : Text) : Blob { Text.encodeUtf8(Text.trimStart(path, #char '/')) };

  func compareBlob(a : Blob, b : Blob) : { #less; #equal; #greater } {
    let aa = Blob.toArray(a);
    let bb = Blob.toArray(b);
    let n = if (aa.size() < bb.size()) aa.size() else bb.size();
    var i = 0;
    while (i < n) {
      if (aa[i] < bb[i]) { return #less };
      if (aa[i] > bb[i]) { return #greater };
      i += 1;
    };
    if (aa.size() < bb.size()) { #less } else if (aa.size() > bb.size()) { #greater } else { #equal };
  };

  func sortedSegs(paths : [Text]) : [Blob] {
    Array.sort(Array.map<Text, Blob>(paths, seg), compareBlob);
  };

  /// The full http_expr tree for the registered single-segment paths.
  func tree(paths : [Text]) : Tree {
    let segs = sortedSegs(paths);
    let children = Array.map<Blob, Tree>(segs, func(s) { #labeled(s, terminal()) });
    #labeled(httpExpr(), buildBalanced(children));
  };

  /// Root hash to install via CertifiedData.set.
  public func rootHash(paths : [Text]) : Blob { hash(tree(paths)) };

  /// Witness revealing one path; every sibling pruned (root hash preserved).
  func witness(paths : [Text], path : Text) : Tree {
    let target = seg(path);
    let segs = sortedSegs(paths);
    let children = Array.map<Blob, Tree>(
      segs,
      func(s) {
        if (compareBlob(s, target) == #equal) { #labeled(s, terminal()) } else {
          #pruned(hash(#labeled(s, terminal())));
        };
      },
    );
    #labeled(httpExpr(), buildBalanced(children));
  };

  func exprPathCbor(path : Text) : Blob {
    let b = Buffer.Buffer<Nat8>(48);
    selfDescribe(b);
    cborHeader(b, 4, 3); // array of 3: ["http_expr", <seg>, "<$>"]
    cborBytes(b, httpExpr());
    cborBytes(b, seg(path));
    cborBytes(b, terminalMarker());
    Blob.fromArray(Buffer.toArray(b));
  };

  /// The IC-Certificate header value for `path`, given the system certificate.
  public func headerValue(paths : [Text], path : Text, cert : Blob) : Text {
    "certificate=:" # base64(cert)
    # ":, tree=:" # base64(encodeTree(witness(paths, path)))
    # ":, version=2, expr_path=:" # base64(exprPathCbor(path)) # ":";
  };
};
