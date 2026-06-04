---
title: Upgrade-Stable Persistence
section: Interactivity
slug: persistence
---

# Upgrade-Stable Persistence

By default, the state in a MotoView [service](services.md) lives in ordinary
Motoko `var`s and collections, which persist for as long as the canister is
*running* but reset on a code **upgrade** (`dfx deploy --mode upgrade`) — only
Motoko `stable` variables survive an upgrade. For real apps you want your data
to outlive deploys. MotoView makes that opt-in with two methods on a service.

## Opt in: `mvStableSave` / `mvStableLoad`

A stateful service is a `public class Name()` (see [Services](services.md)). To
make it upgrade-stable, add a snapshot pair:

```motoko
// src/Services/Notes.mo
import HashMap "mo:base/HashMap";
import Iter "mo:base/Iter";
import Nat "mo:base/Nat";
import Hash "mo:base/Hash";

module {
  public class Notes() {
    public type Note = { id : Nat; owner : Principal; body : Text };

    var nextId : Nat = 1;
    let notes = HashMap.HashMap<Nat, Note>(64, Nat.equal, Hash.hash);

    public func add(owner : Principal, body : Text) : Nat {
      let id = nextId; nextId += 1;
      notes.put(id, { id; owner; body }); id;
    };
    public func all() : [Note] { Iter.toArray(notes.vals()) };

    // ---- upgrade-stable persistence ----
    public func mvStableSave() : Blob {
      to_candid ((nextId, Iter.toArray(notes.entries())));
    };
    public func mvStableLoad(b : Blob) {
      switch (from_candid (b) : ?(Nat, [(Nat, Note)])) {
        case (?(n, entries)) {
          nextId := n;
          for (k in Iter.toArray(notes.keys()).vals()) { notes.delete(k) };
          for ((k, v) in entries.vals()) { notes.put(k, v) };
        };
        case null {};
      };
    };
  };
}
```

That's it. When the compiler sees `public func mvStableSave` in a service it
generates, in the actor, a `stable var Notes__state : Blob` plus the
`preupgrade`/`postupgrade` hooks that call your methods:

```motoko
stable var Notes__state : Blob = "" : Blob;
system func preupgrade()  { Notes__state := Notes.mvStableSave() };
system func postupgrade() { Notes.mvStableLoad(Notes__state) };
```

`preupgrade` snapshots your live state to the stable Blob just before the
upgrade; `postupgrade` restores it into the fresh instance just after.

## The three rules

1. **Snapshot every mutable field.** Put each `var` scalar and each collection
   into the `to_candid` tuple. Maps → `Iter.toArray(map.entries())`, Buffers →
   `Buffer.toArray(buf)`. Immutable `let`s never need saving.
2. **Save and load must match exactly.** The `from_candid` type annotation must
   list the same fields in the same order as the `to_candid` tuple. Everything
   must be a *shared* type (records, variants, arrays, primitives, `Principal` —
   no functions or objects). If a record holds a `var` field or a `Buffer`,
   define a flat *snapshot* record for it and convert on save/load.
3. **Restore by replacing, not appending.** On upgrade the constructor runs
   first (re-seeding any starter data), *then* `postupgrade` runs. So clear each
   collection before refilling (`delete` keys / `Buffer.clear()`) and assign
   scalars — otherwise seeded data would accumulate. Done right, persistence is
   idempotent across repeated upgrades.

## Verifying

```bash
dfx deploy                       # install
# ... create some data through the app ...
dfx deploy --mode upgrade        # runs preupgrade -> postupgrade
# ... your data is still there ...
```

> **Scope.** This is per-service opt-in with a Candid round-trip — simple and
> robust. A future option will use Motoko's enhanced orthogonal persistence to
> skip the serialization entirely, plus a schema-migration story for evolving
> service types.
