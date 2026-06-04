/// Status service — WhatsApp-style 24h ephemeral statuses.
///
/// Stateful service: the MotoView compiler instantiates one shared `Status`
/// at actor scope, so every page sees the same store for the canister's
/// lifetime. (A plain `module` cannot hold mutable state — see the framework's
/// stateful-service convention: a service file exporting `public class <Name>()`.)
///
/// A status expires 24h after it is posted. Expired statuses are pruned lazily
/// on every read (any row with `expiresAt < Time.now()` is dropped before the
/// result is computed). Real media upload is the client's job — for `#Image`
/// statuses the page passes an image reference/URL in `text`; the canister
/// stores it honestly as opaque Text and never fabricates content.
import HashMap "mo:base/HashMap";
import Principal "mo:base/Principal";
import Text "mo:base/Text";
import Time "mo:base/Time";
import Iter "mo:base/Iter";
import Array "mo:base/Array";
import Nat "mo:base/Nat";
import Int "mo:base/Int";
import Hash "mo:base/Hash";
import Buffer "mo:base/Buffer";

module {
  /// What kind of status this is. `#Text` renders `text` on a colored card;
  /// `#Image` treats `text` as an image reference/URL supplied by the client.
  
  /// A single ephemeral status update.
  ///
  /// NOTE: the record type is named `Update` (not `Status`) because the class
  /// itself is `Status` (class name === file name, per the contract) and a
  /// Motoko block cannot define a type and a class with the same name. Pages
  /// reference it as `Status.Update`.
  
  public class Status() {
    public type Kind = { #Text; #Image };
    public type Update = {
    id : Nat;
    author : Principal;
    authorHandle : Text;  // handle snapshot at post time (page supplies it)
    at : Int;             // Time.now() nanoseconds when posted
    text : Text;          // status text, or image ref for #Image kind
    kind : Kind;
    color : Text;         // #hex background color for #Text statuses
    expiresAt : Int;      // at + 24h, in nanoseconds
    views : Nat;          // aggregate view count (optional tracking)
  };

    // 24 hours in nanoseconds.
    let DAY_NS : Int = 24 * 60 * 60 * 1_000_000_000;
    let DEFAULT_COLOR : Text = "#075E54"; // WhatsApp green default for text status

    var nextId : Nat = 1;
    let statuses = HashMap.HashMap<Nat, Update>(64, Nat.equal, Hash.hash);

    // ---- upgrade-stable persistence (MotoView compiler hooks) ----

    public func mvStableSave() : Blob {
      to_candid ((
        nextId,
        Iter.toArray(statuses.entries()),
      ));
    };

    public func mvStableLoad(b : Blob) {
      switch (from_candid (b) : ?(Nat, [(Nat, Update)])) {
        case (?(savedNextId, savedStatuses)) {
          nextId := savedNextId;
          for (k in Iter.toArray(statuses.keys()).vals()) { statuses.delete(k) };
          for ((k, v) in savedStatuses.vals()) { statuses.put(k, v) };
        };
        case null {};
      };
    };

    /// Post a new status. `handle` is the author's display handle snapshot
    /// (the page reads it from the Identity service). `text` is the status body
    /// (or image ref for image statuses). `color` is the #hex background for
    /// text statuses — empty falls back to the default. Returns the new id.
    public func postStatus(caller : Principal, handle : Text, text : Text, color : Text) : Nat {
      postStatusKind(caller, handle, text, color, #Text);
    };

    /// Post a status of an explicit kind (#Text or #Image). Returns the new id.
    public func postStatusKind(caller : Principal, handle : Text, text : Text, color : Text, kind : Kind) : Nat {
      let id = nextId;
      nextId += 1;
      let now = Time.now();
      let c = if (Text.trim(color, #char ' ') == "") DEFAULT_COLOR else color;
      let s : Update = {
        id;
        author = caller;
        authorHandle = handle;
        at = now;
        text;
        kind;
        color = c;
        expiresAt = now + DAY_NS;
        views = 0;
      };
      statuses.put(id, s);
      id;
    };

    /// All non-expired statuses, newest first. Prunes expired rows lazily.
    public func active() : [Update] {
      sortNewest(liveArray());
    };

    /// Non-expired statuses for one author handle, newest first.
    public func byAuthor(handle : Text) : [Update] {
      let h = handle;
      sortNewest(Array.filter<Update>(liveArray(), func(s : Update) : Bool { s.authorHandle == h }));
    };

    /// Non-expired statuses posted by a principal, newest first.
    public func byPrincipal(caller : Principal) : [Update] {
      sortNewest(Array.filter<Update>(liveArray(), func(s : Update) : Bool { s.author == caller }));
    };

    /// Distinct author handles that currently have at least one live status,
    /// ordered by most-recent activity first (the group/ring rail).
    public func authorsWithActive() : [Text] {
      let live = sortNewest(liveArray()); // newest first
      let seen = Buffer.Buffer<Text>(16);
      for (s in live.vals()) {
        if (not contains(seen, s.authorHandle)) { seen.add(s.authorHandle) };
      };
      Buffer.toArray(seen);
    };

    /// Number of distinct authors with a live status.
    public func authorCount() : Nat { authorsWithActive().size() };

    /// Total number of live (non-expired) statuses.
    public func activeCount() : Nat { liveArray().size() };

    /// Fetch a single status by id if it exists and is still live.
    public func get(id : Nat) : ?Update {
      switch (statuses.get(id)) {
        case (?s) { if (isLive(s)) ?s else null };
        case null null;
      };
    };

    /// Record a view of a status (increments its aggregate view count).
    /// Returns false if the status is missing or already expired.
    public func markViewed(id : Nat) : Bool {
      switch (statuses.get(id)) {
        case (?s) {
          if (not isLive(s)) { return false };
          statuses.put(id, { s with views = s.views + 1 });
          true;
        };
        case null { false };
      };
    };

    /// Delete a status — only the author may remove their own. Returns true if
    /// a row owned by the caller was removed.
    public func delete(caller : Principal, id : Nat) : Bool {
      switch (statuses.get(id)) {
        case (?s) {
          if (s.author != caller) { return false };
          statuses.delete(id);
          true;
        };
        case null { false };
      };
    };

    /// Eagerly drop every expired status; returns the number removed. Reads
    /// already prune lazily, so this is just an optional housekeeping hook.
    public func pruneExpired() : Nat {
      let now = Time.now();
      let dead = Buffer.Buffer<Nat>(16);
      for (s in statuses.vals()) { if (s.expiresAt < now) { dead.add(s.id) } };
      for (id in dead.vals()) { statuses.delete(id) };
      dead.size();
    };

    // ---- Text-returning render helpers (thin .mview pages call these) ----

    /// Human-friendly relative age of a timestamp, e.g. "just now", "5m ago",
    /// "3h ago", "yesterday".
    public func relativeTime(at : Int) : Text {
      let diff = Time.now() - at;
      if (diff < 0) { return "just now" };
      let secs = diff / 1_000_000_000;
      if (secs < 45) { return "just now" };
      let mins = secs / 60;
      if (mins < 1) { return "just now" };
      if (mins < 60) { return Int.toText(mins) # "m ago" };
      let hours = mins / 60;
      if (hours < 24) { return Int.toText(hours) # "h ago" };
      let days = hours / 24;
      if (days == 1) { return "yesterday" };
      Int.toText(days) # "d ago";
    };

    /// Time left before a status expires, e.g. "expires in 7h" / "expiring soon".
    public func timeLeft(expiresAt : Int) : Text {
      let diff = expiresAt - Time.now();
      if (diff <= 0) { return "expired" };
      let mins = diff / 1_000_000_000 / 60;
      if (mins < 60) {
        if (mins <= 1) { return "expiring soon" };
        return "expires in " # Int.toText(mins) # "m";
      };
      let hours = mins / 60;
      "expires in " # Int.toText(hours) # "h";
    };

    /// Label for a status kind ("Text" / "Photo").
    public func kindLabel(kind : Kind) : Text {
      switch (kind) { case (#Text) "Text"; case (#Image) "Photo" };
    };

    // ---- helpers ----

    func isLive(s : Update) : Bool { s.expiresAt >= Time.now() };

    // All currently-live statuses (unsorted).
    func liveArray() : [Update] {
      Array.filter<Update>(Iter.toArray(statuses.vals()), isLive);
    };

    // Sort a status array newest-first by post time.
    func sortNewest(arr : [Update]) : [Update] {
      Array.sort(arr, func(a : Update, b : Update) : { #less; #equal; #greater } {
        if (a.at > b.at) #less else if (a.at < b.at) #greater else #equal
      });
    };

    func contains(buf : Buffer.Buffer<Text>, t : Text) : Bool {
      for (x in buf.vals()) { if (x == t) { return true } };
      false;
    };
  };
};
