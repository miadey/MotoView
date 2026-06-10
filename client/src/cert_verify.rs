//! Slice 4 — the chain-key certificate verifier (LIBRARY, not yet wired in).
//!
//! A native MotoView client must NOT trust the boundary node: before acting on
//! any consequential IC response it should verify the response's certificate
//! against a **pinned** NNS root public key — never `fetchRootKey` on mainnet.
//!
//! STATUS: this module is the verified, tested *primitive* for that check. It
//! is compiled only under the `cert-verify` cargo feature and is **not yet
//! called from `lib.rs`'s response path** (`mv_on_response` still acts on the
//! parsed body without invoking it). Wiring `verify_response` into the brain
//! before `Bridge::apply` — and exporting it across the UniFFI/extern-C
//! boundary so native shells can drive it — is the separate integration step.
//! Nothing here is a deployed control until that wiring lands.
//!
//! This mirrors, step for step, the reference Rust verifier the IC ships
//! (`ic-certificate-verification` 3.2.0 →
//! `ic-certificate-verification-3.2.0/src/certificate_verification.rs`, and
//! `ic-agent`'s `verify_cert`):
//!
//!   1. CBOR-decode `Certificate { tree, signature, delegation? }`.
//!   2. Reconstruct the HashTree root hash with the IC's labeled-SHA-256
//!      construction and the four `ic-hashtree-*` domain separators. This is
//!      byte-for-byte identical to the canister side in `runtime/src/CertV2.mo`
//!      (`hash()` / `domainSep()`), and to `ic-certification`'s `digest()`.
//!   3. BLS verify: `message = b"\x0Dic-state-root" || root_hash`, checked with
//!      `ic_verify_bls_signature::verify_bls_signature` — the SAME crate the IC
//!      reference uses (signature on G1, public key on G2, hash-to-G1 with DST
//!      `BLS_SIG_BLS12381G1_XMD:SHA-256_SSWU_RO_NUL_`).
//!   4. Delegation: verify the delegation certificate against the **root** key,
//!      reject any nested delegation, assert the target canister id is inside
//!      the subnet's certified `canister_ranges`, then use the subnet
//!      `public_key` (not root) to verify the outer certificate.
//!   5. Freshness: decode the certified `/time` (unsigned LEB128) and reject if
//!      it is outside `±max_offset` of `now`.
//!
//! Every check fails closed: a `verify_response` that returns `Ok` means the
//! certificate chains to the pinned root, covers the requested canister, is
//! fresh, and (when checked) certifies the exact response body hash.
//!
//! References pinned at implementation time (cited so nothing is transcribed
//! from memory):
//!   * `dfinity/agent-rs` `ic-agent/src/agent/mod.rs`:
//!       `IC_STATE_ROOT_DOMAIN_SEPARATOR = b"\x0Dic-state-root"`,
//!       `IC_ROOT_KEY` (133-byte DER), `DER_PREFIX = [48,42,...]` for subnet keys.
//!   * `ic-certificate-verification-3.2.0` `certificate_verification.rs`:
//!       `DER_PREFIX` (37 bytes) + `KEY_LENGTH = 96`, the delegation /
//!       canister-range / time logic reproduced below.
//!   * `ic-certification-3.2.0` `hash_tree/mod.rs::digest` — domain separators
//!       and node hashing.

extern crate alloc;

use alloc::collections::BTreeMap;
use alloc::vec::Vec;
use ciborium::value::Value as Cbor;
use sha2::{Digest, Sha256};

// ---------------------------------------------------------------------------
// Pinned constants — DO NOT edit without re-deriving from an authoritative
// dfinity source. These are copied verbatim from the live `dfinity/agent-rs`
// `ic-agent/src/agent/mod.rs` and `ic-certificate-verification` sources.
// ---------------------------------------------------------------------------

/// The canonical mainnet NNS root public key, DER-encoded (133 bytes), exactly
/// as embedded in `ic-agent`'s `IC_ROOT_KEY`. The trailing 96 bytes are the
/// BLS12-381 G2 public key; the leading 37 bytes are the SPKI/DER prefix
/// (`DER_PREFIX` below).
///
/// Source: `dfinity/agent-rs` `ic-agent/src/agent/mod.rs`
///   `pub(crate) const IC_ROOT_KEY: &[u8; 133] = b"\x30\x81\x82..."`.
pub const IC_ROOT_KEY: [u8; 133] = [
    0x30, 0x81, 0x82, 0x30, 0x1d, 0x06, 0x0d, 0x2b, 0x06, 0x01, 0x04, 0x01, 0x82, 0xdc, 0x7c, 0x05,
    0x03, 0x01, 0x02, 0x01, 0x06, 0x0c, 0x2b, 0x06, 0x01, 0x04, 0x01, 0x82, 0xdc, 0x7c, 0x05, 0x03,
    0x02, 0x01, 0x03, 0x61, 0x00, 0x81, 0x4c, 0x0e, 0x6e, 0xc7, 0x1f, 0xab, 0x58, 0x3b, 0x08, 0xbd,
    0x81, 0x37, 0x3c, 0x25, 0x5c, 0x3c, 0x37, 0x1b, 0x2e, 0x84, 0x86, 0x3c, 0x98, 0xa4, 0xf1, 0xe0,
    0x8b, 0x74, 0x23, 0x5d, 0x14, 0xfb, 0x5d, 0x9c, 0x0c, 0xd5, 0x46, 0xd9, 0x68, 0x5f, 0x91, 0x3a,
    0x0c, 0x0b, 0x2c, 0xc5, 0x34, 0x15, 0x83, 0xbf, 0x4b, 0x43, 0x92, 0xe4, 0x67, 0xdb, 0x96, 0xd6,
    0x5b, 0x9b, 0xb4, 0xcb, 0x71, 0x71, 0x12, 0xf8, 0x47, 0x2e, 0x0d, 0x5a, 0x4d, 0x14, 0x50, 0x5f,
    0xfd, 0x74, 0x84, 0xb0, 0x12, 0x91, 0x09, 0x1c, 0x5f, 0x87, 0xb9, 0x88, 0x83, 0x46, 0x3f, 0x98,
    0x09, 0x1a, 0x0b, 0xaa, 0xae,
];

/// The 37-byte DER/SPKI prefix in front of every IC BLS12-381 G2 public key
/// (root key and subnet keys alike). Source:
/// `ic-certificate-verification-3.2.0` `const DER_PREFIX: &[u8; 37]`.
const DER_PREFIX: [u8; 37] = [
    0x30, 0x81, 0x82, 0x30, 0x1d, 0x06, 0x0d, 0x2b, 0x06, 0x01, 0x04, 0x01, 0x82, 0xdc, 0x7c, 0x05,
    0x03, 0x01, 0x02, 0x01, 0x06, 0x0c, 0x2b, 0x06, 0x01, 0x04, 0x01, 0x82, 0xdc, 0x7c, 0x05, 0x03,
    0x02, 0x01, 0x03, 0x61, 0x00,
];

/// Raw BLS12-381 G2 public key length (compressed). Source: same crate,
/// `const KEY_LENGTH: usize = 96`.
const BLS_KEY_LEN: usize = 96;

/// The `ic-state-root` signing domain separator: `byte(13) || "ic-state-root"`.
/// Source: `ic-agent` `IC_STATE_ROOT_DOMAIN_SEPARATOR = b"\x0Dic-state-root"`.
const IC_STATE_ROOT_DOMAIN_SEPARATOR: &[u8; 14] = b"\x0Dic-state-root";

/// Default freshness window: reject certificates whose `/time` is more than
/// this far from `now`, in nanoseconds (5 minutes — the IC reference default
/// `MAX_CERT_TIME_OFFSET_NS`).
pub const DEFAULT_MAX_TIME_OFFSET_NS: u128 = 300_000_000_000;

// ---------------------------------------------------------------------------
// Errors — every failure path is named so callers (and tests) can assert the
// exact reason a certificate was rejected. There is NO catch-all "Ok".
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CertError {
    /// Outer CBOR (or an embedded certificate) failed to decode.
    Cbor,
    /// The `Certificate` map was missing `tree` or `signature`, or a field had
    /// the wrong CBOR type.
    Structure,
    /// The HashTree contained an unknown/invalid node tag.
    BadTree,
    /// A required path (e.g. `/time`, `/subnet/<id>/public_key`) was absent or
    /// pruned away in the tree.
    PathAbsent,
    /// The certified `/time` leaf was not valid unsigned LEB128.
    TimeDecode,
    /// `/time` is further than the allowed window from `now`.
    TimeOutOfRange { certificate_ns: u128, now_ns: u128 },
    /// A DER-wrapped BLS key was the wrong length or had the wrong prefix.
    DerKey,
    /// The BLS signature did not verify against the (sub)net key.
    Signature,
    /// A delegation certificate itself carried a delegation (multi-hop).
    NestedDelegation,
    /// `canister_ranges` could not be decoded.
    BadRanges,
    /// The target canister id is outside the subnet's certified ranges.
    CanisterOutOfRange,
}

pub type CertResult<T> = Result<T, CertError>;

// ---------------------------------------------------------------------------
// HashTree — decoded from CBOR, hashed exactly like the canister side.
// ---------------------------------------------------------------------------

/// The IC certified HashTree. Node encoding (CBOR array, integer tag first) is
/// identical to `runtime/src/CertV2.mo::encodeInto`:
///   [0]                = Empty
///   [1, left, right]   = Fork
///   [2, label, child]  = Labeled
///   [3, value]         = Leaf
///   [4, hash]          = Pruned
#[derive(Debug, Clone)]
enum Node {
    Empty,
    Fork(Box<Node>, Box<Node>),
    Labeled(Vec<u8>, Box<Node>),
    Leaf(Vec<u8>),
    Pruned([u8; 32]),
}

impl Node {
    fn from_cbor(v: &Cbor) -> CertResult<Node> {
        let arr = v.as_array().ok_or(CertError::BadTree)?;
        let tag = arr
            .first()
            .and_then(|t| t.as_integer())
            .and_then(|i| u8::try_from(i).ok())
            .ok_or(CertError::BadTree)?;
        match tag {
            0 => Ok(Node::Empty),
            1 => {
                if arr.len() != 3 {
                    return Err(CertError::BadTree);
                }
                let l = Node::from_cbor(&arr[1])?;
                let r = Node::from_cbor(&arr[2])?;
                Ok(Node::Fork(Box::new(l), Box::new(r)))
            }
            2 => {
                if arr.len() != 3 {
                    return Err(CertError::BadTree);
                }
                let label = bytes_of(&arr[1])?;
                let child = Node::from_cbor(&arr[2])?;
                Ok(Node::Labeled(label, Box::new(child)))
            }
            3 => {
                if arr.len() != 2 {
                    return Err(CertError::BadTree);
                }
                Ok(Node::Leaf(bytes_of(&arr[1])?))
            }
            4 => {
                if arr.len() != 2 {
                    return Err(CertError::BadTree);
                }
                let h = bytes_of(&arr[1])?;
                let h: [u8; 32] = h.as_slice().try_into().map_err(|_| CertError::BadTree)?;
                Ok(Node::Pruned(h))
            }
            _ => Err(CertError::BadTree),
        }
    }

    /// The 32-byte digest of this node. Byte-for-byte the construction in
    /// `runtime/src/CertV2.mo::hash` and `ic-certification`'s `digest()`:
    /// domain separator is `byte(|name|) || name`; pruned nodes contribute
    /// their stored hash directly (no re-hashing).
    fn digest(&self) -> [u8; 32] {
        match self {
            Node::Pruned(h) => *h,
            Node::Empty => sha(&[&dsep(b"ic-hashtree-empty")]),
            Node::Leaf(bytes) => sha(&[&dsep(b"ic-hashtree-leaf"), bytes]),
            Node::Labeled(label, child) => {
                sha(&[&dsep(b"ic-hashtree-labeled"), label, &child.digest()])
            }
            Node::Fork(l, r) => sha(&[&dsep(b"ic-hashtree-fork"), &l.digest(), &r.digest()]),
        }
    }

    /// Look up the leaf value at `path`, flattening forks. Mirrors the spec's
    /// `lookup_path`: a `Pruned` or absent label yields `PathAbsent` (we never
    /// invent data), and the terminal node must be a `Leaf`.
    fn lookup<'a>(&'a self, path: &[&[u8]]) -> CertResult<&'a [u8]> {
        match path.split_first() {
            None => match self {
                Node::Leaf(v) => Ok(v),
                _ => Err(CertError::PathAbsent),
            },
            Some((head, rest)) => {
                let child = self.find_label(head).ok_or(CertError::PathAbsent)?;
                child.lookup(rest)
            }
        }
    }

    /// Find the subtree under label `label`, descending through forks.
    fn find_label<'a>(&'a self, label: &[u8]) -> Option<&'a Node> {
        match self {
            Node::Labeled(l, child) if l.as_slice() == label => Some(child),
            Node::Fork(l, r) => l.find_label(label).or_else(|| r.find_label(label)),
            _ => None,
        }
    }

    /// Find the *subtree node* (not a leaf) at `path`, so a caller can enumerate
    /// children. Used for `canister_ranges` and similar.
    fn subtree<'a>(&'a self, path: &[&[u8]]) -> Option<&'a Node> {
        match path.split_first() {
            None => Some(self),
            Some((head, rest)) => self.find_label(head)?.subtree(rest),
        }
    }
}

/// `byte(|name|) || name` — the IC hash-tree domain separator.
fn dsep(name: &[u8]) -> Vec<u8> {
    let mut v = Vec::with_capacity(1 + name.len());
    v.push(name.len() as u8);
    v.extend_from_slice(name);
    v
}

fn sha(parts: &[&[u8]]) -> [u8; 32] {
    let mut h = Sha256::new();
    for p in parts {
        h.update(p);
    }
    h.finalize().into()
}

fn bytes_of(v: &Cbor) -> CertResult<Vec<u8>> {
    v.as_bytes().cloned().ok_or(CertError::Structure)
}

// ---------------------------------------------------------------------------
// Certificate — decoded from CBOR.
// ---------------------------------------------------------------------------

#[derive(Debug)]
struct Delegation {
    subnet_id: Vec<u8>,
    certificate: Vec<u8>,
}

#[derive(Debug)]
struct Certificate {
    tree: Node,
    signature: Vec<u8>,
    delegation: Option<Delegation>,
}

/// Strip the CBOR self-describe tag (55799) wrapper the IC emits in front of
/// certificates / trees / canister_ranges, returning the inner value. Tolerant
/// of either tagged or untagged input.
fn untag(v: &Cbor) -> &Cbor {
    match v {
        Cbor::Tag(55799, inner) => inner,
        other => other,
    }
}

/// Decode one complete CBOR value from `bytes`, REJECTING any trailing bytes
/// after the value. The IC mandates canonical/deterministic CBOR; ciborium's
/// `from_reader` would otherwise silently ignore appended garbage, opening a
/// parser-differential between this verifier, the boundary node, and the
/// canister. We read once, then assert the reader is at EOF.
fn decode_cbor_strict(bytes: &[u8], err: CertError) -> CertResult<Cbor> {
    let mut cursor = bytes;
    let v: Cbor = ciborium::de::from_reader(&mut cursor).map_err(|_| err.clone())?;
    // `cursor` is a `&[u8]` advanced past the consumed bytes; anything left is
    // trailing data and is rejected.
    if !cursor.is_empty() {
        return Err(err);
    }
    Ok(v)
}

/// Decode a CBOR map by string/bytes key into a lookup table. Tolerates (and
/// ignores) the self-describe tag 55799 wrapper that the IC emits, but REJECTS
/// duplicate keys: ciborium preserves duplicates and a naive `insert` would let
/// the last occurrence silently win (a parser-differential). Deterministic CBOR
/// forbids duplicate keys, so we fail closed instead.
fn cbor_map(v: &Cbor) -> CertResult<BTreeMap<Vec<u8>, Cbor>> {
    let entries = untag(v).as_map().ok_or(CertError::Structure)?;
    let mut m = BTreeMap::new();
    for (k, val) in entries {
        let key = match k {
            Cbor::Text(t) => t.as_bytes().to_vec(),
            Cbor::Bytes(b) => b.clone(),
            _ => continue,
        };
        if m.insert(key, val.clone()).is_some() {
            // Duplicate key -> reject (non-deterministic CBOR).
            return Err(CertError::Structure);
        }
    }
    Ok(m)
}

impl Certificate {
    fn from_cbor(bytes: &[u8]) -> CertResult<Certificate> {
        let v = decode_cbor_strict(bytes, CertError::Cbor)?;
        let m = cbor_map(&v)?;
        let tree_v = m.get(b"tree".as_slice()).ok_or(CertError::Structure)?;
        let tree = Node::from_cbor(tree_v)?;
        let signature = m
            .get(b"signature".as_slice())
            .and_then(|s| s.as_bytes().cloned())
            .ok_or(CertError::Structure)?;
        let delegation = match m.get(b"delegation".as_slice()) {
            None => None,
            Some(d) => {
                let dm = cbor_map(d)?;
                let subnet_id = dm
                    .get(b"subnet_id".as_slice())
                    .and_then(|s| s.as_bytes().cloned())
                    .ok_or(CertError::Structure)?;
                let certificate = dm
                    .get(b"certificate".as_slice())
                    .and_then(|s| s.as_bytes().cloned())
                    .ok_or(CertError::Structure)?;
                Some(Delegation {
                    subnet_id,
                    certificate,
                })
            }
        };
        Ok(Certificate {
            tree,
            signature,
            delegation,
        })
    }
}

// ---------------------------------------------------------------------------
// BLS key handling + signature check.
// ---------------------------------------------------------------------------

/// Strip the 37-byte DER prefix off a 133-byte IC BLS key, returning the raw
/// 96-byte G2 key. Mirrors `ic-certificate-verification::extract_der`: any
/// length or prefix mismatch is a hard error (fail closed).
fn extract_der(der: &[u8]) -> CertResult<&[u8]> {
    if der.len() != DER_PREFIX.len() + BLS_KEY_LEN {
        return Err(CertError::DerKey);
    }
    if der[..DER_PREFIX.len()] != DER_PREFIX[..] {
        return Err(CertError::DerKey);
    }
    Ok(&der[DER_PREFIX.len()..])
}

/// Verify the `ic-state-root` BLS signature on a certificate's tree against a
/// DER-wrapped (sub)net key. Uses the IC's own `verify_bls_signature` so the
/// curve assignment (sig∈G1, key∈G2) and ciphersuite match exactly.
fn verify_cert_signature(cert: &Certificate, der_key: &[u8]) -> CertResult<()> {
    let root_hash = cert.tree.digest();
    let mut msg = Vec::with_capacity(IC_STATE_ROOT_DOMAIN_SEPARATOR.len() + 32);
    msg.extend_from_slice(IC_STATE_ROOT_DOMAIN_SEPARATOR);
    msg.extend_from_slice(&root_hash);

    let key = extract_der(der_key)?;
    ic_verify_bls_signature::verify_bls_signature(&cert.signature, &msg, key)
        .map_err(|_| CertError::Signature)
}

// ---------------------------------------------------------------------------
// canister_ranges
// ---------------------------------------------------------------------------

/// Decode the certified `canister_ranges` leaf: CBOR `[[lo, hi], ...]` where
/// each bound is a principal byte string. (Optionally tag-55799 wrapped.)
fn parse_canister_ranges(blob: &[u8]) -> CertResult<Vec<(Vec<u8>, Vec<u8>)>> {
    let v = decode_cbor_strict(blob, CertError::BadRanges)?;
    let outer = untag(&v).as_array().ok_or(CertError::BadRanges)?;
    let mut ranges = Vec::with_capacity(outer.len());
    for pair in outer {
        let p = pair.as_array().ok_or(CertError::BadRanges)?;
        if p.len() != 2 {
            return Err(CertError::BadRanges);
        }
        let lo = p[0].as_bytes().cloned().ok_or(CertError::BadRanges)?;
        let hi = p[1].as_bytes().cloned().ok_or(CertError::BadRanges)?;
        ranges.push((lo, hi));
    }
    Ok(ranges)
}

/// Compare two principal byte strings using the IC's `Principal` `Ord`.
///
/// `candid`/`ic_principal`'s `Principal` derives `Ord` over
/// `(len: u8, [u8; 29] /* zero-padded */)`, i.e. it compares the LENGTH FIRST,
/// then the zero-padded 29-byte content. A plain lexicographic slice compare
/// over the *un-padded* slices diverges whenever the operands differ in length
/// (e.g. `[0x02]` vs `[0x01, 0xFF]`: lexicographic orders the latter first;
/// the reference orders `[0x02]` first because `len 1 < len 2`). For real
/// mainnet ids (all 10 bytes) the two coincide, but to be byte-for-byte the
/// reference — and to stay sound for any future/native-supplied id length — we
/// reproduce the `(len, padded)` ordering exactly.
fn principal_cmp(a: &[u8], b: &[u8]) -> core::cmp::Ordering {
    use core::cmp::Ordering;
    // Principals are at most 29 bytes; anything longer can never be a valid
    // principal and is treated as strictly greater by length (still total).
    match a.len().cmp(&b.len()) {
        Ordering::Equal => a.cmp(b),
        ne => ne,
    }
}

/// True iff `canister_id` falls within any `[lo, hi]` (inclusive) range, using
/// the IC `Principal` ordering above.
fn canister_in_ranges(canister_id: &[u8], ranges: &[(Vec<u8>, Vec<u8>)]) -> bool {
    use core::cmp::Ordering::Greater;
    ranges.iter().any(|(lo, hi)| {
        principal_cmp(lo, canister_id) != Greater && principal_cmp(canister_id, hi) != Greater
    })
}

// ---------------------------------------------------------------------------
// Delegation
// ---------------------------------------------------------------------------

/// Resolve the key that should have signed `cert`. With no delegation that is
/// the (DER) root key; with a delegation we verify the delegation certificate
/// against the root key, reject nested delegations, assert the target canister
/// is in range, and return the subnet's DER public key.
///
/// `root_der_key` is always the pinned root for the *delegation* check — the
/// delegation cert is signed by the root. Mirrors
/// `ic-certificate-verification::verify_delegation`.
fn resolve_signing_key(
    cert: &Certificate,
    canister_id: &[u8],
    root_der_key: &[u8],
) -> CertResult<Vec<u8>> {
    match &cert.delegation {
        None => Ok(root_der_key.to_vec()),
        Some(del) => {
            let inner = Certificate::from_cbor(&del.certificate)?;
            // Reject multi-hop delegation: a delegation cert may not itself
            // carry a delegation.
            if inner.delegation.is_some() {
                return Err(CertError::NestedDelegation);
            }
            // The delegation certificate is signed by the ROOT key.
            verify_cert_signature(&inner, root_der_key)?;

            // Assert the target canister is within the subnet's certified
            // ranges. Two on-wire layouts exist:
            //
            //   * classic, non-sharded: /subnet/<subnet_id>/canister_ranges is a
            //     single leaf holding ALL ranges (what mainnet serves for
            //     read_state today; the delegated fixture exercises this);
            //   * sharded: /canister_ranges/<subnet_id>/<shard> holds MANY leaves,
            //     one per shard, each a slice of the ranges (newer layout).
            //
            // We union the ranges from EVERY shard leaf (matching the IC
            // reference `verify_delegation`, which folds over all range keys),
            // never just the first shard — otherwise a canister legitimately
            // covered by a later shard would be wrongly rejected. If NO ranges
            // are found anywhere, that is a hard error (fail closed). If ranges
            // ARE found, the canister MUST be inside one of them.
            let sid = del.subnet_id.as_slice();
            let mut ranges: Vec<(Vec<u8>, Vec<u8>)> = Vec::new();
            if let Ok(blob) = inner.tree.lookup(&[b"subnet", sid, b"canister_ranges"]) {
                ranges.extend(parse_canister_ranges(blob)?);
            }
            if let Some(subtree) = inner.tree.subtree(&[b"canister_ranges", sid]) {
                let mut shard_blobs: Vec<&[u8]> = Vec::new();
                collect_leaves(subtree, &mut shard_blobs);
                for blob in shard_blobs {
                    ranges.extend(parse_canister_ranges(blob)?);
                }
            }
            if ranges.is_empty() {
                return Err(CertError::PathAbsent);
            }
            if !canister_in_ranges(canister_id, &ranges) {
                return Err(CertError::CanisterOutOfRange);
            }

            // Use the SUBNET key (not root) to verify the outer certificate.
            let subnet_der = inner
                .tree
                .lookup(&[b"subnet", sid, b"public_key"])?
                .to_vec();
            Ok(subnet_der)
        }
    }
}

/// Collect EVERY `Leaf` value found in a subtree (depth-first), used to union
/// all shard range blobs in the sharded `canister_ranges` layout. Pruned and
/// empty subtrees contribute nothing (we never invent data).
fn collect_leaves<'a>(node: &'a Node, out: &mut Vec<&'a [u8]>) {
    match node {
        Node::Leaf(v) => out.push(v),
        Node::Labeled(_, c) => collect_leaves(c, out),
        Node::Fork(l, r) => {
            collect_leaves(l, out);
            collect_leaves(r, out);
        }
        Node::Empty | Node::Pruned(_) => {}
    }
}

// ---------------------------------------------------------------------------
// Freshness
// ---------------------------------------------------------------------------

/// Read the certified `/time` (unsigned LEB128, nanoseconds) and enforce the
/// window. Returns the parsed time on success.
fn verify_time(cert: &Certificate, now_ns: u128, max_offset_ns: u128) -> CertResult<u128> {
    let mut leaf = cert.tree.lookup(&[b"time"])?;
    let t = leb128::read::unsigned(&mut leaf).map_err(|_| CertError::TimeDecode)? as u128;
    let max = now_ns.saturating_add(max_offset_ns);
    let min = now_ns.saturating_sub(max_offset_ns);
    if t > max || t < min {
        return Err(CertError::TimeOutOfRange {
            certificate_ns: t,
            now_ns,
        });
    }
    Ok(t)
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// A successfully verified certificate. Holds the parsed certified `/time` and
/// the verified tree so the caller can `lookup_*` certified values from it.
#[derive(Debug)]
pub struct VerifiedCertificate {
    cert: Certificate,
    pub time_ns: u128,
}

impl VerifiedCertificate {
    /// Look up a certified leaf value by path (e.g.
    /// `&[b"canister", canister_id, b"certified_data"]`).
    pub fn lookup(&self, path: &[&[u8]]) -> CertResult<&[u8]> {
        self.cert.tree.lookup(path)
    }
}

/// Whose root key to trust. Mainnet ALWAYS uses the pinned NNS key. A local
/// replica's key may be supplied explicitly — but only ever via `Local`, never
/// fetched silently, so a mainnet caller can never be tricked into trusting a
/// boundary-supplied key.
pub enum RootKey<'a> {
    /// The pinned mainnet NNS root key. The only correct choice for ic0.app /
    /// icp-api.io.
    Mainnet,
    /// A DER-encoded root key for a local/dev replica, passed explicitly by the
    /// operator. Use ONLY against a known local replica.
    Local(&'a [u8]),
}

impl RootKey<'_> {
    fn der(&self) -> &[u8] {
        match self {
            RootKey::Mainnet => &IC_ROOT_KEY,
            RootKey::Local(k) => k,
        }
    }
}

/// Verify a CBOR-encoded `Certificate` for `canister_id` against `root` at time
/// `now_ns`, enforcing the `max_offset_ns` freshness window. This is the full
/// chain: tree-hash reconstruction → (delegation → ranges → subnet key) → BLS
/// signature → freshness.
///
/// On success the returned `VerifiedCertificate` is the ONLY trustworthy view
/// of the response; read certified values from it, never from the raw bytes.
pub fn verify_certificate(
    cert_cbor: &[u8],
    canister_id: &[u8],
    root: RootKey<'_>,
    now_ns: u128,
    max_offset_ns: u128,
) -> CertResult<VerifiedCertificate> {
    let cert = Certificate::from_cbor(cert_cbor)?;

    // 1) Resolve the key that must have signed this certificate (root, or via a
    //    fully-checked delegation → subnet key).
    let signing_der = resolve_signing_key(&cert, canister_id, root.der())?;

    // 2) BLS-verify the outer certificate against that key.
    verify_cert_signature(&cert, &signing_der)?;

    // 3) Freshness.
    let time_ns = verify_time(&cert, now_ns, max_offset_ns)?;

    Ok(VerifiedCertificate {
        cert,
        time_ns,
    })
}

/// Verify a full certificate AND bind a response `body` to it, for the
/// **`certified_data`-equals-`SHA256(body)`** certification mode: the leaf at
/// `cert_path` must hold exactly `SHA256(body)`. Returns the certified `/time`.
///
/// SCOPE / CAVEAT: this is the simple "the certified leaf IS the body hash"
/// binding. It does **NOT** implement the HTTP-Gateway response-certification v2
/// (`http_expr`) path that `runtime/src/CertV2.mo` uses to certify served HTTP
/// bodies — there the leaf is a *representation-independent* `response_hash`
/// over status + certified headers + `SHA256(body)`, reached by walking an
/// `expr_path`, not a bare `certified_data == SHA256(body)` leaf. So this
/// function correctly verifies update-call replies / `certified_data` witnesses,
/// but it CANNOT yet verify a MotoView v2-certified HTTP response body. Before
/// wiring the v2 HTTP path into the brain, add an `http_expr` walker that
/// recomputes `response_hash` exactly as `CertV2.responseHash` does.
///
/// `cert_path` is the tree path whose leaf holds `SHA256(body)`, e.g. a
/// per-response witness path or `&[b"canister", id, b"certified_data"]`.
#[allow(clippy::too_many_arguments)]
pub fn verify_response(
    cert_cbor: &[u8],
    canister_id: &[u8],
    cert_path: &[&[u8]],
    body: &[u8],
    root: RootKey<'_>,
    now_ns: u128,
    max_offset_ns: u128,
) -> CertResult<u128> {
    let verified = verify_certificate(cert_cbor, canister_id, root, now_ns, max_offset_ns)?;
    let certified = verified.lookup(cert_path)?;
    // The certified leaf is the SHA-256 of the body. Constant in length; a
    // mismatch (tampered body) fails closed.
    let body_hash = sha(&[body]);
    if certified != body_hash {
        return Err(CertError::Signature);
    }
    Ok(verified.time_ns)
}

#[cfg(test)]
mod tests;
