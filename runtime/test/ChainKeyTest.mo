// Unit test for ChainKey.mo pure helpers. Run:
//   moc -r --package base <base> runtime/test/ChainKeyTest.mo
//
// Only the DETERMINISTIC helpers are exercised here. The async management-canister
// methods (sign_with_ecdsa / sign_with_schnorr / *_public_key) need a deployed
// replica or mainnet cycles and are NOT called.
import ChainKey "../src/ChainKey";
import Principal "mo:base/Principal";
import Blob "mo:base/Blob";
import Hex "../src/Hex";
import Debug "mo:base/Debug";

// ── keyName env gate ─────────────────────────────────────────────────────────
assert (ChainKey.keyName("ic", #production) == "key_1");
assert (ChainKey.keyName("mainnet", #production) == "key_1");
assert (ChainKey.keyName("ic", #test) == "test_key_1");
assert (ChainKey.keyName("mainnet", #test) == "test_key_1");
assert (ChainKey.keyName("local", #production) == "dfx_test_key");
assert (ChainKey.keyName("local", #test) == "dfx_test_key");
assert (ChainKey.keyName("", #production) == "dfx_test_key");
assert (ChainKey.keyName("playground", #production) == "dfx_test_key");

// normalisation: capitalised / whitespaced mainnet strings must NOT fall back to
// the local key (this was the "accidental dfx_test_key on mainnet" bug).
assert (ChainKey.keyName("IC", #production) == "key_1");
assert (ChainKey.keyName("Mainnet", #production) == "key_1");
assert (ChainKey.keyName("MAINNET", #production) == "key_1");
assert (ChainKey.keyName(" ic", #production) == "key_1");
assert (ChainKey.keyName("ic ", #production) == "key_1");
assert (ChainKey.keyName("  Mainnet  ", #test) == "test_key_1");
// genuinely non-mainnet strings still get the local key (no production leak).
assert (ChainKey.keyName("Local", #production) == "dfx_test_key");
assert (ChainKey.keyName("testnet", #production) == "dfx_test_key");

// requireKeyName hard-fail backstop: on a mainnet target it resolves to the
// production key (never dfx_test_key), so it does NOT trap. (The trapping branch
// is structurally unreachable via keyName and so cannot be asserted under moc -r
// without aborting the run.)
assert (ChainKey.requireKeyName("ic", #production) == "key_1");
assert (ChainKey.requireKeyName("MAINNET", #production) == "key_1");
assert (ChainKey.requireKeyName(" ic ", #test) == "test_key_1");
assert (ChainKey.requireKeyName("local", #production) == "dfx_test_key");

// ── derivationPath: deterministic, distinct per (caller, chain) ──────────────
let alice = Principal.fromText("rrkah-fqaaa-aaaaa-aaaaq-cai");
let bob = Principal.fromText("ryjl3-tyaaa-aaaaa-aaaba-cai");

let aliceBtc = ChainKey.derivationPath(alice, "btc");
let aliceBtc2 = ChainKey.derivationPath(alice, "btc");
let aliceEth = ChainKey.derivationPath(alice, "eth");
let bobBtc = ChainKey.derivationPath(bob, "btc");

// shape: [ principal-bytes, chainTag-utf8 ]
assert (aliceBtc.size() == 2);
assert (aliceBtc[0] == Principal.toBlob(alice));
assert (Blob.equal(aliceBtc[1], "btc" : Blob)); // utf8 of "btc"

// deterministic: same inputs -> identical bytes
assert (aliceBtc[0] == aliceBtc2[0]);
assert (aliceBtc[1] == aliceBtc2[1]);

// distinct per chain (same caller, different tag)
assert (aliceBtc[0] == aliceEth[0]);          // same caller component
assert (aliceBtc[1] != aliceEth[1]);          // different chain component

// distinct per caller (same chain, different caller)
assert (aliceBtc[0] != bobBtc[0]);            // different caller component
assert (aliceBtc[1] == bobBtc[1]);            // same chain component

// ── toHex: pure encoding that matches Hex.encode and is stable ───────────────
let pk : Blob = "\02\ab\cd\ef\00\ff"; // stand-in compressed-pubkey bytes
assert (ChainKey.toHex(pk) == "02abcdef00ff");
assert (ChainKey.toHex(pk) == Hex.encode(pk));
assert (ChainKey.toHex("" : Blob) == "");

Debug.print("CHAINKEY_TEST_OK");
