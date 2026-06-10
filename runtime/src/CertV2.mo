/// IC HTTP response-certification v2 — BODY-BOUND form.
///
/// Lets the canister serve responses as fast *certified queries* (no consensus
/// round-trip) instead of upgrading every request to an update call, while
/// committing the RESPONSE BODY (and status + certified headers) into the
/// certificate — so a boundary node / MITM cannot swap the body of a certified
/// response undetected.
///
/// We use the response-only certification expression
///   default_certification(ValidationArgs{certification:Certification{
///     no_request_certification:Empty{},
///     response_certification:ResponseCertification{
///       response_header_exclusions:ResponseHeaderList{headers:[]}}}})
/// (no request certification; all response headers certified — we exclude none).
/// `expr_hash = SHA256(expression)`.
///
/// Tree shape, per registered path (single or multi segment), per the
/// HTTP Gateway Protocol spec (response certification v2):
///   "http_expr" → <segment…> → "<$>"|"<*>"
///                  → <expr_hash> → "" (empty request-hash, no req cert)
///                  → <response_hash> → leaf("")
/// The response_hash is the representation-independent hash of
///   { ":ic-cert-status" -> status } ∪ certified-headers  concatenated with
///   SHA256(body), then SHA256 of that — exactly as the boundary recomputes it
///   from the served response, so the witness proves the body.
///
/// Spec: internetcomputer.org/docs/references/http-gateway-protocol-spec
/// (response certification v2) and the dfinity/response-verification reference.
import Sha256 "Sha256";
import Blob "mo:base/Blob";
import Text "mo:base/Text";
import Buffer "mo:base/Buffer";
import Array "mo:base/Array";
import Nat8 "mo:base/Nat8";
import Nat "mo:base/Nat";
import Char "mo:base/Char";
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
  public func encodeTree(t : Tree) : Blob {
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

  // ---- representation-independent hash + response_hash --------------------
  // Per the spec (dfinity/ic-representation-independent-hash): for a map of
  // (key, value) pairs, hash each key (SHA256 of its utf8 bytes) and each value
  // to a 32-byte digest, form (keyHash, valueHash) pairs, SORT them, concat all
  // (keyHash · valueHash) and SHA256 the result. Value hashing depends on type:
  //   * a string value -> SHA256(utf8 bytes)
  //   * a number value (the status code) -> SHA256(unsigned LEB128 of the number)

  func leb128(n : Nat) : Blob {
    let out = Buffer.Buffer<Nat8>(4);
    var v = n;
    loop {
      let byte = v % 128;
      v /= 128;
      if (v != 0) { out.add(Nat8.fromNat(byte) | 0x80) } else {
        out.add(Nat8.fromNat(byte));
        return Blob.fromArray(Buffer.toArray(out));
      };
    };
  };

  func lower(t : Text) : Text {
    Text.map(t, func(c : Char) : Char {
      if (c >= 'A' and c <= 'Z') { Char.fromNat32(Char.toNat32(c) + 32) } else { c };
    });
  };

  // representation-independent hash over (key, valueHash) pairs (value already
  // hashed by the caller, so we can mix string- and number-valued entries).
  func repIndependentHash(pairs : [(Text, Blob)]) : Blob {
    let hashed = Array.map<(Text, Blob), (Blob, Blob)>(pairs, func((k, vh)) {
      (Sha256.hash(Text.encodeUtf8(k)), vh);
    });
    let sorted = Array.sort(hashed, func(a : (Blob, Blob), b : (Blob, Blob)) : { #less; #equal; #greater } {
      switch (compareBlob(a.0, b.0)) { case (#equal) { compareBlob(a.1, b.1) }; case other { other } };
    });
    let parts = Buffer.Buffer<Blob>(sorted.size() * 2);
    for ((kh, vh) in sorted.vals()) { parts.add(kh); parts.add(vh) };
    Sha256.hash(concat(Buffer.toArray(parts)));
  };

  /// The representation-independent hash of the certified response, per spec:
  ///   responseHash = SHA256( RIH({":ic-cert-status" -> status} ∪ headers)
  ///                          · SHA256(body) )
  /// Header names are lowercased; values hashed as utf8 strings. The status
  /// pseudo-header value is hashed as an unsigned-LEB128 number. We certify ALL
  /// response headers (response_header_exclusions with an empty list), so the
  /// caller passes exactly the headers it will serve (minus IC-Certificate,
  /// which is never certified — it carries the proof itself).
  public func responseHash(status : Nat, headers : [(Text, Text)], body : Blob) : Blob {
    let pairs = Buffer.Buffer<(Text, Blob)>(headers.size() + 1);
    pairs.add((":ic-cert-status", Sha256.hash(leb128(status))));
    for ((k, v) in headers.vals()) {
      let lk = lower(k);
      // IC-Certificate is never part of the certified set (it is the proof).
      if (lk != "ic-certificate") { pairs.add((lk, Sha256.hash(Text.encodeUtf8(v)))) };
    };
    let headerHash = repIndependentHash(Buffer.toArray(pairs));
    Sha256.hash(concat([headerHash, Sha256.hash(body)]));
  };

  // ---- v2 certification expression ----
  // EXACT response-only expression (matches dfinity/response-verification's
  // create_default_response_only_cel_expr with no header exclusions). No
  // whitespace. exprHash = SHA256(expression).
  public let expression : Text = "default_certification(ValidationArgs{certification:Certification{no_request_certification:Empty{},response_certification:ResponseCertification{response_header_exclusions:ResponseHeaderList{headers:[]}}}})";
  func exprHash() : Blob { Sha256.hash(Text.encodeUtf8(expression)) };
  func terminalMarker() : Blob { Text.encodeUtf8("<$>") };
  func httpExpr() : Blob { Text.encodeUtf8("http_expr") };
  func wildcardMarker() : Blob { Text.encodeUtf8("<*>") };
  func markerOf(wild : Bool) : Blob { if (wild) { wildcardMarker() } else { terminalMarker() } };
  // The "no request certification" request-hash label is the empty blob.
  func emptyReqHash() : Blob { "" };

  // "/forum/new" -> ["forum","new"]; "/" -> []. Empty segments dropped.
  func split(path : Text) : [Text] { Iter.toArray(Text.tokens(path, #char '/')) };

  /// A body-bound certified entry: a path (exact) or a static prefix
  /// (wildcard), TOGETHER with the response_hash that commits that path's
  /// served response (status + certified headers + body). For a wildcard entry
  /// the terminal is "<*>", matching the prefix and anything below it — used for
  /// parameterized routes like /u/{handle} (prefix "/u").
  public type BoundEntry = { path : Text; wild : Bool; respHash : Blob };

  // Backwards-compatible alias: an Entry without a body hash. Constructing the
  // tree from these certifies the path set only (no body) — kept so callers that
  // genuinely cannot precompute a body hash still compile, but App.mo now always
  // supplies a respHash.
  public type Entry = { path : Text; wild : Bool };

  func addDistinctText(b : Buffer.Buffer<Text>, s : Text) {
    for (x in b.vals()) { if (x == s) { return } };
    b.add(s);
  };

  // The body-bound terminal subtree placed under each marker ("<$>"/"<*>"):
  //   <expr_hash> → "" (empty request hash) → <response_hash> → leaf("")
  func boundTerminal(respHash : Blob) : Tree {
    #labeled(exprHash(),
      #labeled(emptyReqHash(),
        #labeled(respHash, #leaf(""))));
  };

  // A marker key carries its body-bound terminal. We dedupe markers by their
  // (marker, respHash) so two distinct entries that share a segment path but
  // certify different bodies do not collide (they cannot, in practice, since a
  // path maps to one response — but we key defensively).
  type MarkerLeaf = { marker : Blob; respHash : Blob };

  // Recursively build the http_expr inner subtree. Each entry contributes a
  // body-bound terminal at the depth where its segments end.
  func buildSubtree(entries : [([Text], Bool, Blob)], depth : Nat) : Tree {
    let markers = Buffer.Buffer<MarkerLeaf>(2);
    let segSet = Buffer.Buffer<Text>(8);
    for (e in entries.vals()) {
      let (segs, wild, rh) = e;
      if (segs.size() == depth) {
        let m = markerOf(wild);
        var seen = false;
        for (x in markers.vals()) { if (x.marker == m and x.respHash == rh) { seen := true } };
        if (not seen) { markers.add({ marker = m; respHash = rh }) };
      } else if (segs.size() > depth) {
        addDistinctText(segSet, segs[depth]);
      };
    };
    let pairs = Buffer.Buffer<(Blob, Tree)>(markers.size() + segSet.size());
    for (m in markers.vals()) { pairs.add((m.marker, boundTerminal(m.respHash))) };
    for (s in segSet.vals()) {
      let bucket = Buffer.Buffer<([Text], Bool, Blob)>(4);
      for (e in entries.vals()) { if (e.0.size() > depth and e.0[depth] == s) { bucket.add(e) } };
      pairs.add((Text.encodeUtf8(s), buildSubtree(Buffer.toArray(bucket), depth + 1)));
    };
    let sorted = Array.sort(Buffer.toArray(pairs), func(a : (Blob, Tree), b : (Blob, Tree)) : { #less; #equal; #greater } { compareBlob(a.0, b.0) });
    buildBalancedLabeled(sorted);
  };

  func segmented(entries : [BoundEntry]) : [([Text], Bool, Blob)] {
    Array.map<BoundEntry, ([Text], Bool, Blob)>(entries, func(e) { (split(e.path), e.wild, e.respHash) });
  };

  func tree(entries : [BoundEntry]) : Tree { #labeled(httpExpr(), buildSubtree(segmented(entries), 0)) };

  /// Root hash to install via CertifiedData.set. Commits the body-bound tree.
  public func rootHash(entries : [BoundEntry]) : Blob { hash(tree(entries)) };

  // Walk the inner subtree, keeping the witness path (segments, then `marker`,
  // then through expr_hash → "" → respHash) and pruning every sibling.
  func pruneToPath(node : Tree, segs : [Text], marker : Blob, respHash : Blob, depth : Nat) : Tree {
    switch node {
      case (#fork(l, r)) { #fork(pruneToPath(l, segs, marker, respHash, depth), pruneToPath(r, segs, marker, respHash, depth)) };
      case (#labeled(lbl, sub)) {
        var onPath = false;
        if (depth < segs.size()) { onPath := (lbl == Text.encodeUtf8(segs[depth])) }
        else if (depth == segs.size()) { onPath := (lbl == marker) }
        else if (depth == segs.size() + 1) { onPath := (lbl == exprHash()) }
        else if (depth == segs.size() + 2) { onPath := (lbl == emptyReqHash()) }
        else if (depth == segs.size() + 3) { onPath := (lbl == respHash) };
        if (onPath) { #labeled(lbl, pruneToPath(sub, segs, marker, respHash, depth + 1)) } else { #pruned(hash(#labeled(lbl, sub))) };
      };
      case other { other };
    };
  };

  /// The pruned HashTree witness for `target`: it reconstructs the same root as
  /// `rootHash(entries)` while exposing only the target's path AND its
  /// response_hash (every sibling pruned). Exposed for unit tests.
  public func witnessTree(entries : [BoundEntry], target : BoundEntry) : Tree {
    #labeled(httpExpr(), pruneToPath(buildSubtree(segmented(entries), 0), split(target.path), markerOf(target.wild), target.respHash, 0));
  };

  func exprPathCbor(target : BoundEntry) : Blob {
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
  /// wildcard prefix), given the full entry set and the system certificate. The
  /// witnessed tree now commits the response body, so the boundary verifies the
  /// served body against the certificate.
  public func headerValue(entries : [BoundEntry], target : BoundEntry, cert : Blob) : Text {
    "certificate=:" # base64(cert)
    # ":, tree=:" # base64(encodeTree(witnessTree(entries, target)))
    # ":, version=2, expr_path=:" # base64(exprPathCbor(target)) # ":";
  };
};
