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

    /// Bind (or update) the caller's handle + display name. Returns false if the
    /// handle is already taken by a different principal.
    public func bind(caller : Principal, handle : Text, display : Text) : Bool {
      let h = normalize(handle);
      if (h == "") { return false };
      switch (byHandle.get(h)) {
        case (?owner) { if (owner != caller) { return false } };
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
        display = if (display == "") h else display;
        bio = switch existing { case (?e) e.bio; case null "" };
        avatar = switch existing { case (?e) e.avatar; case null avatarFor(h) };
        joined = switch existing { case (?e) e.joined; case null Time.now() };
      };
      byPrincipal.put(caller, prof);
      byHandle.put(h, caller);
      true
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
    public func mvStableSave() : Blob {
      to_candid ((
        Iter.toArray(byPrincipal.entries()),
        Iter.toArray(byHandle.entries()),
      ));
    };

    public func mvStableLoad(b : Blob) {
      switch (
        from_candid (b) : ?(
          [(Principal, Profile)],
          [(Text, Principal)],
        )
      ) {
        case (?(savedByPrincipal, savedByHandle)) {
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
