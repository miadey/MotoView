/// Chat service — Discord-style rooms/channels + messages, reactions, threads,
/// and typing/presence for the Pulse super-app.
///
/// Stateful service: the MotoView compiler instantiates ONE shared `Chat` at
/// actor scope, so every page sees the same rooms/messages for the canister's
/// lifetime. (A plain `module` cannot hold mutable state — see the framework's
/// stateful-service convention: a service file exporting `public class <Name>()`.)
///
/// Servers/guilds, roles, and moderation live in a SEPARATE `Servers` service.
/// This file owns only rooms, messages, reactions, threads, and presence.
/// Cross-service coordination (e.g. resolving author display names via the
/// Identity service) happens in PAGES — author display handles are passed in
/// as a `handle` Text snapshot on mutating methods.
import HashMap "mo:base/HashMap";
import Principal "mo:base/Principal";
import Text "mo:base/Text";
import Time "mo:base/Time";
import Array "mo:base/Array";
import Buffer "mo:base/Buffer";
import Char "mo:base/Char";
import Nat "mo:base/Nat";
import Int "mo:base/Int";
import Hash "mo:base/Hash";
import Iter "mo:base/Iter";

module {
  // ---- public types ----

  /// A chat room / channel. `serverId == 0` is the default (no-guild) server.
  
  /// A chat message. Delete is a tombstone: text blanked, `deleted` set to now.
  
  /// One entry in a message's thread (lightweight inline reply).
  
  public class Chat() {
    public type Room = {
    id : Nat;
    name : Text; // sanitized lowercase a-z0-9-_, <= 20 chars
    createdAt : Int;
    serverId : Nat;
  };
    public type Message = {
    id : Nat;
    roomId : Nat;
    at : Int;
    author : Principal;
    authorHandle : Text; // handle snapshot supplied by the page (Identity service)
    text : Text; // <= 280 chars
    replyTo : Nat; // 0 = none
    edited : Int; // 0 = not edited, else Time.now() of last edit
    deleted : Int; // 0 = live, else Time.now() of deletion (tombstone)
  };
    public type ThreadMsg = {
    author : Principal;
    handle : Text;
    at : Int;
    text : Text;
  };

    // ---- state ----
    var nextRoomId : Nat = 1;
    var nextMsgId : Nat = 1;

    let rooms = HashMap.HashMap<Nat, Room>(32, Nat.equal, Hash.hash);
    // messages stored per-room, oldest -> newest, FIFO-capped at MAX_PER_ROOM.
    let roomMsgs = HashMap.HashMap<Nat, Buffer.Buffer<Message>>(32, Nat.equal, Hash.hash);
    // fast id -> message lookup for edit/delete/react/thread.
    let msgIndex = HashMap.HashMap<Nat, Message>(256, Nat.equal, Hash.hash);
    // per message id: ordered reaction tallies (emoji -> count).
    let reacts = HashMap.HashMap<Nat, Buffer.Buffer<(Text, Nat)>>(64, Nat.equal, Hash.hash);
    // per parent message id: thread entries (oldest -> newest).
    let threads = HashMap.HashMap<Nat, Buffer.Buffer<ThreadMsg>>(64, Nat.equal, Hash.hash);
    // presence: principal -> last activity time.
    let lastSeen = HashMap.HashMap<Principal, Int>(64, Principal.equal, Principal.hash);
    // typing: per room id -> (handle -> last ping time).
    let typingByRoom = HashMap.HashMap<Nat, HashMap.HashMap<Text, Int>>(32, Nat.equal, Hash.hash);
    // recent room members: per room id -> (handle -> last activity time).
    let membersByRoom = HashMap.HashMap<Nat, HashMap.HashMap<Text, Int>>(32, Nat.equal, Hash.hash);

    let MAX_PER_ROOM : Nat = 500;
    let MAX_TEXT : Nat = 280;
    let MAX_NAME : Nat = 20;
    let ONLINE_WINDOW : Int = 120_000_000_000; // 2 min in ns -> "online"
    let TYPING_WINDOW : Int = 6_000_000_000; // 6 s in ns -> "currently typing"
    let allowedEmojis : [Text] = ["\u{1F44D}", "\u{2764}", "\u{1F600}", "\u{1F389}", "\u{1F525}", "\u{1F32F}"];
    // 👍 ❤️ 😀 🎉 🔥 🌯

    // ---- rooms ----

    /// Create a room in `serverId`. Name is sanitized to lowercase a-z0-9-_ and
    /// capped at 20 chars. Returns the new room id (or 0 if the name is empty
    /// after sanitizing).
    public func createRoom(name : Text, serverId : Nat) : Nat {
      let clean = sanitizeName(name);
      if (clean == "") { return 0 };
      let id = nextRoomId;
      nextRoomId += 1;
      rooms.put(id, { id; name = clean; createdAt = Time.now(); serverId });
      roomMsgs.put(id, Buffer.Buffer<Message>(64));
      id;
    };

    /// All rooms in a server, oldest -> newest (by createdAt then id).
    public func listRooms(serverId : Nat) : [Room] {
      let out = Buffer.Buffer<Room>(16);
      for (r in rooms.vals()) { if (r.serverId == serverId) { out.add(r) } };
      Array.sort(
        Buffer.toArray(out),
        func(a : Room, b : Room) : { #less; #equal; #greater } {
          if (a.id < b.id) #less else if (a.id > b.id) #greater else #equal;
        },
      );
    };

    /// Look up a single room by id.
    public func room(id : Nat) : ?Room { rooms.get(id) };

    /// Total room count (all servers).
    public func roomCount() : Nat { rooms.size() };

    // ---- messages ----

    /// Post a message to a room. `handle` is the author's display handle
    /// snapshot (resolved by the page via the Identity service). `replyTo` is a
    /// message id or 0 for none. Text is trimmed and capped at 280 chars.
    /// Returns the new message id (or 0 if the room is missing or text empty).
    public func post(caller : Principal, handle : Text, roomId : Nat, text : Text, replyTo : Nat) : Nat {
      switch (rooms.get(roomId)) { case null { return 0 }; case (?_) {} };
      let body = clampText(text);
      if (body == "") { return 0 };
      let id = nextMsgId;
      nextMsgId += 1;
      let now = Time.now();
      let m : Message = {
        id;
        roomId;
        at = now;
        author = caller;
        authorHandle = handle;
        text = body;
        replyTo;
        edited = 0;
        deleted = 0;
      };
      let buf = bufFor(roomId);
      buf.add(m);
      // FIFO cap: drop oldest while over capacity (also unindex them).
      while (buf.size() > MAX_PER_ROOM) {
        let dropped = buf.remove(0);
        msgIndex.delete(dropped.id);
      };
      msgIndex.put(id, m);
      touch(caller, handle, roomId, now, false);
      id;
    };

    /// Edit a message's text. Only the original author may edit, and only a live
    /// (non-tombstoned) message. Returns true on success.
    public func edit(caller : Principal, msgId : Nat, text : Text) : Bool {
      switch (msgIndex.get(msgId)) {
        case null { false };
        case (?m) {
          if (m.author != caller or m.deleted != 0) { return false };
          let body = clampText(text);
          if (body == "") { return false };
          let updated = { m with text = body; edited = Time.now() };
          replaceMessage(updated);
          true;
        };
      };
    };

    /// Delete (tombstone) a message: blanks the text and sets `deleted` to now.
    /// The row is kept so reply/thread references stay valid. Only the author.
    public func del(caller : Principal, msgId : Nat) : Bool {
      switch (msgIndex.get(msgId)) {
        case null { false };
        case (?m) {
          if (m.author != caller) { return false };
          if (m.deleted != 0) { return true };
          let tomb = { m with text = ""; deleted = Time.now() };
          replaceMessage(tomb);
          true;
        };
      };
    };

    /// All messages in a room (live messages AND tombstones), oldest -> newest,
    /// capped at ~500.
    public func messages(roomId : Nat) : [Message] {
      switch (roomMsgs.get(roomId)) {
        case null { [] };
        case (?buf) { Buffer.toArray(buf) };
      };
    };

    /// Look up a single message by id (returns tombstones too).
    public func message(msgId : Nat) : ?Message { msgIndex.get(msgId) };

    /// The most recent LIVE message in a room (tombstones skipped), or null for
    /// an empty/unknown room. Cheap: walks the per-room buffer from the end —
    /// for "latest activity" summaries (e.g. the dashboard) without paging the
    /// whole room.
    public func lastMessageIn(roomId : Nat) : ?Message {
      switch (roomMsgs.get(roomId)) {
        case null { null };
        case (?buf) {
          var i = buf.size();
          while (i > 0) {
            i -= 1;
            let m = buf.get(i);
            if (m.deleted == 0) { return ?m };
          };
          null;
        };
      };
    };

    /// Number of live (non-deleted) messages in a room.
    public func messageCount(roomId : Nat) : Nat {
      switch (roomMsgs.get(roomId)) {
        case null { 0 };
        case (?buf) {
          var n : Nat = 0;
          for (m in buf.vals()) { if (m.deleted == 0) { n += 1 } };
          n;
        };
      };
    };

    // ---- reactions ----

    /// Add one to the tally for `emoji` on a message. The emoji must be in the
    /// allowed set (👍 ❤️ 😀 🎉 🔥 🌯). Returns true on success.
    public func react(caller : Principal, msgId : Nat, emoji : Text) : Bool {
      ignore caller;
      if (not isAllowedEmoji(emoji)) { return false };
      switch (msgIndex.get(msgId)) { case null { return false }; case (?_) {} };
      let buf = switch (reacts.get(msgId)) {
        case (?b) b;
        case null {
          let b = Buffer.Buffer<(Text, Nat)>(4);
          reacts.put(msgId, b);
          b;
        };
      };
      var found = false;
      var i : Nat = 0;
      while (i < buf.size()) {
        let (e, c) = buf.get(i);
        if (e == emoji) {
          buf.put(i, (e, c + 1));
          found := true;
        };
        i += 1;
      };
      if (not found) { buf.add((emoji, 1)) };
      true;
    };

    /// Aggregate reaction tallies for a message, in first-reacted order.
    public func reactions(msgId : Nat) : [(Text, Nat)] {
      switch (reacts.get(msgId)) {
        case null { [] };
        case (?buf) { Buffer.toArray(buf) };
      };
    };

    // ---- threads ----

    /// Add a reply to a message's thread. `handle` is the author handle snapshot.
    /// Text is trimmed/capped at 280. Returns true on success.
    public func replyThread(caller : Principal, handle : Text, parentMsgId : Nat, text : Text) : Bool {
      switch (msgIndex.get(parentMsgId)) { case null { return false }; case (?_) {} };
      let body = clampText(text);
      if (body == "") { return false };
      let buf = switch (threads.get(parentMsgId)) {
        case (?b) b;
        case null {
          let b = Buffer.Buffer<ThreadMsg>(8);
          threads.put(parentMsgId, b);
          b;
        };
      };
      buf.add({ author = caller; handle; at = Time.now(); text = body });
      true;
    };

    /// All thread replies for a parent message, oldest -> newest.
    public func thread(parentMsgId : Nat) : [ThreadMsg] {
      switch (threads.get(parentMsgId)) {
        case null { [] };
        case (?buf) { Buffer.toArray(buf) };
      };
    };

    /// Number of thread replies on a parent message.
    public func threadCount(parentMsgId : Nat) : Nat {
      switch (threads.get(parentMsgId)) {
        case null { 0 };
        case (?buf) { buf.size() };
      };
    };

    // ---- typing / presence ----

    /// Record that `caller` (display `handle`) is active in `roomId` and is
    /// currently typing. Updates presence + room membership + typing state.
    public func ping(caller : Principal, handle : Text, roomId : Nat) : () {
      touch(caller, handle, roomId, Time.now(), true);
    };

    /// Handles currently typing in a room within the typing window, excluding
    /// `exceptHandle` (pass the caller's own handle to skip yourself).
    public func typingIn(roomId : Nat, exceptHandle : Text) : [Text] {
      let now = Time.now();
      let out = Buffer.Buffer<Text>(8);
      switch (typingByRoom.get(roomId)) {
        case null {};
        case (?m) {
          for ((h, t) in m.entries()) {
            if (h != exceptHandle and (now - t) <= TYPING_WINDOW) { out.add(h) };
          };
        };
      };
      sortedText(out);
    };

    /// Count of distinct principals seen active within the online window.
    public func onlineCount() : Nat {
      let now = Time.now();
      var n : Nat = 0;
      for (t in lastSeen.vals()) { if ((now - t) <= ONLINE_WINDOW) { n += 1 } };
      n;
    };

    /// Recently-active member handles of a room (within the online window),
    /// alphabetical.
    public func roomMembers(roomId : Nat) : [Text] {
      let now = Time.now();
      let out = Buffer.Buffer<Text>(16);
      switch (membersByRoom.get(roomId)) {
        case null {};
        case (?m) {
          for ((h, t) in m.entries()) {
            if ((now - t) <= ONLINE_WINDOW) { out.add(h) };
          };
        };
      };
      sortedText(out);
    };

    /// Count of recently-active members in a room.
    public func roomMemberCount(roomId : Nat) : Nat { roomMembers(roomId).size() };

    // ---- text helpers (exposed for thin pages) ----

    /// Human-friendly relative time from a past `Int` timestamp (ns) to now,
    /// e.g. "now", "5m", "3h", "2d".
    public func relativeTime(at : Int) : Text {
      let diff = Time.now() - at;
      if (diff < 0) { return "now" };
      let secs = diff / 1_000_000_000;
      if (secs < 5) { return "now" };
      if (secs < 60) { return Int.toText(secs) # "s" };
      let mins = secs / 60;
      if (mins < 60) { return Int.toText(mins) # "m" };
      let hrs = mins / 60;
      if (hrs < 24) { return Int.toText(hrs) # "h" };
      let days = hrs / 24;
      if (days < 7) { return Int.toText(days) # "d" };
      let weeks = days / 7;
      if (weeks < 5) { return Int.toText(weeks) # "w" };
      Int.toText(days / 30) # "mo";
    };

    /// The allowed reaction emoji set, for rendering a reaction picker.
    public func emojiSet() : [Text] { allowedEmojis };

    // ---- private helpers ----

    func bufFor(roomId : Nat) : Buffer.Buffer<Message> {
      switch (roomMsgs.get(roomId)) {
        case (?b) b;
        case null {
          let b = Buffer.Buffer<Message>(64);
          roomMsgs.put(roomId, b);
          b;
        };
      };
    };

    // Replace a message in both the per-room buffer and the id index.
    func replaceMessage(updated : Message) {
      msgIndex.put(updated.id, updated);
      switch (roomMsgs.get(updated.roomId)) {
        case null {};
        case (?buf) {
          var i : Nat = 0;
          while (i < buf.size()) {
            if (buf.get(i).id == updated.id) { buf.put(i, updated) };
            i += 1;
          };
        };
      };
    };

    // Record presence + room membership, and optionally mark typing.
    func touch(caller : Principal, handle : Text, roomId : Nat, now : Int, typing : Bool) {
      lastSeen.put(caller, now);
      // room membership (recent active handles)
      let mem = switch (membersByRoom.get(roomId)) {
        case (?m) m;
        case null {
          let m = HashMap.HashMap<Text, Int>(16, Text.equal, Text.hash);
          membersByRoom.put(roomId, m);
          m;
        };
      };
      mem.put(handle, now);
      // typing
      if (typing) {
        let typ = switch (typingByRoom.get(roomId)) {
          case (?m) m;
          case null {
            let m = HashMap.HashMap<Text, Int>(16, Text.equal, Text.hash);
            typingByRoom.put(roomId, m);
            m;
          };
        };
        typ.put(handle, now);
      };
    };

    func isAllowedEmoji(e : Text) : Bool {
      for (a in allowedEmojis.vals()) { if (a == e) { return true } };
      false;
    };

    // Trim leading/trailing spaces and cap text length at MAX_TEXT chars.
    func clampText(t : Text) : Text {
      let trimmed = Text.trim(t, #char ' ');
      let chars = Text.toArray(trimmed);
      if (chars.size() <= MAX_TEXT) { return trimmed };
      Text.fromIter(Array.subArray(chars, 0, MAX_TEXT).vals());
    };

    // Lowercase, keep only a-z 0-9 - _, cap at MAX_NAME chars.
    func sanitizeName(name : Text) : Text {
      let lowered = Text.map(
        Text.trim(name, #char ' '),
        func(c : Char) : Char {
          if (c >= 'A' and c <= 'Z') { Char.fromNat32(Char.toNat32(c) + 32) } else { c };
        },
      );
      let kept = Buffer.Buffer<Char>(MAX_NAME);
      for (c in lowered.chars()) {
        let ok = (c >= 'a' and c <= 'z') or (c >= '0' and c <= '9') or c == '-' or c == '_';
        if (ok and kept.size() < MAX_NAME) { kept.add(c) };
      };
      Text.fromIter(kept.vals());
    };

    func sortedText(buf : Buffer.Buffer<Text>) : [Text] {
      Array.sort(Buffer.toArray(buf), Text.compare);
    };

    // ---- upgrade-stable persistence ----
    // The MotoView compiler detects `mvStableSave`/`mvStableLoad` and generates
    // the actor-level stable backing plus preupgrade/postupgrade hooks. All
    // serialized types are shared (no functions), so to_candid/from_candid work.
    // Nested Buffers are flattened to arrays; nested HashMaps to [(key, value)]
    // arrays, in a fixed order matched exactly by the from_candid annotation.
    public func mvStableSave() : Blob {
      to_candid ({
        nextRoomId = nextRoomId;
        nextMsgId = nextMsgId;
        rooms = Iter.toArray(rooms.entries());
        roomMsgs = Array.map<(Nat, Buffer.Buffer<Message>), (Nat, [Message])>(
          Iter.toArray(roomMsgs.entries()),
          func((k, b)) { (k, Buffer.toArray(b)) },
        );
        msgIndex = Iter.toArray(msgIndex.entries());
        reacts = Array.map<(Nat, Buffer.Buffer<(Text, Nat)>), (Nat, [(Text, Nat)])>(
          Iter.toArray(reacts.entries()),
          func((k, b)) { (k, Buffer.toArray(b)) },
        );
        threads = Array.map<(Nat, Buffer.Buffer<ThreadMsg>), (Nat, [ThreadMsg])>(
          Iter.toArray(threads.entries()),
          func((k, b)) { (k, Buffer.toArray(b)) },
        );
        lastSeen = Iter.toArray(lastSeen.entries());
        typingByRoom = Array.map<(Nat, HashMap.HashMap<Text, Int>), (Nat, [(Text, Int)])>(
          Iter.toArray(typingByRoom.entries()),
          func((k, m)) { (k, Iter.toArray(m.entries())) },
        );
        membersByRoom = Array.map<(Nat, HashMap.HashMap<Text, Int>), (Nat, [(Text, Int)])>(
          Iter.toArray(membersByRoom.entries()),
          func((k, m)) { (k, Iter.toArray(m.entries())) },
        );
      });
    };
    public func mvStableLoad(b : Blob) {
      switch (
        from_candid (b) : ?{
          nextRoomId : Nat;
          nextMsgId : Nat;
          rooms : [(Nat, Room)];
          roomMsgs : [(Nat, [Message])];
          msgIndex : [(Nat, Message)];
          reacts : [(Nat, [(Text, Nat)])];
          threads : [(Nat, [ThreadMsg])];
          lastSeen : [(Principal, Int)];
          typingByRoom : [(Nat, [(Text, Int)])];
          membersByRoom : [(Nat, [(Text, Int)])];
        }
      ) {
        case (?saved) {
          let savedNextRoomId = saved.nextRoomId;
          let savedNextMsgId = saved.nextMsgId;
          let savedRooms = saved.rooms;
          let savedRoomMsgs = saved.roomMsgs;
          let savedMsgIndex = saved.msgIndex;
          let savedReacts = saved.reacts;
          let savedThreads = saved.threads;
          let savedLastSeen = saved.lastSeen;
          let savedTypingByRoom = saved.typingByRoom;
          let savedMembersByRoom = saved.membersByRoom;
          // scalars
          nextRoomId := savedNextRoomId;
          nextMsgId := savedNextMsgId;
          // rooms
          for (k in Iter.toArray(rooms.keys()).vals()) { rooms.delete(k) };
          for ((k, v) in savedRooms.vals()) { rooms.put(k, v) };
          // roomMsgs (Buffer values)
          for (k in Iter.toArray(roomMsgs.keys()).vals()) { roomMsgs.delete(k) };
          for ((k, arr) in savedRoomMsgs.vals()) {
            let buf = Buffer.Buffer<Message>(arr.size());
            for (x in arr.vals()) { buf.add(x) };
            roomMsgs.put(k, buf);
          };
          // msgIndex
          for (k in Iter.toArray(msgIndex.keys()).vals()) { msgIndex.delete(k) };
          for ((k, v) in savedMsgIndex.vals()) { msgIndex.put(k, v) };
          // reacts (Buffer values)
          for (k in Iter.toArray(reacts.keys()).vals()) { reacts.delete(k) };
          for ((k, arr) in savedReacts.vals()) {
            let buf = Buffer.Buffer<(Text, Nat)>(arr.size());
            for (x in arr.vals()) { buf.add(x) };
            reacts.put(k, buf);
          };
          // threads (Buffer values)
          for (k in Iter.toArray(threads.keys()).vals()) { threads.delete(k) };
          for ((k, arr) in savedThreads.vals()) {
            let buf = Buffer.Buffer<ThreadMsg>(arr.size());
            for (x in arr.vals()) { buf.add(x) };
            threads.put(k, buf);
          };
          // lastSeen
          for (k in Iter.toArray(lastSeen.keys()).vals()) { lastSeen.delete(k) };
          for ((k, v) in savedLastSeen.vals()) { lastSeen.put(k, v) };
          // typingByRoom (nested HashMap values)
          for (k in Iter.toArray(typingByRoom.keys()).vals()) { typingByRoom.delete(k) };
          for ((k, entries) in savedTypingByRoom.vals()) {
            let inner = HashMap.HashMap<Text, Int>(16, Text.equal, Text.hash);
            for ((h, t) in entries.vals()) { inner.put(h, t) };
            typingByRoom.put(k, inner);
          };
          // membersByRoom (nested HashMap values)
          for (k in Iter.toArray(membersByRoom.keys()).vals()) { membersByRoom.delete(k) };
          for ((k, entries) in savedMembersByRoom.vals()) {
            let inner = HashMap.HashMap<Text, Int>(16, Text.equal, Text.hash);
            for ((h, t) in entries.vals()) { inner.put(h, t) };
            membersByRoom.put(k, inner);
          };
        };
        case null {};
      };
    };

    // ---- seed ----
    // A couple of starter rooms in the default server (0) so the UI feels alive.
    // No fabricated user metrics or messages — just empty channels to post into.
    // Runs after all helpers/methods are defined (Motoko evaluates the class
    // body top-to-bottom, so seeding must come last).
    ignore createRoom("general", 0);
    ignore createRoom("random", 0);
  };
};
