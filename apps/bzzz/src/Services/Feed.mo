/// Feed service — X-style social feed: posts, follows, likes, reposts, replies.
///
/// Stateful service: the MotoView compiler instantiates one shared `Feed`
/// at actor scope, so every page sees the same feed for the canister's
/// lifetime. (A plain `module` cannot hold mutable state — see the framework's
/// stateful-service convention: a service file exporting `public class <Name>()`.)
///
/// Cross-service coordination happens in PAGES, not service→service calls.
/// Pages resolve author display handles via the Identity service and pass the
/// handle snapshot Text into the mutating methods here; we store the Principal
/// plus that handle snapshot on each record.
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
  /// A single post in the feed. `replyTo` = 0 means a top-level post; otherwise
  /// it is the id of the post being replied to. `likes` / `reposts` /
  /// `replyCount` are aggregate counts derived from the per-user sets.
  
  public class Feed() {
    public type FeedPost = {
    id : Nat;
    author : Principal;
    authorHandle : Text; // snapshot of the author's handle at post time
    at : Int; // Time.now() nanoseconds
    text : Text; // <= 280 chars
    likes : Nat;
    reposts : Nat;
    replyCount : Nat;
    replyTo : Nat; // 0 = top-level post, else id of parent post
  };

    // ---- state ----
    var nextId : Nat = 1;
    let postsById = HashMap.HashMap<Nat, FeedPost>(128, Nat.equal, Hash.hash);

    // follower -> set of followee principals (value Bool always true when present)
    let following = HashMap.HashMap<Principal, HashMap.HashMap<Principal, Bool>>(64, Principal.equal, Principal.hash);

    // postId -> set of principals who liked it
    let likesByPost = HashMap.HashMap<Nat, HashMap.HashMap<Principal, Bool>>(128, Nat.equal, Hash.hash);
    // postId -> set of principals who reposted it
    let repostsByPost = HashMap.HashMap<Nat, HashMap.HashMap<Principal, Bool>>(128, Nat.equal, Hash.hash);

    // ---- posting ----

    /// Create a top-level post. Returns the new post id (0 if rejected: empty
    /// or over the 280-char limit).
    public func post(caller : Principal, handle : Text, text : Text) : Nat {
      createPost(caller, handle, text, 0);
    };

    /// Create a reply to `postId`. Returns the new reply's id (0 if rejected or
    /// the parent post does not exist). Increments the parent's `replyCount`.
    public func reply(caller : Principal, handle : Text, postId : Nat, text : Text) : Nat {
      switch (postsById.get(postId)) {
        case null { 0 };
        case (?parent) {
          let id = createPost(caller, handle, text, postId);
          if (id != 0) {
            postsById.put(parent.id, { parent with replyCount = parent.replyCount + 1 });
          };
          id;
        };
      };
    };

    func createPost(caller : Principal, handle : Text, text : Text, replyTo : Nat) : Nat {
      let body = Text.trim(text, #char ' ');
      if (body == "") { return 0 };
      if (Text.size(body) > 280) { return 0 };
      let id = nextId;
      nextId += 1;
      let p : FeedPost = {
        id;
        author = caller;
        authorHandle = handle;
        at = Time.now();
        text = body;
        likes = 0;
        reposts = 0;
        replyCount = 0;
        replyTo;
      };
      postsById.put(id, p);
      id;
    };

    // ---- reads ----

    /// All top-level posts, newest first.
    public func posts() : [FeedPost] {
      let buf = Buffer.Buffer<FeedPost>(postsById.size());
      for (p in postsById.vals()) { if (p.replyTo == 0) { buf.add(p) } };
      sortDesc(Buffer.toArray(buf));
    };

    /// A single post by id (top-level or reply).
    public func get(id : Nat) : ?FeedPost { postsById.get(id) };

    /// Top-level posts authored by the given handle, newest first.
    public func byAuthor(handle : Text) : [FeedPost] {
      let buf = Buffer.Buffer<FeedPost>(16);
      for (p in postsById.vals()) {
        if (p.replyTo == 0 and p.authorHandle == handle) { buf.add(p) };
      };
      sortDesc(Buffer.toArray(buf));
    };

    /// Replies to a post, oldest first (conversation order).
    public func replies(postId : Nat) : [FeedPost] {
      let buf = Buffer.Buffer<FeedPost>(16);
      for (p in postsById.vals()) { if (p.replyTo == postId) { buf.add(p) } };
      sortAsc(Buffer.toArray(buf));
    };

    /// Total number of top-level posts.
    public func postCount() : Nat {
      var n = 0;
      for (p in postsById.vals()) { if (p.replyTo == 0) { n += 1 } };
      n;
    };

    // ---- follows ----

    /// Toggle following `target`. Returns the new state (true = now following).
    /// Following yourself is a no-op that returns false.
    public func follow(caller : Principal, target : Principal) : Bool {
      if (caller == target) { return false };
      let set = followSet(caller);
      switch (set.get(target)) {
        case (?_) { set.delete(target); false };
        case null { set.put(target, true); true };
      };
    };

    public func isFollowing(caller : Principal, target : Principal) : Bool {
      switch (following.get(caller)) {
        case (?set) { set.get(target) != null };
        case null { false };
      };
    };

    /// How many principals follow `principal`.
    public func followerCount(principal : Principal) : Nat {
      var n = 0;
      for (set in following.vals()) {
        if (set.get(principal) != null) { n += 1 };
      };
      n;
    };

    /// How many principals `principal` follows.
    public func followingCount(principal : Principal) : Nat {
      switch (following.get(principal)) {
        case (?set) { set.size() };
        case null { 0 };
      };
    };

    /// The principals the caller follows.
    public func followingPrincipals(caller : Principal) : [Principal] {
      switch (following.get(caller)) {
        case (?set) { Iter.toArray(set.keys()) };
        case null { [] };
      };
    };

    func followSet(caller : Principal) : HashMap.HashMap<Principal, Bool> {
      switch (following.get(caller)) {
        case (?set) { set };
        case null {
          let set = HashMap.HashMap<Principal, Bool>(8, Principal.equal, Principal.hash);
          following.put(caller, set);
          set;
        };
      };
    };

    // ---- likes ----

    /// Toggle the caller's like on `postId`. Returns the new state
    /// (true = now liked). Keeps the post's aggregate `likes` count in sync.
    public func like(caller : Principal, postId : Nat) : Bool {
      switch (postsById.get(postId)) {
        case null { false };
        case (?p) {
          let set = likeSet(postId);
          let nowLiked = switch (set.get(caller)) {
            case (?_) { set.delete(caller); false };
            case null { set.put(caller, true); true };
          };
          postsById.put(postId, { p with likes = set.size() });
          nowLiked;
        };
      };
    };

    public func hasLiked(caller : Principal, postId : Nat) : Bool {
      switch (likesByPost.get(postId)) {
        case (?set) { set.get(caller) != null };
        case null { false };
      };
    };

    public func likeCount(postId : Nat) : Nat {
      switch (likesByPost.get(postId)) {
        case (?set) { set.size() };
        case null { 0 };
      };
    };

    func likeSet(postId : Nat) : HashMap.HashMap<Principal, Bool> {
      switch (likesByPost.get(postId)) {
        case (?set) { set };
        case null {
          let set = HashMap.HashMap<Principal, Bool>(8, Principal.equal, Principal.hash);
          likesByPost.put(postId, set);
          set;
        };
      };
    };

    // ---- reposts ----

    /// Toggle the caller's repost of `postId`. Returns the new state
    /// (true = now reposted). Keeps the post's aggregate `reposts` count in sync.
    public func repost(caller : Principal, postId : Nat) : Bool {
      switch (postsById.get(postId)) {
        case null { false };
        case (?p) {
          let set = repostSet(postId);
          let nowReposted = switch (set.get(caller)) {
            case (?_) { set.delete(caller); false };
            case null { set.put(caller, true); true };
          };
          postsById.put(postId, { p with reposts = set.size() });
          nowReposted;
        };
      };
    };

    public func hasReposted(caller : Principal, postId : Nat) : Bool {
      switch (repostsByPost.get(postId)) {
        case (?set) { set.get(caller) != null };
        case null { false };
      };
    };

    public func repostCount(postId : Nat) : Nat {
      switch (repostsByPost.get(postId)) {
        case (?set) { set.size() };
        case null { 0 };
      };
    };

    func repostSet(postId : Nat) : HashMap.HashMap<Principal, Bool> {
      switch (repostsByPost.get(postId)) {
        case (?set) { set };
        case null {
          let set = HashMap.HashMap<Principal, Bool>(8, Principal.equal, Principal.hash);
          repostsByPost.put(postId, set);
          set;
        };
      };
    };

    // ---- timeline ----

    /// Home timeline for `caller`: top-level posts authored by the caller or by
    /// any principal the caller follows, newest first. Falls back to ALL
    /// top-level posts when the caller follows nobody (so the feed never looks
    /// empty for a new user).
    public func homeTimeline(caller : Principal) : [FeedPost] {
      let follows = switch (following.get(caller)) {
        case (?set) { set };
        case null { HashMap.HashMap<Principal, Bool>(0, Principal.equal, Principal.hash) };
      };
      if (follows.size() == 0) { return posts() };
      let buf = Buffer.Buffer<FeedPost>(postsById.size());
      for (p in postsById.vals()) {
        if (p.replyTo == 0 and (p.author == caller or follows.get(p.author) != null)) {
          buf.add(p);
        };
      };
      sortDesc(Buffer.toArray(buf));
    };

    // ---- display helpers ----

    /// A coarse "x ago" string for a `Time.now()` timestamp, for thin pages.
    public func relativeTime(at : Int) : Text {
      let nowNs = Time.now();
      var deltaSec = (nowNs - at) / 1_000_000_000;
      if (deltaSec < 0) { deltaSec := 0 };
      if (deltaSec < 60) { return "now" };
      let mins = deltaSec / 60;
      if (mins < 60) { return Int.toText(mins) # "m" };
      let hours = mins / 60;
      if (hours < 24) { return Int.toText(hours) # "h" };
      let days = hours / 24;
      if (days < 7) { return Int.toText(days) # "d" };
      let weeks = days / 7;
      if (weeks < 52) { return Int.toText(weeks) # "w" };
      Int.toText(days / 365) # "y";
    };

    // ---- sorting helpers ----

    func sortDesc(arr : [FeedPost]) : [FeedPost] {
      Array.sort(arr, func(a : FeedPost, b : FeedPost) : { #less; #equal; #greater } {
        if (a.at > b.at) #less else if (a.at < b.at) #greater else #equal;
      });
    };

    func sortAsc(arr : [FeedPost]) : [FeedPost] {
      Array.sort(arr, func(a : FeedPost, b : FeedPost) : { #less; #equal; #greater } {
        if (a.at < b.at) #less else if (a.at > b.at) #greater else #equal;
      });
    };

    // ---- upgrade-stable persistence ----
    // MotoView detects mvStableSave/mvStableLoad and generates the actor-level
    // stable backing plus preupgrade/postupgrade hooks. Nested HashMaps are
    // flattened to arrays of key/value pairs (HashMap is not directly shareable).

    public func mvStableSave() : Blob {
      let followingEntries = Array.map<(Principal, HashMap.HashMap<Principal, Bool>), (Principal, [(Principal, Bool)])>(
        Iter.toArray(following.entries()),
        func((k, set) : (Principal, HashMap.HashMap<Principal, Bool>)) : (Principal, [(Principal, Bool)]) {
          (k, Iter.toArray(set.entries()));
        },
      );
      let likesEntries = Array.map<(Nat, HashMap.HashMap<Principal, Bool>), (Nat, [(Principal, Bool)])>(
        Iter.toArray(likesByPost.entries()),
        func((k, set) : (Nat, HashMap.HashMap<Principal, Bool>)) : (Nat, [(Principal, Bool)]) {
          (k, Iter.toArray(set.entries()));
        },
      );
      let repostsEntries = Array.map<(Nat, HashMap.HashMap<Principal, Bool>), (Nat, [(Principal, Bool)])>(
        Iter.toArray(repostsByPost.entries()),
        func((k, set) : (Nat, HashMap.HashMap<Principal, Bool>)) : (Nat, [(Principal, Bool)]) {
          (k, Iter.toArray(set.entries()));
        },
      );
      to_candid ((
        nextId,
        Iter.toArray(postsById.entries()),
        followingEntries,
        likesEntries,
        repostsEntries,
      ));
    };

    public func mvStableLoad(b : Blob) {
      switch (
        from_candid (b) : ?(
          Nat,
          [(Nat, FeedPost)],
          [(Principal, [(Principal, Bool)])],
          [(Nat, [(Principal, Bool)])],
          [(Nat, [(Principal, Bool)])],
        )
      ) {
        case (?(savedNextId, savedPosts, savedFollowing, savedLikes, savedReposts)) {
          // scalar
          nextId := savedNextId;
          // postsById
          for (k in Iter.toArray(postsById.keys()).vals()) { postsById.delete(k) };
          for ((k, v) in savedPosts.vals()) { postsById.put(k, v) };
          // following (nested)
          for (k in Iter.toArray(following.keys()).vals()) { following.delete(k) };
          for ((k, inner) in savedFollowing.vals()) {
            let set = HashMap.HashMap<Principal, Bool>(8, Principal.equal, Principal.hash);
            for ((ik, iv) in inner.vals()) { set.put(ik, iv) };
            following.put(k, set);
          };
          // likesByPost (nested)
          for (k in Iter.toArray(likesByPost.keys()).vals()) { likesByPost.delete(k) };
          for ((k, inner) in savedLikes.vals()) {
            let set = HashMap.HashMap<Principal, Bool>(8, Principal.equal, Principal.hash);
            for ((ik, iv) in inner.vals()) { set.put(ik, iv) };
            likesByPost.put(k, set);
          };
          // repostsByPost (nested)
          for (k in Iter.toArray(repostsByPost.keys()).vals()) { repostsByPost.delete(k) };
          for ((k, inner) in savedReposts.vals()) {
            let set = HashMap.HashMap<Principal, Bool>(8, Principal.equal, Principal.hash);
            for ((ik, iv) in inner.vals()) { set.put(ik, iv) };
            repostsByPost.put(k, set);
          };
        };
        case null {};
      };
    };
  };
};
