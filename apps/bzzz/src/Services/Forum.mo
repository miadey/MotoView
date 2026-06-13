/// Forum service — Discourse-style categories, topics, posts and likes.
///
/// Stateful service: the MotoView compiler instantiates ONE shared `Forum`
/// at actor scope, so every page sees the same forum for the canister's
/// lifetime. (A plain `module` cannot hold mutable state — see the framework's
/// stateful-service convention: a service file exporting `public class <Name>()`.)
///
/// All domain types are defined here (self-contained). Pages reference them as
/// `Forum.Category`, `Forum.Topic`, `Forum.Post`. Mutating methods take the
/// caller's `Principal` first (the page passes `ctx.caller`) plus a `handle`
/// snapshot Text resolved from the Identity service by the page.
import HashMap "mo:base/HashMap";
import Principal "mo:base/Principal";
import Text "mo:base/Text";
import Time "mo:base/Time";
import Array "mo:base/Array";
import Buffer "mo:base/Buffer";
import Nat "mo:base/Nat";
import Int "mo:base/Int";
import Hash "mo:base/Hash";
import Char "mo:base/Char";
import Iter "mo:base/Iter";

module {

  
  
  
  // Internal mutable topic record (the public Topic is a snapshot with derived
  // counts; we keep mutable fields here and project to Topic on read).
  type TopicRec = {
    id : Nat;
    categoryId : Nat;
    title : Text;
    slug : Text;
    author : Principal;
    authorHandle : Text;
    createdAt : Int;
    var bumpedAt : Int;
    tags : [Text];
    var pinned : Bool;
    var closed : Bool;
    var acceptedPostId : Nat;
    var views : Nat;
  };

  type PostRec = {
    id : Nat;
    topicId : Nat;
    author : Principal;
    authorHandle : Text;
    at : Int;
    var body : Text;
    replyToPost : Nat;
    likers : Buffer.Buffer<Principal>;
  };

  type CategoryRec = {
    id : Nat;
    name : Text;
    slug : Text;
    description : Text;
    color : Text;
  };

  public class Forum() {
    public type Category = {
    id : Nat;
    name : Text;
    slug : Text;
    description : Text;
    color : Text; // #hex
    topicCount : Nat; // derived
    postCount : Nat; // derived
  };
    public type Topic = {
    id : Nat;
    categoryId : Nat;
    title : Text;
    slug : Text;
    author : Principal;
    authorHandle : Text;
    createdAt : Int;
    bumpedAt : Int;
    tags : [Text];
    pinned : Bool;
    closed : Bool;
    acceptedPostId : Nat; // 0 = none
    views : Nat;
  };
    public type Post = {
    id : Nat;
    topicId : Nat;
    author : Principal;
    authorHandle : Text;
    at : Int;
    body : Text;
    replyToPost : Nat; // 0 = none
    likeCount : Nat;
  };


    var nextTopicId : Nat = 1;
    var nextPostId : Nat = 1;

    let categories_ = Buffer.Buffer<CategoryRec>(12);
    let topics_ = HashMap.HashMap<Nat, TopicRec>(64, Nat.equal, Hash.hash);
    let posts_ = HashMap.HashMap<Nat, PostRec>(256, Nat.equal, Hash.hash);

    // O(1) counters maintained incrementally on mutation, so reply/post/score
    // reads are O(1) instead of full posts_ scans per topic row (the audit's
    // top scale blocker: O(topics × posts) per forum render). Derived from
    // posts_, so NOT persisted — rebuilt once from posts_ after an upgrade.
    let postCountByTopic = HashMap.HashMap<Nat, Nat>(64, Nat.equal, Hash.hash);
    let likeCountByTopic = HashMap.HashMap<Nat, Nat>(64, Nat.equal, Hash.hash);
    // Per-topic post-id index in ascending id order (== OP first, then replies
    // chronologically). Lets `posts`/`postsPage` fetch one topic's posts WITHOUT
    // scanning the whole posts_ store (the audit's ForumTopic blocker:
    // O(all posts) per topic-page load). Derived from posts_, NOT persisted —
    // rebuilt once from posts_ after an upgrade alongside the counters.
    let postsByTopic = HashMap.HashMap<Nat, Buffer.Buffer<Nat>>(64, Nat.equal, Hash.hash);
    func idxAdd(topicId : Nat, postId : Nat) {
      switch (postsByTopic.get(topicId)) {
        case (?b) { b.add(postId) };
        case null { let b = Buffer.Buffer<Nat>(8); b.add(postId); postsByTopic.put(topicId, b) };
      };
    };
    func cntGet(m : HashMap.HashMap<Nat, Nat>, k : Nat) : Nat { switch (m.get(k)) { case (?n) n; case null 0 } };
    func cntBump(m : HashMap.HashMap<Nat, Nat>, k : Nat, delta : Int) {
      let cur : Int = cntGet(m, k);
      let next = cur + delta;
      m.put(k, if (next < 0) 0 else Int.abs(next));
    };
    // Recompute both counters from posts_ in one pass — after the constructor
    // seed and after mvStableLoad (which refills posts_ without going through
    // makePost/like).
    func rebuildCounters() {
      for (k in Iter.toArray(postCountByTopic.keys()).vals()) { postCountByTopic.delete(k) };
      for (k in Iter.toArray(likeCountByTopic.keys()).vals()) { likeCountByTopic.delete(k) };
      for (k in Iter.toArray(postsByTopic.keys()).vals()) { postsByTopic.delete(k) };
      for (p in posts_.vals()) {
        postCountByTopic.put(p.topicId, cntGet(postCountByTopic, p.topicId) + 1);
        if (p.likers.size() > 0) { likeCountByTopic.put(p.topicId, cntGet(likeCountByTopic, p.topicId) + p.likers.size()) };
        idxAdd(p.topicId, p.id);
      };
      // posts_.vals() is unordered; sort each topic's index ascending by id so
      // it matches live insertion order (OP first, replies chronological).
      for (b in postsByTopic.vals()) {
        let sorted = Array.sort(Buffer.toArray(b), Nat.compare);
        b.clear();
        for (id in sorted.vals()) { b.add(id) };
      };
    };

    // ---- Categories ----

    func addCategory(id : Nat, name : Text, color : Text, description : Text) {
      categories_.add({
        id;
        name;
        slug = slugify(name);
        description;
        color;
      });
    };

    func categoryOf(id : Nat) : ?CategoryRec {
      for (c in categories_.vals()) { if (c.id == id) { return ?c } };
      null;
    };

    func projectCategory(c : CategoryRec) : Category {
      var tc : Nat = 0;
      var pc : Nat = 0;
      for (t in topics_.vals()) {
        if (t.categoryId == c.id) {
          tc += 1;
          pc += postCountOf(t.id);
        };
      };
      {
        id = c.id;
        name = c.name;
        slug = c.slug;
        description = c.description;
        color = c.color;
        topicCount = tc;
        postCount = pc;
      };
    };

    public func categories() : [Category] {
      let out = Buffer.Buffer<Category>(categories_.size());
      for (c in categories_.vals()) { out.add(projectCategory(c)) };
      Buffer.toArray(out);
    };

    public func category(id : Nat) : ?Category {
      switch (categoryOf(id)) {
        case (?c) ?projectCategory(c);
        case null null;
      };
    };

    /// Convenience: the colored square hex for a category (falls back to grey).
    public func categoryColor(id : Nat) : Text {
      switch (categoryOf(id)) { case (?c) c.color; case null "#9aa0a6" };
    };

    /// Convenience: a category's display name (falls back to "Uncategorized").
    public func categoryName(id : Nat) : Text {
      switch (categoryOf(id)) { case (?c) c.name; case null "Uncategorized" };
    };

    // ---- Topics ----

    func projectTopic(t : TopicRec) : Topic {
      {
        id = t.id;
        categoryId = t.categoryId;
        title = t.title;
        slug = t.slug;
        author = t.author;
        authorHandle = t.authorHandle;
        createdAt = t.createdAt;
        bumpedAt = t.bumpedAt;
        tags = t.tags;
        pinned = t.pinned;
        closed = t.closed;
        acceptedPostId = t.acceptedPostId;
        views = t.views;
      };
    };

    /// Create a topic AND its opening post (OP). Returns the new topic id.
    public func createTopic(
      caller : Principal,
      handle : Text,
      categoryId : Nat,
      title : Text,
      tags : [Text],
      body : Text,
    ) : Nat {
      let id = nextTopicId;
      nextTopicId += 1;
      let now = Time.now();
      let rec : TopicRec = {
        id;
        categoryId;
        title;
        slug = slugify(title);
        author = caller;
        authorHandle = handle;
        createdAt = now;
        var bumpedAt = now;
        tags;
        var pinned = false;
        var closed = false;
        var acceptedPostId = 0;
        var views = 0;
      };
      topics_.put(id, rec);
      // OP post (replyToPost = 0)
      ignore makePost(caller, handle, id, body, 0, now);
      id;
    };

    /// All topics in a category, pinned first then most-recently-bumped.
    public func topics(categoryId : Nat) : [Topic] {
      let buf = Buffer.Buffer<TopicRec>(16);
      for (t in topics_.vals()) { if (t.categoryId == categoryId) { buf.add(t) } };
      sortedTopics(buf);
    };

    /// All topics, latest (by bumpedAt desc), pinned floated to the top.
    public func topicsLatest() : [Topic] {
      let buf = Buffer.Buffer<TopicRec>(topics_.size());
      for (t in topics_.vals()) { buf.add(t) };
      sortedTopics(buf);
    };

    /// All topics purely by recency (bumpedAt desc) — NO pinned float, for
    /// "latest activity" summaries like the dashboard where an old pinned
    /// topic must not head the list.
    public func topicsChronological() : [Topic] {
      let buf = Buffer.Buffer<TopicRec>(topics_.size());
      for (t in topics_.vals()) { buf.add(t) };
      let sorted = Array.sort(Buffer.toArray(buf), func(a : TopicRec, b : TopicRec) : { #less; #equal; #greater } {
        if (a.bumpedAt > b.bumpedAt) #less else if (a.bumpedAt < b.bumpedAt) #greater else #equal;
      });
      Array.map<TopicRec, Topic>(sorted, projectTopic);
    };

    /// All topics ranked "top": by total like count, then reply count.
    public func topicsTop() : [Topic] {
      let buf = Buffer.Buffer<TopicRec>(topics_.size());
      for (t in topics_.vals()) { buf.add(t) };
      let arr = Buffer.toArray(buf);
      let sorted = Array.sort(arr, func(a : TopicRec, b : TopicRec) : { #less; #equal; #greater } {
        let sa = topicScore(a.id);
        let sb = topicScore(b.id);
        if (sa > sb) #less else if (sa < sb) #greater
        else if (a.bumpedAt > b.bumpedAt) #less else if (a.bumpedAt < b.bumpedAt) #greater
        else #equal;
      });
      Array.map<TopicRec, Topic>(sorted, projectTopic);
    };

    /// One render-bounded window of the board's topics, plus the TOTAL topic
    /// count. `top` picks the ranking (true = by score, false = latest with
    /// pinned floated). Sorts the full set (cheap — O(T log T) with O(1)
    /// per-topic score) then slices, so a board with thousands of topics still
    /// renders a constant-size page instead of dumping every row.
    public func topicsPage(top : Bool, offset : Nat, limit : Nat) : { items : [Topic]; total : Nat } {
      let all = if (top) topicsTop() else topicsLatest();
      let total = all.size();
      if (limit == 0 or offset >= total) { return { items = (if (offset >= total) [] else all); total } };
      let stop = Nat.min(total, offset + limit);
      { items = Array.subArray(all, offset, stop - offset); total };
    };

    public func topic(id : Nat) : ?Topic {
      switch (topics_.get(id)) {
        case (?t) ?projectTopic(t);
        case null null;
      };
    };

    /// Increment a topic's view counter (call from the topic page onLoad).
    public func bumpView(id : Nat) {
      switch (topics_.get(id)) {
        case (?t) { t.views += 1 };
        case null {};
      };
    };

    func sortedTopics(buf : Buffer.Buffer<TopicRec>) : [Topic] {
      let arr = Buffer.toArray(buf);
      let sorted = Array.sort(arr, func(a : TopicRec, b : TopicRec) : { #less; #equal; #greater } {
        // pinned floats to top
        if (a.pinned and not b.pinned) #less
        else if (b.pinned and not a.pinned) #greater
        else if (a.bumpedAt > b.bumpedAt) #less
        else if (a.bumpedAt < b.bumpedAt) #greater
        else #equal;
      });
      Array.map<TopicRec, Topic>(sorted, projectTopic);
    };

    func topicScore(topicId : Nat) : Nat {
      // O(1): likes*3 + replies, from the incremental counters.
      cntGet(likeCountByTopic, topicId) * 3 + replyCount(topicId);
    };

    // ---- Posts ----

    func projectPost(p : PostRec) : Post {
      {
        id = p.id;
        topicId = p.topicId;
        author = p.author;
        authorHandle = p.authorHandle;
        at = p.at;
        body = p.body;
        replyToPost = p.replyToPost;
        likeCount = p.likers.size();
      };
    };

    func makePost(caller : Principal, handle : Text, topicId : Nat, body : Text, replyToPost : Nat, at : Int) : Nat {
      let id = nextPostId;
      nextPostId += 1;
      let rec : PostRec = {
        id;
        topicId;
        author = caller;
        authorHandle = handle;
        at;
        var body;
        replyToPost;
        likers = Buffer.Buffer<Principal>(4);
      };
      posts_.put(id, rec);
      cntBump(postCountByTopic, topicId, 1);
      idxAdd(topicId, id);
      id;
    };

    /// Add a reply to a topic. Bumps the topic's `bumpedAt`. Returns post id, or
    /// 0 if the topic does not exist or is closed.
    public func reply(caller : Principal, handle : Text, topicId : Nat, body : Text, replyToPost : Nat) : Nat {
      switch (topics_.get(topicId)) {
        case null { 0 };
        case (?t) {
          if (t.closed) { return 0 };
          let now = Time.now();
          let pid = makePost(caller, handle, topicId, body, replyToPost, now);
          t.bumpedAt := now;
          pid;
        };
      };
    };

    /// All posts in a topic: OP first (lowest id / replyToPost = 0), then the
    /// rest in chronological order.
    public func posts(topicId : Nat) : [Post] {
      // Index-driven: map the topic's post ids (already OP-first/chronological)
      // straight to Posts — no scan of the whole posts_ store.
      switch (postsByTopic.get(topicId)) {
        case null { [] };
        case (?ids) {
          let buf = Buffer.Buffer<Post>(ids.size());
          for (pid in ids.vals()) {
            switch (posts_.get(pid)) { case (?p) buf.add(projectPost(p)); case null {} };
          };
          Buffer.toArray(buf);
        };
      };
    };

    /// One render-bounded window of a topic's posts: `limit` posts starting at
    /// `offset`, plus the topic's TOTAL post count. The OP (offset 0) is always
    /// in the first window. Lets ForumTopic render in pages instead of dumping
    /// every reply (the audit's unbounded-render finding — an 800-reply topic
    /// went from a multi-second full scan to a constant-size slice).
    public func postsPage(topicId : Nat, offset : Nat, limit : Nat) : { items : [Post]; total : Nat } {
      switch (postsByTopic.get(topicId)) {
        case null { { items = []; total = 0 } };
        case (?ids) {
          let total = ids.size();
          let buf = Buffer.Buffer<Post>(limit);
          var i = offset;
          let stop = if (limit == 0) total else Nat.min(total, offset + limit);
          while (i < stop) {
            switch (posts_.get(ids.get(i))) { case (?p) buf.add(projectPost(p)); case null {} };
            i += 1;
          };
          { items = Buffer.toArray(buf); total };
        };
      };
    };

    /// Topic-wide summary figures computed in a SINGLE O(n) pass over the
    /// topic's posts (HashMap dedup for participants — no O(n²) scan). Used by
    /// the ForumTopic page so its header totals stay correct even though only
    /// one page of posts is rendered.
    public func topicStats(topicId : Nat) : {
      postTotal : Nat; likeTotal : Nat; participants : Nat; words : Nat;
      firstAt : Int; lastAt : Int;
    } {
      switch (postsByTopic.get(topicId)) {
        case null { { postTotal = 0; likeTotal = 0; participants = 0; words = 0; firstAt = 0; lastAt = 0 } };
        case (?ids) {
          let seen = HashMap.HashMap<Principal, Bool>(16, Principal.equal, Principal.hash);
          var likes : Nat = 0; var words : Nat = 0; var participants : Nat = 0;
          var firstAt : Int = 0; var lastAt : Int = 0; var any = false;
          for (pid in ids.vals()) {
            switch (posts_.get(pid)) {
              case (?p) {
                likes += p.likers.size();
                words += wordCountOf(p.body);
                if (seen.get(p.author) == null) { seen.put(p.author, true); participants += 1 };
                if (not any) { firstAt := p.at; lastAt := p.at; any := true }
                else { if (p.at < firstAt) firstAt := p.at; if (p.at > lastAt) lastAt := p.at };
              };
              case null {};
            };
          };
          { postTotal = ids.size(); likeTotal = likes; participants; words; firstAt; lastAt };
        };
      };
    };

    func wordCountOf(t : Text) : Nat {
      var n : Nat = 0; var inWord = false;
      for (c in t.chars()) {
        if (c == ' ' or c == '\n' or c == '\t') { inWord := false }
        else if (not inWord) { inWord := true; n += 1 };
      };
      n;
    };

    public func post(id : Nat) : ?Post {
      switch (posts_.get(id)) { case (?p) ?projectPost(p); case null null };
    };

    func postCountOf(topicId : Nat) : Nat { cntGet(postCountByTopic, topicId) };

    /// Number of replies in a topic (total posts minus the OP).
    public func replyCount(topicId : Nat) : Nat {
      let total = postCountOf(topicId);
      if (total > 0) { total - 1 } else { 0 };
    };

    // ---- Likes ----

    func likerIndex(p : PostRec, who : Principal) : ?Nat {
      var i : Nat = 0;
      for (l in p.likers.vals()) {
        if (l == who) { return ?i };
        i += 1;
      };
      null;
    };

    /// Toggle a like on a post. Returns the new liked state (true = now liked).
    public func like(caller : Principal, postId : Nat) : Bool {
      switch (posts_.get(postId)) {
        case null { false };
        case (?p) {
          switch (likerIndex(p, caller)) {
            case (?idx) { ignore p.likers.remove(idx); cntBump(likeCountByTopic, p.topicId, -1); false };
            case null { p.likers.add(caller); cntBump(likeCountByTopic, p.topicId, 1); true };
          };
        };
      };
    };

    public func hasLiked(caller : Principal, postId : Nat) : Bool {
      switch (posts_.get(postId)) {
        case null { false };
        case (?p) { likerIndex(p, caller) != null };
      };
    };

    public func likeCount(postId : Nat) : Nat {
      switch (posts_.get(postId)) { case (?p) p.likers.size(); case null 0 };
    };

    // ---- Solve / pin / close ----

    /// Mark a post as the accepted answer. Topic author (or a future mod) only.
    /// Pass postId = 0 to clear the accepted answer.
    public func accept(caller : Principal, topicId : Nat, postId : Nat) : Bool {
      switch (topics_.get(topicId)) {
        case null { false };
        case (?t) {
          if (t.author != caller) { return false };
          if (postId != 0) {
            // the post must belong to this topic
            switch (posts_.get(postId)) {
              case null { return false };
              case (?p) { if (p.topicId != topicId) { return false } };
            };
          };
          t.acceptedPostId := postId;
          true;
        };
      };
    };

    public func setPinned(caller : Principal, topicId : Nat, pinned : Bool) : Bool {
      switch (topics_.get(topicId)) {
        case null { false };
        case (?t) {
          if (t.author != caller) { return false };
          t.pinned := pinned;
          true;
        };
      };
    };

    public func setClosed(caller : Principal, topicId : Nat, closed : Bool) : Bool {
      switch (topics_.get(topicId)) {
        case null { false };
        case (?t) {
          if (t.author != caller) { return false };
          t.closed := closed;
          true;
        };
      };
    };

    /// Privileged pin/close — NO author check. The PAGE gates these on a global
    /// Moderator+ (cross-scope forum moderation).
    public func forceSetPinned(topicId : Nat, pinned : Bool) : Bool {
      switch (topics_.get(topicId)) {
        case null { false };
        case (?t) { t.pinned := pinned; true };
      };
    };

    public func forceSetClosed(topicId : Nat, closed : Bool) : Bool {
      switch (topics_.get(topicId)) {
        case null { false };
        case (?t) { t.closed := closed; true };
      };
    };

    public func acceptedPostId(topicId : Nat) : Nat {
      switch (topics_.get(topicId)) { case (?t) t.acceptedPostId; case null 0 };
    };

    public func isSolved(topicId : Nat) : Bool {
      switch (topics_.get(topicId)) { case (?t) t.acceptedPostId != 0; case null false };
    };

    // ---- Text helpers (so thin .mview pages can render directly) ----

    /// A short human relative time from a stored `Int` timestamp (nanoseconds).
    public func relativeTime(at : Int) : Text {
      let now = Time.now();
      var diff : Int = now - at;
      if (diff < 0) { diff := 0 };
      let secs = diff / 1_000_000_000;
      if (secs < 60) { return "just now" };
      let mins = secs / 60;
      if (mins < 60) { return Int.toText(mins) # (if (mins == 1) "m ago" else "m ago") };
      let hours = mins / 60;
      if (hours < 24) { return Int.toText(hours) # "h ago" };
      let days = hours / 24;
      if (days < 30) { return Int.toText(days) # "d ago" };
      let months = days / 30;
      if (months < 12) { return Int.toText(months) # "mo ago" };
      let years = days / 365;
      Int.toText(years) # "y ago";
    };

    /// Relative time for a topic's last activity (from bumpedAt).
    public func topicActivity(topicId : Nat) : Text {
      switch (topics_.get(topicId)) {
        case (?t) relativeTime(t.bumpedAt);
        case null "";
      };
    };

    /// Render a topic's tags as a single " · "-joined string (handy for rows).
    public func tagsText(topicId : Nat) : Text {
      switch (topics_.get(topicId)) {
        case null { "" };
        case (?t) { Text.join(" · ", t.tags.vals()) };
      };
    };

    /// Status glyphs for a topic title row (📌 pinned, 🔒 closed, ✅ solved).
    public func statusGlyphs(topicId : Nat) : Text {
      switch (topics_.get(topicId)) {
        case null { "" };
        case (?t) {
          var s = "";
          if (t.pinned) { s #= "📌 " };
          if (t.closed) { s #= "🔒 " };
          if (t.acceptedPostId != 0) { s #= "✅ " };
          s;
        };
      };
    };

    // ---- Counts ----

    public func topicCount() : Nat { topics_.size() };
    public func postCount() : Nat { posts_.size() };
    public func categoryCount() : Nat { categories_.size() };

    // ---- Upgrade-stable persistence (MotoView mvStableSave/mvStableLoad) ----
    //
    // All snapshot types below are fully shared (immutable, no functions), so
    // to_candid / from_candid round-trip cleanly. Mutable `var` fields of
    // TopicRec/PostRec are flattened into plain immutable fields, and the
    // `likers` Buffer is serialized as a [Principal] array.

    type TopicSnap = {
      id : Nat;
      categoryId : Nat;
      title : Text;
      slug : Text;
      author : Principal;
      authorHandle : Text;
      createdAt : Int;
      bumpedAt : Int;
      tags : [Text];
      pinned : Bool;
      closed : Bool;
      acceptedPostId : Nat;
      views : Nat;
    };

    type PostSnap = {
      id : Nat;
      topicId : Nat;
      author : Principal;
      authorHandle : Text;
      at : Int;
      body : Text;
      replyToPost : Nat;
      likers : [Principal];
    };

    public func mvStableSave() : Blob {
      let topicSnaps = Array.map<(Nat, TopicRec), (Nat, TopicSnap)>(
        Iter.toArray(topics_.entries()),
        func((k, t) : (Nat, TopicRec)) : (Nat, TopicSnap) {
          (
            k,
            {
              id = t.id;
              categoryId = t.categoryId;
              title = t.title;
              slug = t.slug;
              author = t.author;
              authorHandle = t.authorHandle;
              createdAt = t.createdAt;
              bumpedAt = t.bumpedAt;
              tags = t.tags;
              pinned = t.pinned;
              closed = t.closed;
              acceptedPostId = t.acceptedPostId;
              views = t.views;
            },
          );
        },
      );
      let postSnaps = Array.map<(Nat, PostRec), (Nat, PostSnap)>(
        Iter.toArray(posts_.entries()),
        func((k, p) : (Nat, PostRec)) : (Nat, PostSnap) {
          (
            k,
            {
              id = p.id;
              topicId = p.topicId;
              author = p.author;
              authorHandle = p.authorHandle;
              at = p.at;
              body = p.body;
              replyToPost = p.replyToPost;
              likers = Buffer.toArray(p.likers);
            },
          );
        },
      );
      to_candid ({
        nextTopicId = nextTopicId;
        nextPostId = nextPostId;
        categories = Buffer.toArray(categories_);
        topics = topicSnaps;
        posts = postSnaps;
      });
    };

    public func mvStableLoad(b : Blob) {
      switch (
        from_candid (b) : ?{
          nextTopicId : Nat;
          nextPostId : Nat;
          categories : [CategoryRec];
          topics : [(Nat, TopicSnap)];
          posts : [(Nat, PostSnap)];
        }
      ) {
        case (?saved) {
          let savedNextTopicId = saved.nextTopicId;
          let savedNextPostId = saved.nextPostId;
          let savedCategories = saved.categories;
          let savedTopics = saved.topics;
          let savedPosts = saved.posts;
          // scalars
          nextTopicId := savedNextTopicId;
          nextPostId := savedNextPostId;

          // categories (Buffer): replace
          categories_.clear();
          for (c in savedCategories.vals()) { categories_.add(c) };

          // topics (HashMap): delete existing keys then re-put
          for (k in Iter.toArray(topics_.keys()).vals()) { topics_.delete(k) };
          for ((k, t) in savedTopics.vals()) {
            topics_.put(
              k,
              {
                id = t.id;
                categoryId = t.categoryId;
                title = t.title;
                slug = t.slug;
                author = t.author;
                authorHandle = t.authorHandle;
                createdAt = t.createdAt;
                var bumpedAt = t.bumpedAt;
                tags = t.tags;
                var pinned = t.pinned;
                var closed = t.closed;
                var acceptedPostId = t.acceptedPostId;
                var views = t.views;
              },
            );
          };

          // posts (HashMap): delete existing keys then re-put
          for (k in Iter.toArray(posts_.keys()).vals()) { posts_.delete(k) };
          for ((k, p) in savedPosts.vals()) {
            let likersBuf = Buffer.Buffer<Principal>(p.likers.size());
            for (l in p.likers.vals()) { likersBuf.add(l) };
            posts_.put(
              k,
              {
                id = p.id;
                topicId = p.topicId;
                author = p.author;
                authorHandle = p.authorHandle;
                at = p.at;
                var body = p.body;
                replyToPost = p.replyToPost;
                likers = likersBuf;
              },
            );
          };
          // posts_ was refilled outside makePost/like, so recompute the O(1)
          // reply/like counters from the restored posts in one pass.
          rebuildCounters();
        };
        case null {};
      };
    };

    // ---- helpers ----

    func slugify(t : Text) : Text {
      let lowered = Text.map(Text.trim(t, #char ' '), func(c : Char) : Char {
        if (c >= 'A' and c <= 'Z') { Char.fromNat32(Char.toNat32(c) + 32) } else { c };
      });
      // replace any non [a-z0-9] run with a single '-'
      var out = "";
      var prevDash = true; // suppress leading dash
      for (c in lowered.chars()) {
        let ok = (c >= 'a' and c <= 'z') or (c >= '0' and c <= '9');
        if (ok) { out #= Text.fromChar(c); prevDash := false }
        else if (not prevDash) { out #= "-"; prevDash := true };
      };
      // trim a single trailing dash
      let chars = Text.toArray(out);
      let n = chars.size();
      if (n > 0 and chars[n - 1] == '-') {
        Text.fromIter(Array.subArray(chars, 0, n - 1).vals());
      } else { out };
    };

    // ---- Seed: 8 categories with EXACT colors/names from BUILD_SPEC PART 4 ----

    func seed() {
      addCategory(1, "Developers", "#F15A24", "Building on the Internet Computer.");
      addCategory(2, "Getting Started", "#0088CC", "New here? Start your journey.");
      addCategory(3, "Motoko", "#c22d7f", "The native language of the IC.");
      addCategory(4, "Rust", "#f74c00", "Writing canisters in Rust.");
      addCategory(5, "JavaScript", "#f0db4f", "Front-end and agent-js.");
      addCategory(6, "Showcase", "#F7941D", "Show off what you've shipped.");
      addCategory(7, "Internet Identity", "#0088CC", "Authentication and identity.");
      addCategory(8, "Education", "#12A89D", "Learning resources and guides.");
    };

    // 2-3 sample topics so the list renders. Authored by an anonymous seed
    // principal with descriptive handles (NOT fabricated user metrics).
    func seedTopics() {
      // A distinct principal per handle (derived from the handle bytes) so the
      // forum looks realistic: different authors -> different avatars, and the
      // OP badge shows only on the topic author's posts.
      func seedP(h : Text) : Principal { Principal.fromBlob(Text.encodeUtf8(h)) };

      let t1 = createTopic(
        seedP("motoko-team"), "motoko-team", 3,
        "Welcome to the Motoko category",
        ["motoko", "welcome"],
        "This is the place to discuss the Motoko programming language — actors, async, stable memory and more. Introduce yourself!",
      );
      ignore reply(seedP("dev-relations"), "dev-relations", t1,
        "Pinning this so newcomers can find it. Happy hacking!", 0);
      ignore setPinned(seedP("motoko-team"), t1, true);

      let t2 = createTopic(
        seedP("newcomer"), "newcomer", 2,
        "How do I deploy my first canister?",
        ["dfx", "deploy", "beginner"],
        "I just installed dfx. What's the shortest path from `dfx new` to a running canister on the local replica?",
      );
      let a2 = reply(seedP("ic-helper"), "ic-helper", t2,
        "Run `dfx start --background`, then `dfx deploy`. Your canister URLs print at the end. That's it!", 0);
      ignore accept(seedP("newcomer"), t2, a2);

      let t3 = createTopic(
        seedP("builder"), "builder", 6,
        "Showcase: I built a server-driven UI framework",
        ["showcase", "motoview", "ui"],
        "MotoView lets you write `.mview` pages backed by stateful Motoko services. Feedback welcome!",
      );
      ignore reply(seedP("curious"), "curious", t3,
        "This looks great — does it support forms and validation out of the box?", 0);
    };

    // run seeds at construction
    seed();
    seedTopics();
  };
};
