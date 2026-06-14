/// Servers service — Discord-style servers/guilds with channels, roles,
/// moderation, and membership.
///
/// Stateful service: the MotoView compiler instantiates ONE shared `Servers`
/// at actor scope, so every page sees the same directory for the canister's
/// lifetime. (A plain `module` cannot hold mutable state — see the framework's
/// stateful-service convention: a service file exporting `public class <Name>()`.)
///
/// Cross-service coordination (e.g. author display handles from `Identity`)
/// happens in PAGES, not here: mutating methods take the caller's `Principal`
/// (the page passes `ctx.caller`) and accept a handle snapshot `Text` where a
/// human label is worth storing.
import HashMap "mo:base/HashMap";
import Principal "mo:base/Principal";
import Text "mo:base/Text";
import Time "mo:base/Time";
import Iter "mo:base/Iter";
import Array "mo:base/Array";
import Buffer "mo:base/Buffer";
import Nat "mo:base/Nat";
import Int "mo:base/Int";
import Hash "mo:base/Hash";

module {

  /// A server/guild. `kind` is immutable after creation. `channelIds` lists the
  /// Chat-room ids attached to this server (room 0 is the global default).
  
  
  /// Ordered role within a single server. Global super-admin lives in `Admin`.
  
  /// A member record: principal + a handle snapshot + when they joined.
  
  public class Servers() {
    public type ServerKind = { #Discussion; #Forum; #Feed };
    public type Server = {
    id : Nat;
    name : Text;
    createdAt : Int;
    owner : Principal;
    channelIds : [Nat];
    kind : ServerKind;
    private_ : Bool;
  };
    public type Role = { #None; #Moderator; #Admin; #Owner };
    public type Member = {
    principal : Principal;
    handle : Text;
    joinedAt : Int;
  };


    // ---- core server state ----
    var nextId : Nat = 1;
    let servers_ = HashMap.HashMap<Nat, Server>(32, Nat.equal, Hash.hash);

    // roles: serverId -> (principal -> Role)
    let roles = HashMap.HashMap<Nat, HashMap.HashMap<Principal, Role>>(32, Nat.equal, Hash.hash);

    // membership: serverId -> (principal -> Member)
    let memberships = HashMap.HashMap<Nat, HashMap.HashMap<Principal, Member>>(32, Nat.equal, Hash.hash);

    // moderation
    // pinned message ids per server: serverId -> (msgId -> ())
    let pins = HashMap.HashMap<Nat, HashMap.HashMap<Nat, Bool>>(32, Nat.equal, Hash.hash);
    // locked room ids (global set): roomId -> Bool
    let locks = HashMap.HashMap<Nat, Bool>(64, Nat.equal, Hash.hash);
    // mutes: serverId -> (principal -> untilNanos:Int)
    let mutes = HashMap.HashMap<Nat, HashMap.HashMap<Principal, Int>>(32, Nat.equal, Hash.hash);

    // -----------------------------------------------------------------
    // Server CRUD
    // -----------------------------------------------------------------

    /// Create a server owned by `caller`. Owner is auto-joined and given the
    /// Owner role. Returns the new server id.
    public func createServer(caller : Principal, name : Text, kind : ServerKind) : Nat {
      let id = nextId;
      nextId += 1;
      let now = Time.now();
      let srv : Server = {
        id;
        name = trim(name);
        createdAt = now;
        owner = caller;
        channelIds = [];
        kind;
        private_ = false;
      };
      servers_.put(id, srv);
      setRole(id, caller, #Owner);
      putMember(id, caller, "", now);
      id
    };

    /// All servers, newest first.
    public func servers() : [Server] {
      let arr = Iter.toArray(servers_.vals());
      Array.sort(arr, func(a : Server, b : Server) : { #less; #equal; #greater } {
        if (a.createdAt > b.createdAt) #less
        else if (a.createdAt < b.createdAt) #greater
        else #equal
      });
    };

    public func server(id : Nat) : ?Server { servers_.get(id) };

    public func serverCount() : Nat { servers_.size() };

    /// Attach an existing channel/room id to a server. Caller must be able to
    /// moderate. Idempotent: re-adding the same room is a no-op success.
    public func addChannel(caller : Principal, serverId : Nat, roomId : Nat) : Bool {
      switch (servers_.get(serverId)) {
        case null { false };
        case (?srv) {
          if (not canModerate(serverId, caller)) { return false };
          if (containsNat(srv.channelIds, roomId)) { return true };
          let updated = { srv with channelIds = appendNat(srv.channelIds, roomId) };
          servers_.put(serverId, updated);
          true
        };
      };
    };

    /// Remove a channel/room id from a server. Caller must be able to moderate.
    public func removeChannel(caller : Principal, serverId : Nat, roomId : Nat) : Bool {
      switch (servers_.get(serverId)) {
        case null { false };
        case (?srv) {
          if (not canModerate(serverId, caller)) { return false };
          let kept = Array.filter<Nat>(srv.channelIds, func(x) { x != roomId });
          servers_.put(serverId, { srv with channelIds = kept });
          true
        };
      };
    };

    /// Channel/room ids attached to a server (empty if unknown server).
    public func channelsOf(serverId : Nat) : [Nat] {
      switch (servers_.get(serverId)) {
        case (?srv) { srv.channelIds };
        case null { [] };
      };
    };

    public func channelCount(serverId : Nat) : Nat { channelsOf(serverId).size() };

    /// Mark a server public/private. Owner or admin only.
    public func setPrivate(caller : Principal, serverId : Nat, on : Bool) : Bool {
      switch (servers_.get(serverId)) {
        case null { false };
        case (?srv) {
          if (roleRank(roleOf(serverId, caller)) < roleRank(#Admin)) { return false };
          servers_.put(serverId, { srv with private_ = on });
          true
        };
      };
    };

    public func isPrivate(serverId : Nat) : Bool {
      switch (servers_.get(serverId)) { case (?s) s.private_; case null false };
    };

    /// Delete a server (owner only). Cleans up roles/members/mutes/pins.
    public func deleteServer(caller : Principal, id : Nat) : Bool {
      switch (servers_.get(id)) {
        case null { false };
        case (?srv) {
          if (srv.owner != caller) { return false };
          servers_.delete(id);
          roles.delete(id);
          memberships.delete(id);
          pins.delete(id);
          mutes.delete(id);
          true
        };
      };
    };

    // -----------------------------------------------------------------
    // Roles
    // -----------------------------------------------------------------

    /// Role of `principal` within a server (#None if not assigned).
    public func roleOf(serverId : Nat, principal : Principal) : Role {
      switch (roles.get(serverId)) {
        case null { #None };
        case (?m) { switch (m.get(principal)) { case (?r) r; case null #None } };
      };
    };

    public func roleLabel(serverId : Nat, principal : Principal) : Text {
      switch (roleOf(serverId, principal)) {
        case (#Owner) "role.owner";
        case (#Admin) "role.admin";
        case (#Moderator) "role.moderator";
        case (#None) "role.member";
      };
    };

    /// Grant a role. Enforcement:
    ///  - owner/admin may grant #Moderator,
    ///  - only the owner may grant #Admin,
    ///  - #Owner cannot be granted via this method (set at create / transfer),
    ///  - granting #None is treated as a revoke.
    public func grant(caller : Principal, serverId : Nat, target : Principal, role : Role) : Bool {
      switch (servers_.get(serverId)) {
        case null { return false };
        case (?_srv) {};
      };
      let callerRole = roleOf(serverId, caller);
      switch (role) {
        case (#None) { revoke(caller, serverId, target) };
        case (#Moderator) {
          if (roleRank(callerRole) < roleRank(#Admin)) { return false };
          setRole(serverId, target, #Moderator);
          ensureMember(serverId, target);
          true
        };
        case (#Admin) {
          if (callerRole != #Owner) { return false };
          setRole(serverId, target, #Admin);
          ensureMember(serverId, target);
          true
        };
        case (#Owner) {
          // ownership transfer is not exposed through grant
          false
        };
      };
    };

    /// Revoke any role from `target`, returning them to #None. The server owner
    /// cannot be revoked. Owner/admin may revoke; an admin cannot revoke another
    /// admin (only the owner can).
    public func revoke(caller : Principal, serverId : Nat, target : Principal) : Bool {
      switch (servers_.get(serverId)) {
        case null { return false };
        case (?srv) {
          if (target == srv.owner) { return false };
          let callerRole = roleOf(serverId, caller);
          if (roleRank(callerRole) < roleRank(#Admin)) { return false };
          let targetRole = roleOf(serverId, target);
          if (targetRole == #Admin and callerRole != #Owner) { return false };
          setRole(serverId, target, #None);
          true
        };
      };
    };

    /// Privileged role set — NO caller authorization. The PAGE must gate this
    /// (cross-scope: a global Admin/SuperAdmin managing a server they're not in).
    /// The owner role is fixed and cannot be overwritten here.
    public func forceSetRole(serverId : Nat, target : Principal, role : Role) : Bool {
      switch (servers_.get(serverId)) {
        case null { false };
        case (?srv) {
          if (target == srv.owner) { return false };
          setRole(serverId, target, role);
          if (role != #None) { ensureMember(serverId, target) };
          true;
        };
      };
    };

    /// Privileged channel attach — NO server-level permission check; the PAGE
    /// gates it (a global Moderator+ acting on a server they don't moderate).
    /// Mirrors addChannel without the `canModerate` gate.
    public func forceAddChannel(serverId : Nat, roomId : Nat) : Bool {
      switch (servers_.get(serverId)) {
        case null { false };
        case (?srv) {
          if (containsNat(srv.channelIds, roomId)) { return true };
          servers_.put(serverId, { srv with channelIds = appendNat(srv.channelIds, roomId) });
          true;
        };
      };
    };

    /// Privileged channel lock/unlock — NO permission check; the PAGE gates it.
    /// (`locks` is keyed globally by roomId, so no serverId is needed.)
    public func forceLock(roomId : Nat, on : Bool) : Bool {
      if (on) { locks.put(roomId, true) } else { locks.delete(roomId) };
      true;
    };

    /// Privileged delete — NO caller authorization (the page gates on a global
    /// Admin/SuperAdmin). Mirrors deleteServer without the owner check.
    public func forceDelete(serverId : Nat) : Bool {
      switch (servers_.get(serverId)) {
        case null { false };
        case (?_srv) {
          servers_.delete(serverId);
          roles.delete(serverId);
          memberships.delete(serverId);
          pins.delete(serverId);
          mutes.delete(serverId);
          true;
        };
      };
    };

    /// True if `principal`'s role is >= #Moderator.
    public func canModerate(serverId : Nat, principal : Principal) : Bool {
      roleRank(roleOf(serverId, principal)) >= roleRank(#Moderator)
    };

    /// True if `principal`'s role is >= #Admin.
    public func canAdmin(serverId : Nat, principal : Principal) : Bool {
      roleRank(roleOf(serverId, principal)) >= roleRank(#Admin)
    };

    public func isOwner(serverId : Nat, principal : Principal) : Bool {
      roleOf(serverId, principal) == #Owner
    };

    /// Principals with a role >= #Moderator in this server (mod team).
    public func moderators(serverId : Nat) : [Principal] {
      switch (roles.get(serverId)) {
        case null { [] };
        case (?m) {
          let buf = Buffer.Buffer<Principal>(8);
          for ((p, r) in m.entries()) {
            if (roleRank(r) >= roleRank(#Moderator)) { buf.add(p) };
          };
          Buffer.toArray(buf)
        };
      };
    };

    // -----------------------------------------------------------------
    // Moderation: pins / locks / mutes
    // -----------------------------------------------------------------

    /// Pin a message id within a server. Caller must be able to moderate.
    public func pin(caller : Principal, serverId : Nat, msgId : Nat) : Bool {
      if (not canModerate(serverId, caller)) { return false };
      let set = switch (pins.get(serverId)) {
        case (?s) s;
        case null {
          let s = HashMap.HashMap<Nat, Bool>(16, Nat.equal, Hash.hash);
          pins.put(serverId, s); s
        };
      };
      set.put(msgId, true);
      true
    };

    /// Unpin a message id. Caller must be able to moderate.
    public func unpin(caller : Principal, serverId : Nat, msgId : Nat) : Bool {
      if (not canModerate(serverId, caller)) { return false };
      switch (pins.get(serverId)) { case (?s) { s.delete(msgId) }; case null {} };
      true
    };

    /// Is this message id pinned anywhere?
    public func isPinned(msgId : Nat) : Bool {
      for (set in pins.vals()) {
        switch (set.get(msgId)) { case (?_) { return true }; case null {} };
      };
      false
    };

    /// Pinned message ids for a server.
    public func pinned(serverId : Nat) : [Nat] {
      switch (pins.get(serverId)) {
        case null { [] };
        case (?s) { Iter.toArray(s.keys()) };
      };
    };

    public func pinnedCount(serverId : Nat) : Nat {
      switch (pins.get(serverId)) { case (?s) s.size(); case null 0 };
    };

    /// Lock or unlock a room/channel. Locked rooms block new posts from
    /// non-moderators (the Chat page enforces this via `isLocked`). Caller must
    /// be able to moderate the server that owns the room.
    public func lock(caller : Principal, serverId : Nat, roomId : Nat, on : Bool) : Bool {
      if (not canModerate(serverId, caller)) { return false };
      if (on) { locks.put(roomId, true) } else { locks.delete(roomId) };
      true
    };

    public func isLocked(roomId : Nat) : Bool {
      switch (locks.get(roomId)) { case (?v) v; case null false };
    };

    /// Mute `target` in a server for `minutes`. Caller must be able to moderate
    /// and outrank the target; the owner can never be muted. `minutes = 0`
    /// clears the mute.
    public func mute(caller : Principal, serverId : Nat, target : Principal, minutes : Nat) : Bool {
      switch (servers_.get(serverId)) {
        case null { return false };
        case (?srv) {
          if (not canModerate(serverId, caller)) { return false };
          if (target == srv.owner) { return false };
          if (roleRank(roleOf(serverId, caller)) <= roleRank(roleOf(serverId, target)) and caller != target) {
            return false;
          };
          let map = switch (mutes.get(serverId)) {
            case (?m) m;
            case null {
              let m = HashMap.HashMap<Principal, Int>(16, Principal.equal, Principal.hash);
              mutes.put(serverId, m); m
            };
          };
          if (minutes == 0) { map.delete(target) }
          else {
            let until = Time.now() + (minutes * 60 * 1_000_000_000);
            map.put(target, until);
          };
          true
        };
      };
    };

    /// Clear a mute on `target`. Caller must be able to moderate.
    public func unmute(caller : Principal, serverId : Nat, target : Principal) : Bool {
      if (not canModerate(serverId, caller)) { return false };
      switch (mutes.get(serverId)) { case (?m) { m.delete(target) }; case null {} };
      true
    };

    /// Is `principal` currently muted in this server? Expired mutes read false.
    public func isMuted(serverId : Nat, principal : Principal) : Bool {
      switch (mutes.get(serverId)) {
        case null { false };
        case (?m) {
          switch (m.get(principal)) {
            case null { false };
            case (?until) { Time.now() < until };
          };
        };
      };
    };

    /// Remaining mute time in seconds (0 if not muted / expired).
    public func muteRemainingSeconds(serverId : Nat, principal : Principal) : Nat {
      switch (mutes.get(serverId)) {
        case null { 0 };
        case (?m) {
          switch (m.get(principal)) {
            case null { 0 };
            case (?until) {
              let diff = until - Time.now();
              if (diff <= 0) { 0 } else { Int.abs(diff) / 1_000_000_000 };
            };
          };
        };
      };
    };

    // -----------------------------------------------------------------
    // Membership
    // -----------------------------------------------------------------

    /// Join a server. `caller` becomes a member (role unchanged). Idempotent.
    /// Pass an empty handle if none is known; pages should pass the caller's
    /// handle snapshot from the Identity service.
    public func join(caller : Principal, serverId : Nat) : Bool {
      joinAs(caller, serverId, "")
    };

    /// Join carrying a handle snapshot (preferred — pages pass the Identity
    /// handle so member rails can render without a second lookup).
    public func joinAs(caller : Principal, serverId : Nat, handle : Text) : Bool {
      switch (servers_.get(serverId)) {
        case null { false };
        case (?_srv) {
          putMember(serverId, caller, handle, Time.now());
          true
        };
      };
    };

    /// Leave a server. The owner cannot leave (must delete or transfer first).
    public func leave(caller : Principal, serverId : Nat) : Bool {
      switch (servers_.get(serverId)) {
        case null { false };
        case (?srv) {
          if (srv.owner == caller) { return false };
          switch (memberships.get(serverId)) {
            case (?m) { m.delete(caller) };
            case null {};
          };
          // also drop any role they held
          switch (roles.get(serverId)) { case (?r) { r.delete(caller) }; case null {} };
          true
        };
      };
    };

    /// Members of a server, oldest-joined first.
    public func members(serverId : Nat) : [Principal] {
      let recs = memberRecords(serverId);
      Array.map<Member, Principal>(recs, func(m) { m.principal });
    };

    /// Full member records (principal + handle snapshot + joinedAt), oldest first.
    public func memberRecords(serverId : Nat) : [Member] {
      switch (memberships.get(serverId)) {
        case null { [] };
        case (?m) {
          let arr = Iter.toArray(m.vals());
          Array.sort(arr, func(a : Member, b : Member) : { #less; #equal; #greater } {
            if (a.joinedAt < b.joinedAt) #less
            else if (a.joinedAt > b.joinedAt) #greater
            else #equal
          });
        };
      };
    };

    public func isMember(serverId : Nat, principal : Principal) : Bool {
      switch (memberships.get(serverId)) {
        case null { false };
        case (?m) { m.get(principal) != null };
      };
    };

    public func memberCount(serverId : Nat) : Nat {
      switch (memberships.get(serverId)) { case (?m) m.size(); case null 0 };
    };

    /// Servers `principal` belongs to, newest first.
    public func serversOf(principal : Principal) : [Server] {
      let buf = Buffer.Buffer<Server>(8);
      for (srv in servers().vals()) {
        if (isMember(srv.id, principal)) { buf.add(srv) };
      };
      Buffer.toArray(buf)
    };

    // -----------------------------------------------------------------
    // Text / display helpers (so thin .mview pages can render directly)
    // -----------------------------------------------------------------

    public func kindLabel(kind : ServerKind) : Text {
      switch (kind) {
        case (#Discussion) "servers.kind_discussion";
        case (#Forum) "servers.kind_forum";
        case (#Feed) "servers.kind_feed";
      };
    };

    // Raw English kind label for the legacy summaryOf only (kindLabel now
    // returns an i18n key; the presentation layer translates it via Lang.tc).
    func kindRaw(kind : ServerKind) : Text {
      switch (kind) { case (#Discussion) "Discussion"; case (#Forum) "Forum"; case (#Feed) "Feed" };
    };

    /// Compact human summary, e.g. "Discussion · 3 channels · 12 members".
    public func summaryOf(serverId : Nat) : Text {
      switch (servers_.get(serverId)) {
        case null { "" };
        case (?srv) {
          kindRaw(srv.kind)
          # " · " # Nat.toText(srv.channelIds.size()) # plural(srv.channelIds.size(), " channel", " channels")
          # " · " # Nat.toText(memberCount(serverId)) # plural(memberCount(serverId), " member", " members")
        };
      };
    };

    /// Relative time string from a past `Int` timestamp (nanoseconds), e.g.
    /// "just now", "5m", "3h", "2d", "4w".
    public func relativeTime(past : Int) : Text {
      let now = Time.now();
      var diff = now - past;
      if (diff < 0) { diff := 0 };
      let secs = Int.abs(diff) / 1_000_000_000;
      if (secs < 45) { "just now" }
      else if (secs < 3600) { Nat.toText(secs / 60) # "m" }
      else if (secs < 86400) { Nat.toText(secs / 3600) # "h" }
      else if (secs < 604800) { Nat.toText(secs / 86400) # "d" }
      else { Nat.toText(secs / 604800) # "w" };
    };

    // -----------------------------------------------------------------
    // internal helpers
    // -----------------------------------------------------------------

    func roleRank(r : Role) : Nat {
      switch (r) { case (#None) 0; case (#Moderator) 1; case (#Admin) 2; case (#Owner) 3 };
    };

    func setRole(serverId : Nat, principal : Principal, role : Role) {
      let m = switch (roles.get(serverId)) {
        case (?m) m;
        case null {
          let m = HashMap.HashMap<Principal, Role>(16, Principal.equal, Principal.hash);
          roles.put(serverId, m); m
        };
      };
      switch (role) {
        case (#None) { m.delete(principal) };
        case (_) { m.put(principal, role) };
      };
    };

    func putMember(serverId : Nat, principal : Principal, handle : Text, at : Int) {
      let m = switch (memberships.get(serverId)) {
        case (?m) m;
        case null {
          let m = HashMap.HashMap<Principal, Member>(16, Principal.equal, Principal.hash);
          memberships.put(serverId, m); m
        };
      };
      // preserve original joinedAt if already a member; refresh handle if given
      let joinedAt = switch (m.get(principal)) { case (?old) old.joinedAt; case null at };
      let h = switch (m.get(principal)) {
        case (?old) { if (handle == "") old.handle else handle };
        case null { handle };
      };
      m.put(principal, { principal; handle = h; joinedAt });
    };

    // ensure the target is at least a plain member (used when granting a role)
    func ensureMember(serverId : Nat, principal : Principal) {
      if (not isMember(serverId, principal)) {
        putMember(serverId, principal, "", Time.now());
      };
    };

    func containsNat(arr : [Nat], x : Nat) : Bool {
      for (v in arr.vals()) { if (v == x) { return true } };
      false
    };

    func appendNat(arr : [Nat], x : Nat) : [Nat] {
      let buf = Buffer.Buffer<Nat>(arr.size() + 1);
      for (v in arr.vals()) { buf.add(v) };
      buf.add(x);
      Buffer.toArray(buf)
    };

    func plural(n : Nat, one : Text, many : Text) : Text {
      if (n == 1) one else many
    };

    func trim(t : Text) : Text { Text.trim(t, #char ' ') };

    // One-time brand rename: the live mainnet was first seeded BEFORE the Pulse
    // rename, so stored community names can still read "Bzzz" (e.g. "Bzzz HQ").
    // mvStableLoad applies this to each loaded name; idempotent (a no-op once no
    // "Bzzz" remains), and only touches the brand token — never other content.
    func brandFix(t : Text) : Text { Text.replace(t, #text "Bzzz", "Pulse") };

    // -----------------------------------------------------------------
    // Upgrade-stable persistence
    //
    // The MotoView compiler detects `mvStableSave`/`mvStableLoad` and generates
    // the actor-level stable backing plus preupgrade/postupgrade hooks. The
    // constructor re-seeds starter data on every (re)instantiation, so
    // `mvStableLoad` runs AFTER that seeding on upgrade and must REPLACE state,
    // not append to it. Nested HashMaps are flattened to nested arrays.
    // -----------------------------------------------------------------

    public func mvStableSave() : Blob {
      let serversArr : [(Nat, Server)] = Iter.toArray(servers_.entries());
      let rolesArr : [(Nat, [(Principal, Role)])] =
        Array.map<(Nat, HashMap.HashMap<Principal, Role>), (Nat, [(Principal, Role)])>(
          Iter.toArray(roles.entries()),
          func((sid, m)) { (sid, Iter.toArray(m.entries())) }
        );
      let membershipsArr : [(Nat, [(Principal, Member)])] =
        Array.map<(Nat, HashMap.HashMap<Principal, Member>), (Nat, [(Principal, Member)])>(
          Iter.toArray(memberships.entries()),
          func((sid, m)) { (sid, Iter.toArray(m.entries())) }
        );
      let pinsArr : [(Nat, [(Nat, Bool)])] =
        Array.map<(Nat, HashMap.HashMap<Nat, Bool>), (Nat, [(Nat, Bool)])>(
          Iter.toArray(pins.entries()),
          func((sid, m)) { (sid, Iter.toArray(m.entries())) }
        );
      let locksArr : [(Nat, Bool)] = Iter.toArray(locks.entries());
      let mutesArr : [(Nat, [(Principal, Int)])] =
        Array.map<(Nat, HashMap.HashMap<Principal, Int>), (Nat, [(Principal, Int)])>(
          Iter.toArray(mutes.entries()),
          func((sid, m)) { (sid, Iter.toArray(m.entries())) }
        );
      to_candid ({
        nextId = nextId;
        serversArr = serversArr;
        rolesArr = rolesArr;
        membershipsArr = membershipsArr;
        pinsArr = pinsArr;
        locksArr = locksArr;
        mutesArr = mutesArr;
      });
    };

    public func mvStableLoad(b : Blob) {
      switch (
        from_candid (b) : ?{
          nextId : Nat;
          serversArr : [(Nat, Server)];
          rolesArr : [(Nat, [(Principal, Role)])];
          membershipsArr : [(Nat, [(Principal, Member)])];
          pinsArr : [(Nat, [(Nat, Bool)])];
          locksArr : [(Nat, Bool)];
          mutesArr : [(Nat, [(Principal, Int)])];
        }
      ) {
        case (?saved) {
          let savedNextId = saved.nextId;
          let serversArr = saved.serversArr;
          let rolesArr = saved.rolesArr;
          let membershipsArr = saved.membershipsArr;
          let pinsArr = saved.pinsArr;
          let locksArr = saved.locksArr;
          let mutesArr = saved.mutesArr;
          // scalar
          nextId := savedNextId;

          // servers_ : HashMap<Nat, Server>
          for (k in Iter.toArray(servers_.keys()).vals()) { servers_.delete(k) };
          for ((k, v) in serversArr.vals()) { servers_.put(k, { v with name = brandFix(v.name) }) };

          // roles : HashMap<Nat, HashMap<Principal, Role>>
          for (k in Iter.toArray(roles.keys()).vals()) { roles.delete(k) };
          for ((sid, entries) in rolesArr.vals()) {
            let m = HashMap.HashMap<Principal, Role>(16, Principal.equal, Principal.hash);
            for ((p, r) in entries.vals()) { m.put(p, r) };
            roles.put(sid, m);
          };

          // memberships : HashMap<Nat, HashMap<Principal, Member>>
          for (k in Iter.toArray(memberships.keys()).vals()) { memberships.delete(k) };
          for ((sid, entries) in membershipsArr.vals()) {
            let m = HashMap.HashMap<Principal, Member>(16, Principal.equal, Principal.hash);
            for ((p, mem) in entries.vals()) { m.put(p, mem) };
            memberships.put(sid, m);
          };

          // pins : HashMap<Nat, HashMap<Nat, Bool>>
          for (k in Iter.toArray(pins.keys()).vals()) { pins.delete(k) };
          for ((sid, entries) in pinsArr.vals()) {
            let m = HashMap.HashMap<Nat, Bool>(16, Nat.equal, Hash.hash);
            for ((mid, on) in entries.vals()) { m.put(mid, on) };
            pins.put(sid, m);
          };

          // locks : HashMap<Nat, Bool>
          for (k in Iter.toArray(locks.keys()).vals()) { locks.delete(k) };
          for ((k, v) in locksArr.vals()) { locks.put(k, v) };

          // mutes : HashMap<Nat, HashMap<Principal, Int>>
          for (k in Iter.toArray(mutes.keys()).vals()) { mutes.delete(k) };
          for ((sid, entries) in mutesArr.vals()) {
            let m = HashMap.HashMap<Principal, Int>(16, Principal.equal, Principal.hash);
            for ((p, until) in entries.vals()) { m.put(p, until) };
            mutes.put(sid, m);
          };
        };
        case null {};
      };
    };

    // -----------------------------------------------------------------
    // Starter data — makes the Servers UI feel alive without fabricating
    // fake user metrics. Owned by an anonymous principal until claimed.
    // -----------------------------------------------------------------
    let seedOwner = Principal.fromText("2vxsx-fae"); // anonymous
    ignore createServer(seedOwner, "Pulse HQ", #Discussion);
    ignore createServer(seedOwner, "Motoko Builders", #Forum);
    ignore createServer(seedOwner, "The Feed", #Feed);
  };
};
