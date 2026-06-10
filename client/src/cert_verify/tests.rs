//! Tests for the chain-key certificate verifier.
//!
//! VERIFICATION DEPTH: these run against TWO real, live mainnet certificates
//! captured from `https://icp-api.io/api/v2/canister/<id>/read_state` for the
//! `/time` path (committed under `client/tests/fixtures/`):
//!
//!   * `live_time_nns_ryjl3.cbor`      — ICP ledger `ryjl3-tyaaa-aaaaa-aaaba-cai`,
//!     on the NNS subnet, signed DIRECTLY by the root key (no delegation).
//!   * `live_time_delegated_3xwpq.cbor` — `3xwpq-ziaaa-aaaah-qcn4a-cai`, on an
//!     application subnet, signed via a DELEGATION (root → subnet key), with a
//!     certified `canister_ranges`.
//!
//! Both verify against the PINNED `IC_ROOT_KEY` — the positive path is a real
//! end-to-end mainnet BLS chain-of-trust check, not a self-built fixture.
//!
//! Because the captured certificates have a fixed `/time`, the freshness tests
//! pass `now = <the cert's own time>`; separate tests prove the window rejects
//! stale/future times. Every negative test mutates one thing and asserts the
//! verifier fails closed with the precise error.

use super::*;

const NNS_CERT: &[u8] = include_bytes!("../../tests/fixtures/live_time_nns_ryjl3.cbor");
const DELEGATED_CERT: &[u8] = include_bytes!("../../tests/fixtures/live_time_delegated_3xwpq.cbor");

// Raw principal bytes (CRC-stripped) of the fixture canisters.
// ryjl3-tyaaa-aaaaa-aaaba-cai
const RYJL3_ID: &[u8] = &[0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x02, 0x01, 0x01];
// 3xwpq-ziaaa-aaaah-qcn4a-cai
const XWPQ_ID: &[u8] = &[0x00, 0x00, 0x00, 0x00, 0x00, 0xf0, 0x13, 0x78, 0x01, 0x01];

const WIDE: u128 = u128::MAX / 2; // effectively-infinite freshness window for fixtures

/// Unwrap the `{ "certificate": <bytes> }` read_state envelope to the inner
/// certificate CBOR — that inner blob is what `verify_certificate` consumes.
fn inner_certificate(envelope: &[u8]) -> Vec<u8> {
    let v: ciborium::value::Value = ciborium::de::from_reader(envelope).unwrap();
    // The read_state response is wrapped in the CBOR self-describe tag 55799.
    let v = match &v {
        ciborium::value::Value::Tag(55799, inner) => (**inner).clone(),
        other => other.clone(),
    };
    let map = v.as_map().unwrap();
    for (k, val) in map {
        if let ciborium::value::Value::Text(t) = k {
            if t == "certificate" {
                return val.as_bytes().unwrap().clone();
            }
        }
    }
    panic!("no certificate key in read_state envelope");
}

/// Read the certified `/time` out of a (verified-shape) certificate so tests
/// can use it as `now`.
fn cert_time(cert_cbor: &[u8]) -> u128 {
    let cert = Certificate::from_cbor(cert_cbor).unwrap();
    let mut leaf = cert.tree.lookup(&[b"time"]).unwrap();
    leb128::read::unsigned(&mut leaf).unwrap() as u128
}

// ---------------------------------------------------------------------------
// POSITIVE — real mainnet certs verify against the pinned root key.
// ---------------------------------------------------------------------------

#[test]
fn live_nns_cert_verifies_against_pinned_root() {
    let cert = inner_certificate(NNS_CERT);
    let now = cert_time(&cert);
    let v = verify_certificate(&cert, RYJL3_ID, RootKey::Mainnet, now, DEFAULT_MAX_TIME_OFFSET_NS)
        .expect("real NNS-subnet mainnet cert must verify against the pinned root key");
    assert_eq!(v.time_ns, now);
}

#[test]
fn live_delegated_cert_verifies_against_pinned_root() {
    let cert = inner_certificate(DELEGATED_CERT);
    let now = cert_time(&cert);
    let v = verify_certificate(&cert, XWPQ_ID, RootKey::Mainnet, now, DEFAULT_MAX_TIME_OFFSET_NS)
        .expect("real delegated mainnet cert must verify (root -> subnet key -> cert)");
    assert_eq!(v.time_ns, now);
}

/// The pinned key really is exercised: a Local root key that is NOT the NNS key
/// must make the same real cert fail (signature). Proves we aren't no-op'ing.
#[test]
fn live_cert_rejected_under_wrong_root_key() {
    let cert = inner_certificate(NNS_CERT);
    let now = cert_time(&cert);
    // Flip one byte of the pinned DER key -> a different, valid-length DER key.
    let mut wrong = IC_ROOT_KEY;
    wrong[100] ^= 0x01;
    let err = verify_certificate(
        &cert,
        RYJL3_ID,
        RootKey::Local(&wrong),
        now,
        DEFAULT_MAX_TIME_OFFSET_NS,
    )
    .unwrap_err();
    assert_eq!(err, CertError::Signature);
}

// ---------------------------------------------------------------------------
// NEGATIVE — every one MUST fail closed.
// ---------------------------------------------------------------------------

/// Flip one byte of the BLS signature -> signature check fails.
#[test]
fn tampered_signature_rejected() {
    let cert = inner_certificate(NNS_CERT);
    let now = cert_time(&cert);
    // Re-encode the cert with one signature byte flipped.
    let mutated = mutate_signature(&cert);
    let err = verify_certificate(&mutated, RYJL3_ID, RootKey::Mainnet, now, WIDE).unwrap_err();
    assert_eq!(err, CertError::Signature);
}

/// Corrupt one byte of the certified /time leaf -> the tree root hash changes,
/// so the signature no longer matches (a tampered body/leaf is caught by the
/// signature, exactly as the IC intends).
#[test]
fn corrupted_tree_leaf_rejected() {
    let cert = inner_certificate(NNS_CERT);
    let now = cert_time(&cert);
    let mutated = mutate_time_leaf(&cert);
    let err = verify_certificate(&mutated, RYJL3_ID, RootKey::Mainnet, now, WIDE).unwrap_err();
    // Changing the leaf changes root_hash -> BLS verify fails.
    assert_eq!(err, CertError::Signature);
}

/// Swap G1/G2: feed a 96-byte G2-public-key-shaped blob where a 48-byte G1
/// signature is expected. The BLS deserializer rejects the wrong length, so
/// the verify fails closed (never silently treated as valid).
#[test]
fn swapped_g1_g2_rejected() {
    let cert = inner_certificate(NNS_CERT);
    let now = cert_time(&cert);
    // Replace the 48-byte signature with the 96-byte raw root key (G2 bytes).
    let raw_g2 = &IC_ROOT_KEY[37..]; // 96 bytes
    let mutated = replace_signature(&cert, raw_g2);
    let err = verify_certificate(&mutated, RYJL3_ID, RootKey::Mainnet, now, WIDE).unwrap_err();
    assert_eq!(err, CertError::Signature);
}

/// Wrong signing domain separator: if the message prefix were not
/// `\x0Dic-state-root`, verification fails. We prove the separator is
/// load-bearing by verifying the SAME root_hash with a different prefix and a
/// real key — it must NOT pass.
#[test]
fn wrong_domain_separator_rejected() {
    let cert_cbor = inner_certificate(NNS_CERT);
    let cert = Certificate::from_cbor(&cert_cbor).unwrap();
    let root_hash = cert.tree.digest();
    let key = extract_der(&IC_ROOT_KEY).unwrap();

    // Correct separator over this exact root_hash is what the real signature
    // covers; a different separator yields a different message that the real
    // signature cannot satisfy.
    let mut wrong_msg = Vec::new();
    wrong_msg.extend_from_slice(b"\x0Dic-STATE-root"); // corrupted separator
    wrong_msg.extend_from_slice(&root_hash);
    let r = ic_verify_bls_signature::verify_bls_signature(&cert.signature, &wrong_msg, key);
    assert!(r.is_err(), "a wrong domain separator must not verify");

    // Sanity: the correct separator DOES verify (otherwise the test is vacuous).
    let mut ok_msg = Vec::new();
    ok_msg.extend_from_slice(IC_STATE_ROOT_DOMAIN_SEPARATOR);
    ok_msg.extend_from_slice(&root_hash);
    assert!(ic_verify_bls_signature::verify_bls_signature(&cert.signature, &ok_msg, key).is_ok());
}

/// Target canister id OUTSIDE the subnet's certified ranges -> rejected, even
/// though the delegation chain itself is valid.
#[test]
fn canister_out_of_range_rejected() {
    let cert = inner_certificate(DELEGATED_CERT);
    let now = cert_time(&cert);
    // A canister id far outside the captured ranges (all-0xFF in the high bytes
    // beyond the certified prefix). The delegation's ranges cover
    // 0x..f000.. .. 0x..ffffff.., but with the high prefix bytes 00 00 00 00 00.
    // Use a clearly-out-of-range id with a different leading byte region.
    let out = &[0x01u8, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0x01, 0x01];
    let err = verify_certificate(&cert, out, RootKey::Mainnet, now, WIDE).unwrap_err();
    assert_eq!(err, CertError::CanisterOutOfRange);
}

/// A multi-hop delegation (delegation cert that itself carries a delegation)
/// MUST be rejected before any signature is trusted.
#[test]
fn multi_hop_delegation_rejected() {
    let cert = inner_certificate(DELEGATED_CERT);
    let now = cert_time(&cert);
    // Synthesize a nested delegation: wrap the real delegated cert so that its
    // inner delegation certificate ALSO has a delegation.
    let nested = make_nested_delegation(&cert);
    let err = verify_certificate(&nested, XWPQ_ID, RootKey::Mainnet, now, WIDE).unwrap_err();
    assert_eq!(err, CertError::NestedDelegation);
}

/// Freshness: a certificate whose /time is far in the past is rejected.
#[test]
fn stale_time_rejected() {
    let cert = inner_certificate(NNS_CERT);
    let t = cert_time(&cert);
    // now = cert time + 10 minutes; window is 5 minutes -> too far in the past.
    let now = t + 600_000_000_000;
    let err =
        verify_certificate(&cert, RYJL3_ID, RootKey::Mainnet, now, DEFAULT_MAX_TIME_OFFSET_NS)
            .unwrap_err();
    match err {
        CertError::TimeOutOfRange { .. } => {}
        other => panic!("expected TimeOutOfRange, got {other:?}"),
    }
}

/// Freshness: a certificate whose /time is far in the future is rejected.
#[test]
fn future_time_rejected() {
    let cert = inner_certificate(NNS_CERT);
    let t = cert_time(&cert);
    // now = cert time - 10 minutes -> cert is 10 min in the "future".
    let now = t - 600_000_000_000;
    let err =
        verify_certificate(&cert, RYJL3_ID, RootKey::Mainnet, now, DEFAULT_MAX_TIME_OFFSET_NS)
            .unwrap_err();
    match err {
        CertError::TimeOutOfRange { .. } => {}
        other => panic!("expected TimeOutOfRange, got {other:?}"),
    }
}

/// Garbage / non-CBOR input fails closed.
#[test]
fn garbage_input_rejected() {
    let err = verify_certificate(&[0xff, 0x00, 0x13, 0x37], RYJL3_ID, RootKey::Mainnet, 0, WIDE)
        .unwrap_err();
    assert!(matches!(err, CertError::Cbor | CertError::Structure));
}

/// The pinned root key is exactly 133 bytes and decodes to a valid 96-byte
/// raw G2 key under the DER prefix.
#[test]
fn pinned_root_key_shape() {
    assert_eq!(IC_ROOT_KEY.len(), 133);
    let raw = extract_der(&IC_ROOT_KEY).unwrap();
    assert_eq!(raw.len(), 96);
}

/// `verify_response` rejects a body whose hash is not the certified one. We
/// build a fresh deterministic certificate (so we control the certified leaf)
/// using the real BLS primitives, then prove a tampered body fails closed and
/// the exact body passes.
#[test]
fn verify_response_body_hash_enforced() {
    use ic_verify_bls_signature::PrivateKey;
    // Build a minimal certificate we control end to end, signed by a throwaway
    // key acting as an explicit "local root", and prove the body-hash binding.
    let body = b"the exact certified bytes";
    let body_hash = sha(&[body]);

    // Deterministic test key (small scalar, in range).
    let sk = PrivateKey::deserialize(&[7u8; 32]).unwrap();
    let pk_raw = sk.public_key().serialize(); // 96-byte raw G2

    // Wrap the raw 96-byte key in the IC DER prefix so extract_der accepts it.
    let mut der_key = Vec::new();
    der_key.extend_from_slice(&DER_PREFIX);
    der_key.extend_from_slice(&pk_raw);

    // tree = Fork(Labeled("reply", Leaf(body_hash)), Labeled("time", Leaf(leb)))
    // so both the certified body hash and freshness are present.
    let now: u128 = 1_700_000_000_000_000_000;
    let mut time_leb = Vec::new();
    leb128::write::unsigned(&mut time_leb, now as u64).unwrap();
    let tree = Node::Fork(
        Box::new(Node::Labeled(
            b"reply".to_vec(),
            Box::new(Node::Leaf(body_hash.to_vec())),
        )),
        Box::new(Node::Labeled(b"time".to_vec(), Box::new(Node::Leaf(time_leb)))),
    );
    let root_hash = tree.digest();
    let mut msg = Vec::new();
    msg.extend_from_slice(IC_STATE_ROOT_DOMAIN_SEPARATOR);
    msg.extend_from_slice(&root_hash);
    let sig = sk.sign(&msg).serialize();

    let cert_cbor = encode_certificate(&tree, &sig, None);

    // Correct body verifies.
    let ok = verify_response(
        &cert_cbor,
        RYJL3_ID,
        &[b"reply"],
        body,
        RootKey::Local(&der_key),
        now,
        DEFAULT_MAX_TIME_OFFSET_NS,
    );
    assert!(ok.is_ok(), "exact certified body must verify: {ok:?}");

    // Tampered body fails closed.
    let bad = verify_response(
        &cert_cbor,
        RYJL3_ID,
        &[b"reply"],
        b"a different body",
        RootKey::Local(&der_key),
        now,
        DEFAULT_MAX_TIME_OFFSET_NS,
    );
    assert_eq!(bad.unwrap_err(), CertError::Signature);
}

// ---------------------------------------------------------------------------
// HARDENING — parser-differential + principal-ordering fixes (review Slice 4).
// ---------------------------------------------------------------------------

/// Trailing bytes after a complete certificate CBOR must be rejected
/// (deterministic-CBOR rule; ciborium's `from_reader` would otherwise silently
/// ignore them, a parser-differential vs the boundary node / canister).
#[test]
fn trailing_bytes_after_certificate_rejected() {
    let cert = inner_certificate(NNS_CERT);
    let now = cert_time(&cert);
    // Sanity: the clean cert verifies.
    assert!(verify_certificate(&cert, RYJL3_ID, RootKey::Mainnet, now, WIDE).is_ok());
    // Append one garbage byte -> must fail closed at decode.
    let mut with_trailer = cert.clone();
    with_trailer.push(0x00);
    let err = verify_certificate(&with_trailer, RYJL3_ID, RootKey::Mainnet, now, WIDE).unwrap_err();
    assert_eq!(err, CertError::Cbor);
}

/// A `Certificate` map carrying a DUPLICATE `tree` key must be rejected (the
/// old `BTreeMap` insert let the last occurrence silently win). Deterministic
/// CBOR forbids duplicate keys.
#[test]
fn duplicate_map_key_rejected() {
    use ciborium::value::Value as V;
    let cert = Certificate::from_cbor(&inner_certificate(NNS_CERT)).unwrap();
    // Hand-build a cert map with `tree` present TWICE (valid + a decoy empty).
    let map = V::Map(vec![
        (V::Text("tree".into()), encode_node(&cert.tree)),
        (V::Text("tree".into()), encode_node(&Node::Empty)),
        (V::Text("signature".into()), V::Bytes(cert.signature.clone())),
    ]);
    let mut out = Vec::new();
    ciborium::ser::into_writer(&map, &mut out).unwrap();
    let err = verify_certificate(&out, RYJL3_ID, RootKey::Mainnet, 0, WIDE).unwrap_err();
    assert_eq!(err, CertError::Structure);
}

/// Trailing bytes after a `canister_ranges` blob are rejected by the strict
/// decoder used in delegation verification.
#[test]
fn trailing_bytes_in_canister_ranges_rejected() {
    use ciborium::value::Value as V;
    // A minimal valid ranges blob: [[lo, hi]].
    let lo = vec![0u8; 10];
    let hi = vec![0xffu8; 10];
    let ranges = V::Array(vec![V::Array(vec![V::Bytes(lo), V::Bytes(hi)])]);
    let mut blob = Vec::new();
    ciborium::ser::into_writer(&ranges, &mut blob).unwrap();
    assert!(parse_canister_ranges(&blob).is_ok());
    blob.push(0xff); // trailing garbage
    assert_eq!(parse_canister_ranges(&blob).unwrap_err(), CertError::BadRanges);
}

/// The canister-range membership test must use the IC `Principal` ordering
/// `(len, then bytes)`, NOT raw lexicographic. At a length boundary the two
/// disagree: target `[0x02]` is OUT of range `[ [0x01], [0x01,0xFF] ]` under the
/// reference (len 1 vs the hi-bound's len 2 means `[0x02] < [0x01,0xFF]` is
/// FALSE -> [0x02] sorts AFTER the hi bound -> out of range), whereas raw
/// lexicographic would (wrongly) place `[0x02]` inside because `0x02 > 0x01`
/// byte-wise puts it past lo and `[0x02] <= [0x01,0xFF]`? No — raw-lex
/// `[0x02] > [0x01,0xFF]`. The KEY property we lock: differing-length bounds are
/// ordered length-first, matching `ic_principal::Principal::Ord`.
#[test]
fn canister_in_ranges_uses_principal_ordering() {
    // len-first ordering: [0x01] < [0x02] < [0x01, 0x00] (len 1 before len 2).
    let lo = vec![0x01u8];
    let hi = vec![0x02u8];
    let ranges = vec![(lo, hi)];
    // [0x01,0x00] has len 2 -> sorts AFTER every len-1 principal -> OUT of a
    // len-1..len-1 range, even though raw-lex would put [0x01,0x00] just above
    // [0x01] and below... it would compare [0x01,0x00] > [0x02] is false by lex,
    // so raw-lex would WRONGLY include it. principal_cmp must EXCLUDE it.
    assert!(
        !canister_in_ranges(&[0x01u8, 0x00], &ranges),
        "len-2 id must be out of a len-1..len-1 range under Principal ordering"
    );
    // A len-1 id inside the range is included.
    assert!(canister_in_ranges(&[0x01u8], &ranges));
    assert!(canister_in_ranges(&[0x02u8], &ranges));
    // Real 10-byte mainnet ids: ordering coincides with raw-lex (regression
    // guard that the fix did not break the normal path).
    let lo10 = vec![0x00u8; 10];
    let mut hi10 = vec![0x00u8; 10];
    hi10[5] = 0xff;
    let r10 = vec![(lo10, hi10)];
    assert!(canister_in_ranges(
        &[0x00, 0x00, 0x00, 0x00, 0x00, 0x10, 0, 0, 0, 0],
        &r10
    ));
}

/// `collect_leaves` must gather EVERY shard leaf (not just the first), so a
/// canister covered by a later shard is accepted. We assert the helper unions
/// all leaves in a multi-leaf fork.
#[test]
fn collect_leaves_gathers_all_shards() {
    let tree = Node::Fork(
        Box::new(Node::Labeled(b"a".to_vec(), Box::new(Node::Leaf(vec![1])))),
        Box::new(Node::Fork(
            Box::new(Node::Labeled(b"b".to_vec(), Box::new(Node::Leaf(vec![2])))),
            Box::new(Node::Labeled(b"c".to_vec(), Box::new(Node::Pruned([0u8; 32])))),
        )),
    );
    let mut out: Vec<&[u8]> = Vec::new();
    collect_leaves(&tree, &mut out);
    assert_eq!(out, vec![&[1u8][..], &[2u8][..]]);
}

// ---------------------------------------------------------------------------
// CBOR mutation helpers — re-encode certificates with one thing changed.
// ---------------------------------------------------------------------------

/// Encode a Node back to the IC CBOR array form.
fn encode_node(node: &Node) -> ciborium::value::Value {
    use ciborium::value::Value as V;
    match node {
        Node::Empty => V::Array(vec![V::Integer(0u8.into())]),
        Node::Fork(l, r) => V::Array(vec![
            V::Integer(1u8.into()),
            encode_node(l),
            encode_node(r),
        ]),
        Node::Labeled(label, child) => V::Array(vec![
            V::Integer(2u8.into()),
            V::Bytes(label.clone()),
            encode_node(child),
        ]),
        Node::Leaf(v) => V::Array(vec![V::Integer(3u8.into()), V::Bytes(v.clone())]),
        Node::Pruned(h) => V::Array(vec![V::Integer(4u8.into()), V::Bytes(h.to_vec())]),
    }
}

fn encode_certificate(tree: &Node, sig: &[u8], delegation: Option<(&[u8], &[u8])>) -> Vec<u8> {
    use ciborium::value::Value as V;
    let mut map = vec![
        (V::Text("tree".into()), encode_node(tree)),
        (V::Text("signature".into()), V::Bytes(sig.to_vec())),
    ];
    if let Some((subnet_id, inner)) = delegation {
        let del = V::Map(vec![
            (V::Text("subnet_id".into()), V::Bytes(subnet_id.to_vec())),
            (V::Text("certificate".into()), V::Bytes(inner.to_vec())),
        ]);
        map.push((V::Text("delegation".into()), del));
    }
    let mut out = Vec::new();
    ciborium::ser::into_writer(&V::Map(map), &mut out).unwrap();
    out
}

/// Re-encode a certificate with one signature byte flipped.
fn mutate_signature(cert_cbor: &[u8]) -> Vec<u8> {
    let cert = Certificate::from_cbor(cert_cbor).unwrap();
    let mut sig = cert.signature.clone();
    sig[0] ^= 0x01;
    let del = cert
        .delegation
        .as_ref()
        .map(|d| (d.subnet_id.as_slice(), d.certificate.as_slice()));
    encode_certificate(&cert.tree, &sig, del)
}

/// Re-encode a certificate, replacing the signature with arbitrary bytes.
fn replace_signature(cert_cbor: &[u8], new_sig: &[u8]) -> Vec<u8> {
    let cert = Certificate::from_cbor(cert_cbor).unwrap();
    let del = cert
        .delegation
        .as_ref()
        .map(|d| (d.subnet_id.as_slice(), d.certificate.as_slice()));
    encode_certificate(&cert.tree, new_sig, del)
}

/// Re-encode a certificate with one byte of the /time leaf flipped (keeps the
/// signature, so the tree-hash mismatch must be caught by the signature check).
fn mutate_time_leaf(cert_cbor: &[u8]) -> Vec<u8> {
    let cert = Certificate::from_cbor(cert_cbor).unwrap();
    let tree = flip_time_leaf(&cert.tree);
    let del = cert
        .delegation
        .as_ref()
        .map(|d| (d.subnet_id.as_slice(), d.certificate.as_slice()));
    encode_certificate(&tree, &cert.signature, del)
}

fn flip_time_leaf(node: &Node) -> Node {
    match node {
        Node::Labeled(l, c) if l.as_slice() == b"time" => {
            if let Node::Leaf(v) = c.as_ref() {
                let mut v = v.clone();
                v[0] ^= 0x01;
                Node::Labeled(l.clone(), Box::new(Node::Leaf(v)))
            } else {
                Node::Labeled(l.clone(), Box::new(flip_time_leaf(c)))
            }
        }
        Node::Labeled(l, c) => Node::Labeled(l.clone(), Box::new(flip_time_leaf(c))),
        Node::Fork(l, r) => Node::Fork(Box::new(flip_time_leaf(l)), Box::new(flip_time_leaf(r))),
        other => other.clone(),
    }
}

/// Build a certificate that has a delegation whose inner certificate ITSELF has
/// a delegation (multi-hop). We reuse the real delegated cert's structure: take
/// its delegation, and give the inner cert a (bogus) delegation so the
/// nested-delegation guard fires.
fn make_nested_delegation(cert_cbor: &[u8]) -> Vec<u8> {
    let cert = Certificate::from_cbor(cert_cbor).unwrap();
    let del = cert.delegation.as_ref().expect("fixture has a delegation");
    let inner = Certificate::from_cbor(&del.certificate).unwrap();

    // Re-encode the inner delegation cert WITH a delegation field added, so it
    // is now multi-hop. The inner delegation's subnet_id/certificate content is
    // irrelevant — the guard must reject before reading it.
    let inner_with_del = encode_certificate(
        &inner.tree,
        &inner.signature,
        Some((&del.subnet_id, &del.certificate)),
    );

    // Outer cert points its delegation at this now-nested inner cert.
    encode_certificate(
        &cert.tree,
        &cert.signature,
        Some((&del.subnet_id, &inner_with_del)),
    )
}
