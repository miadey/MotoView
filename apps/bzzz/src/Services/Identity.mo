/// Identity service — binds an IC `Principal` to a public handle + profile.
///
/// Stateful service: the MotoView compiler instantiates one shared `Identity`
/// at actor scope, so every page sees the same directory for the canister's
/// lifetime. (A plain `module` cannot hold mutable state — see the framework's
/// stateful-service convention: a service file exporting `public class <Name>()`.)
import HashMap "mo:base/HashMap";
import Principal "mo:base/Principal";
import Text "mo:base/Text";
import Time "mo:base/Time";
import Iter "mo:base/Iter";
import Array "mo:base/Array";
import Char "mo:base/Char";
import Nat32 "mo:base/Nat32";
import Nat "mo:base/Nat";

module {
  
  public class Identity() {
    public type Profile = {
    principal : Principal;
    handle : Text;     // unique, lowercase, no spaces
    display : Text;    // shown name
    bio : Text;
    avatar : Text;     // emoji or short text avatar
    joined : Int;      // Time.now() nanoseconds
  };

    let byPrincipal = HashMap.HashMap<Principal, Profile>(64, Principal.equal, Principal.hash);
    let byHandle = HashMap.HashMap<Text, Principal>(64, Text.equal, Text.hash);

    /// Outcome of creating/changing a handle: ok (with the normalized handle),
    /// already taken by someone else, or invalid format (with a message).
    public type BindResult = { #ok : Text; #taken; #invalid : Text };

    /// Validate a handle's FORMAT only (independent of availability). Returns
    /// ?message when invalid, null when acceptable. This — together with the
    /// uniqueness check in bindChecked — is the single rule every part of the
    /// app goes through to create a handle.
    public func validateHandle(handle : Text) : ?Text {
      let chars = Text.toArray(normalize(handle));
      let n = chars.size();
      if (n < 2) { return ?"valid.handle_min" };
      if (n > 20) { return ?"valid.handle_max" };
      if (not isLetter(chars[0])) { return ?"valid.handle_start" };
      for (c in chars.vals()) {
        if (not (isLetter(c) or isDigit(c) or c == '_')) {
          return ?"valid.handle_chars";
        };
      };
      null;
    };

    /// True if the (normalized) handle is free, or already owned by `forCaller`.
    public func isAvailable(handle : Text, forCaller : Principal) : Bool {
      switch (byHandle.get(normalize(handle))) { case (?owner) { owner == forCaller }; case null { true } };
    };

    /// Bind (or update) the caller's handle + display name — the single
    /// chokepoint for handle creation. Validates FORMAT, then UNIQUENESS, then
    /// links the handle to the caller's principal (bidirectional index).
    public func bindChecked(caller : Principal, handle : Text, display : Text) : BindResult {
      switch (validateHandle(handle)) { case (?msg) { return #invalid(msg) }; case null {} };
      let h = normalize(handle);
      switch (byHandle.get(h)) {
        case (?owner) { if (owner != caller) { return #taken } };
        case null {};
      };
      // free the caller's previous handle if changing it
      switch (byPrincipal.get(caller)) {
        case (?old) { if (old.handle != h) { byHandle.delete(old.handle) } };
        case null {};
      };
      let existing = byPrincipal.get(caller);
      let prof : Profile = {
        principal = caller;
        handle = h;
        // An empty display argument means "no change": a CUSTOM display name
        // survives a rename (never silently reset to the handle). A display
        // that just mirrored the old handle was never customized, so it
        // follows the new handle; first-time binds fall back to the handle.
        display = if (display != "") display
                  else switch existing {
                    case (?e) { if (e.display != "" and e.display != e.handle) e.display else h };
                    case null h;
                  };
        bio = switch existing { case (?e) e.bio; case null "" };
        avatar = switch existing { case (?e) e.avatar; case null avatarFor(h) };
        joined = switch existing { case (?e) e.joined; case null Time.now() };
      };
      byPrincipal.put(caller, prof);
      byHandle.put(h, caller);
      #ok(h);
    };

    /// Back-compat boolean bind (true on success).
    public func bind(caller : Principal, handle : Text, display : Text) : Bool {
      switch (bindChecked(caller, handle, display)) { case (#ok _) { true }; case _ { false } };
    };

    /// Derive a valid, UNIQUE handle from the caller's Internet Identity
    /// principal — for the one-click "use my II as my handle". Keeps the leading
    /// alphanumerics of the principal, ensures it starts with a letter, and
    /// appends a number until it is free for this caller.
    public func suggestHandle(caller : Principal) : Text {
      let raw = Principal.toText(caller);
      var base = "";
      var k : Nat = 0;
      for (c in raw.chars()) {
        if (k < 14) {
          let lc = if (c >= 'A' and c <= 'Z') { Char.fromNat32(Char.toNat32(c) + 32) } else { c };
          if (isLetter(lc) or isDigit(lc)) { base := base # Text.fromChar(lc); k += 1 };
        };
      };
      if (base == "") { base := "user" };
      if (not isLetter(Text.toArray(base)[0])) { base := "u" # base };
      if (isAvailable(base, caller)) { return base };
      var i : Nat = 2;
      var cand = base # Nat.toText(i);
      while (not isAvailable(cand, caller)) { i += 1; cand := base # Nat.toText(i) };
      cand;
    };

    public func setBio(caller : Principal, bio : Text) : Bool {
      switch (byPrincipal.get(caller)) {
        case (?p) { byPrincipal.put(caller, { p with bio }); true };
        case null { false };
      };
    };

    public func profileOf(caller : Principal) : ?Profile { byPrincipal.get(caller) };

    public func byHandleLookup(handle : Text) : ?Profile {
      switch (byHandle.get(normalize(handle))) {
        case (?p) byPrincipal.get(p);
        case null null;
      };
    };

    /// A display handle for any principal — their bound handle, else a short id.
    public func handleOf(caller : Principal) : Text {
      switch (byPrincipal.get(caller)) {
        case (?p) p.handle;
        case null shortId(caller);
      };
    };

    /// The handle prefixed with "@" (for chips/mentions), e.g. "@madey".
    public func atHandleOf(caller : Principal) : Text { "@" # handleOf(caller) };

    public func displayOf(caller : Principal) : Text {
      switch (byPrincipal.get(caller)) {
        case (?p) p.display;
        case null shortId(caller);
      };
    };

    public func avatarOf(caller : Principal) : Text {
      switch (byPrincipal.get(caller)) {
        case (?p) p.avatar;
        case null avatarFor(shortId(caller));
      };
    };

    public func isBound(caller : Principal) : Bool { byPrincipal.get(caller) != null };
    public func count() : Nat { byPrincipal.size() };

    public func all() : [Profile] {
      Array.sort(Iter.toArray(byPrincipal.vals()), func(a : Profile, b : Profile) : { #less; #equal; #greater } {
        if (a.joined < b.joined) #less else if (a.joined > b.joined) #greater else #equal
      });
    };

    // ---- upgrade-stable persistence (MotoView hooks) ----
    // NOTE: persist as a RECORD, not a tuple — Motoko's to_candid/from_candid
    // does NOT round-trip tuples (from_candid returns null), which silently wipes
    // state on every classic-persistence upgrade. Records round-trip correctly.
    public func mvStableSave() : Blob {
      to_candid ({
        byPrincipal = Iter.toArray(byPrincipal.entries());
        byHandle = Iter.toArray(byHandle.entries());
      });
    };

    public func mvStableLoad(b : Blob) {
      switch (
        from_candid (b) : ?{
          byPrincipal : [(Principal, Profile)];
          byHandle : [(Text, Principal)];
        }
      ) {
        case (?saved) {
          let savedByPrincipal = saved.byPrincipal;
          let savedByHandle = saved.byHandle;
          for (k in Iter.toArray(byPrincipal.keys()).vals()) { byPrincipal.delete(k) };
          for ((k, v) in savedByPrincipal.vals()) { byPrincipal.put(k, v) };
          for (k in Iter.toArray(byHandle.keys()).vals()) { byHandle.delete(k) };
          for ((k, v) in savedByHandle.vals()) { byHandle.put(k, v) };
        };
        case null {};
      };
    };

    // ---- helpers ----
    func normalize(t : Text) : Text {
      Text.map(Text.trim(t, #char ' '), func(c : Char) : Char {
        if (c >= 'A' and c <= 'Z') { Char.fromNat32(Char.toNat32(c) + 32) } else { c }
      });
    };

    func isLetter(c : Char) : Bool { c >= 'a' and c <= 'z' };
    func isDigit(c : Char) : Bool { c >= '0' and c <= '9' };

    func shortId(p : Principal) : Text {
      let t = Principal.toText(p);
      let chars = Text.toArray(t);
      if (chars.size() <= 5) { t } else { Text.fromIter(Array.subArray(chars, 0, 5).vals()) };
    };

    func avatarFor(seed : Text) : Text {
      let palette = ["🐝", "🦊", "🐙", "🦉", "🐬", "🦋", "🐢", "🦄", "🐧", "🦁"];
      var h : Nat32 = 2166136261;
      for (c in seed.chars()) { h := (h ^ Char.toNat32(c)) *% 16777619 };
      palette[Nat32.toNat(h % 10)];
    };
  };
};
