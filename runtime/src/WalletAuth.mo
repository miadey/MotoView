/// Wallet sign-authorization gate (Slice 9B).
///
/// A spend signature (a `ChainKey.signWithEcdsa` / `signWithSchnorr` call that
/// moves real value) must NEVER happen unless ALL of these hold:
///
///   1. a valid SESSION — the caller is the authenticated principal the token
///      was minted for (enforced by `Security.verify`'s principal binding);
///   2. a single-use SECURE TOKEN bound to the EXACT spend intent
///      (amount, destination, chain, …) — so a token minted to authorize a
///      transfer of 5 BTC to address A cannot be replayed to authorize 5 BTC to
///      address B, or 500 BTC to A. The token's `intentHash` is the SHA-256 of
///      the canonical (key,value) intent (`Security.canonicalIntent`/`intentHash`);
///   3. single-use REPLAY protection — the token's nonce is consumed exactly
///      once (the CALLER consumes the returned nonce; we only hand it back);
///   4. a per-principal VELOCITY LIMIT — the rolling-window sum of authorized
///      spend "weight" stays under a cap, so a leaked/abused session cannot be
///      drained in a burst.
///
/// ## What this module does and does NOT do
/// This is the AUTHORIZATION GATE only. It is pure and deterministic, so it is
/// unit-testable without a replica or cycles. It does NOT perform signing, does
/// NOT touch the management canister, and does NOT decide the chain/derivation
/// path. A wallet handler MUST call this gate (via `ctx.authorizeSpend`) and get
/// `#ok` BEFORE it constructs the sighash and calls `ChainKey.signWithEcdsa` /
/// `signWithSchnorr`. Order matters: authorize first, sign second.
///
/// ## FOLLOW-UP (native, NOT implemented here)
/// A hardware DEVICE ASSERTION (`host_device_sign` from the native client — a
/// Secure-Enclave / StrongBox attestation that the human physically approved
/// THIS intent on a trusted device) is a SEPARATE factor that belongs on the
/// native side. It is intentionally NOT faked here. A production wallet handler
/// must ALSO verify that assertion before signing once the native client lands.
import Security "Security";
import HashMap "mo:base/HashMap";
import Text "mo:base/Text";
import Principal "mo:base/Principal";
import Iter "mo:base/Iter";
import Int "mo:base/Int";
import Buffer "mo:base/Buffer";
import Result "mo:base/Result";

module {

  public type Result<T> = Result.Result<T, Text>;

  // ── The spend-authorization gate ────────────────────────────────────────────

  /// Everything the gate needs to authorize ONE spend. The `intent` is the
  /// canonical (key,value) description of WHAT is being authorized — e.g.
  /// `[("amount","100"),("to","abc"),("chain","btc")]`. The token MUST have been
  /// minted (server-side) over the SHA-256 of exactly this canonical intent.
  public type SpendArgs = {
    secret : Blob; // the per-canister HMAC secret (App.mvSecret)
    token : Text; // the submitted secure token (`Security.mint(... intentHash ...)`)
    path : Text; // the route the confirm form lives on
    handler : Text; // the handler id the token was minted for
    caller : Principal; // the authenticated spender (from the session, not the gateway)
    nowNs : Int; // Time.now()
    intent : [(Text, Text)]; // the EXACT spend intent (amount,dest,chain,...)
  };

  /// Authorize a spend. On success returns the token's `nonce` — the CALLER is
  /// responsible for consuming it (single-use replay protection), exactly like
  /// the secure-form path in `App.serveEvent`. On any failure returns the reason
  /// from `Security.verify` (bad signature / route / handler / principal /
  /// schema / **intent mismatch** / expired) so a token minted for intent X can
  /// NEVER authorize a spend of a different intent Y.
  ///
  /// NOTE: this does NOT consume the nonce and does NOT check velocity — those
  /// are stateful and live in the caller (App, which owns the consumed-nonce
  /// store and the velocity limiter). Keeping them out makes this function pure
  /// and unit-testable. `App.authorizeSpend` composes: verify → replay check →
  /// velocity check → consume + record.
  public func authorizeSpend(args : SpendArgs) : Result<{ nonce : Text }> {
    // (a) Re-derive the intent hash the token must be bound to. canonicalIntent
    // is injective (length-prefixed, sorted), so two distinct intents can never
    // collide to the same hash.
    let intentHash = Security.intentHash(Security.canonicalIntent(args.intent));
    // The confirm form's declared fields ARE the intent keys, so the schema is
    // the same canonical material — binding the token to the field set too.
    let schema = Security.schemaHash(intentHash);
    // (b) Full verify: signature + route + handler + principal + schema + intent
    // + expiry. A token minted for intent X yields a DIFFERENT intentHash than Y,
    // so verify returns #invalid("intent mismatch") for the wrong spend.
    switch (
      Security.verify(
        args.secret,
        args.token,
        args.path,
        args.handler,
        Principal.toText(args.caller),
        args.nowNs,
        schema,
        intentHash,
      )
    ) {
      case (#invalid(reason)) { #err(reason) };
      case (#ok({ nonce })) { #ok({ nonce }) };
    };
  };

  /// The schema a spend token MUST be minted with so that `authorizeSpend`
  /// accepts it: the schema hash OVER the intent hash. Mint and verify share
  /// this ONE derivation so a spend token is self-consistent — the confirm
  /// form's field set is exactly the intent it binds. (`mintSpendToken` uses it;
  /// tests use it to build a token the gate will accept.)
  public func intentSchema(intent : [(Text, Text)]) : Text {
    Security.schemaHash(Security.intentHash(Security.canonicalIntent(intent)));
  };

  /// Mint a spend token that `authorizeSpend` will accept for exactly this
  /// `intent`. This is the symmetric mint side of the gate: it binds
  /// route + handler + caller + expiry + a fresh single-use nonce + the
  /// intent-derived schema + the intent hash. A wallet confirm page calls this
  /// (via `ctx`/App) to render the hidden token; `authorizeSpend` re-derives the
  /// same schema and intent hash and rejects anything that does not match.
  ///
  /// `nonce` MUST be a fresh, unguessable single-use value (App mints it from a
  /// monotone counter + time, like its other tokens). The token is ONLY as
  /// strong as the nonce's single-use enforcement in the caller's consumed store.
  public func mintSpendToken(
    secret : Blob,
    path : Text,
    handler : Text,
    caller : Principal,
    expiryNs : Int,
    nonce : Text,
    intent : [(Text, Text)],
  ) : Text {
    let ih = Security.intentHash(Security.canonicalIntent(intent));
    Security.mint(
      secret,
      path,
      handler,
      Principal.toText(caller),
      expiryNs,
      nonce,
      Security.schemaHash(ih),
      ih,
    );
  };

  // ── Per-principal velocity limiter ──────────────────────────────────────────

  /// One authorized-spend event: when it happened (ns) and its weight. "Weight"
  /// is an app-chosen integer cost (e.g. the amount, or a normalized risk score)
  /// summed over the rolling window and compared against the limit.
  public type Entry = { ts : Int; weight : Nat };

  /// A per-principal velocity limiter. State is `principal -> recent entries`
  /// within the rolling window. `check` is PURE (decides without mutating);
  /// `record` mutates. `dump`/`load` snapshot to/from a Candid array so the
  /// generated actor can hold the whole thing in a `stable var` (exactly like the
  /// consumed-nonce store and the role store).
  ///
  /// The window is enforced per call: callers pass `windowNs` and `limit`, so a
  /// single limiter can serve different policies (a low cap for high-value
  /// chains, a higher cap elsewhere) without per-policy state.
  public class Velocity() {
    // principal-text -> entries. Keyed by text so dump/load is plain Candid and
    // upgrade-stable without custom hashing of Principal across versions.
    let log = HashMap.HashMap<Text, [Entry]>(64, Text.equal, Text.hash);

    // Entries within [now - windowNs, now]. Drops anything older (those no longer
    // count toward the rolling sum) so the stored slice stays bounded by the rate.
    func live(entries : [Entry], nowNs : Int, windowNs : Int) : [Entry] {
      let cutoff = nowNs - windowNs;
      let b = Buffer.Buffer<Entry>(entries.size());
      for (e in entries.vals()) { if (e.ts > cutoff) { b.add(e) } };
      Buffer.toArray(b);
    };

    func sumWeight(entries : [Entry]) : Nat {
      var s : Nat = 0;
      for (e in entries.vals()) { s += e.weight };
      s;
    };

    /// Would authorizing `weight` more for `caller` right now stay within the
    /// limit? PURE — does not record. Returns true iff
    /// `(sum of live weights) + weight <= limit`. A zero or oversized single
    /// weight that alone exceeds the limit is rejected.
    public func check(caller : Principal, weight : Nat, nowNs : Int, limit : Nat, windowNs : Int) : Bool {
      let pt = Principal.toText(caller);
      let cur = switch (log.get(pt)) { case (?es) { live(es, nowNs, windowNs) }; case null { [] } };
      sumWeight(cur) + weight <= limit;
    };

    /// Record an authorized spend of `weight` for `caller` at `nowNs`, pruning
    /// entries that have fallen out of the window. Call this ONLY after `check`
    /// passed (and the spend was actually authorized).
    public func record(caller : Principal, weight : Nat, nowNs : Int, windowNs : Int) {
      let pt = Principal.toText(caller);
      let cur = switch (log.get(pt)) { case (?es) { live(es, nowNs, windowNs) }; case null { [] } };
      let b = Buffer.Buffer<Entry>(cur.size() + 1);
      for (e in cur.vals()) { b.add(e) };
      b.add({ ts = nowNs; weight });
      log.put(pt, Buffer.toArray(b));
    };

    /// Atomic check-and-record: if within the limit, record and return true;
    /// otherwise leave state untouched and return false. This is the call the
    /// App gate uses so the window sum can never be raced past the limit within
    /// a single message.
    public func tryRecord(caller : Principal, weight : Nat, nowNs : Int, limit : Nat, windowNs : Int) : Bool {
      if (not check(caller, weight, nowNs, limit, windowNs)) { return false };
      record(caller, weight, nowNs, windowNs);
      true;
    };

    /// Snapshot for `stable var` persistence (Candid array). Mirrors
    /// `App.dumpConsumed` / `Roles.dump`.
    public func dump() : [(Text, [Entry])] { Iter.toArray(log.entries()) };

    /// Restore from a snapshot. We DROP nothing here (the caller may not know the
    /// window at load time); stale entries are pruned lazily by `live` on the
    /// next `check`/`record` for that principal.
    public func load(es : [(Text, [Entry])]) { for ((k, v) in es.vals()) { log.put(k, v) } };
  };
};
