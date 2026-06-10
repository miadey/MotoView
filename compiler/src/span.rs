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

/// A generated-actor -> `.mview`-source line map (R11). It is a **side artifact**:
/// codegen records it as the actor String is assembled, but never emits anything
/// into the actor itself, so `main.mo` stays byte-identical (the 130 golden tests
/// don't move). It exists only to remap `moc` type errors — which are reported at
/// the GENERATED `main.mo` line — back to the originating `.mview` line.
///
/// Each [`MapEntry`] anchors ONE contiguous, line-preserving region (today: each
/// `@code` function body, which is emitted near-verbatim). For a `moc` error at
/// generated line `G`, [`resolve`] finds the entry whose generated range covers
/// `G` and returns `(file, src_line)` by **linear extrapolation**:
/// `src_line = src_start_line + (G - gen_start_line)`. That is exact for a body
/// emitted line-for-line; it only drifts if a transform (e.g. `await`-stripping
/// across a newline) adds/removes a line *inside* the body, which is rare.
#[derive(Debug, Clone, Default)]
pub struct SourceMap {
    pub entries: Vec<MapEntry>,
}

/// One line-preserving region: `[gen_start_line, gen_end_line]` in the generated
/// actor maps onto the `.mview` `file`, starting at `src_start_line`, line-for-line.
/// All lines are **1-based**, matching `moc`'s reporting and [`line_col`].
#[derive(Debug, Clone)]
pub struct MapEntry {
    /// `.mview` source path (project-relative, e.g. `src/Pages/Counter.mview`).
    pub file: String,
    /// First generated `main.mo` line this region occupies (1-based, inclusive).
    pub gen_start_line: usize,
    /// Last generated `main.mo` line this region occupies (1-based, inclusive).
    pub gen_end_line: usize,
    /// The `.mview` line that `gen_start_line` corresponds to (1-based).
    pub src_start_line: usize,
}

impl SourceMap {
    pub fn new() -> Self {
        SourceMap { entries: Vec::new() }
    }

    /// Record a region: generated `[gen_start_line, gen_end_line]` maps to `file`
    /// starting at `.mview` line `src_start_line`, line-for-line.
    pub fn push(&mut self, file: String, gen_start_line: usize, gen_end_line: usize, src_start_line: usize) {
        self.entries.push(MapEntry { file, gen_start_line, gen_end_line, src_start_line });
    }

    /// Resolve a generated `main.mo` line to `(.mview file, .mview line)`.
    /// Returns `None` when no mapped region covers `gen_line` (e.g. a moc error in
    /// template/boilerplate that no entry spans) — callers then fall back to the
    /// `// mv:src` FILE markers / the generated line.
    pub fn resolve(&self, gen_line: usize) -> Option<(String, usize)> {
        // Most-specific (last-pushed covering) entry wins, mirroring how nested
        // regions are emitted innermost-last.
        self.entries
            .iter()
            .rev()
            .find(|e| gen_line >= e.gen_start_line && gen_line <= e.gen_end_line)
            .map(|e| {
                let src_line = e.src_start_line + (gen_line - e.gen_start_line);
                (e.file.clone(), src_line)
            })
    }

    /// Serialize to the on-disk `.mvbuild/main.mo.map` format: one
    /// `gen_start gen_end src_start <file>` record per line (space-separated, path
    /// last so it may contain spaces). A trivial text format — no JSON dep — that
    /// `check` reads back via [`SourceMap::parse`].
    pub fn to_text(&self) -> String {
        let mut out = String::new();
        for e in &self.entries {
            out.push_str(&format!(
                "{} {} {} {}\n",
                e.gen_start_line, e.gen_end_line, e.src_start_line, e.file
            ));
        }
        out
    }

    /// Parse the on-disk format produced by [`SourceMap::to_text`]. Malformed or
    /// blank lines are skipped (a stale/partial map must never crash `check`).
    pub fn parse(text: &str) -> Self {
        let mut m = SourceMap::new();
        for line in text.lines() {
            let mut it = line.splitn(4, ' ');
            let g0 = it.next().and_then(|x| x.parse::<usize>().ok());
            let g1 = it.next().and_then(|x| x.parse::<usize>().ok());
            let s0 = it.next().and_then(|x| x.parse::<usize>().ok());
            let file = it.next();
            if let (Some(g0), Some(g1), Some(s0), Some(file)) = (g0, g1, s0, file) {
                m.push(file.to_string(), g0, g1, s0);
            }
        }
        m
    }
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
