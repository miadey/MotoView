// Unit test for the wallet spend-authorization gate (WalletAuth.mo). Run:
//   moc -r --package base <base> runtime/test/WalletAuthTest.mo
//
// Exercises ONLY the pure/deterministic authorization gate + velocity limiter.
// Real signing (ChainKey.signWithEcdsa/signWithSchnorr) needs a replica/cycles
// and is NOT called here — the gate is what guards it, and the gate is what we
// can verify offline.
import WalletAuth "../src/WalletAuth";
import Principal "mo:base/Principal";
import Debug "mo:base/Debug";
import HashMap "mo:base/HashMap";
import Text "mo:base/Text";
import Int "mo:base/Int";

// A fixed 32-byte secret (the real one comes from raw_rand at runtime).
let secret : Blob = "\00\01\02\03\04\05\06\07\08\09\0a\0b\0c\0d\0e\0f\10\11\12\13\14\15\16\17\18\19\1a\1b\1c\1d\1e\1f";

let path = "/wallet/confirm";
let handler = "sign";
let alice = Principal.fromText("rrkah-fqaaa-aaaaa-aaaaq-cai");
let mallory = Principal.fromText("ryjl3-tyaaa-aaaaa-aaaba-cai");
let nonceA = "nonce-A-1";
let now : Int = 1_000_000_000;
let expiry : Int = now + 60_000_000_000; // 60s ahead

// The EXACT spend the user is confirming: 100 of "btc" to address "abc".
let intentX : [(Text, Text)] = [("amount", "100"), ("to", "abc"), ("chain", "btc")];
// A TAMPERED spend: same handler/route/caller, but amount changed to 9999.
let intentY : [(Text, Text)] = [("amount", "9999"), ("to", "abc"), ("chain", "btc")];

// Mint a spend token for intent X using the symmetric mint side of the gate.
let tokX = WalletAuth.mintSpendToken(secret, path, handler, alice, expiry, nonceA, intentX);

// Helper to build SpendArgs for a given intent / caller / now.
func argsFor(token : Text, intent : [(Text, Text)], caller : Principal, nowNs : Int) : WalletAuth.SpendArgs {
  { secret; token; path; handler; caller; nowNs; intent };
};

// ── 1. ACCEPT: token minted for intent X authorizes a spend of intent X ──────
switch (WalletAuth.authorizeSpend(argsFor(tokX, intentX, alice, now))) {
  case (#ok({ nonce })) { assert (nonce == nonceA) };
  case (#err(r)) { Debug.print("X-for-X unexpectedly rejected: " # r); assert false };
};

// ── 2. REJECT: same token, TAMPERED intent Y (amount changed) ────────────────
// A token minted for X must NEVER authorize a different spend Y. The spend
// schema is DERIVED from the intent (the confirm form's fields ARE the intent),
// so a tampered intent fails the schema binding first (verify checks schema
// before the intent hash); either binding-mismatch reason proves X != Y. The
// invariant under test is "rejected, not authorized", which is what matters.
switch (WalletAuth.authorizeSpend(argsFor(tokX, intentY, alice, now))) {
  case (#err(r)) { assert (r == "field schema mismatch" or r == "intent mismatch") };
  case (#ok(_)) { Debug.print("tampered intent Y accepted!"); assert false };
};

// ── 2b. REJECT: right intent X but WRONG caller (mallory) — session binding ──
switch (WalletAuth.authorizeSpend(argsFor(tokX, intentX, mallory, now))) {
  case (#err(r)) { assert (r == "principal mismatch") };
  case (#ok(_)) { Debug.print("wrong-caller spend accepted!"); assert false };
};

// ── 2c. REJECT: tampered MAC (forged token) ──────────────────────────────────
let forged = do {
  let parts = Text.split(tokX, #char '.');
  let payload = switch (parts.next()) { case (?p) p; case null "" };
  payload # ".deadbeef";
};
switch (WalletAuth.authorizeSpend(argsFor(forged, intentX, alice, now))) {
  case (#err(r)) { assert (r == "bad signature") };
  case (#ok(_)) { Debug.print("forged token accepted!"); assert false };
};

// ── 3. REJECT: expired token (now > expiry) ──────────────────────────────────
switch (WalletAuth.authorizeSpend(argsFor(tokX, intentX, alice, expiry + 1))) {
  case (#err(r)) { assert (r == "token expired") };
  case (#ok(_)) { Debug.print("expired token accepted!"); assert false };
};

// ── 4. REJECT: replay — once the nonce is consumed, the same token can't sign ─
// The gate returns the nonce; the CALLER consumes it (as App does). We model
// App's consumed-nonce store and assert the second use is a replay.
let consumed = HashMap.HashMap<Text, Int>(8, Text.equal, Text.hash);
// First authorization succeeds and yields the nonce; caller consumes it.
let firstNonce = switch (WalletAuth.authorizeSpend(argsFor(tokX, intentX, alice, now))) {
  case (#ok({ nonce })) { nonce };
  case (#err(r)) { Debug.print("first spend rejected: " # r); assert false; "" };
};
assert (firstNonce == nonceA);
switch (consumed.get(firstNonce)) { case (?_) { assert false }; case null { consumed.put(firstNonce, expiry) } };
// Second use of the SAME token: gate still verifies (it is stateless), but the
// caller's replay check rejects it because the nonce was already consumed.
switch (WalletAuth.authorizeSpend(argsFor(tokX, intentX, alice, now))) {
  case (#ok({ nonce })) {
    switch (consumed.get(nonce)) {
      case (?_) { /* replay correctly detected */ };
      case null { Debug.print("replay NOT detected — nonce was fresh!"); assert false };
    };
  };
  case (#err(r)) { Debug.print("second verify unexpectedly invalid: " # r); assert false };
};

// ── 5. Velocity limiter: accept within limit, reject over, reset after window ─
let vel = WalletAuth.Velocity();
let limit : Nat = 100;
let windowNs : Int = 1_000; // tiny window so we can step past it
let t0 : Int = 10_000;

// 5a. within limit: 40 + 30 = 70 <= 100, both accepted.
assert (vel.tryRecord(alice, 40, t0, limit, windowNs) == true);
assert (vel.tryRecord(alice, 30, t0, limit, windowNs) == true);
// 5b. over limit: 70 + 40 = 110 > 100 -> rejected, and state UNCHANGED (the
// rejected weight must not count), so a 30 (70+30=100) still fits right after.
assert (vel.tryRecord(alice, 40, t0, limit, windowNs) == false);
assert (vel.check(alice, 30, t0, limit, windowNs) == true);
assert (vel.tryRecord(alice, 30, t0, limit, windowNs) == true); // now exactly 100
// 5c. at the cap: any positive weight now exceeds it.
assert (vel.tryRecord(alice, 1, t0, limit, windowNs) == false);
assert (vel.check(alice, 1, t0, limit, windowNs) == false);
// 5d. reset after the window: step now past t0 + windowNs; old entries fall out
// of the rolling window, so the full limit is available again.
let t1 : Int = t0 + windowNs + 1;
assert (vel.check(alice, 100, t1, limit, windowNs) == true);
assert (vel.tryRecord(alice, 100, t1, limit, windowNs) == true);
assert (vel.tryRecord(alice, 1, t1, limit, windowNs) == false); // full again at t1

// 5e. per-principal isolation: mallory's velocity is independent of alice's.
assert (vel.tryRecord(mallory, 100, t1, limit, windowNs) == true);

// 5f. dump/load round-trips the limiter state (upgrade-stability).
let snap = vel.dump();
let vel2 = WalletAuth.Velocity();
vel2.load(snap);
// alice is at the cap within the window at t1 -> still rejected after reload.
assert (vel2.tryRecord(alice, 1, t1, limit, windowNs) == false);

// ── 6. intentSchema/canonicalIntent: a single weight that alone exceeds the
//        limit is rejected (no "first spend is free" hole). ─────────────────
let vel3 = WalletAuth.Velocity();
assert (vel3.tryRecord(alice, limit + 1, t0, limit, windowNs) == false);

Debug.print("WALLETAUTH_TEST_OK");
