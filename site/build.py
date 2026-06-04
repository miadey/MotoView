#!/usr/bin/env python3
"""
MotoView documentation site generator.

Node-free, dependency-free (Python 3 stdlib only). Reads ../docs/*.md (with
`--- title/section/slug ---` frontmatter), converts Markdown to HTML, and emits
a Laravel-style static site (left sidebar grouped by section, clean typography)
into ./dist. Run:  python3 build.py
"""
import os
import re
import html
import shutil

HERE = os.path.dirname(os.path.abspath(__file__))
DOCS = os.path.normpath(os.path.join(HERE, "..", "docs"))
DIST = os.path.join(HERE, "docs")  # built HTML lives at site/docs (landing links ./docs/)

# Sidebar order: (Section, [slugs])
NAV = [
    ("Prologue", ["introduction", "why-motoview"]),
    ("Getting Started", ["installation", "quickstart", "project-structure"]),
    ("The .mview Format", ["mview-files", "pages-and-routing", "layouts", "components", "control-flow"]),
    ("Interactivity", ["events", "forms", "validation", "state"]),
    ("Styling", ["styling-and-themes", "svg"]),
    ("Architecture", ["protocol", "security", "client-bridge", "runtime"]),
    ("Deployment", ["deployment", "cli"]),
    ("Tooling", ["ai-tools"]),
    ("Reference", ["directives-reference", "roadmap"]),
]


def read_doc(slug):
    path = os.path.join(DOCS, slug + ".md")
    if not os.path.exists(path):
        return None
    with open(path, encoding="utf-8") as f:
        text = f.read()
    title = slug
    body = text
    m = re.match(r"^---\s*\n(.*?)\n---\s*\n(.*)$", text, re.DOTALL)
    if m:
        front, body = m.group(1), m.group(2)
        tm = re.search(r"^title:\s*(.+)$", front, re.MULTILINE)
        if tm:
            title = tm.group(1).strip()
    return {"slug": slug, "title": title, "body": body}


# ---- a small but capable Markdown -> HTML converter ----------------------

def inline(text):
    # escape first, then re-introduce inline markup
    text = html.escape(text, quote=False)
    # inline code
    text = re.sub(r"`([^`]+)`", lambda m: "<code>" + m.group(1) + "</code>", text)
    # links [t](u)
    text = re.sub(r"\[([^\]]+)\]\(([^)]+)\)",
                  lambda m: '<a href="%s">%s</a>' % (rewrite_link(m.group(2)), m.group(1)), text)
    # bold then italic
    text = re.sub(r"\*\*([^*]+)\*\*", r"<strong>\1</strong>", text)
    text = re.sub(r"(?<![\w*])\*([^*\n]+)\*(?![\w*])", r"<em>\1</em>", text)
    return text


def rewrite_link(url):
    # local .md links -> .html
    if url.endswith(".md"):
        return url[:-3] + ".html"
    return url


def render_markdown(md):
    lines = md.split("\n")
    out = []
    i = 0
    n = len(lines)
    while i < n:
        line = lines[i]

        # fenced code
        fence = re.match(r"^```(\w*)\s*$", line)
        if fence:
            lang = fence.group(1) or "text"
            i += 1
            code = []
            while i < n and not lines[i].startswith("```"):
                code.append(lines[i])
                i += 1
            i += 1  # closing fence
            esc = html.escape("\n".join(code), quote=False)
            out.append('<pre class="lang-%s"><code>%s</code></pre>' % (lang, esc))
            continue

        # headings
        h = re.match(r"^(#{1,4})\s+(.*)$", line)
        if h:
            level = len(h.group(1))
            text = inline(h.group(2).strip())
            anchor = re.sub(r"[^a-z0-9]+", "-", h.group(2).strip().lower()).strip("-")
            out.append('<h%d id="%s">%s</h%d>' % (level, anchor, text, level))
            i += 1
            continue

        # hr
        if re.match(r"^---+\s*$", line):
            out.append("<hr>")
            i += 1
            continue

        # blockquote
        if line.startswith(">"):
            quote = []
            while i < n and lines[i].startswith(">"):
                quote.append(lines[i][1:].lstrip())
                i += 1
            out.append("<blockquote>" + inline(" ".join(quote)) + "</blockquote>")
            continue

        # table
        if "|" in line and i + 1 < n and re.match(r"^\s*\|?[\s:|-]+\|[\s:|-]*$", lines[i + 1]):
            header = [c.strip() for c in line.strip().strip("|").split("|")]
            i += 2
            rows = []
            while i < n and "|" in lines[i] and lines[i].strip():
                rows.append([c.strip() for c in lines[i].strip().strip("|").split("|")])
                i += 1
            t = ['<table class="mv-doc-table"><thead><tr>']
            t += ["<th>%s</th>" % inline(c) for c in header]
            t.append("</tr></thead><tbody>")
            for r in rows:
                t.append("<tr>" + "".join("<td>%s</td>" % inline(c) for c in r) + "</tr>")
            t.append("</tbody></table>")
            out.append("".join(t))
            continue

        # lists (unordered / ordered)
        lm = re.match(r"^(\s*)([-*]|\d+\.)\s+(.*)$", line)
        if lm:
            ordered = bool(re.match(r"\d+\.", lm.group(2)))
            tag = "ol" if ordered else "ul"
            items = []
            while i < n:
                m2 = re.match(r"^(\s*)([-*]|\d+\.)\s+(.*)$", lines[i])
                if not m2:
                    break
                items.append("<li>" + inline(m2.group(3).strip()) + "</li>")
                i += 1
            out.append("<%s>%s</%s>" % (tag, "".join(items), tag))
            continue

        # blank
        if line.strip() == "":
            i += 1
            continue

        # paragraph (gather until blank / block start)
        para = [line]
        i += 1
        while i < n and lines[i].strip() != "" and not re.match(r"^(#{1,4}\s|```|>|\s*[-*]\s|\s*\d+\.\s|---+\s*$)", lines[i]):
            para.append(lines[i])
            i += 1
        out.append("<p>" + inline(" ".join(s.strip() for s in para)) + "</p>")
    return "\n".join(out)


# ---- site templating ------------------------------------------------------

def sidebar_html(active_slug, docs_by_slug):
    parts = ['<nav class="mv-sidebar"><a class="mv-brand" href="../index.html">▼ MotoView</a>']
    for section, slugs in NAV:
        parts.append('<div class="mv-nav-section">%s</div><ul>' % html.escape(section))
        for slug in slugs:
            d = docs_by_slug.get(slug)
            if not d:
                continue
            cls = ' class="active"' if slug == active_slug else ""
            parts.append('<li><a%s href="%s.html">%s</a></li>' % (cls, slug, html.escape(d["title"])))
        parts.append("</ul>")
    parts.append("</nav>")
    return "".join(parts)


def page_html(doc, docs_by_slug, prev_doc, next_doc):
    pager = '<div class="mv-pager">'
    if prev_doc:
        pager += '<a class="prev" href="%s.html">← %s</a>' % (prev_doc["slug"], html.escape(prev_doc["title"]))
    else:
        pager += "<span></span>"
    if next_doc:
        pager += '<a class="next" href="%s.html">%s →</a>' % (next_doc["slug"], html.escape(next_doc["title"]))
    pager += "</div>"

    return TEMPLATE.format(
        title=html.escape(doc["title"]),
        sidebar=sidebar_html(doc["slug"], docs_by_slug),
        content=render_markdown(doc["body"]),
        pager=pager,
    )


TEMPLATE = """<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>{title} — MotoView Docs</title>
<link rel="stylesheet" href="assets/site.css">
</head>
<body class="mv-docs">
<header class="mv-topbar">
  <a class="mv-topbar-brand" href="../index.html">▼ MotoView</a>
  <nav class="mv-topbar-links">
    <a href="introduction.html">Docs</a>
    <a href="https://github.com/miadey/MotoView">GitHub</a>
  </nav>
</header>
<div class="mv-layout">
  {sidebar}
  <main class="mv-content">
    <article class="mv-prose">
      {content}
    </article>
    {pager}
  </main>
</div>
</body>
</html>
"""


def main():
    if os.path.isdir(DIST):
        shutil.rmtree(DIST)
    os.makedirs(os.path.join(DIST, "assets"))

    flat = [slug for _, slugs in NAV for slug in slugs]
    docs_by_slug = {}
    for slug in flat:
        d = read_doc(slug)
        if d:
            docs_by_slug[slug] = d

    present = [s for s in flat if s in docs_by_slug]
    for idx, slug in enumerate(present):
        doc = docs_by_slug[slug]
        prev_doc = docs_by_slug[present[idx - 1]] if idx > 0 else None
        next_doc = docs_by_slug[present[idx + 1]] if idx + 1 < len(present) else None
        out = page_html(doc, docs_by_slug, prev_doc, next_doc)
        with open(os.path.join(DIST, slug + ".html"), "w", encoding="utf-8") as f:
            f.write(out)

    # index of docs -> introduction
    with open(os.path.join(DIST, "index.html"), "w", encoding="utf-8") as f:
        f.write('<!DOCTYPE html><meta http-equiv="refresh" content="0; url=introduction.html">')

    with open(os.path.join(DIST, "assets", "site.css"), "w", encoding="utf-8") as f:
        f.write(CSS)

    print("built %d doc pages -> %s" % (len(present), DIST))


CSS = r""":root{--p:#6d28d9;--p2:#5b21b6;--bg:#fff;--fg:#1c1b29;--soft:#6b7280;--border:#e6e6ef;--muted:#f7f7fb;--code-bg:#1e1b2e;--code-fg:#e7e3f5;--font:Inter,ui-sans-serif,system-ui,-apple-system,"Segoe UI",Roboto,Helvetica,Arial,sans-serif;--mono:ui-monospace,SFMono-Regular,Menlo,Consolas,monospace}
*{box-sizing:border-box}html{scroll-behavior:smooth}
body{margin:0;font-family:var(--font);color:var(--fg);background:var(--bg);line-height:1.7;-webkit-font-smoothing:antialiased}
a{color:var(--p);text-decoration:none}a:hover{text-decoration:underline}
.mv-topbar{position:sticky;top:0;z-index:20;display:flex;align-items:center;justify-content:space-between;height:60px;padding:0 1.5rem;background:rgba(255,255,255,.85);backdrop-filter:blur(8px);border-bottom:1px solid var(--border)}
.mv-topbar-brand{font-weight:800;color:var(--p);font-size:1.1rem}
.mv-topbar-links a{margin-left:1.25rem;color:var(--fg);font-weight:600;font-size:.95rem}
.mv-layout{display:grid;grid-template-columns:280px minmax(0,1fr);max-width:1200px;margin:0 auto}
.mv-sidebar{position:sticky;top:60px;align-self:start;height:calc(100vh - 60px);overflow-y:auto;padding:1.5rem 1rem 3rem 1.5rem;border-right:1px solid var(--border)}
.mv-brand{display:none}
.mv-nav-section{margin:1.25rem 0 .35rem;font-size:.72rem;font-weight:700;letter-spacing:.08em;text-transform:uppercase;color:var(--soft)}
.mv-sidebar ul{list-style:none;margin:0;padding:0}
.mv-sidebar li a{display:block;padding:.28rem .6rem;border-radius:7px;color:var(--fg);font-size:.92rem}
.mv-sidebar li a:hover{background:var(--muted);text-decoration:none}
.mv-sidebar li a.active{background:var(--p);color:#fff;font-weight:600}
.mv-content{padding:2.5rem 3rem 5rem;min-width:0}
.mv-prose{max-width:760px}
.mv-prose h1{font-size:2.3rem;font-weight:800;margin:0 0 1rem;letter-spacing:-.02em}
.mv-prose h2{font-size:1.5rem;margin:2.4rem 0 .8rem;padding-top:.5rem;border-top:1px solid var(--border)}
.mv-prose h3{font-size:1.15rem;margin:1.8rem 0 .5rem}
.mv-prose p{margin:.9rem 0}
.mv-prose ul,.mv-prose ol{margin:.8rem 0;padding-left:1.4rem}
.mv-prose li{margin:.3rem 0}
.mv-prose blockquote{margin:1.2rem 0;padding:.6rem 1.1rem;border-left:4px solid var(--p);background:var(--muted);border-radius:0 8px 8px 0;color:#3f3d56}
.mv-prose code{font-family:var(--mono);font-size:.88em;background:var(--muted);padding:.12em .4em;border-radius:5px;color:var(--p2)}
.mv-prose pre{margin:1.1rem 0;padding:1.1rem 1.25rem;background:var(--code-bg);color:var(--code-fg);border-radius:12px;overflow-x:auto;font-size:.86rem;line-height:1.6}
.mv-prose pre code{background:none;color:inherit;padding:0;font-size:inherit}
.mv-prose pre.lang-razor{border-left:4px solid #635bff}
.mv-prose pre.lang-motoko{border-left:4px solid #7048e8}
.mv-prose pre.lang-bash{border-left:4px solid #16a34a}
.mv-prose pre.lang-json{border-left:4px solid #d97706}
.mv-doc-table{width:100%;border-collapse:collapse;margin:1.2rem 0;font-size:.92rem}
.mv-doc-table th,.mv-doc-table td{text-align:left;padding:.55rem .7rem;border-bottom:1px solid var(--border);vertical-align:top}
.mv-doc-table th{color:var(--soft);font-size:.78rem;text-transform:uppercase;letter-spacing:.04em}
.mv-prose hr{border:none;border-top:1px solid var(--border);margin:2rem 0}
.mv-pager{display:flex;justify-content:space-between;max-width:760px;margin-top:3rem;padding-top:1.5rem;border-top:1px solid var(--border)}
.mv-pager a{font-weight:600}
@media(max-width:820px){.mv-layout{grid-template-columns:1fr}.mv-sidebar{position:static;height:auto;border-right:none;border-bottom:1px solid var(--border)}.mv-content{padding:1.5rem}}
"""


if __name__ == "__main__":
    main()
