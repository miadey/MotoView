# cert-verify live mainnet fixtures

These are **real, unmodified IC mainnet certificates**, captured for the Slice 4
chain-key certificate verifier (`client/src/cert_verify.rs`). They are NOT
hand-built test vectors — each is the raw CBOR body of a live `read_state`
response and verifies against the pinned NNS root key
(`IC_ROOT_KEY`).

## Provenance

Each file is the exact response body of an anonymous `read_state` for the
`/time` path:

```
POST https://icp-api.io/api/v2/canister/<CANISTER_ID>/read_state
Content-Type: application/cbor
body: CBOR { content: { request_type: "read_state",
                        sender: 0x04 (anonymous),
                        ingress_expiry: <now+4min, ns>,
                        paths: [["time"]] } }
```

| file | canister | subnet | shape |
|------|----------|--------|-------|
| `live_time_nns_ryjl3.cbor`       | `ryjl3-tyaaa-aaaaa-aaaba-cai` (ICP ledger) | NNS | signed directly by the root key, **no delegation** |
| `live_time_delegated_3xwpq.cbor` | `3xwpq-ziaaa-aaaah-qcn4a-cai`               | application subnet | signed via a **delegation** (root → subnet key), with certified `canister_ranges` |

Together they exercise both verification paths: the direct-root path and the
full delegation path (root verifies the delegation cert → `canister_ranges`
authorizes the target canister → subnet `public_key` verifies the outer cert).

## Refreshing

The certified `/time` is fixed at capture, so the tests pass `now = <the cert's
own /time>` for the freshness window (and separately prove the window rejects
stale/future times). To refresh, re-run the `read_state` above and overwrite the
files; nothing else needs to change.

## Verification depth — honest scope

* **Positive depth = live-mainnet-cert.** Both fixtures are real, unmodified
  mainnet certificates and they verify end-to-end against the *pinned*
  `IC_ROOT_KEY` (the NNS-direct path and the full delegation path:
  root → `canister_ranges` → subnet key → outer cert). Flipping one byte of the
  pinned key, the signature, or a tree leaf makes them fail closed.

* **No independent-reference differential.** There is currently **no**
  agent-js / agent-rs (`ic-certificate-verification`) cross-check that runs the
  *same* fixture bytes through a second implementation and asserts identical
  accept/reject. The evidence here is (a) the live certs chaining to the pinned
  root and (b) the BLS pairing being the IC's *own* `ic-verify-bls-signature`
  crate (the exact function `ic-agent` calls), so the curve/ciphersuite step is
  the reference itself — but that is self-referential at the verifier level, not
  an independent agreement test. (An independent `blspy` differential is
  impossible: `blspy` hardcodes the opposite curve assignment — pubkey∈G1,
  sig∈G2 — to the IC's sig∈G1/pubkey∈G2, and `ic-py` does no BLS verification.
  A real differential needs agent-js or agent-rs, which is not vendored here.)

* **Only the classic non-sharded `canister_ranges` layout is fixture-covered.**
  The delegated fixture exercises `/subnet/<id>/canister_ranges` (a single
  leaf). The sharded `/canister_ranges/<id>/<shard>` layout's multi-leaf union
  (`collect_leaves`) is unit-tested in isolation but NOT against a live sharded
  subnet, because mainnet does not serve that layout for `read_state` today.
