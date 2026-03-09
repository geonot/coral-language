use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct Span {
    pub start: usize,
    pub end: usize,
    pub file_id: u32,
}

impl Span {
    pub fn new(start: usize, end: usize) -> Self {
        Self { start, end, file_id: 0 }
    }

    pub fn with_file(start: usize, end: usize, file_id: u32) -> Self {
        Self { start, end, file_id }
    }

    pub fn join(self, other: Span) -> Span {
        Span {
            start: self.start.min(other.start),
            end: self.end.max(other.end),
            file_id: self.file_id,
        }
    }

    pub fn shift(self, offset: usize) -> Span {
        Span {
            start: self.start.saturating_add(offset),
            end: self.end.saturating_add(offset),
            file_id: self.file_id,
        }
    }
}

impl fmt::Display for Span {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}..{}", self.start, self.end)
    }
}

// ── CC2.1: Source-mapped error locations ─────────────────────────────

/// Pre-computed line-start index for O(log n) byte-offset → line:col lookup.
#[derive(Debug, Clone)]
pub struct LineIndex {
    /// Byte offset of the start of each line (0-indexed lines, 1-indexed in display).
    line_starts: Vec<usize>,
}

impl LineIndex {
    /// Build a `LineIndex` from a source string.
    pub fn new(source: &str) -> Self {
        let mut line_starts = vec![0usize];
        for (i, b) in source.bytes().enumerate() {
            if b == b'\n' {
                line_starts.push(i + 1);
            }
        }
        Self { line_starts }
    }

    /// Convert a byte offset to a 1-based (line, column) pair.
    pub fn line_col(&self, offset: usize) -> (usize, usize) {
        let line_idx = match self.line_starts.binary_search(&offset) {
            Ok(exact) => exact,
            Err(insert) => insert.saturating_sub(1),
        };
        let col = offset.saturating_sub(self.line_starts[line_idx]);
        (line_idx + 1, col + 1)
    }

    /// Format a `Span` as `line:col` (or `line:col-line:col` for multi-line).
    pub fn fmt_span(&self, span: Span) -> String {
        let (sl, sc) = self.line_col(span.start);
        let (el, ec) = self.line_col(span.end.saturating_sub(1).max(span.start));
        if sl == el {
            if sc == ec {
                format!("{}:{}", sl, sc)
            } else {
                format!("{}:{}-{}", sl, sc, ec)
            }
        } else {
            format!("{}:{}-{}:{}", sl, sc, el, ec)
        }
    }

    /// Return the source text for the line containing `offset` (0-indexed line).
    pub fn line_text<'a>(&self, source: &'a str, offset: usize) -> &'a str {
        let (line, _) = self.line_col(offset);
        let start = self.line_starts[line - 1];
        let end = self
            .line_starts
            .get(line)
            .copied()
            .unwrap_or(source.len());
        source[start..end].trim_end_matches('\n').trim_end_matches('\r')
    }
}
