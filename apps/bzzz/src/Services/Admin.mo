/// Admin service — the GLOBAL (app-wide) half of Pulse's role-scope system.
///
/// Two scopes make up the complete model:
///   * GLOBAL (this service): #SuperAdmin > #Admin > #Moderator > #None.
///   * SERVER (Servers.mo):    #Owner > #Admin > #Moderator > #Member.
/// Pages combine the two so a global staffer can act in ANY server/forum even
/// without a server role (cross-scope override): e.g. a moderation handler gates
/// on `Servers.canModerate(sid, caller) or Admin.isMod(caller)`.
///
/// Capabilities by global tier:
///   * SuperAdmin — manage EVERYTHING: grant/revoke any global role (incl. other
///     admins), moderate/manage every server, delete servers, moderate the forum.
///   * Admin      — manage users + content app-wide, grant/revoke Moderator,
///     moderate every server + forum.
///   * Moderator  — moderate content app-wide (pin/lock/close, mute).
///
/// Bootstrap: while there is no staff at all, the first caller to
/// `claimBootstrap` becomes the founding Super Admin. After that only existing
/// staff manage roles, and the last Super Admin can never be demoted (no lock-out).
import HashMap "mo:base/HashMap";
import Principal "mo:base/Principal";
import Text "mo:base/Text";
import Time "mo:base/Time";
import Iter "mo:base/Iter";
import Array "mo:base/Array";
import Int "mo:base/Int";

module {
  public class Admin() {
    public type Role = { #None; #Moderator; #Admin; #SuperAdmin };

    public type Entry = {
      principal : Principal;
      principalText : Text;
      handle : Text; // snapshot supplied by the page (Identity service)
      role : Role;
      grantedAt : Int; // Time.now() nanoseconds
    };

    /// Caller-centric summary for the /admin gate + UI.
    public type WhoAmI = {
      principalText : Text;
      role : Role;
      roleLabel : Text;
      isStaff : Bool; // Moderator or above
      isAdmin : Bool; // Admin or above
      isSuperAdmin : Bool;
      staffCount : Nat;
    };

    // principal -> Entry. Membership === having a global role >= Moderator.
    let staff = HashMap.HashMap<Principal, Entry>(16, Principal.equal, Principal.hash);

    func rank(r : Role) : Nat {
      switch r { case (#None) 0; case (#Moderator) 1; case (#Admin) 2; case (#SuperAdmin) 3 };
    };

    // ---- queries ----

    public func roleOf(p : Principal) : Role {
      switch (staff.get(p)) { case (?e) e.role; case null #None };
    };

    public func roleLabel(p : Principal) : Text { labelOf(roleOf(p)) };

    func labelOf(r : Role) : Text {
      switch r {
        case (#SuperAdmin) "admin.role_super";
        case (#Admin) "admin.role_admin";
        case (#Moderator) "admin.role_mod";
        case (#None) "admin.role_member";
      };
    };

    /// Cross-scope helpers — the policy pages use to OR against server roles.
    public func isSuperAdmin(p : Principal) : Bool { rank(roleOf(p)) >= 3 };
    public func isAdmin(p : Principal) : Bool { rank(roleOf(p)) >= 2 }; // Admin or SuperAdmin
    public func isMod(p : Principal) : Bool { rank(roleOf(p)) >= 1 };   // Moderator or above

    public func staffCount() : Nat { staff.size() };

    func superAdminCount() : Nat {
      var n = 0;
      for (e in staff.vals()) { if (e.role == #SuperAdmin) { n += 1 } };
      n;
    };

    /// All staff, highest rank first then oldest grant first.
    public func entries() : [Entry] { sorted() };

    public func handleOf(p : Principal) : Text {
      switch (staff.get(p)) { case (?e) e.handle; case null "" };
    };

    public func whoami(caller : Principal) : WhoAmI {
      let r = roleOf(caller);
      {
        principalText = Principal.toText(caller);
        role = r;
        roleLabel = labelOf(r);
        isStaff = rank(r) >= 1;
        isAdmin = rank(r) >= 2;
        isSuperAdmin = rank(r) >= 3;
        staffCount = staff.size();
      };
    };

    // ---- mutations ----

    /// Bootstrap claim. If there is no staff at all, the caller becomes the
    /// founding Super Admin and this returns true; otherwise false.
    public func claimBootstrap(caller : Principal, handle : Text) : Bool {
      if (staff.size() != 0) { return false };
      staff.put(caller, entry(caller, handle, #SuperAdmin, Time.now()));
      true;
    };

    /// Set `target`'s global role. Returns false if the rules forbid it:
    ///  - caller must be Admin+ to manage any role;
    ///  - affecting an Admin/SuperAdmin role (target's current OR the new role)
    ///    requires the caller to be SuperAdmin;
    ///  - you cannot grant a role above your own rank;
    ///  - you cannot change someone who outranks you;
    ///  - the last SuperAdmin cannot be demoted.
    /// `role = #None` revokes. A blank `handle` keeps any existing snapshot.
    public func setRole(caller : Principal, target : Principal, handle : Text, role : Role) : Bool {
      let cr = roleOf(caller);
      if (rank(cr) < 2) { return false };
      let tr = roleOf(target);
      let touchesHigh = rank(role) >= 2 or rank(tr) >= 2;
      if (touchesHigh and rank(cr) < 3) { return false };
      if (rank(role) > rank(cr)) { return false };
      if (rank(tr) > rank(cr)) { return false };
      if (tr == #SuperAdmin and role != #SuperAdmin and superAdminCount() <= 1) { return false };
      if (role == #None) {
        staff.delete(target);
      } else {
        let h = if (handle == "") { switch (staff.get(target)) { case (?e) e.handle; case null "" } } else { handle };
        staff.put(target, entry(target, h, role, Time.now()));
      };
      true;
    };

    /// Convenience revoke (role -> #None).
    public func removeRole(caller : Principal, target : Principal) : Bool {
      setRole(caller, target, "", #None);
    };

    // ---- helpers ----

    func entry(p : Principal, handle : Text, role : Role, at : Int) : Entry {
      { principal = p; principalText = Principal.toText(p); handle = handle; role = role; grantedAt = at };
    };

    func sorted() : [Entry] {
      Array.sort(
        Iter.toArray(staff.vals()),
        func(a : Entry, b : Entry) : { #less; #equal; #greater } {
          if (rank(a.role) > rank(b.role)) { #less } else if (rank(a.role) < rank(b.role)) { #greater } else if (a.grantedAt < b.grantedAt) {
            #less;
          } else if (a.grantedAt > b.grantedAt) { #greater } else { #equal };
        },
      );
    };

    /// Relative time helper (Text). `at`/`now` are nanosecond timestamps.
    public func relativeTime(at : Int, now : Int) : Text {
      let diff = now - at;
      if (diff < 0) { return "just now" };
      let secs = diff / 1_000_000_000;
      if (secs < 60) { return "just now" };
      let mins = secs / 60;
      if (mins < 60) { return natText(mins) # "m ago" };
      let hours = mins / 60;
      if (hours < 24) { return natText(hours) # "h ago" };
      let days = hours / 24;
      if (days < 30) { return natText(days) # "d ago" };
      let months = days / 30;
      if (months < 12) { return natText(months) # "mo ago" };
      natText(months / 12) # "y ago";
    };

    func natText(n : Int) : Text { if (n < 0) { "0" } else { Int.toText(n) } };

    // ---- upgrade-stable persistence ----

    public func mvStableSave() : Blob { to_candid (Iter.toArray(staff.entries())) };

    public func mvStableLoad(b : Blob) {
      switch (from_candid (b) : ?[(Principal, Entry)]) {
        case (?es) {
          for (k in Iter.toArray(staff.keys()).vals()) { staff.delete(k) };
          for ((k, v) in es.vals()) { staff.put(k, v) };
        };
        case null {};
      };
    };
  };
};
