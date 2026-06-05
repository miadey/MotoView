//! MotoView in-browser vetKeys crypto.
//!
//! This crate is the BROWSER half of MotoView's zero-trust, end-to-end
//! encrypted capability. The canister only ever sees ciphertext; the *plaintext*
//! key material lives here, in `wasm32-unknown-unknown`, driven by the dumb JS
//! "hands" (which only ferry bytes in/out of linear memory).
//!
//! It links [`ic-vetkeys`] 0.7 — the **same** crate the generated Motoko actors
//! and `tools/vetkeys-roundtrip` use — so the wire format is identical on both
//! ends of the transport. The four exported operations are exactly the client
//! side of the vetKeys flow:
//!
//!   1. [`mvc_transport_secret_from_seed`] / [`mvc_transport_public`] — make the
//!      32-byte transport secret from a caller seed and its 48-byte G1 public key
//!      (what you send to `mvVetkdDeriveKey`).
//!   2. [`mvc_unwrap_vetkey`] — turn the canister's 192-byte encrypted vetKey into
//!      the raw 48-byte vetKey (BLS signature), verifying it against the master
//!      public key and the derivation input.
//!   3. [`mvc_ibe_encrypt`] — IBE-encrypt a plaintext to an identity under the
//!      master public key (canister stores this ciphertext).
//!   4. [`mvc_ibe_decrypt`] — IBE-decrypt with the unwrapped vetKey, recovering
//!      the plaintext locally.
//!
//! ## ABI
//!
//! Bytes cross the JS<->WASM boundary as `(ptr, len)` over shared linear memory,
//! the same convention as the brain's [`abi`](../motoview_client/abi). JS:
//!
//!   * [`mvc_alloc(len)`] to get a buffer, fills it with input bytes;
//!   * calls an operation, passing `(ptr, len)` for each input;
//!   * the operation writes its result into a per-thread **output buffer** and
//!     returns a status code (`0` = ok, negative = error). On success JS reads
//!     [`mvc_out_ptr()`]/[`mvc_out_len()`] and copies the bytes out;
//!   * [`mvc_dealloc(ptr, len)`] to free each input buffer.
//!
//! Returning via a single out-buffer (instead of returning a pointer) keeps the
//! ABI free of out-params and lets every op share one error channel. WASM is
//! single-threaded, so the `thread_local!` out-buffer is effectively a global.

#![allow(clippy::missing_safety_doc)]

use std::alloc::{alloc, dealloc, Layout};
use std::cell::RefCell;

use ic_vetkeys::{
    DerivedPublicKey, EncryptedVetKey, IbeCiphertext, IbeIdentity, IbeSeed, TransportSecretKey,
};

// ---------------------------------------------------------------------------
// getrandom "custom" backend (wasm only).
//
// `getrandom` is pulled in transitively (rand_core -> aes-gcm -> ic-vetkeys) and
// has no default backend on `wasm32-unknown-unknown`. We refuse the "js" backend
// because it links wasm-bindgen — the JS glue must stay dumb. With the "custom"
// feature we register this hook instead, which adds ZERO host imports.
//
// None of the four exported operations call the RNG (each takes an explicit
// caller-provided seed), so this is unreachable on every real code path. If some
// future path ever did need randomness it must be passed in as a seed from the
// browser CSPRNG by the glue; this hook deliberately fails closed rather than
// silently producing weak/zero entropy.
#[cfg(target_arch = "wasm32")]
pub fn getrandom_custom(_buf: &mut [u8]) -> Result<(), getrandom::Error> {
    // CUSTOM_RNG_UNAVAILABLE: a non-zero, app-defined getrandom error code.
    Err(getrandom::Error::from(
        core::num::NonZeroU32::new(getrandom::Error::CUSTOM_START + 0x4d56).unwrap(),
    ))
}

#[cfg(target_arch = "wasm32")]
getrandom::register_custom_getrandom!(getrandom_custom);

// ---------------------------------------------------------------------------
// Status codes (returned by every operation; JS treats <0 as failure).
// ---------------------------------------------------------------------------

/// Success.
pub const MVC_OK: i32 = 0;
/// A `(ptr, len)` input was null/zero where bytes were required.
pub const MVC_ERR_NULL_INPUT: i32 = -1;
/// An input had the wrong length for its fixed-size role.
pub const MVC_ERR_BAD_LEN: i32 = -2;
/// `ic-vetkeys` rejected/failed to parse an input (e.g. bad encoding).
pub const MVC_ERR_PARSE: i32 = -3;
/// The cryptographic operation itself failed (e.g. vetKey verification, or
/// IBE decrypt with the wrong key).
pub const MVC_ERR_CRYPTO: i32 = -4;

// Fixed sizes for the BLS12-381 vetKeys wire format.
const TRANSPORT_SECRET_LEN: usize = 32; // input seed AND serialized secret
const MASTER_PK_LEN: usize = 96; // DerivedPublicKey (G2)
const ENCRYPTED_KEY_LEN: usize = 192; // EncryptedVetKey
const MIN_IBE_SEED_LEN: usize = 16; // IbeSeed::from_bytes minimum

// ---------------------------------------------------------------------------
// Linear-memory allocator + shared output buffer (the marshaling primitives).
// ---------------------------------------------------------------------------

thread_local! {
    /// Result of the most recent successful operation. JS reads it via
    /// [`mvc_out_ptr`]/[`mvc_out_len`] before issuing the next call.
    static OUT: RefCell<Vec<u8>> = const { RefCell::new(Vec::new()) };
}

/// Allocate `len` bytes in WASM linear memory (align 1). Called by JS to stage
/// an input buffer.
#[no_mangle]
pub extern "C" fn mvc_alloc(len: usize) -> *mut u8 {
    if len == 0 {
        return std::ptr::null_mut();
    }
    // SAFETY: len >= 1, align 1 is always a valid layout.
    unsafe { alloc(Layout::from_size_align(len, 1).unwrap()) }
}

/// Free a buffer previously returned by [`mvc_alloc`]. Called by JS.
#[no_mangle]
pub extern "C" fn mvc_dealloc(ptr: *mut u8, len: usize) {
    if ptr.is_null() || len == 0 {
        return;
    }
    // SAFETY: ptr/len came from mvc_alloc with align 1.
    unsafe { dealloc(ptr, Layout::from_size_align(len, 1).unwrap()) }
}

/// Pointer to the current output buffer (valid until the next operation runs).
#[no_mangle]
pub extern "C" fn mvc_out_ptr() -> *const u8 {
    OUT.with(|o| o.borrow().as_ptr())
}

/// Length of the current output buffer.
#[no_mangle]
pub extern "C" fn mvc_out_len() -> usize {
    OUT.with(|o| o.borrow().len())
}

/// Borrow a JS-provided `(ptr, len)` input as a slice for the duration of a call.
///
/// # Safety
/// `ptr` must point to at least `len` valid bytes (e.g. from `mvc_alloc`).
unsafe fn input(ptr: *const u8, len: usize) -> Option<&'static [u8]> {
    if ptr.is_null() || len == 0 {
        return None;
    }
    Some(std::slice::from_raw_parts(ptr, len))
}

/// Move `bytes` into the shared output buffer and report success.
fn finish(bytes: Vec<u8>) -> i32 {
    OUT.with(|o| *o.borrow_mut() = bytes);
    MVC_OK
}

// ---------------------------------------------------------------------------
// (1) Transport key.
// ---------------------------------------------------------------------------

/// Derive the 32-byte transport secret from a 32-byte caller seed.
///
/// Output (on `MVC_OK`): the 32-byte serialized [`TransportSecretKey`], to be fed
/// back into [`mvc_transport_public`] and [`mvc_unwrap_vetkey`].
///
/// The seed must be generated by the browser CSPRNG (`window.crypto`) in the JS
/// glue — that is the one place randomness legitimately comes from the host.
#[no_mangle]
pub unsafe extern "C" fn mvc_transport_secret_from_seed(seed_ptr: *const u8, seed_len: usize) -> i32 {
    let seed = match input(seed_ptr, seed_len) {
        Some(s) => s,
        None => return MVC_ERR_NULL_INPUT,
    };
    if seed.len() != TRANSPORT_SECRET_LEN {
        return MVC_ERR_BAD_LEN;
    }
    let tsk = match TransportSecretKey::from_seed(seed.to_vec()) {
        Ok(t) => t,
        Err(_) => return MVC_ERR_PARSE,
    };
    finish(tsk.serialize())
}

/// Compute the 48-byte G1 transport *public* key from a transport secret.
///
/// Input: the 32-byte secret from [`mvc_transport_secret_from_seed`].
/// Output (on `MVC_OK`): the 48-byte public key to pass to `mvVetkdDeriveKey`.
#[no_mangle]
pub unsafe extern "C" fn mvc_transport_public(sk_ptr: *const u8, sk_len: usize) -> i32 {
    let sk = match input(sk_ptr, sk_len) {
        Some(s) => s,
        None => return MVC_ERR_NULL_INPUT,
    };
    let tsk = match TransportSecretKey::deserialize(sk) {
        Ok(t) => t,
        Err(_) => return MVC_ERR_PARSE,
    };
    finish(tsk.public_key())
}

// ---------------------------------------------------------------------------
// (2) Unwrap the encrypted vetKey.
// ---------------------------------------------------------------------------

/// Unwrap the canister's encrypted vetKey into the raw 48-byte vetKey.
///
/// Inputs:
///   * `master_pk` — 96-byte master public key from `mvVetkdPublicKey`.
///   * `encrypted_key` — 192-byte encrypted vetKey from `mvVetkdDeriveKey`.
///   * `input` — the derivation input (the caller's principal bytes).
///   * `transport_sk` — the 32-byte transport secret used to request the key.
///
/// Output (on `MVC_OK`): the 48-byte vetKey (the BLS signature) for IBE decrypt.
/// Returns `MVC_ERR_CRYPTO` if the encrypted key fails verification against the
/// master key + input (i.e. the canister returned something not bound to us).
#[no_mangle]
pub unsafe extern "C" fn mvc_unwrap_vetkey(
    master_pk_ptr: *const u8,
    master_pk_len: usize,
    encrypted_key_ptr: *const u8,
    encrypted_key_len: usize,
    input_ptr: *const u8,
    input_len: usize,
    transport_sk_ptr: *const u8,
    transport_sk_len: usize,
) -> i32 {
    let (master_pk, encrypted_key, derivation_input, transport_sk) = match (
        input(master_pk_ptr, master_pk_len),
        input(encrypted_key_ptr, encrypted_key_len),
        input(input_ptr, input_len),
        input(transport_sk_ptr, transport_sk_len),
    ) {
        (Some(a), Some(b), Some(c), Some(d)) => (a, b, c, d),
        _ => return MVC_ERR_NULL_INPUT,
    };
    if master_pk.len() != MASTER_PK_LEN || encrypted_key.len() != ENCRYPTED_KEY_LEN {
        return MVC_ERR_BAD_LEN;
    }
    let dpk = match DerivedPublicKey::deserialize(master_pk) {
        Ok(k) => k,
        Err(_) => return MVC_ERR_PARSE,
    };
    let evk = match EncryptedVetKey::deserialize(encrypted_key) {
        Ok(k) => k,
        Err(_) => return MVC_ERR_PARSE,
    };
    let tsk = match TransportSecretKey::deserialize(transport_sk) {
        Ok(t) => t,
        Err(_) => return MVC_ERR_PARSE,
    };
    match evk.decrypt_and_verify(&tsk, &dpk, derivation_input) {
        Ok(vk) => finish(vk.signature_bytes().to_vec()),
        Err(_) => MVC_ERR_CRYPTO,
    }
}

// ---------------------------------------------------------------------------
// (3) IBE encrypt.
// ---------------------------------------------------------------------------

/// IBE-encrypt `plaintext` to `identity` under the master public key.
///
/// Inputs:
///   * `master_pk` — 96-byte master public key from `mvVetkdPublicKey`.
///   * `identity` — the identity bytes (e.g. a principal) the data is encrypted
///     to; only the holder of that identity's vetKey can decrypt.
///   * `plaintext` — the secret to encrypt.
///   * `seed` — >= 16 random bytes (32 used directly) from the browser CSPRNG.
///
/// Output (on `MVC_OK`): the serialized [`IbeCiphertext`] to hand to the canister.
#[no_mangle]
pub unsafe extern "C" fn mvc_ibe_encrypt(
    master_pk_ptr: *const u8,
    master_pk_len: usize,
    identity_ptr: *const u8,
    identity_len: usize,
    plaintext_ptr: *const u8,
    plaintext_len: usize,
    seed_ptr: *const u8,
    seed_len: usize,
) -> i32 {
    // master_pk, identity and seed must be present; plaintext may be empty.
    let (master_pk, identity, seed) = match (
        input(master_pk_ptr, master_pk_len),
        input(identity_ptr, identity_len),
        input(seed_ptr, seed_len),
    ) {
        (Some(a), Some(b), Some(c)) => (a, b, c),
        _ => return MVC_ERR_NULL_INPUT,
    };
    let plaintext: &[u8] = if plaintext_ptr.is_null() || plaintext_len == 0 {
        &[]
    } else {
        std::slice::from_raw_parts(plaintext_ptr, plaintext_len)
    };
    if master_pk.len() != MASTER_PK_LEN {
        return MVC_ERR_BAD_LEN;
    }
    if seed.len() < MIN_IBE_SEED_LEN {
        return MVC_ERR_BAD_LEN;
    }
    let dpk = match DerivedPublicKey::deserialize(master_pk) {
        Ok(k) => k,
        Err(_) => return MVC_ERR_PARSE,
    };
    let id = IbeIdentity::from_bytes(identity);
    let iseed = match IbeSeed::from_bytes(seed) {
        Ok(s) => s,
        Err(_) => return MVC_ERR_PARSE,
    };
    let ct = IbeCiphertext::encrypt(&dpk, &id, plaintext, &iseed);
    finish(ct.serialize())
}

// ---------------------------------------------------------------------------
// (4) IBE decrypt.
// ---------------------------------------------------------------------------

/// IBE-decrypt `ciphertext` with the unwrapped 48-byte `vetkey`.
///
/// Inputs:
///   * `vetkey` — the 48-byte vetKey from [`mvc_unwrap_vetkey`].
///   * `ciphertext` — a serialized [`IbeCiphertext`] (from [`mvc_ibe_encrypt`] or
///     the canister).
///
/// Output (on `MVC_OK`): the recovered plaintext. Returns `MVC_ERR_CRYPTO` if the
/// vetKey does not match the ciphertext's identity (decryption fails).
#[no_mangle]
pub unsafe extern "C" fn mvc_ibe_decrypt(
    vetkey_ptr: *const u8,
    vetkey_len: usize,
    ciphertext_ptr: *const u8,
    ciphertext_len: usize,
) -> i32 {
    let (vetkey, ciphertext) = match (
        input(vetkey_ptr, vetkey_len),
        input(ciphertext_ptr, ciphertext_len),
    ) {
        (Some(a), Some(b)) => (a, b),
        _ => return MVC_ERR_NULL_INPUT,
    };
    // The vetKey is reconstructed via decrypt_and_verify -> serialize roundtrip.
    // ic-vetkeys exposes VetKey::deserialize for the 48-byte signature form.
    let vk = match ic_vetkeys::VetKey::deserialize(vetkey) {
        Ok(k) => k,
        Err(_) => return MVC_ERR_PARSE,
    };
    let ct = match IbeCiphertext::deserialize(ciphertext) {
        Ok(c) => c,
        Err(_) => return MVC_ERR_PARSE,
    };
    match ct.decrypt(&vk) {
        Ok(pt) => finish(pt),
        Err(_) => MVC_ERR_CRYPTO,
    }
}

// ---------------------------------------------------------------------------
// Native round-trip test: transport -> unwrap -> IBE encrypt/decrypt.
//
// This drives the crate's *real* C-ABI exports — the exact code path the browser
// runs — over the full vetKeys flow, with NO replica. To exercise the real
// `mvc_unwrap_vetkey` path we reconstruct a genuine vetKD-`EncryptedVetKey`
// locally using the SAME BLS12-381 primitives ic-vetkeys uses internally (we own
// the master secret here, the way the IC's threshold network owns it in prod):
//
//   * vetKey:        k  = augmented_hash_to_g1(master_point, input) * msk
//   * encrypted key: c1 = g1*t, c2 = g2*t, c3 = k + tpk*t   (tpk = the transport
//     public key the crate produced from the seed)
//
// Then we serialize that to the 192-byte wire format and feed it to the crate's
// `mvc_unwrap_vetkey`, which independently recovers `k` (= c3 - c1*r) AND verifies
// the BLS signature against the master key. The recovered vetKey then decrypts an
// IBE ciphertext the crate itself produced. If any byte of the format, the
// pairing math, or the ABI marshaling were wrong, this fails.
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use ff::Field;
    use ic_bls12_381::hash_to_curve::{ExpandMsgXmd, HashToCurve};
    use ic_bls12_381::{G1Affine, G1Projective, G2Affine, Scalar};
    use rand::SeedableRng;
    use rand_chacha::ChaCha20Rng;

    /// Read whatever the last ABI op wrote into the shared out-buffer.
    fn out() -> Vec<u8> {
        OUT.with(|o| o.borrow().clone())
    }

    /// Call an ABI op with one input buffer; assert OK; return the output.
    unsafe fn op1(f: unsafe extern "C" fn(*const u8, usize) -> i32, a: &[u8]) -> Vec<u8> {
        let rc = f(a.as_ptr(), a.len());
        assert_eq!(rc, MVC_OK, "ABI op returned {rc}");
        out()
    }

    /// The BLS12-381 `augmented_hash_to_g1` used by ic-vetkeys: hash the message
    /// prefixed with the compressed master point, with the AUG domain separator.
    /// (Mirrors ic-vetkeys utils::augmented_hash_to_g1 exactly.)
    fn augmented_hash_to_g1(master: &G2Affine, data: &[u8]) -> G1Affine {
        let dst = b"BLS_SIG_BLS12381G1_XMD:SHA-256_SSWU_RO_AUG_";
        let mut input = Vec::with_capacity(96 + data.len());
        input.extend_from_slice(&master.to_compressed());
        input.extend_from_slice(data);
        let pt = <G1Projective as HashToCurve<ExpandMsgXmd<sha2::Sha256>>>::hash_to_curve(input, dst);
        G1Affine::from(pt)
    }

    /// Recompute the transport secret scalar the way `TransportSecretKey::from_seed`
    /// does (`Scalar::random(ChaCha20Rng::from_seed(seed))`), so the test can build
    /// `c3 = k + tpk*t` against the crate's own transport key.
    fn transport_scalar_from_seed(seed: &[u8; 32]) -> Scalar {
        let mut rng = ChaCha20Rng::from_seed(*seed);
        Scalar::random(&mut rng)
    }

    /// Build a (master_pk_96, encrypted_key_192) pair that the crate's real
    /// `mvc_unwrap_vetkey` will accept and verify, for the given derivation input
    /// and the transport key derived from `transport_seed`.
    fn local_vetkd(
        msk: &Scalar,
        input: &[u8],
        transport_seed: &[u8; 32],
        t: &Scalar,
    ) -> (Vec<u8>, Vec<u8>) {
        let master_point = G2Affine::from(G2Affine::generator() * msk);
        let master_pk = master_point.to_compressed().to_vec();

        // The vetKey is the BLS signature: k = H(input)^msk.
        let h = augmented_hash_to_g1(&master_point, input);
        let k = G1Affine::from(G1Projective::from(h) * msk);

        // Transport public key tpk = g1 * r (r from the seed, same as the crate).
        let r = transport_scalar_from_seed(transport_seed);
        let tpk = G1Projective::from(G1Affine::generator()) * r;

        // Encrypted vetKey, exactly as vetKD produces it:
        //   c1 = g1*t, c2 = g2*t, c3 = k + tpk*t
        let c1 = G1Affine::from(G1Affine::generator() * t);
        let c2 = G2Affine::from(G2Affine::generator() * t);
        let c3 = G1Affine::from(G1Projective::from(k) + tpk * t);

        let mut ek = Vec::with_capacity(192);
        ek.extend_from_slice(&c1.to_compressed()); // 48
        ek.extend_from_slice(&c2.to_compressed()); // 96
        ek.extend_from_slice(&c3.to_compressed()); // 48
        assert_eq!(ek.len(), 192);
        (master_pk, ek)
    }

    #[test]
    fn alloc_dealloc_roundtrips() {
        let p = mvc_alloc(64);
        assert!(!p.is_null());
        mvc_dealloc(p, 64);
        assert!(mvc_alloc(0).is_null());
    }

    #[test]
    fn transport_public_is_48_g1_bytes() {
        unsafe {
            let seed = [42u8; 32];
            let sk = op1(mvc_transport_secret_from_seed, &seed);
            assert_eq!(sk.len(), TRANSPORT_SECRET_LEN);
            let pk = op1(mvc_transport_public, &sk);
            assert_eq!(pk.len(), 48, "transport public key must be 48-byte G1");
            assert!(ic_vetkeys::is_valid_transport_public_key_encoding(&pk));
        }
    }

    #[test]
    fn bad_lengths_are_rejected() {
        unsafe {
            let short = [0u8; 8];
            assert_eq!(
                mvc_transport_secret_from_seed(short.as_ptr(), short.len()),
                MVC_ERR_BAD_LEN
            );
            assert_eq!(
                mvc_transport_secret_from_seed(std::ptr::null(), 0),
                MVC_ERR_NULL_INPUT
            );
            let bad_master = [0u8; 10];
            let id = [1u8; 4];
            let pt = [2u8; 4];
            let seed = [3u8; 32];
            assert_eq!(
                mvc_ibe_encrypt(
                    bad_master.as_ptr(),
                    bad_master.len(),
                    id.as_ptr(),
                    id.len(),
                    pt.as_ptr(),
                    pt.len(),
                    seed.as_ptr(),
                    seed.len(),
                ),
                MVC_ERR_BAD_LEN
            );
        }
    }

    #[test]
    fn full_transport_unwrap_ibe_roundtrip_through_abi() {
        // Deterministic master secret + encryption nonce for a reproducible test.
        let msk = Scalar::from(0x4d6f746f_5669_6577u64); // "MotoView"-ish
        let t = Scalar::from(0x7a65726f_7472_7374u64); // "zerotrst"
        let identity = b"user-principal-derivation-input";
        let transport_seed = [42u8; 32];

        // Canister side (reconstructed locally): master key + encrypted vetKey.
        let (master_pk, encrypted_key) = local_vetkd(&msk, identity, &transport_seed, &t);

        unsafe {
            // (1) Client: transport secret + public key from the seed.
            let tsk = op1(mvc_transport_secret_from_seed, &transport_seed);
            assert_eq!(tsk.len(), 32);
            let tpk = op1(mvc_transport_public, &tsk);
            assert_eq!(tpk.len(), 48);

            // (2) Client: unwrap the encrypted vetKey through the REAL ABI path.
            // This both recovers k (= c3 - c1*r) and verifies the BLS signature
            // against master_pk + identity; a wrong byte anywhere => MVC_ERR_*.
            let rc = mvc_unwrap_vetkey(
                master_pk.as_ptr(),
                master_pk.len(),
                encrypted_key.as_ptr(),
                encrypted_key.len(),
                identity.as_ptr(),
                identity.len(),
                tsk.as_ptr(),
                tsk.len(),
            );
            assert_eq!(rc, MVC_OK, "mvc_unwrap_vetkey rc={rc}");
            let vetkey = out();
            assert_eq!(vetkey.len(), 48, "vetKey must be a 48-byte BLS signature");

            // (3) Client: IBE-encrypt a secret to the identity under master_pk.
            let plaintext = b"motoview zero-trust secret payload";
            let iseed = [7u8; 32];
            let rc = mvc_ibe_encrypt(
                master_pk.as_ptr(),
                master_pk.len(),
                identity.as_ptr(),
                identity.len(),
                plaintext.as_ptr(),
                plaintext.len(),
                iseed.as_ptr(),
                iseed.len(),
            );
            assert_eq!(rc, MVC_OK, "mvc_ibe_encrypt rc={rc}");
            let ciphertext = out();
            assert!(ciphertext.len() > plaintext.len());

            // (4) Client: IBE-decrypt with the unwrapped vetKey -> recover plaintext.
            let rc = mvc_ibe_decrypt(
                vetkey.as_ptr(),
                vetkey.len(),
                ciphertext.as_ptr(),
                ciphertext.len(),
            );
            assert_eq!(rc, MVC_OK, "mvc_ibe_decrypt rc={rc}");
            let recovered = out();
            assert_eq!(&recovered, plaintext, "plaintext not recovered E2E");

            // Negative: an encrypted key the canister did NOT bind to us must be
            // rejected by verification (use a different master secret).
            let (wrong_master, wrong_ek) =
                local_vetkd(&Scalar::from(9999u64), identity, &transport_seed, &t);
            let rc = mvc_unwrap_vetkey(
                wrong_master.as_ptr(),
                wrong_master.len(),
                encrypted_key.as_ptr(), // encrypted under msk, verified against wrong key
                encrypted_key.len(),
                identity.as_ptr(),
                identity.len(),
                tsk.as_ptr(),
                tsk.len(),
            );
            assert_eq!(rc, MVC_ERR_CRYPTO, "mismatched master key must fail verify");
            let _ = (wrong_ek,);

            // Negative: a vetKey for a different identity must not decrypt.
            let (m2, ek2) = local_vetkd(&msk, b"someone-else", &transport_seed, &t);
            let rc = mvc_unwrap_vetkey(
                m2.as_ptr(),
                m2.len(),
                ek2.as_ptr(),
                ek2.len(),
                b"someone-else".as_ptr(),
                b"someone-else".len(),
                tsk.as_ptr(),
                tsk.len(),
            );
            assert_eq!(rc, MVC_OK);
            let other_vetkey = out();
            let rc = mvc_ibe_decrypt(
                other_vetkey.as_ptr(),
                other_vetkey.len(),
                ciphertext.as_ptr(),
                ciphertext.len(),
            );
            assert_eq!(rc, MVC_ERR_CRYPTO, "wrong-identity vetKey must not decrypt");
        }
    }
}

