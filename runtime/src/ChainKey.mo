/// Chain-key wallet primitives: threshold ECDSA + threshold Schnorr from the IC
/// management canister (`aaaaa-aa`).
///
/// ## Threshold security model
/// There is no private key sitting in this canister, on one node, or anywhere a
/// single party can read it. The signing key is *secret-shared* across the nodes
/// of a signing subnet; producing a signature requires a threshold of nodes to
/// cooperate, and no subset below the threshold ever learns the key. The IC's
/// consensus tolerates `< 1/3` Byzantine nodes, so a signature is only ever
/// produced when honest nodes agree — the canister authorises *what* gets signed
/// (here: a per-user, per-chain derivation path bound to the caller's principal),
/// the subnet does the cryptography. This is what lets a canister custody a
/// Bitcoin / Ethereum / Solana address without ever holding its private key.
///
/// This module wraps four management-canister methods:
///   * `ecdsa_public_key` / `sign_with_ecdsa`        (curve `secp256k1`)
///   * `schnorr_public_key` / `sign_with_schnorr`    (`ed25519` and `bip340secp256k1`)
/// using the current (post-`2024`) Candid shapes.
///
/// ## Key names (env gate)
/// Local dfx ships `dfx_test_key`; the IC has a beta `test_key_1` and the
/// production `key_1`. `keyName` normalises the network string (trim whitespace +
/// lowercase) and matches "ic"/"mainnet", exactly like the compiler's
/// `vetkd_key_name` gate (`compiler/src/project.rs`). `requireKeyName` is the
/// hard-fail backstop equivalent of that gate's `enforce_network_gate`: it traps
/// if the LOCAL `dfx_test_key` would be used for a mainnet target.
///
/// ## CAVEAT — this layer authorises *what path*, not *what spend*
/// This module only authorises WHICH derivation path is signed (a path bound to
/// the caller's principal). It does NOT bind the signed bytes to a spend intent,
/// does NOT prevent replay, and does NOT certify the response. A safe wallet must
/// layer intentHash-bound single-use tokens + a stable consumed-nonce store +
/// per-principal velocity limits + (native) hardware assertion in
/// `App.mo`/`Security.mo` BEFORE calling `signWithEcdsa`/`signWithSchnorr` — see
/// Phase 5 in `docs/native-vision-and-plan.md`.
///
/// ## Cycle costs
/// On mainnet a single `sign_with_ecdsa` / `sign_with_schnorr` costs ~26B cycles
/// (the exact figure depends on the subnet size; the 34-node fiduciary subnet is
/// 26_153_846_153). We attach a margin so the call does not under-fund. Reading a
/// public key is free.
import Principal "mo:base/Principal";
import Text "mo:base/Text";
import Char "mo:base/Char";
import Blob "mo:base/Blob";
import Debug "mo:base/Debug";
import Hex "Hex";

module {
  // ── Management-canister Candid types ──────────────────────────────────────

  public type EcdsaCurve = { #secp256k1 };
  public type EcdsaKeyId = { curve : EcdsaCurve; name : Text };

  public type SchnorrAlgorithm = { #ed25519; #bip340secp256k1 };
  public type SchnorrKeyId = { algorithm : SchnorrAlgorithm; name : Text };

  /// BIP-341 taproot tweak for `bip340secp256k1`. `null` = no tweak (BIP-340
  /// key-spend with an untweaked key); `?{ merkle_root_hash = blob }` applies the
  /// taproot tweak (`blob` may be the empty blob for a key-path-only output).
  public type SignWithSchnorrAux = {
    #bip341 : { merkle_root_hash : Blob };
  };

  type Mgmt = actor {
    ecdsa_public_key : ({
      canister_id : ?Principal;
      derivation_path : [Blob];
      key_id : EcdsaKeyId;
    }) -> async ({ public_key : Blob; chain_code : Blob });

    sign_with_ecdsa : ({
      message_hash : Blob;
      derivation_path : [Blob];
      key_id : EcdsaKeyId;
    }) -> async ({ signature : Blob });

    schnorr_public_key : ({
      canister_id : ?Principal;
      derivation_path : [Blob];
      key_id : SchnorrKeyId;
    }) -> async ({ public_key : Blob; chain_code : Blob });

    sign_with_schnorr : ({
      message : Blob;
      derivation_path : [Blob];
      key_id : SchnorrKeyId;
      aux : ?SignWithSchnorrAux;
    }) -> async ({ signature : Blob });
  };

  let mgmt : Mgmt = actor "aaaaa-aa";

  // 34-node fiduciary subnet charges 26_153_846_153 cycles per signature; margin.
  let SIGN_CYCLES : Nat = 30_000_000_000;

  // ── Async management-canister calls ───────────────────────────────────────

  /// SEC1-encoded compressed (33-byte) secp256k1 public key for `derivationPath`
  /// under `keyName`. Free. This is the key you turn into a BTC/ETH address.
  public func ecdsaPublicKey(keyName : Text, derivationPath : [Blob]) : async Blob {
    let r = await mgmt.ecdsa_public_key({
      canister_id = null;
      derivation_path = derivationPath;
      key_id = { curve = #secp256k1; name = keyName };
    });
    r.public_key;
  };

  /// Threshold-ECDSA signature (64-byte `r||s`) over the 32-byte `messageHash`.
  /// `messageHash` MUST already be the hash you intend to sign (e.g. the
  /// double-SHA256 sighash for Bitcoin, or keccak256 for Ethereum).
  public func signWithEcdsa(keyName : Text, derivationPath : [Blob], messageHash : Blob) : async Blob {
    let r = await (with cycles = SIGN_CYCLES) mgmt.sign_with_ecdsa({
      message_hash = messageHash;
      derivation_path = derivationPath;
      key_id = { curve = #secp256k1; name = keyName };
    });
    r.signature;
  };

  /// Schnorr public key (32-byte ed25519 point, or 33-byte compressed
  /// bip340secp256k1 point) for `derivationPath`. Free.
  public func schnorrPublicKey(algorithm : SchnorrAlgorithm, keyName : Text, derivationPath : [Blob]) : async Blob {
    let r = await mgmt.schnorr_public_key({
      canister_id = null;
      derivation_path = derivationPath;
      key_id = { algorithm; name = keyName };
    });
    r.public_key;
  };

  /// Threshold-Schnorr signature over the raw `message` (NOT pre-hashed — the
  /// scheme hashes internally). For `ed25519` pass `aux = null`; for taproot
  /// (`bip340secp256k1`) pass `?#bip341 { merkle_root_hash }` to tweak.
  public func signWithSchnorr(
    algorithm : SchnorrAlgorithm,
    keyName : Text,
    derivationPath : [Blob],
    message : Blob,
    aux : ?SignWithSchnorrAux,
  ) : async Blob {
    let r = await (with cycles = SIGN_CYCLES) mgmt.sign_with_schnorr({
      message;
      derivation_path = derivationPath;
      key_id = { algorithm; name = keyName };
      aux;
    });
    r.signature;
  };

  // ── Pure helpers (unit-tested) ────────────────────────────────────────────

  /// Per-deployment chain-key name. `network` "ic"/"mainnet" selects the real
  /// `key_1` (or the beta `test_key_1` when `tier = #test`); anything else (local
  /// dfx, a test network) gets `dfx_test_key`.
  ///
  /// The network string is normalised (whitespace trimmed + lowercased) before
  /// matching, exactly like the compiler's `vetkd_key_name` gate — so "IC",
  /// "Mainnet", " ic ", and "MAINNET" all resolve to the production family, not
  /// the local test key.
  public type Tier = { #production; #test };

  /// Normalise a free-form network string the same way `vetkd_key_name` does:
  /// trim surrounding whitespace, then lowercase.
  func normalizeNetwork(network : Text) : Text {
    Text.toLowercase(Text.trim(network, #predicate(Char.isWhitespace)));
  };

  func isMainnet(network : Text) : Bool {
    let n = normalizeNetwork(network);
    n == "ic" or n == "mainnet";
  };

  public func keyName(network : Text, tier : Tier) : Text {
    if (isMainnet(network)) {
      switch (tier) {
        case (#production) "key_1";
        case (#test) "test_key_1";
      };
    } else {
      "dfx_test_key";
    };
  };

  /// Hard-fail backstop, the runtime twin of the compiler's `enforce_network_gate`
  /// (`compiler/src/project.rs`): resolve the key name and TRAP if the local
  /// `dfx_test_key` would be used for a mainnet target. The local test key does
  /// not exist on `ic`, so signing with it there would silently fail; this refuses
  /// to even attempt it. Call this instead of `keyName` on the signing path when
  /// you want a loud failure rather than a wrong/missing key.
  public func requireKeyName(network : Text, tier : Tier) : Text {
    let key = keyName(network, tier);
    if (isMainnet(network) and key == "dfx_test_key") {
      Debug.trap(
        "ChainKey network gate: refusing to use `dfx_test_key` for mainnet network `"
        # network # "` — mainnet requires a production key."
      );
    };
    key;
  };

  /// Deterministic derivation path for a per-user, per-chain key. The same
  /// `(caller, chainTag)` always derives the same address; different callers or
  /// chains derive disjoint keys. `chainTag` is an app-chosen label such as
  /// "btc", "eth", or "sol".
  public func derivationPath(caller : Principal, chainTag : Text) : [Blob] {
    [Principal.toBlob(caller), Text.encodeUtf8(chainTag)];
  };

  /// Lowercase hex of a public key / blob (e.g. a 33-byte compressed secp256k1
  /// key). Pure, total, round-trips with `Hex` decoding of the same bytes.
  public func toHex(pubkey : Blob) : Text {
    Hex.encode(pubkey);
  };
}
