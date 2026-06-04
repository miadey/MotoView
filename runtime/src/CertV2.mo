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
import Iter "mo:base/Iter";

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

  // balanced fork of sorted (label, subtree) pairs, each wrapped as labeled().
  func buildBalancedLabeled(pairs : [(Blob, Tree)]) : Tree { buildRangeL(pairs, 0, pairs.size()) };
  func buildRangeL(pairs : [(Blob, Tree)], lo : Nat, hi : Nat) : Tree {
    if (hi - lo == 0) { return #empty };
    if (hi - lo == 1) { let (l, st) = pairs[lo]; return #labeled(l, st) };
    let mid = lo + (hi - lo) / 2;
    #fork(buildRangeL(pairs, lo, mid), buildRangeL(pairs, mid, hi));
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

  // "/forum/new" -> ["forum","new"]; "/" -> []. Empty segments dropped.
  func split(path : Text) : [Text] { Iter.toArray(Text.tokens(path, #char '/')) };

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

  func wildcardMarker() : Blob { Text.encodeUtf8("<*>") };
  func markerOf(wild : Bool) : Blob { if (wild) { wildcardMarker() } else { terminalMarker() } };

  // A certified entry: a path (exact) or a static prefix (wildcard). For a
  // wildcard entry the terminal is "<*>", which matches the prefix and anything
  // below it — used for parameterized routes like /u/{handle} (prefix "/u").
  public type Entry = { path : Text; wild : Bool };

  func addDistinctText(b : Buffer.Buffer<Text>, s : Text) {
    for (x in b.vals()) { if (x == s) { return } };
    b.add(s);
  };
  func addDistinctBlob(b : Buffer.Buffer<Blob>, v : Blob) {
    for (x in b.vals()) { if (x == v) { return } };
    b.add(v);
  };

  // Recursively build the http_expr inner subtree. Each entry contributes a
  // terminal marker ("<$>" or "<*>") at the depth where its segments end.
  func buildSubtree(entries : [([Text], Bool)], depth : Nat) : Tree {
    let markers = Buffer.Buffer<Blob>(2);
    let segSet = Buffer.Buffer<Text>(8);
    for (e in entries.vals()) {
      let (segs, wild) = e;
      if (segs.size() == depth) { addDistinctBlob(markers, markerOf(wild)) } else if (segs.size() > depth) {
        addDistinctText(segSet, segs[depth]);
      };
    };
    let pairs = Buffer.Buffer<(Blob, Tree)>(markers.size() + segSet.size());
    for (m in markers.vals()) { pairs.add((m, #labeled(exprHash(), #leaf("")))) };
    for (s in segSet.vals()) {
      let bucket = Buffer.Buffer<([Text], Bool)>(4);
      for (e in entries.vals()) { if (e.0.size() > depth and e.0[depth] == s) { bucket.add(e) } };
      pairs.add((Text.encodeUtf8(s), buildSubtree(Buffer.toArray(bucket), depth + 1)));
    };
    let sorted = Array.sort(Buffer.toArray(pairs), func(a : (Blob, Tree), b : (Blob, Tree)) : { #less; #equal; #greater } { compareBlob(a.0, b.0) });
    buildBalancedLabeled(sorted);
  };

  func segmented(entries : [Entry]) : [([Text], Bool)] {
    Array.map<Entry, ([Text], Bool)>(entries, func(e) { (split(e.path), e.wild) });
  };

  func tree(entries : [Entry]) : Tree { #labeled(httpExpr(), buildSubtree(segmented(entries), 0)) };

  /// Root hash to install via CertifiedData.set.
  public func rootHash(entries : [Entry]) : Blob { hash(tree(entries)) };

  // Walk the inner subtree, keeping the witness path (segments then `marker`)
  // and pruning every sibling.
  func pruneToPath(node : Tree, segs : [Text], marker : Blob, depth : Nat) : Tree {
    switch node {
      case (#fork(l, r)) { #fork(pruneToPath(l, segs, marker, depth), pruneToPath(r, segs, marker, depth)) };
      case (#labeled(lbl, sub)) {
        var onPath = false;
        if (depth < segs.size()) { onPath := (lbl == Text.encodeUtf8(segs[depth])) } else if (depth == segs.size()) {
          onPath := (lbl == marker);
        } else if (depth == segs.size() + 1) { onPath := (lbl == exprHash()) };
        if (onPath) { #labeled(lbl, pruneToPath(sub, segs, marker, depth + 1)) } else { #pruned(hash(#labeled(lbl, sub))) };
      };
      case other { other };
    };
  };

  func witness(entries : [Entry], target : Entry) : Tree {
    #labeled(httpExpr(), pruneToPath(buildSubtree(segmented(entries), 0), split(target.path), markerOf(target.wild), 0));
  };

  func exprPathCbor(target : Entry) : Blob {
    let segs = split(target.path);
    let b = Buffer.Buffer<Nat8>(48);
    selfDescribe(b);
    cborHeader(b, 4, segs.size() + 2); // ["http_expr", ...segments, "<$>"|"<*>"]
    cborBytes(b, httpExpr());
    for (s in segs.vals()) { cborBytes(b, Text.encodeUtf8(s)) };
    cborBytes(b, markerOf(target.wild));
    Blob.fromArray(Buffer.toArray(b));
  };

  /// The IC-Certificate header value for the certified `target` (exact path or
  /// wildcard prefix), given the full entry set and the system certificate.
  public func headerValue(entries : [Entry], target : Entry, cert : Blob) : Text {
    "certificate=:" # base64(cert)
    # ":, tree=:" # base64(encodeTree(witness(entries, target)))
    # ":, version=2, expr_path=:" # base64(exprPathCbor(target)) # ":";
  };
};
