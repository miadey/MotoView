// Unit test for the role store. Run:
//   moc -r --package base <base> runtime/test/RolesTest.mo
import Roles "../src/Roles";
import Principal "mo:base/Principal";
import Debug "mo:base/Debug";

let s = Roles.Store();
let alice = Principal.fromText("aaaaa-aa");
let bob = Principal.fromText("rrkah-fqaaa-aaaaa-aaaaq-cai");
let anon = Principal.fromText("2vxsx-fae");

// first-come bootstrap
assert (s.claim(alice, "Admin") == true);     // first claim wins
assert (s.claim(bob, "Admin") == false);      // already held by someone
assert (s.has(alice, "Admin"));
assert (not s.has(bob, "Admin"));
assert (s.anyHas("Admin"));
assert (not s.claim(anon, "Whatever"));       // anonymous can never claim
assert (not s.has(anon, "Whatever"));

// grant / revoke / multi-role
s.grant(bob, "Editor");
s.grant(bob, "Editor");                        // idempotent
assert (s.has(bob, "Editor"));
assert (s.rolesOf(bob).size() == 1);
s.grant(bob, "Moderator");
assert (s.rolesOf(bob).size() == 2);
s.revoke(bob, "Editor");
assert (not s.has(bob, "Editor"));
assert (s.has(bob, "Moderator"));
s.grant(anon, "X");                            // anonymous grant is a no-op
assert (not s.has(anon, "X"));

// persistence round-trip (dump -> new store -> load)
let snapshot = s.dump();
let s2 = Roles.Store();
s2.load(snapshot);
assert (s2.has(alice, "Admin"));
assert (s2.has(bob, "Moderator"));
assert (not s2.has(bob, "Editor"));

Debug.print("ROLES_TEST_OK");
