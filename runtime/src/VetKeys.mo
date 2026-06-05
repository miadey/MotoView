/// vetKeys: threshold-derived encryption keys from the IC management canister.
///
/// This wraps the two management-canister (`aaaaa-aa`) methods an app needs to
/// build identity-based encryption (IBE) on top of: fetch the master public key
/// for a context, and derive a vetKey for a given identity (`input`), encrypted
/// to a client-generated transport key. The BLS12-381 unwrap + IBE itself runs
/// in the client (the Rust brain via `ic-vetkeys`), never in the canister — the
/// canister only mediates the threshold derivation.
///
/// Key names: local dfx ships `dfx_test_key` / `test_key_1`; mainnet uses
/// `key_1`. `deriveKey` attaches cycles (the derive is charged ~26.2B locally).
import Principal "mo:base/Principal";

module {
  public type Curve = { #bls12_381_g2 };
  public type KeyId = { curve : Curve; name : Text };

  type Mgmt = actor {
    vetkd_public_key : ({ canister_id : ?Principal; context : Blob; key_id : KeyId }) -> async ({ public_key : Blob });
    vetkd_derive_key : ({ input : Blob; context : Blob; transport_public_key : Blob; key_id : KeyId }) -> async ({ encrypted_key : Blob });
  };
  let mgmt : Mgmt = actor "aaaaa-aa";

  // The local replica charges ~26_153_846_153 cycles per derive; attach a margin.
  let DERIVE_CYCLES : Nat = 30_000_000_000;

  /// The 96-byte BLS12-381 G2 master public key for `context` under `keyName`.
  /// Free; safe to call from anywhere (it reveals no secret).
  public func publicKey(keyName : Text, context : Blob) : async Blob {
    let r = await mgmt.vetkd_public_key({
      canister_id = null;
      context;
      key_id = { curve = #bls12_381_g2; name = keyName };
    });
    r.public_key;
  };

  /// Derive a vetKey for identity `input`, encrypted to `transportKey` (a 48-byte
  /// G1 point the client generated). Returns the 192-byte `encrypted_key` the
  /// client unwraps with its transport secret. Bind `input` to the authenticated
  /// caller's principal for per-user keys.
  public func deriveKey(keyName : Text, context : Blob, input : Blob, transportKey : Blob) : async Blob {
    let r = await (with cycles = DERIVE_CYCLES) mgmt.vetkd_derive_key({
      input;
      context;
      transport_public_key = transportKey;
      key_id = { curve = #bls12_381_g2; name = keyName };
    });
    r.encrypted_key;
  };
}
