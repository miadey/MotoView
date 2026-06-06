/// Messenger service — WhatsApp-style encrypted conversations.
///
/// Stateful service: the MotoView compiler instantiates one shared `Messenger`
/// at actor scope, so every page sees the same conversation store for the
/// canister's lifetime. (A plain `module` cannot hold mutable state — see the
/// framework's stateful-service convention: a service file exporting
/// `public class <Name>()`.)
///
/// IMPORTANT: This service stores CIPHERTEXT ONLY. Each `DmMessage.ciphertext`
/// is a base64 E2EE envelope produced by the client; the canister NEVER
/// decrypts it. Real cryptography (X25519 / AEAD) runs client-side against the
/// public-key bundles served by the Keys service. The on-chain part is purely a
/// key directory plus a ciphertext relay + read-receipt / typing bookkeeping.
import HashMap "mo:base/HashMap";
import Principal "mo:base/Principal";
import Text "mo:base/Text";
import Time "mo:base/Time";
import Array "mo:base/Array";
import Buffer "mo:base/Buffer";
import Nat "mo:base/Nat";
import Int "mo:base/Int";
import Hash "mo:base/Hash";
import Iter "mo:base/Iter";

module {
  /// A 1:1 or group conversation. `members` is the full participant set.
  /// For a `#Direct` conversation it holds exactly the two sorted principals;
  /// for `#Group` it holds the creator plus the invited members.
  
  /// A single message inside a conversation. `ciphertext` is an opaque base64
  /// E2EE envelope — stored verbatim, never decrypted server-side.
  
  public class Messenger() {
    public type Conversation = {
    id : Nat;
    kind : { #Direct; #Group };
    members : [Principal];
    name : Text; // group name; for direct convos a short descriptive label
    createdAt : Int;
    lastAt : Int; // bumped on every send — drives recency sort
  };
    public type DmMessage = {
    id : Nat;
    convoId : Nat;
    sender : Principal;
    senderHandle : Text; // handle snapshot supplied by the page at send time
    at : Int;
    ciphertext : Text; // base64 E2EE envelope — opaque to the canister
    readBy : [Principal];
  };

    var nextConvoId : Nat = 1;
    var nextMsgId : Nat = 1;

    let convos = HashMap.HashMap<Nat, Conversation>(64, Nat.equal, Hash.hash);
    // convoId -> ordered (oldest-first) buffer of messages
    let msgs = HashMap.HashMap<Nat, Buffer.Buffer<DmMessage>>(64, Nat.equal, Hash.hash);
    // sorted-pair key ("p1|p2") -> convoId, for idempotent direct convos
    let directIndex = HashMap.HashMap<Text, Nat>(64, Text.equal, Text.hash);
    // convoId -> typing entries (who + when they last signalled typing)
    let typingState = HashMap.HashMap<Nat, Buffer.Buffer<TypingEntry>>(64, Nat.equal, Hash.hash);

    type TypingEntry = { who : Principal; at : Int };

    // Typing signals older than this (nanoseconds) are considered stale: 6s.
    let TYPING_TTL : Int = 6_000_000_000;

    // ---- conversation creation ------------------------------------------

    /// Start (or reuse) a 1:1 conversation between `caller` and `peer`.
    /// Idempotent: the same unordered pair always maps to the same convo id.
    public func startDirect(caller : Principal, peer : Principal) : Nat {
      let key = pairKey(caller, peer);
      switch (directIndex.get(key)) {
        case (?id) { id };
        case null {
          let id = nextConvoId;
          nextConvoId += 1;
          let now = Time.now();
          let members = sortedPair(caller, peer);
          let convo : Conversation = {
            id;
            kind = #Direct;
            members;
            name = "Direct message";
            createdAt = now;
            lastAt = now;
          };
          convos.put(id, convo);
          msgs.put(id, Buffer.Buffer<DmMessage>(16));
          directIndex.put(key, id);
          id;
        };
      };
    };

    /// Start a group conversation owned by `caller`. The caller is always a
    /// member; duplicate / caller-equal entries in `members` are de-duped.
    public func startGroup(caller : Principal, name : Text, members : [Principal]) : Nat {
      let id = nextConvoId;
      nextConvoId += 1;
      let now = Time.now();
      let set = Buffer.Buffer<Principal>(members.size() + 1);
      set.add(caller);
      for (m in members.vals()) {
        if (not containsPrincipal(set, m)) { set.add(m) };
      };
      let convo : Conversation = {
        id;
        kind = #Group;
        members = Buffer.toArray(set);
        name = if (name == "") "Group" else name;
        createdAt = now;
        lastAt = now;
      };
      convos.put(id, convo);
      msgs.put(id, Buffer.Buffer<DmMessage>(16));
      id;
    };

    // ---- conversation reads ---------------------------------------------

    /// All conversations the caller is a member of, most-recently-active first.
    public func conversations(caller : Principal) : [Conversation] {
      let mine = Buffer.Buffer<Conversation>(16);
      for (c in convos.vals()) {
        if (memberOf(c, caller)) { mine.add(c) };
      };
      Array.sort(
        Buffer.toArray(mine),
        func(a : Conversation, b : Conversation) : { #less; #equal; #greater } {
          if (a.lastAt > b.lastAt) #less else if (a.lastAt < b.lastAt) #greater else #equal;
        },
      );
    };

    public func conversation(id : Nat) : ?Conversation { convos.get(id) };

    /// True iff `principal` is a member of conversation `convoId`.
    public func isMember(convoId : Nat, principal : Principal) : Bool {
      switch (convos.get(convoId)) {
        case (?c) { memberOf(c, principal) };
        case null { false };
      };
    };

    /// Number of conversations the caller participates in.
    public func conversationCount(caller : Principal) : Nat {
      var n : Nat = 0;
      for (c in convos.vals()) { if (memberOf(c, caller)) { n += 1 } };
      n;
    };

    // ---- messages -------------------------------------------------------

    /// Append a ciphertext message from `caller` to `convoId`; bumps `lastAt`.
    /// `senderHandle` is a display snapshot supplied by the page (from Identity).
    /// Returns 0 if the caller is not a member of the conversation.
    public func send(caller : Principal, convoId : Nat, senderHandle : Text, ciphertext : Text) : Nat {
      switch (convos.get(convoId)) {
        case null { 0 };
        case (?c) {
          if (not memberOf(c, caller)) { return 0 };
          let id = nextMsgId;
          nextMsgId += 1;
          let now = Time.now();
          let m : DmMessage = {
            id;
            convoId;
            sender = caller;
            senderHandle;
            at = now;
            ciphertext;
            readBy = [caller]; // sender has implicitly read their own message
          };
          let buf = switch (msgs.get(convoId)) {
            case (?b) b;
            case null {
              let b = Buffer.Buffer<DmMessage>(16);
              msgs.put(convoId, b);
              b;
            };
          };
          buf.add(m);
          convos.put(convoId, { c with lastAt = now });
          // sending clears the sender's typing flag for this convo
          clearTyping(convoId, caller);
          id;
        };
      };
    };

    /// Messages of `convoId` oldest-first, but ONLY if `caller` is a member.
    /// Returns `[]` for non-members (privacy) or unknown convos.
    public func messages(caller : Principal, convoId : Nat) : [DmMessage] {
      if (not isMember(convoId, caller)) { return [] };
      switch (msgs.get(convoId)) {
        case (?b) { Buffer.toArray(b) };
        case null { [] };
      };
    };

    /// Mark every message in `convoId` as read by `caller` (idempotent).
    public func markRead(caller : Principal, convoId : Nat) {
      if (not isMember(convoId, caller)) { return };
      switch (msgs.get(convoId)) {
        case null {};
        case (?b) {
          var i = 0;
          let n = b.size();
          while (i < n) {
            let m = b.get(i);
            if (not containsArr(m.readBy, caller)) {
              b.put(i, { m with readBy = appendPrincipal(m.readBy, caller) });
            };
            i += 1;
          };
        };
      };
    };

    /// How many messages in `convoId` the caller has NOT yet read. Messages the
    /// caller authored never count as unread. Non-members / unknown -> 0.
    public func unreadCount(caller : Principal, convoId : Nat) : Nat {
      if (not isMember(convoId, caller)) { return 0 };
      switch (msgs.get(convoId)) {
        case null { 0 };
        case (?b) {
          var n : Nat = 0;
          for (m in b.vals()) {
            if (m.sender != caller and not containsArr(m.readBy, caller)) { n += 1 };
          };
          n;
        };
      };
    };

    /// Total unread across ALL conversations the caller belongs to (nav badge).
    public func totalUnread(caller : Principal) : Nat {
      var total : Nat = 0;
      for (c in convos.vals()) {
        if (memberOf(c, caller)) { total += unreadCount(caller, c.id) };
      };
      total;
    };

    public func messageCount(convoId : Nat) : Nat {
      switch (msgs.get(convoId)) {
        case (?b) { b.size() };
        case null { 0 };
      };
    };

    /// The most recent message of a conversation, if any (for list previews).
    public func lastMessage(caller : Principal, convoId : Nat) : ?DmMessage {
      if (not isMember(convoId, caller)) { return null };
      switch (msgs.get(convoId)) {
        case (?b) { if (b.size() == 0) null else ?b.get(b.size() - 1) };
        case null { null };
      };
    };

    // ---- typing ---------------------------------------------------------

    /// Signal that `caller` is currently typing in `convoId`. No-op for
    /// non-members. Refreshes the caller's typing timestamp.
    public func typing(caller : Principal, convoId : Nat) {
      if (not isMember(convoId, caller)) { return };
      let now = Time.now();
      let buf = switch (typingState.get(convoId)) {
        case (?b) b;
        case null {
          let b = Buffer.Buffer<TypingEntry>(4);
          typingState.put(convoId, b);
          b;
        };
      };
      var found = false;
      var i = 0;
      let n = buf.size();
      while (i < n) {
        let e = buf.get(i);
        if (e.who == caller) { buf.put(i, { who = caller; at = now }); found := true };
        i += 1;
      };
      if (not found) { buf.add({ who = caller; at = now }) };
    };

    /// Principals currently typing in `convoId`, excluding `exceptPrincipal`
    /// (typically the viewer). Only signals within the typing TTL are returned.
    public func typingIn(convoId : Nat, exceptPrincipal : Principal) : [Principal] {
      switch (typingState.get(convoId)) {
        case null { [] };
        case (?b) {
          let now = Time.now();
          let live = Buffer.Buffer<Principal>(b.size());
          for (e in b.vals()) {
            if (e.who != exceptPrincipal and (now - e.at) <= TYPING_TTL) {
              live.add(e.who);
            };
          };
          Buffer.toArray(live);
        };
      };
    };

    /// True iff anyone other than `exceptPrincipal` is typing in `convoId`.
    public func isTypingIn(convoId : Nat, exceptPrincipal : Principal) : Bool {
      typingIn(convoId, exceptPrincipal).size() > 0;
    };

    // ---- text helpers (for thin .mview rendering) -----------------------

    /// Human-friendly relative time for a past nanosecond timestamp.
    public func relativeTime(at : Int) : Text {
      let now = Time.now();
      let diff = now - at;
      if (diff < 0) { return "just now" };
      let secs = diff / 1_000_000_000;
      if (secs < 5) { return "just now" };
      if (secs < 60) { return Int.toText(secs) # "s ago" };
      let mins = secs / 60;
      if (mins < 60) { return Int.toText(mins) # "m ago" };
      let hrs = mins / 60;
      if (hrs < 24) { return Int.toText(hrs) # "h ago" };
      let days = hrs / 24;
      if (days < 7) { return Int.toText(days) # "d ago" };
      let weeks = days / 7;
      if (weeks < 5) { return Int.toText(weeks) # "w ago" };
      let months = days / 30;
      if (months < 12) { return Int.toText(months) # "mo ago" };
      Int.toText(days / 365) # "y ago";
    };

    /// A display title for a conversation from the caller's perspective. For a
    /// group it is the group name; for a direct convo the page should pass the
    /// peer's handle via `peerHandle` (we have only principals on the record).
    public func titleFor(convoId : Nat, peerHandle : Text) : Text {
      switch (convos.get(convoId)) {
        case null { "Conversation" };
        case (?c) {
          switch (c.kind) {
            case (#Group) { c.name };
            case (#Direct) { if (peerHandle == "") c.name else peerHandle };
          };
        };
      };
    };

    /// The other member of a direct conversation, as seen by `caller`.
    /// Returns `null` for groups, unknown convos, or non-members.
    public func directPeer(caller : Principal, convoId : Nat) : ?Principal {
      switch (convos.get(convoId)) {
        case null { null };
        case (?c) {
          switch (c.kind) {
            case (#Group) { null };
            case (#Direct) {
              if (not memberOf(c, caller)) { return null };
              var peer : ?Principal = null;
              for (m in c.members.vals()) { if (m != caller) { peer := ?m } };
              peer;
            };
          };
        };
      };
    };

    public func memberCount(convoId : Nat) : Nat {
      switch (convos.get(convoId)) {
        case (?c) { c.members.size() };
        case null { 0 };
      };
    };

    // ---- internal helpers ------------------------------------------------

    func memberOf(c : Conversation, p : Principal) : Bool {
      for (m in c.members.vals()) { if (m == p) { return true } };
      false;
    };

    func sortedPair(a : Principal, b : Principal) : [Principal] {
      if (Principal.toText(a) <= Principal.toText(b)) { [a, b] } else { [b, a] };
    };

    func pairKey(a : Principal, b : Principal) : Text {
      let ta = Principal.toText(a);
      let tb = Principal.toText(b);
      if (ta <= tb) { ta # "|" # tb } else { tb # "|" # ta };
    };

    func containsArr(arr : [Principal], p : Principal) : Bool {
      for (x in arr.vals()) { if (x == p) { return true } };
      false;
    };

    func appendPrincipal(arr : [Principal], p : Principal) : [Principal] {
      Array.append(arr, [p]);
    };

    func containsPrincipal(buf : Buffer.Buffer<Principal>, p : Principal) : Bool {
      for (x in buf.vals()) { if (x == p) { return true } };
      false;
    };

    func clearTyping(convoId : Nat, who : Principal) {
      switch (typingState.get(convoId)) {
        case null {};
        case (?b) {
          let kept = Buffer.Buffer<TypingEntry>(b.size());
          for (e in b.vals()) { if (e.who != who) { kept.add(e) } };
          typingState.put(convoId, kept);
        };
      };
    };

    // ---- upgrade-stable persistence -------------------------------------
    // The MotoView compiler detects `mvStableSave` / `mvStableLoad` and wires
    // up the actor-level stable backing plus preupgrade/postupgrade hooks.
    //
    // Snapshot tuple order (must match exactly between save and load):
    //   0: nextConvoId   : Nat
    //   1: nextMsgId     : Nat
    //   2: convos        : [(Nat, Conversation)]
    //   3: msgs          : [(Nat, [DmMessage])]            (Buffer values -> arrays)
    //   4: directIndex   : [(Text, Nat)]
    //   5: typingState   : [(Nat, [TypingEntry])]          (Buffer values -> arrays)

    public func mvStableSave() : Blob {
      to_candid ({
        nextConvoId = nextConvoId;
        nextMsgId = nextMsgId;
        convos = Iter.toArray(convos.entries());
        msgs = Array.map<(Nat, Buffer.Buffer<DmMessage>), (Nat, [DmMessage])>(
          Iter.toArray(msgs.entries()),
          func((k, b) : (Nat, Buffer.Buffer<DmMessage>)) : (Nat, [DmMessage]) {
            (k, Buffer.toArray(b));
          },
        );
        directIndex = Iter.toArray(directIndex.entries());
        typingState = Array.map<(Nat, Buffer.Buffer<TypingEntry>), (Nat, [TypingEntry])>(
          Iter.toArray(typingState.entries()),
          func((k, b) : (Nat, Buffer.Buffer<TypingEntry>)) : (Nat, [TypingEntry]) {
            (k, Buffer.toArray(b));
          },
        );
      });
    };

    public func mvStableLoad(b : Blob) {
      switch (
        from_candid (b) : ?{
          nextConvoId : Nat;
          nextMsgId : Nat;
          convos : [(Nat, Conversation)];
          msgs : [(Nat, [DmMessage])];
          directIndex : [(Text, Nat)];
          typingState : [(Nat, [TypingEntry])];
        }
      ) {
        case (?saved) {
          let savedNextConvoId = saved.nextConvoId;
          let savedNextMsgId = saved.nextMsgId;
          let savedConvos = saved.convos;
          let savedMsgs = saved.msgs;
          let savedDirectIndex = saved.directIndex;
          let savedTypingState = saved.typingState;

          // scalars: replace
          nextConvoId := savedNextConvoId;
          nextMsgId := savedNextMsgId;

          // convos: clear then re-put
          for (k in Iter.toArray(convos.keys()).vals()) { convos.delete(k) };
          for ((k, v) in savedConvos.vals()) { convos.put(k, v) };

          // msgs: clear then re-put (rebuild buffers)
          for (k in Iter.toArray(msgs.keys()).vals()) { msgs.delete(k) };
          for ((k, arr) in savedMsgs.vals()) {
            let buf = Buffer.Buffer<DmMessage>(arr.size());
            for (m in arr.vals()) { buf.add(m) };
            msgs.put(k, buf);
          };

          // directIndex: clear then re-put
          for (k in Iter.toArray(directIndex.keys()).vals()) { directIndex.delete(k) };
          for ((k, v) in savedDirectIndex.vals()) { directIndex.put(k, v) };

          // typingState: clear then re-put (rebuild buffers)
          for (k in Iter.toArray(typingState.keys()).vals()) { typingState.delete(k) };
          for ((k, arr) in savedTypingState.vals()) {
            let buf = Buffer.Buffer<TypingEntry>(arr.size());
            for (e in arr.vals()) { buf.add(e) };
            typingState.put(k, buf);
          };
        };
        case null {};
      };
    };
  };
};
