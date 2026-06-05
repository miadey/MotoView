/// Role store — principal → roles. Backs `@authorize role="..."` and the
/// `ctx.{hasRole,grantRole,revokeRole,claimRole,callerRoles}` API. The generated
/// actor persists it across upgrades via a `stable var` (see project.rs).
import HashMap "mo:base/HashMap";
import Principal "mo:base/Principal";
import Array "mo:base/Array";
import Iter "mo:base/Iter";

module {
  public class Store() {
    let roles = HashMap.HashMap<Principal, [Text]>(16, Principal.equal, Principal.hash);

    public func rolesOf(p : Principal) : [Text] {
      switch (roles.get(p)) { case (?r) { r }; case null { [] } };
    };

    public func has(p : Principal, role : Text) : Bool {
      for (r in rolesOf(p).vals()) { if (r == role) { return true } };
      false;
    };

    /// Does any principal hold this role?
    public func anyHas(role : Text) : Bool {
      for ((_, rs) in roles.entries()) {
        for (r in rs.vals()) { if (r == role) { return true } };
      };
      false;
    };

    public func grant(p : Principal, role : Text) {
      if (not Principal.isAnonymous(p) and not has(p, role)) {
        roles.put(p, Array.append(rolesOf(p), [role]));
      };
    };

    public func revoke(p : Principal, role : Text) {
      let kept = Array.filter<Text>(rolesOf(p), func(r) { r != role });
      roles.put(p, kept);
    };

    /// First-come bootstrap: grant `role` to `p` only if no principal holds it
    /// yet. Returns true if granted. Safe way to seat the first admin.
    public func claim(p : Principal, role : Text) : Bool {
      if (Principal.isAnonymous(p) or anyHas(role)) { return false };
      grant(p, role);
      true;
    };

    public func dump() : [(Principal, [Text])] { Iter.toArray(roles.entries()) };
    public func load(rs : [(Principal, [Text])]) { for ((k, v) in rs.vals()) { roles.put(k, v) } };
  };
};
