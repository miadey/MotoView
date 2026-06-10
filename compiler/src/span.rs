//! Source spans for the `.mview` AST.
//!
//! A [`Span`] is a half-open range of **character offsets** into the source —
//! the same unit the parser's cursor (`Parser::i`) uses, i.e. indices into
//! `source.chars().collect::<Vec<char>>()`. Spans are *additive metadata*: they
//! are recorded by the parser but never consulted by codegen, so the emitted
//! Motoko stays byte-identical whether or not spans are populated.
//!
//! Offsets are kept lean (start/end only). Human-facing line/column positions
//! are derived on demand by [`line_col`] — used later by diagnostics / the
//! language server (R2/R3), not stored on every node.
//!
//! Several helpers here (`line_col`, `Span::slice`, …) are deliberately ahead of
//! their first non-test consumer (R2/R3), so `#![allow(dead_code)]` keeps the
//! build clean until then; the unit tests below exercise them now.
#![allow(dead_code)]

/// A half-open `[start, end)` range of character offsets into the `.mview`
/// source. `start == end` denotes an empty (zero-width) span.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Span {
    /// First character offset (inclusive).
    pub start: usize,
    /// One-past-the-last character offset (exclusive).
    pub end: usize,
}

impl Span {
    /// A span covering `[start, end)`.
    pub fn new(start: usize, end: usize) -> Self {
        Span { start, end }
    }

    /// A zero-width span at `offset` (`start == end == offset`).
    pub fn empty_at(offset: usize) -> Self {
        Span { start: offset, end: offset }
    }

    /// Number of characters the span covers (`0` for an empty span).
    pub fn len(&self) -> usize {
        self.end.saturating_sub(self.start)
    }

    /// Whether the span covers no characters.
    pub fn is_empty(&self) -> bool {
        self.end <= self.start
    }

    /// The source slice this span covers, given the same `&[char]` the offsets
    /// index into. Out-of-range / inverted spans yield an empty string rather
    /// than panicking (diagnostics must never crash the compiler).
    pub fn slice<'a>(&self, src: &'a [char]) -> String {
        let start = self.start.min(src.len());
        let end = self.end.min(src.len());
        if end <= start {
            String::new()
        } else {
            src[start..end].iter().collect()
        }
    }
}

/// Map a character `offset` into `src` to a **1-based** `(line, column)`.
///
/// `src` is the character slice the parser indexes (`source.chars().collect()`),
/// so `offset` is a *character* offset — multi-byte UTF-8 characters count as
/// one column each, matching how editors render columns. A `'\n'` ends a line;
/// the character *after* it is column 1 of the next line. An `offset` past the
/// end clamps to the end of the source (so EOF diagnostics still get a position).
pub fn line_col(src: &[char], offset: usize) -> (u32, u32) {
    let end = offset.min(src.len());
    let mut line: u32 = 1;
    let mut col: u32 = 1;
    for &c in &src[..end] {
        if c == '\n' {
            line += 1;
            col = 1;
        } else {
            col += 1;
        }
    }
    (line, col)
}

/// [`line_col`] over a `&str` convenience wrapper (collects to chars first).
/// Prefer the `&[char]` form on hot paths; this exists for callers that only
/// have the original string.
pub fn line_col_str(src: &str, offset: usize) -> (u32, u32) {
    let chars: Vec<char> = src.chars().collect();
    line_col(&chars, offset)
}

#[cfg(test)]
mod span_self_tests {
    use super::*;

    #[test]
    fn line_col_basics() {
        let src: Vec<char> = "ab\ncd\nef".chars().collect();
        // offset 0 -> line 1 col 1
        assert_eq!(line_col(&src, 0), (1, 1));
        // 'b' at offset 1 -> line 1 col 2
        assert_eq!(line_col(&src, 1), (1, 2));
        // the '\n' at offset 2 -> still line 1 (col 3); next char starts line 2
        assert_eq!(line_col(&src, 2), (1, 3));
        // 'c' at offset 3 -> line 2 col 1
        assert_eq!(line_col(&src, 3), (2, 1));
        // 'e' at offset 6 -> line 3 col 1
        assert_eq!(line_col(&src, 6), (3, 1));
    }

    #[test]
    fn line_col_clamps_past_end() {
        let src: Vec<char> = "x\ny".chars().collect();
        // past EOF clamps to end (line 2 col 2, just after 'y')
        assert_eq!(line_col(&src, 999), (2, 2));
    }

    #[test]
    fn span_slice_and_len() {
        let src: Vec<char> = "hello world".chars().collect();
        let s = Span::new(6, 11);
        assert_eq!(s.slice(&src), "world");
        assert_eq!(s.len(), 5);
        assert!(!s.is_empty());
        assert!(Span::empty_at(3).is_empty());
        // out-of-range slice yields empty, never panics
        assert_eq!(Span::new(100, 200).slice(&src), "");
    }
}
