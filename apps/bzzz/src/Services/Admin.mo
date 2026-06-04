/// Admin service — global super-admin allowlist for the Bzzz super-app.
///
/// Stateful service: the MotoView compiler instantiates one shared `Admin`
/// at actor scope, so every page sees the same allowlist for the canister's
/// lifetime. (A plain `module` cannot hold mutable state — see the framework's
/// stateful-service convention: a service file exporting `public class <Name>()`.)
///
/// Bootstrap rule (BUILD_SPEC PART 3 "Admin"): while the allowlist is empty the
/// first caller to `claimBootstrap(caller)` becomes the founding admin. After
/// that, only existing admins can add/remove admins, and the last admin can
/// never be removed (avoids locking the canister out of administration).
import HashMap "mo:base/HashMap";
import Principal "mo:base/Principal";
import Text "mo:base/Text";
import Time "mo:base/Time";
import Iter "mo:base/Iter";
import Array "mo:base/Array";
import Int "mo:base/Int";

module {
  /// A snapshot of an admin entry — Principal plus a handle snapshot (the page
  /// passes the current display handle from the Identity service) and when the
  /// principal was granted admin.
  
  /// Result of `whoami` — everything a thin /admin page needs to render the gate.
  
  public class Admin() {
    public type AdminEntry = {
    principal : Principal;
    principalText : Text;
    handle : Text; // handle snapshot supplied by the page (Identity service)
    grantedAt : Int; // Time.now() nanoseconds
  };
    public type WhoAmI = {
    principalText : Text;
    isAdmin : Bool;
    adminCount : Nat;
  };

    // principal -> AdminEntry. Membership in this map === being an admin.
    let admins = HashMap.HashMap<Principal, AdminEntry>(16, Principal.equal, Principal.hash);

    /// Bootstrap claim. If the allowlist is currently empty, the caller becomes
    /// the founding admin and this returns true. Otherwise it returns false
    /// (whether or not the caller is already an admin) — bootstrap only ever
    /// fires for the very first admin.
    public func claimBootstrap(caller : Principal) : Bool {
      if (admins.size() != 0) { return false };
      admins.put(caller, entry(caller, "", Time.now()));
      true;
    };

    /// Like `claimBootstrap` but records a handle snapshot for the founding admin.
    public func claimBootstrapWithHandle(caller : Principal, handle : Text) : Bool {
      if (admins.size() != 0) { return false };
      admins.put(caller, entry(caller, handle, Time.now()));
      true;
    };

    /// Is this principal on the admin allowlist?
    public func isAdmin(principal : Principal) : Bool {
      admins.get(principal) != null;
    };

    /// The full admin allowlist, oldest grant first.
    public func admins_() : [Principal] {
      Array.map<AdminEntry, Principal>(sorted(), func(e : AdminEntry) : Principal { e.principal });
    };

    /// The full admin allowlist as renderable entries, oldest grant first.
    public func adminEntries() : [AdminEntry] { sorted() };

    /// How many admins exist.
    public func adminCount() : Nat { admins.size() };

    /// Add `target` to the allowlist. `caller` must already be an admin.
    /// Returns false if the caller is not an admin. Idempotent: if `target` is
    /// already an admin its handle snapshot is refreshed and this returns true.
    public func addAdmin(caller : Principal, target : Principal) : Bool {
      addAdminWithHandle(caller, target, "");
    };

    /// Add `target` (with a handle snapshot supplied by the page) to the
    /// allowlist. `caller` must already be an admin.
    public func addAdminWithHandle(caller : Principal, target : Principal, handle : Text) : Bool {
      if (not isAdmin(caller)) { return false };
      switch (admins.get(target)) {
        case (?existing) {
          // already an admin — refresh the handle snapshot if a new one is given
          let h = if (handle == "") existing.handle else handle;
          admins.put(target, { existing with handle = h });
        };
        case null { admins.put(target, entry(target, handle, Time.now())) };
      };
      true;
    };

    /// Remove `target` from the allowlist. `caller` must be an admin. Returns
    /// false if the caller is not an admin, if `target` is not an admin, or if
    /// `target` is the last remaining admin (the allowlist can never be emptied).
    public func removeAdmin(caller : Principal, target : Principal) : Bool {
      if (not isAdmin(caller)) { return false };
      if (not isAdmin(target)) { return false };
      if (admins.size() <= 1) { return false }; // cannot remove the last admin
      admins.delete(target);
      true;
    };

    /// Caller-centric summary for the /admin page gate.
    public func whoami(caller : Principal) : WhoAmI {
      {
        principalText = Principal.toText(caller);
        isAdmin = isAdmin(caller);
        adminCount = admins.size();
      };
    };

    /// Text passthrough so a thin page can show whether the caller is admin.
    public func roleLabel(caller : Principal) : Text {
      if (isAdmin(caller)) "Admin" else "Member";
    };

    /// The handle snapshot stored for an admin principal (empty if none/unknown).
    public func handleOf(principal : Principal) : Text {
      switch (admins.get(principal)) {
        case (?e) e.handle;
        case null "";
      };
    };

    // ---- helpers ----

    func entry(p : Principal, handle : Text, at : Int) : AdminEntry {
      {
        principal = p;
        principalText = Principal.toText(p);
        handle = handle;
        grantedAt = at;
      };
    };

    func sorted() : [AdminEntry] {
      Array.sort(
        Iter.toArray(admins.vals()),
        func(a : AdminEntry, b : AdminEntry) : { #less; #equal; #greater } {
          if (a.grantedAt < b.grantedAt) #less else if (a.grantedAt > b.grantedAt) #greater else #equal;
        },
      );
    };

    /// Relative time helper (Text) so pages can render "5m ago" style activity.
    /// `at` and `now` are nanosecond timestamps (Time.now()).
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

    func natText(n : Int) : Text {
      if (n < 0) { "0" } else { Int.toText(n) };
    };

    // ---- upgrade-stable persistence (MotoView mvStableSave/mvStableLoad) ----

    public func mvStableSave() : Blob {
      to_candid (Iter.toArray(admins.entries()));
    };

    public func mvStableLoad(b : Blob) {
      switch (from_candid (b) : ?[(Principal, AdminEntry)]) {
        case (?adminsEntries) {
          for (k in Iter.toArray(admins.keys()).vals()) { admins.delete(k) };
          for ((k, v) in adminsEntries.vals()) { admins.put(k, v) };
        };
        case null {};
      };
    };
  };
};
