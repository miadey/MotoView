//! Tiny, dependency-free `.mview` syntax classifier.
//!
//! It is intentionally PURE (no egui types): it turns a line of `.mview` source
//! into a list of `(range, TokenClass)` spans. The egui editor wraps this in a
//! `TextEdit` layouter that maps each `TokenClass` to a color. Keeping it pure
//! means the tokenizer is unit-tested with no window.
//!
//! `.mview` is HTML-ish plus MotoView's directives:
//!   * `@page`, `@layout`, `@title`, `@code`, `@if`, `@authorize`, `@yield`,
//!     `@head`, ... — directives / control words (start with `@`).
//!   * `<tag ...>` / `</tag>` — markup tags.
//!   * `"..."` strings, `<!-- ... -->` comments.
//!   * `@code { ... }` blocks contain Motoko; we keep it simple and just
//!     highlight strings, comments, `@`-words, and tags everywhere.

/// A classified token class. The editor maps each to a color.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TokenClass {
    /// Plain text / Motoko code we don't specially color.
    Plain,
    /// A MotoView directive or control word starting with `@`.
    Directive,
    /// An HTML-ish tag, from `<` through the matching `>`.
    Tag,
    /// A double-quoted string literal.
    StringLit,
    /// A comment (`<!-- -->` HTML or `//` line comment).
    Comment,
}

/// One classified span within a single line: `[start, end)` byte offsets.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Span {
    pub start: usize,
    pub end: usize,
    pub class: TokenClass,
}

/// Classify a single line of `.mview` into ordered, non-overlapping spans that
/// cover the entire line (every byte belongs to exactly one span).
///
/// This is line-local (it does not track multi-line string/comment state),
/// which is plenty for a lightweight editor highlighter and keeps it pure.
pub fn classify_line(line: &str) -> Vec<Span> {
    let bytes = line.as_bytes();
    let n = bytes.len();
    let mut spans: Vec<Span> = Vec::new();
    let mut i = 0usize;
    let mut plain_start = 0usize;

    // flush the pending plain run [plain_start, upto)
    let flush_plain = |spans: &mut Vec<Span>, plain_start: usize, upto: usize| {
        if upto > plain_start {
            spans.push(Span {
                start: plain_start,
                end: upto,
                class: TokenClass::Plain,
            });
        }
    };

    while i < n {
        let c = bytes[i];

        // HTML comment <!-- ... --> (to EOL if unterminated on this line)
        if line[i..].starts_with("<!--") {
            flush_plain(&mut spans, plain_start, i);
            let rest = &line[i..];
            let end = rest.find("-->").map(|p| i + p + 3).unwrap_or(n);
            spans.push(Span { start: i, end, class: TokenClass::Comment });
            i = end;
            plain_start = i;
            continue;
        }

        // `//` line comment (Motoko, inside @code) -> to EOL
        if c == b'/' && i + 1 < n && bytes[i + 1] == b'/' {
            flush_plain(&mut spans, plain_start, i);
            spans.push(Span { start: i, end: n, class: TokenClass::Comment });
            i = n;
            plain_start = i;
            continue;
        }

        // Double-quoted string
        if c == b'"' {
            flush_plain(&mut spans, plain_start, i);
            let mut j = i + 1;
            while j < n {
                if bytes[j] == b'\\' && j + 1 < n {
                    j += 2;
                    continue;
                }
                if bytes[j] == b'"' {
                    j += 1;
                    break;
                }
                j += 1;
            }
            spans.push(Span { start: i, end: j, class: TokenClass::StringLit });
            i = j;
            plain_start = i;
            continue;
        }

        // HTML tag `<...>` (including `</...>`). Not a comment (handled above).
        if c == b'<' {
            // Only treat as a tag if the next char looks like a tag start.
            let next = bytes.get(i + 1).copied();
            let looks_tag = matches!(next, Some(b) if b == b'/' || b.is_ascii_alphabetic() || b == b'!');
            if looks_tag {
                flush_plain(&mut spans, plain_start, i);
                let rest = &line[i..];
                let end = rest.find('>').map(|p| i + p + 1).unwrap_or(n);
                spans.push(Span { start: i, end, class: TokenClass::Tag });
                i = end;
                plain_start = i;
                continue;
            }
        }

        // MotoView directive `@word`
        if c == b'@' {
            // peek for an identifier after @ (or @@ etc.)
            let mut j = i + 1;
            while j < n && (bytes[j].is_ascii_alphanumeric() || bytes[j] == b'_') {
                j += 1;
            }
            if j > i + 1 {
                flush_plain(&mut spans, plain_start, i);
                spans.push(Span { start: i, end: j, class: TokenClass::Directive });
                i = j;
                plain_start = i;
                continue;
            }
        }

        i += 1;
    }

    flush_plain(&mut spans, plain_start, n);
    spans
}

#[cfg(test)]
mod tests {
    use super::*;

    fn classes(line: &str) -> Vec<(String, TokenClass)> {
        classify_line(line)
            .into_iter()
            .map(|s| (line[s.start..s.end].to_string(), s.class))
            .collect()
    }

    #[test]
    fn spans_cover_whole_line() {
        let line = "  <h1>Hello @name</h1>  ";
        let spans = classify_line(line);
        // contiguous + covering
        assert_eq!(spans.first().map(|s| s.start), Some(0));
        assert_eq!(spans.last().map(|s| s.end), Some(line.len()));
        for w in spans.windows(2) {
            assert_eq!(w[0].end, w[1].start, "spans must be contiguous");
        }
    }

    #[test]
    fn directive_is_classified() {
        let c = classes("@page \"/greet\"");
        assert_eq!(c[0].0, "@page");
        assert_eq!(c[0].1, TokenClass::Directive);
        // the quoted route is a string
        assert!(c.iter().any(|(t, k)| t == "\"/greet\"" && *k == TokenClass::StringLit));
    }

    #[test]
    fn tags_and_string_and_directive_together() {
        let c = classes("<form @submit=\"submit\" secure>");
        assert!(c.iter().any(|(t, k)| t.starts_with("<form") && *k == TokenClass::Tag));
        // Note: the whole `<...>` is one Tag span, so @submit/"submit" inside it
        // are part of the tag — that's fine and expected for a line classifier.
    }

    #[test]
    fn html_comment_classified() {
        let c = classes("<!-- hidden --> visible");
        assert_eq!(c[0].0, "<!-- hidden -->");
        assert_eq!(c[0].1, TokenClass::Comment);
        assert!(c.iter().any(|(t, k)| t == " visible" && *k == TokenClass::Plain));
    }

    #[test]
    fn line_comment_to_eol() {
        let c = classes("var x = 1; // a note");
        assert!(c.iter().any(|(t, k)| t == "// a note" && *k == TokenClass::Comment));
    }

    #[test]
    fn less_than_in_text_is_not_a_tag() {
        let c = classes("a < b is plain");
        // The `<` is not followed by a tag-ish char, so it stays Plain.
        assert!(c.iter().all(|(_, k)| *k == TokenClass::Plain));
    }
}
