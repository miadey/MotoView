/// Audit — an append-only audit log of sensitive transitions.
///
/// Zero-trust systems need a tamper-evident record of WHO did WHAT to WHICH
/// target and whether it SUCCEEDED, without ever logging the secret itself. This
/// module records exactly that: each entry gets a monotonically increasing `id`
/// and an IC `timestamp`, and the log is append-only — there is no public API to
/// mutate or delete an existing record. The canister appends on every sensitive
/// path (store/fetch ciphertext, vetKey derivation, role changes, …) so an
/// operator (or the user themselves) can review the trail.
///
/// What it deliberately does NOT store: plaintext, ciphertext, or key material.
/// `action` and `target` are short, non-secret labels (e.g. action="enc.put",
/// target="notes/2026"). Keep callers honest: never pass secret bytes here.
///
/// Stateful service convention: `public class Log()` holds the append-only
/// buffer at actor scope; `mvStableSave()/mvStableLoad()` snapshot it across
/// upgrades via the MotoView preupgrade/postupgrade hooks.
import Principal "mo:base/Principal";
import Time "mo:base/Time";
import Buffer "mo:base/Buffer";
import Array "mo:base/Array";

module {

  /// One immutable audit record. `actor_` is the principal that performed the
  /// action (the authenticated `msg.caller`), recorded as text so it survives a
  /// candid round-trip cleanly and is trivial to filter/display.
  public type Record = {
    id : Nat; // monotonic, gap-free within a canister lifetime
    timestamp : Int; // Time.now() when appended
    actor_ : Text; // who acted (Principal.toText of the caller)
    action : Text; // short non-secret verb, e.g. "enc.put" / "vetkd.derive"
    target : Text; // short non-secret object label, e.g. "notes/2026"
    ok : Bool; // did the action succeed / was it authorized
  };

  public class Log() {

    // Append-only store. A Buffer keeps insertion order and O(1) appends; we
    // never expose a mutator that removes or rewrites an existing element.
    let records = Buffer.Buffer<Record>(256);
    // Next id to assign. Monotonic across the canister's lifetime; persisted so
    // ids never repeat after an upgrade.
    var nextId : Nat = 0;

    /// Append a record. Returns the assigned id. This is the ONLY way to add to
    /// the log, and there is intentionally no way to amend or remove an entry.
    public func record(who : Principal, action : Text, target : Text, ok : Bool) : Nat {
      let id = nextId;
      nextId += 1;
      records.add({
        id;
        timestamp = Time.now();
        actor_ = Principal.toText(who);
        action;
        target;
        ok;
      });
      id;
    };

    /// Total number of records ever appended (== nextId).
    public func size() : Nat { records.size() };

    /// Fetch a single record by id (null if out of range). Because ids are
    /// gap-free and assigned in order, `id` is also the buffer index.
    public func at(id : Nat) : ?Record {
      if (id < records.size()) { ?records.get(id) } else { null };
    };

    /// The most recent `n` records, newest first. Caps at the log size.
    public func recent(n : Nat) : [Record] {
      let total = records.size();
      let take = if (n > total) { total } else { n };
      let out = Buffer.Buffer<Record>(take);
      var i = total;
      var taken = 0;
      while (taken < take and i > 0) {
        i -= 1;
        taken += 1;
        out.add(records.get(i));
      };
      Buffer.toArray(out);
    };

    /// Every record for a given actor, newest first. Useful for "show me my
    /// activity" or an operator auditing one principal.
    public func byActor(who : Principal) : [Record] {
      let target = Principal.toText(who);
      let out = Buffer.Buffer<Record>(16);
      var i = records.size();
      while (i > 0) {
        i -= 1;
        let r = records.get(i);
        if (r.actor_ == target) { out.add(r) };
      };
      Buffer.toArray(out);
    };

    /// The most recent `n` records for a given actor, newest first.
    public func byActorRecent(who : Principal, n : Nat) : [Record] {
      let all = byActor(who);
      if (n >= all.size()) { all } else { Array.subArray<Record>(all, 0, n) };
    };

    // ---- Upgrade-stable persistence (MotoView framework hooks) ----

    public func mvStableSave() : Blob {
      to_candid ({
        records = Buffer.toArray(records);
        nextId = nextId;
      });
    };

    public func mvStableLoad(b : Blob) {
      switch (from_candid (b) : ?{ records : [Record]; nextId : Nat }) {
        case (?saved) {
          let savedRecords = saved.records;
          let savedNext = saved.nextId;
          records.clear();
          for (r in savedRecords.vals()) { records.add(r) };
          nextId := savedNext;
        };
        case null {};
      };
    };
  };
};
