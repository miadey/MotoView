/// Notes — a tiny stateful service used as the R5 gen-loop TEST FIXTURE.
///
/// It follows the MotoView stateful-service convention (a `public class
/// <Name>()` so it can hold mutable state). The R5 palette extractor
/// (signatures.js) scans this file's PUBLIC surface; the palette test asserts on
/// the exact `public func`/`public type`/`public let` names and signatures
/// declared here, and the convergence/gate tests type-check candidate pages that
/// call `Notes.add` / `Notes.all` against it.
import HashMap "mo:base/HashMap";
import Text "mo:base/Text";
import Time "mo:base/Time";
import Iter "mo:base/Iter";
import Nat "mo:base/Nat";
import Hash "mo:base/Hash";

module {

  public class Notes() {

    /// A single note (immutable snapshot — shared, so candid round-trips).
    public type Note = {
      id : Nat;
      title : Text;
      body : Text;
      createdAt : Int;
    };

    /// The empty-state hint shown when there are no notes yet.
    public let emptyHint : Text = "No notes yet — write your first one.";

    var nextId : Nat = 1;
    let notes_ = HashMap.HashMap<Nat, Note>(32, Nat.equal, Hash.hash);

    /// Add a note. Returns the new note id.
    public func add(title : Text, body : Text) : Nat {
      let id = nextId;
      nextId += 1;
      notes_.put(id, { id; title; body; createdAt = Time.now() });
      id;
    };

    /// All notes (unordered).
    public func all() : [Note] {
      Iter.toArray(notes_.vals());
    };

    /// One note by id.
    public func get(id : Nat) : ?Note {
      notes_.get(id);
    };

    /// How many notes exist.
    public func count() : Nat {
      notes_.size();
    };

    // ── Upgrade-stable persistence (compiler-wired; NOT part of the palette) ──
    public func mvStableSave() : Blob {
      to_candid (Iter.toArray(notes_.entries()), nextId);
    };
    public func mvStableLoad(b : Blob) {
      switch (from_candid (b) : ?([(Nat, Note)], Nat)) {
        case (?(entries, savedNext)) {
          nextId := savedNext;
          for ((k, v) in entries.vals()) { notes_.put(k, v) };
        };
        case null {};
      };
    };
  };
}
