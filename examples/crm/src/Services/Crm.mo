/// CRM pipeline logic — a stateless Motoko service.
///
/// A Motoko `module` is a stateless library, so the deal list lives in the
/// page's `@code` (`var deals`) and these pure functions transform it. The
/// `motoview` compiler imports `src/Services/*.mo` into the generated actor
/// automatically, so page code can call `Crm.byStage(deals, "Lead")` etc.
import Array "mo:base/Array";
import Int "mo:base/Int";

module {

  public type Deal = {
    id : Nat;
    title : Text;
    company : Text;
    contact : Text;
    value : Nat; // pipeline value in whole dollars
    stage : Text;
  };

  public let stages : [Text] = ["Lead", "Contacted", "Proposal", "Won"];

  /// Initial demo data.
  public func seed() : [Deal] {
    [
      { id = 1; title = "Website redesign"; company = "Acme Corp"; contact = "Jane Diaz"; value = 12000; stage = "Lead" },
      { id = 2; title = "Mobile app build"; company = "Globex"; contact = "Sam Park"; value = 38000; stage = "Contacted" },
      { id = 3; title = "ICP migration"; company = "Initech"; contact = "Lena Ortiz"; value = 54000; stage = "Proposal" },
      { id = 4; title = "Support retainer"; company = "Umbrella"; contact = "Theo Vance"; value = 9000; stage = "Lead" },
      { id = 5; title = "Data pipeline"; company = "Hooli"; contact = "Priya Nair"; value = 47000; stage = "Contacted" },
      { id = 6; title = "Security audit"; company = "Stark Labs"; contact = "Bruce Wahl"; value = 21000; stage = "Won" },
    ];
  };

  public func byStage(deals : [Deal], s : Text) : [Deal] {
    Array.filter<Deal>(deals, func(d) { d.stage == s });
  };

  public func countByStage(deals : [Deal], s : Text) : Nat { byStage(deals, s).size() };

  public func stageValue(deals : [Deal], s : Text) : Nat {
    var sum : Nat = 0;
    for (d in deals.vals()) { if (d.stage == s) { sum += d.value } };
    sum;
  };

  public func total(deals : [Deal]) : Nat {
    var sum : Nat = 0;
    for (d in deals.vals()) { sum += d.value };
    sum;
  };

  public func add(deals : [Deal], id : Nat, title : Text, company : Text, contact : Text, value : Nat) : [Deal] {
    Array.append<Deal>(deals, [{ id; title; company; contact; value; stage = "Lead" }]);
  };

  public func move(deals : [Deal], id : Nat, stage : Text) : [Deal] {
    Array.map<Deal, Deal>(deals, func(d) {
      if (d.id == id) { { d with stage = stage } } else { d };
    });
  };

  /// Shift a deal forward (+1) or back (-1) through the stage list.
  public func advance(deals : [Deal], id : Nat, delta : Int) : [Deal] {
    let idx : Int = indexOf(stageOf(deals, id));
    let maxIdx : Int = stages.size() - 1;
    var ni : Int = idx + delta;
    if (ni < 0) { ni := 0 };
    if (ni > maxIdx) { ni := maxIdx };
    move(deals, id, stages[Int.abs(ni)]);
  };

  public func remove(deals : [Deal], id : Nat) : [Deal] {
    Array.filter<Deal>(deals, func(d) { d.id != id });
  };

  func stageOf(deals : [Deal], id : Nat) : Text {
    for (d in deals.vals()) { if (d.id == id) { return d.stage } };
    stages[0];
  };

  func indexOf(s : Text) : Int {
    var i : Int = 0;
    for (st in stages.vals()) { if (st == s) { return i }; i += 1 };
    0;
  };
};
