#!/usr/bin/env python3
"""
Generate apps/site/src/Services/Docs.mo from ../../docs/*.md.

This is the content pipeline for the MotoView documentation site — which is
itself a MotoView canister (dogfooding). It reads the markdown docs (with
`--- title/section/slug ---` frontmatter), converts them to HTML, and emits a
Motoko `Docs` service holding each page plus the sidebar nav. The MotoView
`Doc.mview` page then serves them with `@raw(...)` as @cacheable certified
queries.

Stdlib only. Run:  python3 apps/site/tools/gen-docs.py
"""
import os
import re
import html

HERE = os.path.dirname(os.path.abspath(__file__))
DOCS = os.path.normpath(os.path.join(HERE, "..", "..", "..", "docs"))
OUT = os.path.normpath(os.path.join(HERE, "..", "src", "Services", "Docs.mo"))

# Sidebar order: (Section, [slugs]). persistence added under Interactivity.
NAV = [
    ("Prologue", ["introduction", "why-motoview"]),
    ("Getting Started", ["installation", "quickstart", "project-structure"]),
    ("The .mview Format", ["mview-files", "pages-and-routing", "layouts", "components", "control-flow"]),
    ("Interactivity", ["events", "forms", "validation", "state", "persistence", "drag-drop-and-effects"]),
    ("Styling", ["styling-and-themes", "svg"]),
    ("Architecture", ["protocol", "security", "client-bridge", "runtime"]),
    ("Deployment", ["deployment", "cli"]),
    ("Tooling", ["ai-tools"]),
    ("Reference", ["directives-reference", "roadmap"]),
]


def read_doc(slug, section):
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
    return {"slug": slug, "title": title, "section": section, "body": body}


# ---- Markdown -> HTML (ported from site/build.py) -------------------------

def rewrite_link(url):
    # local markdown links -> MotoView doc routes: foo.md -> /docs/foo
    m = re.match(r"^(?:\./)?(?:docs/)?([\w-]+)\.md(#.*)?$", url)
    if m:
        return "/docs/" + m.group(1) + (m.group(2) or "")
    return url


def inline(text):
    text = html.escape(text, quote=False)
    text = re.sub(r"`([^`]+)`", lambda m: "<code>" + m.group(1) + "</code>", text)
    text = re.sub(r"\[([^\]]+)\]\(([^)]+)\)",
                  lambda m: '<a href="%s">%s</a>' % (rewrite_link(m.group(2)), m.group(1)), text)
    text = re.sub(r"\*\*([^*]+)\*\*", r"<strong>\1</strong>", text)
    text = re.sub(r"(?<![\w*])\*([^*\n]+)\*(?![\w*])", r"<em>\1</em>", text)
    return text


def render_markdown(md):
    lines = md.split("\n")
    out = []
    i, n = 0, len(lines)
    while i < n:
        line = lines[i]
        fence = re.match(r"^```(\w*)\s*$", line)
        if fence:
            lang = fence.group(1) or "text"
            i += 1
            code = []
            while i < n and not lines[i].startswith("```"):
                code.append(lines[i]); i += 1
            i += 1
            esc = html.escape("\n".join(code), quote=False)
            out.append('<pre class="lang-%s"><code>%s</code></pre>' % (lang, esc))
            continue
        h = re.match(r"^(#{1,4})\s+(.*)$", line)
        if h:
            level = len(h.group(1))
            txt = inline(h.group(2).strip())
            anchor = re.sub(r"[^a-z0-9]+", "-", h.group(2).strip().lower()).strip("-")
            out.append('<h%d id="%s">%s</h%d>' % (level, anchor, txt, level))
            i += 1
            continue
        if re.match(r"^---+\s*$", line):
            out.append("<hr>"); i += 1; continue
        if line.startswith(">"):
            quote = []
            while i < n and lines[i].startswith(">"):
                quote.append(lines[i][1:].lstrip()); i += 1
            out.append("<blockquote>" + inline(" ".join(quote)) + "</blockquote>")
            continue
        if "|" in line and i + 1 < n and re.match(r"^\s*\|?[\s:|-]+\|[\s:|-]*$", lines[i + 1]):
            header = [c.strip() for c in line.strip().strip("|").split("|")]
            i += 2
            rows = []
            while i < n and "|" in lines[i] and lines[i].strip():
                rows.append([c.strip() for c in lines[i].strip().strip("|").split("|")]); i += 1
            t = ['<table class="mv-doc-table"><thead><tr>']
            t += ["<th>%s</th>" % inline(c) for c in header]
            t.append("</tr></thead><tbody>")
            for r in rows:
                t.append("<tr>" + "".join("<td>%s</td>" % inline(c) for c in r) + "</tr>")
            t.append("</tbody></table>")
            out.append("".join(t)); continue
        lm = re.match(r"^(\s*)([-*]|\d+\.)\s+(.*)$", line)
        if lm:
            ordered = bool(re.match(r"\d+\.", lm.group(2)))
            tag = "ol" if ordered else "ul"
            items = []
            while i < n:
                m2 = re.match(r"^(\s*)([-*]|\d+\.)\s+(.*)$", lines[i])
                if not m2:
                    break
                items.append("<li>" + inline(m2.group(3).strip()) + "</li>"); i += 1
            out.append("<%s>%s</%s>" % (tag, "".join(items), tag)); continue
        if line.strip() == "":
            i += 1; continue
        para = [line]; i += 1
        while i < n and lines[i].strip() != "" and not re.match(r"^(#{1,4}\s|```|>|\s*[-*]\s|\s*\d+\.\s|---+\s*$)", lines[i]):
            para.append(lines[i]); i += 1
        out.append("<p>" + inline(" ".join(s.strip() for s in para)) + "</p>")
    return "\n".join(out)


# ---- Motoko emission ------------------------------------------------------

def mo_escape(s):
    return (s.replace("\\", "\\\\").replace('"', '\\"')
             .replace("\r", "").replace("\n", "\\n").replace("\t", "\\t"))


def main():
    flat = [(slug, section) for section, slugs in NAV for slug in slugs]
    docs = []
    for slug, section in flat:
        d = read_doc(slug, section)
        if d:
            docs.append(d)
        else:
            print("  WARN: missing docs/%s.md" % slug)
    by_slug = {d["slug"]: d for d in docs}

    out = []
    out.append("/// MotoView documentation content — GENERATED by")
    out.append("/// apps/site/tools/gen-docs.py from /docs/*.md. Do not edit by hand.")
    out.append('import Text "mo:base/Text";')
    out.append("")
    out.append("module {")
    out.append("  public class Docs() {")
    out.append("    public type Doc = {")
    out.append("      slug : Text; title : Text; section : Text; html : Text;")
    out.append("      prevSlug : Text; prevTitle : Text; nextSlug : Text; nextTitle : Text;")
    out.append("    };")
    out.append("")
    out.append('    public func firstSlug() : Text { "%s" };' % docs[0]["slug"])
    out.append("")
    out.append("    public func get(slug : Text) : ?Doc {")
    out.append("      switch (slug) {")
    for idx, d in enumerate(docs):
        prev = docs[idx - 1] if idx > 0 else None
        nxt = docs[idx + 1] if idx + 1 < len(docs) else None
        rec = ('?{ slug = "%s"; title = "%s"; section = "%s"; html = "%s"; '
               'prevSlug = "%s"; prevTitle = "%s"; nextSlug = "%s"; nextTitle = "%s" }') % (
            d["slug"], mo_escape(d["title"]), mo_escape(d["section"]),
            mo_escape(render_markdown(d["body"])),
            prev["slug"] if prev else "", mo_escape(prev["title"]) if prev else "",
            nxt["slug"] if nxt else "", mo_escape(nxt["title"]) if nxt else "")
        out.append('        case ("%s") { %s };' % (d["slug"], rec))
    out.append("        case (_) { null };")
    out.append("      };")
    out.append("    };")
    out.append("")
    out.append("    func navLink(active : Text, slug : Text, title : Text) : Text {")
    out.append('      "<li><a" # (if (active == slug) " class=\\"active\\"" else "") # " href=\\"/docs/" # slug # "\\">" # title # "</a></li>";')
    out.append("    };")
    out.append("")
    out.append("    public func sidebar(active : Text) : Text {")
    out.append('      var s = "";')
    for section, slugs in NAV:
        out.append('      s #= "<div class=\\"mv-nav-section\\">%s</div><ul>";' % mo_escape(section))
        for slug in slugs:
            d = by_slug.get(slug)
            if not d:
                continue
            out.append('      s #= navLink(active, "%s", "%s");' % (slug, mo_escape(d["title"])))
        out.append('      s #= "</ul>";')
    out.append("      s;")
    out.append("    };")
    out.append("")
    out.append("    public func sidebarForPath(path : Text) : Text {")
    out.append('      let active = switch (Text.stripStart(path, #text "/docs/")) { case (?x) x; case null "" };')
    out.append("      sidebar(active);")
    out.append("    };")
    out.append("  };")
    out.append("}")

    os.makedirs(os.path.dirname(OUT), exist_ok=True)
    with open(OUT, "w", encoding="utf-8") as f:
        f.write("\n".join(out) + "\n")
    print("generated %s (%d docs)" % (os.path.relpath(OUT), len(docs)))


if __name__ == "__main__":
    main()
