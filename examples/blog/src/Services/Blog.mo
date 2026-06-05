/// Blog content store — a stateless Motoko service.
///
/// A Motoko `module` is a stateless library: the post list is fixed seed
/// content defined here, and these pure functions read it. The `motoview`
/// compiler imports `src/Services/*.mo` into the generated actor automatically,
/// so page code can call `Blog.all()` and `Blog.get(slug)` directly.
///
/// `bodyHtml` is *pre-rendered, trusted* HTML authored here (never user input),
/// so pages can emit it with `@raw(post.bodyHtml)`.
import Array "mo:base/Array";

module {

  public type Post = {
    slug : Text;
    title : Text;
    date : Text; // ISO date, e.g. "2026-05-28"
    excerpt : Text; // plain-text summary, also used as the meta description
    bodyHtml : Text; // trusted, pre-rendered HTML — authored here, not user input
  };

  /// Seed content — three real-looking posts about building on the IC.
  /// Listed oldest-first here; `all()` returns them newest-first.
  ///
  /// This is a `func`, not a module-level `let`: Motoko requires library/module
  /// top-level bindings to be *static*, and the `#` string concatenation used to
  /// build each `bodyHtml` is not static. Returning the array from a function
  /// keeps it a normal (non-static) expression.
  func posts() : [Post] {
    [
    {
      slug = "why-no-frontend-javascript";
      title = "Why MotoView Ships Zero Frontend JavaScript";
      date = "2026-04-12";
      excerpt = "Rendering, validation, and routing all run inside the canister. The browser is a thin renderer, so there is no second place for your logic to drift out of sync.";
      bodyHtml = "<p>Most web stacks split a single app into two programs: a backend that owns the data and a frontend bundle that re-implements half the rules in JavaScript. Every validation, every route guard, every formatting decision now lives in two places — and the two slowly drift apart.</p>" #
        "<p>MotoView collapses that split. Your <code>.mview</code> files compile into one Motoko actor that runs on the Internet Computer. The canister server-renders real HTML, mints secure form tokens, validates input, and dispatches events. The browser only swaps DOM nodes the server already computed.</p>" #
        "<h2>What this buys you</h2>" #
        "<ul><li><strong>One source of truth.</strong> A rule written once cannot disagree with itself.</li>" #
        "<li><strong>SEO for free.</strong> Crawlers get fully rendered HTML — title, meta description, canonical link — on the very first byte.</li>" #
        "<li><strong>No build-time API drift.</strong> There is no separate API surface to version against a client bundle.</li></ul>" #
        "<p>The page you are reading is itself served this way: a certified query straight from a canister, no client framework involved.</p>";
    },
    {
      slug = "certified-queries-and-seo";
      title = "Certified Queries: Fast Pages a Crawler Can Trust";
      date = "2026-05-03";
      excerpt = "Marking a public page @cacheable serves it as a certified query — millisecond responses with a cryptographic proof that the bytes came from your canister.";
      bodyHtml = "<p>An ordinary IC query is fast but unauthenticated: a boundary node could, in principle, return anything. A <em>certified</em> query attaches a signature rooted in the subnet's public key, so the client can verify the response really came from your canister.</p>" #
        "<p>In MotoView you opt a page into this with a single directive:</p>" #
        "<pre><code>@page \"/post/{slug}\"\n@cacheable</code></pre>" #
        "<p>Now the rendered HTML is stored and served as a certified query. The result is the best of both worlds for content pages: the latency of a static CDN and the integrity of on-chain state.</p>" #
        "<h2>Why it matters for SEO</h2>" #
        "<p>Search crawlers reward fast, stable, self-consistent pages. A certified query returns the same canonical HTML to every visitor in milliseconds — title, description, and canonical URL included — without spinning up an update call. Public, non-personalized pages like blog posts and docs are the ideal candidates.</p>" #
        "<p>Keep <code>@cacheable</code> off any page whose content depends on the caller; reserve it for content that is the same for everyone.</p>";
    },
    {
      slug = "secure-forms-by-default";
      title = "Secure Forms Without a Line of Client Code";
      date = "2026-05-28";
      excerpt = "Add the secure attribute and MotoView mints an HMAC token bound to the path, handler, caller principal, a single-use nonce, and the field schema.";
      bodyHtml = "<p>Forms are where most web vulnerabilities live: CSRF, replay, field tampering, and validation that the client can simply skip. MotoView's answer is to make the secure path the default path.</p>" #
        "<p>Mark a form <code>secure</code> and the canister mints an <strong>HMAC-SHA256</strong> token over a tuple that pins the submission down:</p>" #
        "<ul><li><strong>path</strong> — the page the form was rendered on</li>" #
        "<li><strong>handler</strong> — the one event handler allowed to run</li>" #
        "<li><strong>principal</strong> — the caller's identity at render time</li>" #
        "<li><strong>nonce</strong> — single-use, so a captured request cannot be replayed</li>" #
        "<li><strong>expiry</strong> — an absolute deadline</li>" #
        "<li><strong>schema hash</strong> — the exact field set and constraints</li></ul>" #
        "<p>On submit, the canister re-derives the MAC from the live request and rejects anything that does not match — before your handler runs. Validation still executes server-side, so there is no client-only check to bypass.</p>" #
        "<p>You write a handler; the cryptography is handled for you.</p>";
    },
    ];
  };

  /// All posts, sorted newest-first by date.
  public func all() : [Post] {
    Array.sort<Post>(
      posts(),
      func(a, b) {
        // Reverse chronological: compare b.date to a.date so larger dates come first.
        if (a.date > b.date) { #less } else if (a.date < b.date) { #greater } else {
          #equal;
        };
      },
    );
  };

  /// Look up a single post by slug. Returns null when the slug is unknown.
  public func get(slug : Text) : ?Post {
    for (p in posts().vals()) { if (p.slug == slug) { return ?p } };
    null;
  };

  public func count() : Nat { posts().size() };
};
