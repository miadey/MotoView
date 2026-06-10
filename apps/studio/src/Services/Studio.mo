/// Studio — design-time session state for MotokoStudio.
///
/// MotokoStudio is a design-time environment for building MotoView apps. The
/// AUTHORING loop (prompt → AI generate → validate gate → deploy → preview) runs
/// OFF-CANISTER in `tools/studio` — there is no on-canister LLM. This service
/// holds only the design-time SESSION state the studio shell renders: the
/// current project, the .mview source text under edit, and the LAST result of
/// running the unbypassable compiler gate (`motoview check` + `motoview lint`)
/// over that source.
///
/// Stateful service convention: a MotoView service that holds mutable state is a
/// `public class <Name>()` (a plain `module` cannot). The compiler instantiates
/// ONE shared `Studio` at actor scope and wires `mvStableSave` / `mvStableLoad`
/// so the session survives canister upgrades (exactly like Vault / Identity).
import HashMap "mo:base/HashMap";
import Text "mo:base/Text";
import Time "mo:base/Time";
import Iter "mo:base/Iter";
import Nat "mo:base/Nat";

module {

  public class Studio() {

    /// The outcome of running the compiler gate over a project's source. This is
    /// a FAITHFUL mirror of what `tools/studio/validate.sh` decides: the artifact
    /// can only be saved when BOTH `motoview check` and `motoview lint` pass.
    /// `null` means "not validated yet this session" — the studio treats that as
    /// unsaveable, same as a failure.
    public type GateResult = {
      checkPassed : Bool; // `motoview check <dir>` exited 0
      lintPassed : Bool; // `motoview lint <dir>` exited 0
      message : Text; // human summary / first error line, for the UI
      at : Int; // Time.now() when the gate last ran
    };

    /// "Saveable" is the whole point of the security-by-construction gate: a
    /// `.mview` artifact may be saved ONLY when the compiler accepted it. The AI
    /// cannot bypass this; the compiler is the authority, not the model.
    public func saveable(g : GateResult) : Bool {
      g.checkPassed and g.lintPassed
    };

    /// One design-time project under edit. `source` is the current .mview text;
    /// `lastGate` is the last compiler-gate result (null until first validation).
    public type Project = {
      name : Text;
      source : Text;
      lastGate : ?GateResult;
      updated : Int;
    };

    let projects = HashMap.HashMap<Text, Project>(16, Text.equal, Text.hash);

    /// A short, honest starter so the design surface is never an empty void: a
    /// real, lint-clean MotoView page (no mutating form, so it passes the gate as
    /// shown). The studio's job is to keep edits on the right side of the gate.
    public let starterSource : Text =
      "@page \"/\"\n" #
      "@layout Main\n" #
      "@title \"Hello from MotokoStudio\"\n" #
      "\n" #
      "<section class=\"mv-container\">\n" #
      "  <h1>Hello, MotoView</h1>\n" #
      "  <p>Current count: <strong>@count</strong></p>\n" #
      "  <Button kind=\"primary\" @click=\"increment\">+1</Button>\n" #
      "</section>\n" #
      "\n" #
      "@code {\n" #
      "  var count : Nat = 0;\n" #
      "  func increment() : async () { count += 1; };\n" #
      "}\n";

    /// Create (or replace) a project with a starting source. Not yet validated.
    public func open(name : Text, source : Text) : Project {
      let p : Project = { name; source; lastGate = null; updated = Time.now() };
      projects.put(name, p);
      p
    };

    public func get(name : Text) : ?Project { projects.get(name) };

    public func all() : [Project] { Iter.toArray(projects.vals()) };

    public func count() : Nat { projects.size() };

    /// Replace the source text under edit. Editing INVALIDATES the last gate
    /// result — the new text has not been through `check`/`lint`, so it is not
    /// saveable until the studio re-runs the gate. This is the in-canister mirror
    /// of "edited but not yet re-validated".
    public func setSource(name : Text, source : Text) : ?Project {
      switch (projects.get(name)) {
        case (?p) {
          let np : Project = { name = p.name; source; lastGate = null; updated = Time.now() };
          projects.put(name, np);
          ?np
        };
        case null { null };
      };
    };

    /// Record the result of running the compiler gate (what validate.sh decided).
    public func recordGate(name : Text, g : GateResult) : ?Project {
      switch (projects.get(name)) {
        case (?p) {
          let np : Project = { name = p.name; source = p.source; lastGate = ?g; updated = Time.now() };
          projects.put(name, np);
          ?np
        };
        case null { null };
      };
    };

    /// Whether a project's CURRENT source is saveable (last gate passed both).
    public func projectSaveable(name : Text) : Bool {
      switch (projects.get(name)) {
        case (?p) { switch (p.lastGate) { case (?g) { saveable(g) }; case null { false } } };
        case null { false };
      };
    };

    // ── Upgrade-stable persistence (compiler wires these to a stable var) ──────
    public func mvStableSave() : Blob {
      to_candid (Iter.toArray(projects.entries()));
    };
    public func mvStableLoad(blob : Blob) {
      switch (from_candid (blob) : ?[(Text, Project)]) {
        case (?entries) { for ((k, v) in entries.vals()) { projects.put(k, v) } };
        case null {};
      };
    };
  };
}
