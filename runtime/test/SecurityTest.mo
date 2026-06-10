// Unit test for the secure-form token (Security.mo). Run:
//   moc -r --package base <base> runtime/test/SecurityTest.mo
import Security "../src/Security";
import Debug "mo:base/Debug";
import Text "mo:base/Text";

// A fixed 32-byte secret (the real one comes from raw_rand at runtime).
let secret : Blob = "\00\01\02\03\04\05\06\07\08\09\0a\0b\0c\0d\0e\0f\10\11\12\13\14\15\16\17\18\19\1a\1b\1c\1d\1e\1f";

let path = "/pay";
let handler = "submit";
let principal = "aaaaa-aa";
let schema = Security.schemaHash("amount,to");
let nonce = "12345-7";
let now : Int = 1_000_000_000;
let expiry : Int = now + 60_000_000_000; // 60s in the future

// ---- 1. mint + verify round-trip (intent="") => #ok with the right nonce ----
let tok = Security.mint(secret, path, handler, principal, expiry, nonce, schema, "");
switch (Security.verify(secret, tok, path, handler, principal, now, schema, "")) {
  case (#ok({ nonce = n })) { assert (n == nonce) };
  case (#invalid(r)) { Debug.print("unexpected invalid: " # r); assert false };
};

// ---- 2. tampered MAC => #invalid("bad signature") ----
let parts = Text.split(tok, #char '.');
let payload = switch (parts.next()) { case (?p) p; case null "" };
let badTok = payload # "." # "deadbeef"; // wrong MAC
switch (Security.verify(secret, badTok, path, handler, principal, now, schema, "")) {
  case (#invalid(r)) { assert (r == "bad signature") };
  case (#ok(_)) { Debug.print("tampered MAC accepted"); assert false };
};

// ---- 3a. wrong path => route mismatch ----
switch (Security.verify(secret, tok, "/other", handler, principal, now, schema, "")) {
  case (#invalid(r)) { assert (r == "route mismatch") };
  case (#ok(_)) { assert false };
};
// ---- 3b. wrong handler => handler mismatch ----
switch (Security.verify(secret, tok, path, "delete", principal, now, schema, "")) {
  case (#invalid(r)) { assert (r == "handler mismatch") };
  case (#ok(_)) { assert false };
};
// ---- 3c. wrong principal => principal mismatch ----
switch (Security.verify(secret, tok, path, handler, "2vxsx-fae", now, schema, "")) {
  case (#invalid(r)) { assert (r == "principal mismatch") };
  case (#ok(_)) { assert false };
};
// ---- 3d. wrong schema => field schema mismatch ----
switch (Security.verify(secret, tok, path, handler, principal, now, Security.schemaHash("other"), "")) {
  case (#invalid(r)) { assert (r == "field schema mismatch") };
  case (#ok(_)) { assert false };
};

// ---- 4. expired (nowNs > expiry) => token expired ----
switch (Security.verify(secret, tok, path, handler, principal, expiry + 1, schema, "")) {
  case (#invalid(r)) { assert (r == "token expired") };
  case (#ok(_)) { assert false };
};

// ---- 5. intent-bound round-trip + tampered intent => intent mismatch ----
let intent = Security.intentHash(Security.canonicalIntent([("amount", "100"), ("to", "abc")]));
let itok = Security.mint(secret, path, handler, principal, expiry, nonce, schema, intent);
// same intent => #ok
switch (Security.verify(secret, itok, path, handler, principal, now, schema, intent)) {
  case (#ok({ nonce = n })) { assert (n == nonce) };
  case (#invalid(r)) { Debug.print("intent round-trip invalid: " # r); assert false };
};
// tampered intent (amount 9999) => intent mismatch
let badIntent = Security.intentHash(Security.canonicalIntent([("amount", "9999"), ("to", "abc")]));
switch (Security.verify(secret, itok, path, handler, principal, now, schema, badIntent)) {
  case (#invalid(r)) { assert (r == "intent mismatch") };
  case (#ok(_)) { Debug.print("tampered intent accepted"); assert false };
};
// an intent-bound token must also fail the empty-intent (ordinary) check
switch (Security.verify(secret, itok, path, handler, principal, now, schema, "")) {
  case (#invalid(r)) { assert (r == "intent mismatch") };
  case (#ok(_)) { assert false };
};

// ---- 6a. canonicalIntent is order-independent ----
let h1 = Security.intentHash(Security.canonicalIntent([("a", "1"), ("b", "2")]));
let h2 = Security.intentHash(Security.canonicalIntent([("b", "2"), ("a", "1")]));
assert (h1 == h2);

// ---- 6b. no separator-collision: ("a","12"),("b","3") != ("a","1"),("b","23") ----
let c1 = Security.intentHash(Security.canonicalIntent([("a", "12"), ("b", "3")]));
let c2 = Security.intentHash(Security.canonicalIntent([("a", "1"), ("b", "23")]));
assert (c1 != c2);

Debug.print("SECURITY_TEST_OK");
