/// Network-graph data and neighbour logic — a stateless Motoko service.
///
/// A Motoko `module` is a stateless library: the graph itself is fixed seed
/// data returned by `nodes()` / `edges()`, and the pure functions here answer
/// questions about it ("are A and B connected?", "who are A's neighbours?").
/// The `motoview` compiler imports `src/Services/*.mo` into the generated
/// actor, so the page can call `Graph.neighbors(...)` and use `Graph.Node`.
import Array "mo:base/Array";

module {

  /// A node has a stable id, a human caption, and fixed canvas coordinates
  /// (`x`, `y`) inside the 0..600 by 0..400 viewBox. We use `caption` rather
  /// than the reserved-ish `label` so the field name is always safe to read.
  public type Node = {
    id : Nat;
    caption : Text;
    x : Nat;
    y : Nat;
  };

  /// An undirected edge between two node ids.
  public type Edge = {
    from : Nat;
    to : Nat;
  };

  /// Six labelled services laid out across the canvas.
  public func nodes() : [Node] {
    [
      { id = 1; caption = "Gateway"; x = 90;  y = 200 },
      { id = 2; caption = "Auth";    x = 250; y = 80  },
      { id = 3; caption = "API";     x = 300; y = 230 },
      { id = 4; caption = "Ledger";  x = 470; y = 120 },
      { id = 5; caption = "Storage"; x = 490; y = 300 },
      { id = 6; caption = "Worker";  x = 250; y = 340 },
    ];
  };

  /// Seven connections forming a small mesh.
  public func edges() : [Edge] {
    [
      { from = 1; to = 2 },
      { from = 1; to = 3 },
      { from = 2; to = 3 },
      { from = 2; to = 4 },
      { from = 3; to = 4 },
      { from = 3; to = 5 },
      { from = 3; to = 6 },
    ];
  };

  /// Look up a node by id (returns the first node if the id is unknown, which
  /// never happens for our fixed data — the fallback just keeps the type total).
  public func get(id : Nat) : Node {
    let all = nodes();
    for (n in all.vals()) { if (n.id == id) { return n } };
    all[0];
  };

  /// True when `a` and `b` are joined by an edge (edges are undirected).
  public func connected(a : Nat, b : Nat) : Bool {
    for (e in edges().vals()) {
      if ((e.from == a and e.to == b) or (e.from == b and e.to == a)) {
        return true;
      };
    };
    false;
  };

  /// All nodes directly connected to `id`, as full Node records, in id order.
  public func neighbors(id : Nat) : [Node] {
    Array.filter<Node>(nodes(), func(n) { connected(id, n.id) });
  };

  /// The two endpoints of an edge, resolved to nodes for drawing the line.
  public func endpoints(e : Edge) : (Node, Node) {
    (get(e.from), get(e.to));
  };

  /// True when an edge touches the selected node — used to highlight it.
  public func edgeTouches(e : Edge, id : Nat) : Bool {
    e.from == id or e.to == id;
  };
};
