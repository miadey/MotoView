# __NAME__

A MotoView app scaffolded from the **wallet** template — a value-moving spend
flow with intent-bound authorization.

## What it demonstrates

`src/Pages/Home.mview` is a spend-confirmation page that follows the cardinal
wallet rule: **confirm the exact intent → authorize → only then sign.**

- The confirm step renders the **exact intent** (amount, destination, chain)
  and embeds a hidden **intent-bound spend token** minted by
  `ctx.mintSpendToken` over that same intent.
- On submit, `ctx.authorizeSpend(handler, intent, token, weight)` runs the gate
  that atomically checks, in one call:
  1. a valid **session** (the secure token is bound to `ctx.caller`),
  2. **intent binding** — a token minted for spend X cannot authorize spend Y,
  3. **replay** protection (the nonce is consumed exactly once),
  4. a per-principal **velocity** limit on the `weight`.
- Only on `true` does a real handler derive the sighash and call
  **ChainKey** (`signWithEcdsa`). Secrets at rest go through **EncStore**
  (ciphertext only — the canister never sees plaintext).

The `<form ... secure>` is required (mutating). The page is `@authorize`-gated.

> A native hardware device assertion (Secure-Enclave / StrongBox) is a SEPARATE
> factor that must ALSO gate signing once the native client lands. It is
> intentionally not faked here. The ChainKey signing call is left as a comment
> so the template type-checks without a live management-canister call — the
> security-critical part is the authorization gate, which is real.

## Run it

```bash
motoview build        # compile .mview -> .mvbuild/main.mo
motoview lint         # security lint — this template is clean (0 errors)
motoview dev          # build, then `dfx deploy` to a local replica
```

> The generated actor lives in `.mvbuild/main.mo` — a build artifact, gitignored
> and regenerated on every build. Edit the `.mview` files in `src/`.
