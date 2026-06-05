/// Catalog service — a stateful product store.
///
/// This is a MotoView **stateful service**: a service file exporting
/// `public class Catalog()`. The compiler instantiates ONE shared `Catalog`
/// at actor scope, so every page (the list, the create form, the edit form)
/// reads and mutates the SAME product store for the canister's lifetime.
/// (A plain `module {}` cannot hold mutable state.)
///
/// Products live in a `HashMap<Nat, Product>` keyed by an auto-incrementing id.
/// `mvStableSave` / `mvStableLoad` snapshot the store across code upgrades.
import HashMap "mo:base/HashMap";
import Nat "mo:base/Nat";
import Hash "mo:base/Hash";
import Iter "mo:base/Iter";
import Array "mo:base/Array";

module {

  public class Catalog() {

    /// A product record. `price` and `stock` are whole-number `Nat`s.
    public type Product = {
      id : Nat;
      name : Text;
      price : Nat; // whole dollars
      stock : Nat; // units on hand
      category : Text;
    };

    var nextId : Nat = 1;
    let products = HashMap.HashMap<Nat, Product>(32, Nat.equal, Hash.hash);

    /// Insert a starter product directly (used by the seed).
    func put(name : Text, price : Nat, stock : Nat, category : Text) : Nat {
      let id = nextId;
      nextId += 1;
      products.put(id, { id; name; price; stock; category });
      id;
    };

    // Seed three real products on first construction.
    let _seed1 = put("Aurora Helmet", 189, 24, "Gear");
    let _seed2 = put("Trail Gloves", 39, 80, "Apparel");
    let _seed3 = put("Carbon Bottle Cage", 27, 0, "Components");

    /// Create a new product; returns its assigned id.
    public func add(name : Text, price : Nat, stock : Nat, category : Text) : Nat {
      put(name, price, stock, category);
    };

    /// Update an existing product in place. No-op if the id is unknown.
    public func update(id : Nat, name : Text, price : Nat, stock : Nat, category : Text) {
      switch (products.get(id)) {
        case (?_) { products.put(id, { id; name; price; stock; category }) };
        case null {};
      };
    };

    /// Delete a product by id.
    public func delete(id : Nat) {
      products.delete(id);
    };

    /// Look up a single product.
    public func get(id : Nat) : ?Product {
      products.get(id);
    };

    /// All products, sorted by id for a stable display order.
    public func all() : [Product] {
      let arr = Iter.toArray(products.vals());
      Array.sort<Product>(arr, func(a, b) { Nat.compare(a.id, b.id) });
    };

    /// Count of products in the store.
    public func count() : Nat {
      products.size();
    };

    // ---- Upgrade-stable persistence (MotoView mvStableSave/mvStableLoad) ----

    public func mvStableSave() : Blob {
      to_candid ((nextId, Iter.toArray(products.entries())));
    };

    public func mvStableLoad(b : Blob) {
      switch (from_candid (b) : ?(Nat, [(Nat, Product)])) {
        case (?(n, entries)) {
          nextId := n;
          // Replace, don't append: clear the seeded store before refilling.
          for (k in Iter.toArray(products.keys()).vals()) { products.delete(k) };
          for ((k, v) in entries.vals()) { products.put(k, v) };
        };
        case null {};
      };
    };
  };
};
