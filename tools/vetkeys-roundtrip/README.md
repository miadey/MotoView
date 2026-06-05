# vetKeys round-trip verification

A reproducible end-to-end check of the vetKeys endpoints every MotoView actor
exposes (`mvVetkdPublicKey`, `mvVetkdDeriveKey`). It proves the canister-side
foundation works against a **real replica** with the **real client crypto**
([`ic-vetkeys`](https://crates.io/crates/ic-vetkeys)) — not a mock.

```
dfx deploy <your-app>
tools/vetkeys-roundtrip/run.sh <your-app>
# -> ROUND_TRIP_OK master=96 encrypted_key=192 vetkey_sig=48 ibe_ct=159 plaintext_recovered=true
```

What it does:

1. Generates a 48-byte BLS12-381 G1 **transport key** in the client (`ic-vetkeys`).
2. Calls the app's `mvVetkdPublicKey` → the 96-byte master public key.
3. Calls the app's `mvVetkdDeriveKey(transportKey)` → the 192-byte encrypted vetKey
   (the canister attaches ~26.2B cycles and calls the management canister).
4. **Unwraps** the vetKey with the transport secret (`decrypt_and_verify`, which
   checks the BLS signature) and runs an **IBE encrypt→decrypt** round-trip,
   recovering the exact plaintext.

The derivation `input` is the caller's principal, so each principal gets its own
vetKey — the basis for per-user encrypted state.

This is the verified canister foundation; shipping the same `ic-vetkeys` crypto
*inside the MotoView browser brain* (so apps do step 1/4 with no external tool)
is the documented next step — see [docs/security.md](../../docs/security.md).

> Build note: `ic-vetkeys` compiles for the host here; if your shell sets `CC`/
> `CFLAGS` to a cross toolchain, `run.sh` clears them (`env -u CC -u CFLAGS`).
