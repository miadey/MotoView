/// EncStore — a generic, zero-trust encrypted key/value store.
///
/// The canister stores ONLY ciphertext: opaque `Blob`s the BROWSER produced by
/// IBE-encrypting under a vetKey it derived and unwrapped locally (see
/// VetKeys.mo + the Rust→WASM brain). This module NEVER decrypts, inspects, or
/// validates the payload — it is a dumb, owner-scoped vault. The plaintext only
/// ever exists in the client.
///
/// Scoping: every record is keyed by `(owner : Principal, key : Text)`. A caller
/// may only put/get/list/delete under their OWN principal (the generated actor
/// passes `msg.caller` as `owner` — never a client-supplied principal), so one
/// user can never read or overwrite another user's ciphertext. The store keeps a
/// small amount of plaintext METADATA per record (created/updated timestamps and
/// a monotonically increasing version) so the UI can show "last saved" without
/// the canister ever touching the secret bytes.
///
/// Stateful service convention: `public class Store()` holds mutable state at
/// actor scope, and `mvStableSave()/mvStableLoad()` snapshot it to a candid Blob
/// across upgrades — the same hooks the MotoView compiler wires into
/// preupgrade/postupgrade for any persistent service.
import HashMap "mo:base/HashMap";
import Principal "mo:base/Principal";
import Text "mo:base/Text";
import Time "mo:base/Time";
import Iter "mo:base/Iter";
import Buffer "mo:base/Buffer";

module {

  /// One stored ciphertext record. `cipher` is the opaque encrypted payload the
  /// client produced; the canister treats it as bytes and nothing more.
  public type Entry = {
    cipher : Blob; // opaque IBE/AEAD ciphertext — NEVER decrypted server-side
    created : Int; // Time.now() at first put
    updated : Int; // Time.now() at the most recent put
    version : Nat; // bumped on every overwrite (optimistic-concurrency hint)
  };

  /// Plaintext metadata about a record, safe to return to any owner-scoped read
  /// (carries no secret material — just sizes/timestamps for the UI).
  public type Meta = {
    key : Text;
    size : Nat; // ciphertext byte length
    created : Int;
    updated : Int;
    version : Nat;
  };

  public class Store() {

    // Live state: (owner, key) -> Entry. We compose the map key from the
    // owner's principal text and the user key so a single HashMap scopes every
    // user's namespace without leaking across owners.
    let entries = HashMap.HashMap<Text, Entry>(64, Text.equal, Text.hash);

    // Compose the internal map key. The owner principal is length-prefixed-ish
    // by the "\u{0}" separator (illegal in a principal's text form) so distinct
    // (owner, key) pairs can never collide.
    func mk(owner : Principal, key : Text) : Text {
      Principal.toText(owner) # "\u{0}" # key;
    };

    /// Store (or overwrite) the caller's ciphertext under `key`. The canister
    /// records timestamps + a version bump but does not look inside `cipher`.
    /// Returns the new version number.
    public func put(owner : Principal, key : Text, cipher : Blob) : Nat {
      let now = Time.now();
      let id = mk(owner, key);
      switch (entries.get(id)) {
        case (?prev) {
          let v = prev.version + 1;
          entries.put(id, { cipher; created = prev.created; updated = now; version = v });
          v;
        };
        case null {
          entries.put(id, { cipher; created = now; updated = now; version = 1 });
          1;
        };
      };
    };

    /// Fetch the caller's ciphertext under `key` (null if absent). The client
    /// decrypts it locally; the canister only hands back the bytes it was given.
    public func get(owner : Principal, key : Text) : ?Blob {
      switch (entries.get(mk(owner, key))) {
        case (?e) { ?e.cipher };
        case null { null };
      };
    };

    /// Full record (ciphertext + metadata) for the caller's `key`, or null.
    public func getEntry(owner : Principal, key : Text) : ?Entry {
      entries.get(mk(owner, key));
    };

    /// Whether the caller has a record under `key`.
    public func has(owner : Principal, key : Text) : Bool {
      switch (entries.get(mk(owner, key))) { case (?_) true; case null false };
    };

    /// Plaintext metadata for every record the caller owns. Lets the UI list a
    /// user's encrypted items (keys, sizes, timestamps) without ever exposing —
    /// or the canister ever touching — the secret bytes.
    public func list(owner : Principal) : [Meta] {
      let prefix = Principal.toText(owner) # "\u{0}";
      let out = Buffer.Buffer<Meta>(16);
      for ((id, e) in entries.entries()) {
        if (Text.startsWith(id, #text prefix)) {
          let key = Text.replace(id, #text prefix, "");
          out.add({
            key;
            size = e.cipher.size();
            created = e.created;
            updated = e.updated;
            version = e.version;
          });
        };
      };
      Buffer.toArray(out);
    };

    /// Number of records the caller owns.
    public func count(owner : Principal) : Nat { list(owner).size() };

    /// Delete the caller's record under `key`. Returns true if something was
    /// removed. (Tombstoning is the client's job — server-side we just drop it.)
    public func delete(owner : Principal, key : Text) : Bool {
      let id = mk(owner, key);
      switch (entries.get(id)) {
        case (?_) { entries.delete(id); true };
        case null { false };
      };
    };

    /// Drop every record the caller owns (e.g. account wipe). Returns the count
    /// removed.
    public func deleteAll(owner : Principal) : Nat {
      let prefix = Principal.toText(owner) # "\u{0}";
      let doomed = Buffer.Buffer<Text>(16);
      for (id in entries.keys()) {
        if (Text.startsWith(id, #text prefix)) { doomed.add(id) };
      };
      for (id in doomed.vals()) { entries.delete(id) };
      doomed.size();
    };

    /// Total number of records across all owners (operational metric only).
    public func size() : Nat { entries.size() };

    // ---- Upgrade-stable persistence (MotoView framework hooks) ----

    public func mvStableSave() : Blob {
      to_candid ((
        Iter.toArray(entries.entries()),
      ));
    };

    public func mvStableLoad(b : Blob) {
      switch (from_candid (b) : ?([(Text, Entry)])) {
        case (?saved) {
          for (k in Iter.toArray(entries.keys()).vals()) { entries.delete(k) };
          for ((k, v) in saved.vals()) { entries.put(k, v) };
        };
        case null {};
      };
    };
  };
};
