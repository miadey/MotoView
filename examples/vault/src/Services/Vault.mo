import Principal "mo:base/Principal";
import HashMap "mo:base/HashMap";
import Buffer "mo:base/Buffer";
import Time "mo:base/Time";
import Nat "mo:base/Nat";
import Int "mo:base/Int";
import Array "mo:base/Array";

// A zero-trust note store. It holds ONLY ciphertext, keyed by owner principal —
// the canister can never read a note (decryption happens in the browser with a
// vetKey bound to the caller). Every put/list is appended to an audit log.
module {
  public class Vault() {
    public type Note = { id : Nat; ciphertext : Text; at : Int };
    public type Audit = { id : Nat; who : Text; action : Text; at : Int };

    var nextId : Nat = 0;
    var auditId : Nat = 0;
    let store = HashMap.HashMap<Principal, Buffer.Buffer<Note>>(16, Principal.equal, Principal.hash);
    let auditLog = Buffer.Buffer<Audit>(128);

    func record(who : Principal, action : Text) {
      auditId += 1;
      auditLog.add({ id = auditId; who = Principal.toText(who); action = action; at = Time.now() });
      if (auditLog.size() > 500) { ignore auditLog.remove(0) };
    };

    // Store a ciphertext for `owner`. The canister CANNOT read it.
    public func add(owner : Principal, ciphertext : Text) {
      let b = switch (store.get(owner)) {
        case (?b) b;
        case null { let nb = Buffer.Buffer<Note>(8); store.put(owner, nb); nb };
      };
      nextId += 1;
      b.add({ id = nextId; ciphertext; at = Time.now() });
      record(owner, "put encrypted note #" # Nat.toText(nextId));
    };

    public func notesOf(owner : Principal) : [Note] {
      record(owner, "list notes");
      switch (store.get(owner)) { case (?b) Buffer.toArray(b); case null [] };
    };

    public func countOf(owner : Principal) : Nat {
      switch (store.get(owner)) { case (?b) b.size(); case null 0 };
    };

    public func recentAudit(n : Nat) : [Audit] {
      let all = Buffer.toArray(auditLog);
      if (all.size() <= n) all else Array.subArray(all, all.size() - n, n);
    };

    public func mvStableSave() : Blob {
      let owners = Buffer.Buffer<(Principal, [Note])>(store.size());
      for ((p, b) in store.entries()) { owners.add((p, Buffer.toArray(b))) };
      to_candid ((Buffer.toArray(owners), Buffer.toArray(auditLog), nextId, auditId));
    };
    public func mvStableLoad(blob : Blob) {
      switch (from_candid (blob) : ?([(Principal, [Note])], [Audit], Nat, Nat)) {
        case (?(owners, audits, nid, aid)) {
          for ((p, notes) in owners.vals()) {
            let b = Buffer.Buffer<Note>(notes.size());
            for (n in notes.vals()) { b.add(n) };
            store.put(p, b);
          };
          for (a in audits.vals()) { auditLog.add(a) };
          nextId := nid; auditId := aid;
        };
        case null {};
      };
    };
  };
}
