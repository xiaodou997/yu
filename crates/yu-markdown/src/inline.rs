use std::error::Error;
use std::fmt;
use std::iter::Peekable;

use yu_core::{ByteOffset, Revision, TextRange};
use yu_text::{ChunkCursor, TextPositionError, TextSnapshot};

/// A delimiter family recognized by the phase-one inline token scanner.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum InlineDelimiter {
    Star,
    Underscore,
    Code,
}

impl InlineDelimiter {
    #[must_use]
    pub const fn marker(self) -> u8 {
        match self {
            Self::Star => b'*',
            Self::Underscore => b'_',
            Self::Code => 0x60,
        }
    }
}

/// A lossless token kind in an inline source range.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum InlineNodeKind {
    /// Ordinary source bytes that are not syntax delimiters.
    Text,
    /// A backslash escape marker and its escaped scalar, when present.
    Escaped,
    /// A contiguous run of one delimiter family.
    Delimiter { marker: InlineDelimiter },
}

/// A semantic inline span recognized from a matched delimiter pair.
///
/// The span keeps every source range needed by projection and editing. It does
/// not own or normalize the text, and unmatched delimiters intentionally do
/// not produce a span.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum InlineSpanKind {
    Emphasis,
    Strong,
    CodeSpan,
}

/// Source ranges for one parser-owned semantic inline span.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct InlineSpan {
    kind: InlineSpanKind,
    source_range: TextRange,
    opening: TextRange,
    content: TextRange,
    closing: TextRange,
}

impl InlineSpan {
    #[must_use]
    pub const fn kind(self) -> InlineSpanKind {
        self.kind
    }

    #[must_use]
    pub const fn source_range(self) -> TextRange {
        self.source_range
    }

    #[must_use]
    pub const fn opening(self) -> TextRange {
        self.opening
    }

    #[must_use]
    pub const fn content(self) -> TextRange {
        self.content
    }

    #[must_use]
    pub const fn closing(self) -> TextRange {
        self.closing
    }
}

/// A source-backed inline token.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct InlineNode {
    kind: InlineNodeKind,
    range: TextRange,
}

impl InlineNode {
    #[must_use]
    pub const fn kind(self) -> InlineNodeKind {
        self.kind
    }

    #[must_use]
    pub const fn range(self) -> TextRange {
        self.range
    }
}

/// Errors raised while building the lossless inline token stream.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum InlineParseError {
    SourcePosition(TextPositionError),
    OffsetOverflow,
}

impl fmt::Display for InlineParseError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::SourcePosition(error) => error.fmt(formatter),
            Self::OffsetOverflow => formatter.write_str("inline source offset overflow"),
        }
    }
}

impl Error for InlineParseError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::SourcePosition(error) => Some(error),
            Self::OffsetOverflow => None,
        }
    }
}

impl From<TextPositionError> for InlineParseError {
    fn from(error: TextPositionError) -> Self {
        Self::SourcePosition(error)
    }
}

/// A lossless inline token stream for one immutable source revision and range.
#[derive(Clone, Debug)]
pub struct InlineDocument {
    source: TextSnapshot,
    source_range: TextRange,
    nodes: Vec<InlineNode>,
    spans: Vec<InlineSpan>,
}

impl InlineDocument {
    #[must_use]
    pub fn revision(&self) -> Revision {
        self.source.revision()
    }

    #[must_use]
    pub fn source(&self) -> &TextSnapshot {
        &self.source
    }

    #[must_use]
    pub const fn source_range(&self) -> TextRange {
        self.source_range
    }

    #[must_use]
    pub fn nodes(&self) -> &[InlineNode] {
        &self.nodes
    }

    /// Returns parser-owned semantic spans in source order.
    #[must_use]
    pub fn spans(&self) -> &[InlineSpan] {
        &self.spans
    }

    /// Confirms that the ordered token ranges cover this source range exactly.
    #[must_use]
    pub fn has_lossless_coverage(&self) -> bool {
        let mut expected = self.source_range.start();
        for node in &self.nodes {
            if node.range.start() != expected {
                return false;
            }
            expected = node.range.end();
        }
        expected == self.source_range.end()
    }
}

/// Builds a lossless, chunk-aware inline token stream.
///
/// This is intentionally a token layer rather than a complete CommonMark
/// inline parser. Every source byte remains represented by one ordered node;
/// projection and later semantic parsing can use delimiter ranges without
/// copying the source text.
pub fn parse_inline(
    source: &TextSnapshot,
    source_range: TextRange,
) -> Result<InlineDocument, InlineParseError> {
    source.utf16_offset(source_range.start())?;
    source.utf16_offset(source_range.end())?;

    let mut cursor = InlineByteCursor::new(source, source_range)?.peekable();
    let mut nodes = Vec::new();
    let mut text_start = None;

    while let Some((start, byte)) = cursor.next() {
        if byte == b'\\' {
            flush_text(&mut nodes, &mut text_start, start)?;
            let end = consume_escaped_scalar(&mut cursor, start)?;
            nodes.push(InlineNode {
                kind: InlineNodeKind::Escaped,
                range: byte_range(start, end)?,
            });
            continue;
        }

        let Some(marker) = delimiter_for(byte) else {
            text_start.get_or_insert(start);
            continue;
        };

        flush_text(&mut nodes, &mut text_start, start)?;
        let mut end = start
            .checked_add(1)
            .ok_or(InlineParseError::OffsetOverflow)?;
        while cursor.peek().is_some_and(|(_, next)| *next == byte) {
            let (next_start, _) = cursor.next().expect("peeked delimiter must be available");
            end = next_start
                .checked_add(1)
                .ok_or(InlineParseError::OffsetOverflow)?;
        }
        nodes.push(InlineNode {
            kind: InlineNodeKind::Delimiter { marker },
            range: byte_range(start, end)?,
        });
    }

    flush_text(
        &mut nodes,
        &mut text_start,
        usize::try_from(source_range.end()).map_err(|_| InlineParseError::OffsetOverflow)?,
    )?;

    let spans = build_spans(&nodes)?;

    Ok(InlineDocument {
        source: source.clone(),
        source_range,
        nodes,
        spans,
    })
}

fn delimiter_for(byte: u8) -> Option<InlineDelimiter> {
    match byte {
        b'*' => Some(InlineDelimiter::Star),
        b'_' => Some(InlineDelimiter::Underscore),
        0x60 => Some(InlineDelimiter::Code),
        _ => None,
    }
}

fn flush_text(
    nodes: &mut Vec<InlineNode>,
    text_start: &mut Option<usize>,
    end: usize,
) -> Result<(), InlineParseError> {
    let Some(start) = text_start.take() else {
        return Ok(());
    };
    if start < end {
        nodes.push(InlineNode {
            kind: InlineNodeKind::Text,
            range: byte_range(start, end)?,
        });
    }
    Ok(())
}

fn consume_escaped_scalar(
    cursor: &mut Peekable<InlineByteCursor<'_>>,
    start: usize,
) -> Result<usize, InlineParseError> {
    let Some((next_start, next_byte)) = cursor.next() else {
        return start.checked_add(1).ok_or(InlineParseError::OffsetOverflow);
    };
    let mut end = next_start
        .checked_add(1)
        .ok_or(InlineParseError::OffsetOverflow)?;
    if next_byte >= 0x80 {
        while cursor.peek().is_some_and(|(_, byte)| (byte & 0xc0) == 0x80) {
            let (continuation_start, _) = cursor
                .next()
                .expect("peeked UTF-8 continuation must be available");
            end = continuation_start
                .checked_add(1)
                .ok_or(InlineParseError::OffsetOverflow)?;
        }
    }
    Ok(end)
}

fn byte_range(start: usize, end: usize) -> Result<TextRange, InlineParseError> {
    let start = ByteOffset::try_from(start).map_err(|_| InlineParseError::OffsetOverflow)?;
    let end = ByteOffset::try_from(end).map_err(|_| InlineParseError::OffsetOverflow)?;
    TextRange::new(start, end).ok_or(InlineParseError::OffsetOverflow)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Delimiter {
    marker: InlineDelimiter,
    len: usize,
    range: TextRange,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct DelimiterPair {
    opening: Delimiter,
    closing: Delimiter,
}

fn build_spans(nodes: &[InlineNode]) -> Result<Vec<InlineSpan>, InlineParseError> {
    let delimiters = nodes
        .iter()
        .filter_map(|node| match node.kind() {
            InlineNodeKind::Delimiter { marker } => Some((marker, node.range())),
            InlineNodeKind::Text | InlineNodeKind::Escaped => None,
        })
        .map(|(marker, range)| {
            Ok(Delimiter {
                marker,
                len: usize::try_from(range.len()).map_err(|_| InlineParseError::OffsetOverflow)?,
                range,
            })
        })
        .collect::<Result<Vec<_>, InlineParseError>>()?;

    let code_pairs = pair_delimiters(&delimiters, InlineDelimiter::Code);
    let mut spans = code_pairs
        .iter()
        .map(|pair| make_span(*pair, InlineSpanKind::CodeSpan))
        .collect::<Result<Vec<_>, _>>()?;

    let inline_delimiters = delimiters
        .iter()
        .copied()
        .filter(|delimiter| delimiter.marker != InlineDelimiter::Code)
        .filter(|delimiter| {
            !code_pairs.iter().any(|code| {
                code.opening.range.start() < delimiter.range.start()
                    && delimiter.range.end() < code.closing.range.end()
            })
        })
        .collect::<Vec<_>>();
    for marker in [InlineDelimiter::Star, InlineDelimiter::Underscore] {
        for pair in pair_delimiters(&inline_delimiters, marker) {
            let kind = match pair.opening.len {
                1 => InlineSpanKind::Emphasis,
                2 => InlineSpanKind::Strong,
                _ => continue,
            };
            spans.push(make_span(pair, kind)?);
        }
    }

    spans.sort_by_key(|span| {
        (
            span.source_range.start(),
            span.source_range.end(),
            span.kind as u8,
        )
    });
    Ok(spans)
}

fn make_span(pair: DelimiterPair, kind: InlineSpanKind) -> Result<InlineSpan, InlineParseError> {
    let source_range = TextRange::new(pair.opening.range.start(), pair.closing.range.end())
        .ok_or(InlineParseError::OffsetOverflow)?;
    let content = TextRange::new(pair.opening.range.end(), pair.closing.range.start())
        .ok_or(InlineParseError::OffsetOverflow)?;
    Ok(InlineSpan {
        kind,
        source_range,
        opening: pair.opening.range,
        content,
        closing: pair.closing.range,
    })
}

fn pair_delimiters(delimiters: &[Delimiter], marker: InlineDelimiter) -> Vec<DelimiterPair> {
    let mut openings = Vec::new();
    let mut pairs = Vec::new();
    for delimiter in delimiters
        .iter()
        .copied()
        .filter(|item| item.marker == marker)
    {
        if let Some(opening_index) = openings
            .iter()
            .rposition(|opening: &Delimiter| opening.len == delimiter.len)
        {
            let opening = openings.remove(opening_index);
            if opening.range.end() < delimiter.range.start() {
                pairs.push(DelimiterPair {
                    opening,
                    closing: delimiter,
                });
                continue;
            }
        }
        openings.push(delimiter);
    }
    pairs
}

struct InlineByteCursor<'a> {
    chunks: ChunkCursor<'a>,
    requested_start: usize,
    end: usize,
    current: Option<&'a [u8]>,
    current_start: usize,
    current_index: usize,
}

impl<'a> InlineByteCursor<'a> {
    fn new(source: &'a TextSnapshot, range: TextRange) -> Result<Self, InlineParseError> {
        let start = usize::try_from(range.start()).map_err(|_| InlineParseError::OffsetOverflow)?;
        let end = usize::try_from(range.end()).map_err(|_| InlineParseError::OffsetOverflow)?;
        Ok(Self {
            chunks: source.chunk_cursor(range.start())?,
            requested_start: start,
            end,
            current: None,
            current_start: start,
            current_index: start,
        })
    }
}

impl Iterator for InlineByteCursor<'_> {
    type Item = (usize, u8);

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            if let Some(current) = self.current {
                if self.current_index < self.current_start + current.len()
                    && self.current_index < self.end
                {
                    let local = self.current_index - self.current_start;
                    let value = current[local];
                    let position = self.current_index;
                    self.current_index += 1;
                    return Some((position, value));
                }
                self.current = None;
            }

            let chunk = self.chunks.next()?;
            self.current_start = usize::try_from(chunk.start()).ok()?;
            self.current_index = self.current_start.max(self.requested_start);
            self.current = Some(chunk.text().as_bytes());
            if self.current_index < self.end {
                continue;
            }
            return None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use yu_text::{StorageBackend, TextBuffer, retained_snapshot_stats};

    #[test]
    fn inline_tokens_cover_source_and_preserve_delimiter_runs() {
        let source = r"before **羽🙂** \*literal `code *` after";
        let buffer = TextBuffer::new(source);
        let snapshot = buffer.snapshot();
        let range = TextRange::new(ByteOffset::ZERO, snapshot.len_bytes())
            .expect("source range should be ordered");
        let inline = parse_inline(&snapshot, range).expect("inline parse should succeed");

        assert!(inline.has_lossless_coverage());
        assert_eq!(
            inline
                .nodes()
                .iter()
                .filter(|node| matches!(node.kind(), InlineNodeKind::Delimiter { .. }))
                .count(),
            5
        );
        assert!(inline.nodes().iter().any(|node| {
            node.kind() == InlineNodeKind::Escaped
                && &source[usize::try_from(node.range().start().get())
                    .expect("test source offset should fit usize")
                    ..usize::try_from(node.range().end().get())
                        .expect("test source offset should fit usize")]
                    == r"\*"
        }));
    }

    #[test]
    fn inline_tokens_scan_piece_tree_chunks_without_materializing() {
        let mut buffer = TextBuffer::with_backend("", StorageBackend::PieceTree);
        for part in ["**", "羽🙂", "**", " and `", "code", "`"] {
            let at = buffer.snapshot().len_bytes();
            let transaction = yu_text::Transaction::new(
                buffer.revision(),
                [yu_text::Edit::new(TextRange::empty(at), part)],
            );
            buffer.apply(&transaction).expect("append should apply");
        }
        let snapshot = buffer.snapshot();
        let range = TextRange::new(ByteOffset::ZERO, snapshot.len_bytes())
            .expect("source range should be ordered");
        let inline = parse_inline(&snapshot, range).expect("inline parse should succeed");
        assert!(inline.has_lossless_coverage());
        assert_eq!(
            retained_snapshot_stats(std::slice::from_ref(&snapshot)).materialized_buffers(),
            0
        );
    }

    #[test]
    fn inline_parser_rejects_non_boundary_ranges() {
        let snapshot = TextBuffer::new("羽").snapshot();
        let range = TextRange::new(ByteOffset::new(1), ByteOffset::new(3))
            .expect("range should be ordered");
        assert!(matches!(
            parse_inline(&snapshot, range),
            Err(InlineParseError::SourcePosition(
                TextPositionError::NotUtf8Boundary(_)
            ))
        ));
    }

    #[test]
    fn semantic_spans_are_parser_owned_and_preserve_ranges() {
        let source = "**strong** _emphasis_ `code *` *unmatched";
        let snapshot = TextBuffer::new(source).snapshot();
        let range = TextRange::new(ByteOffset::ZERO, snapshot.len_bytes())
            .expect("source range should be ordered");
        let inline = parse_inline(&snapshot, range).expect("inline parse should succeed");

        assert_eq!(inline.spans().len(), 3);
        assert_eq!(inline.spans()[0].kind(), InlineSpanKind::Strong);
        assert_eq!(inline.spans()[1].kind(), InlineSpanKind::Emphasis);
        assert_eq!(inline.spans()[2].kind(), InlineSpanKind::CodeSpan);
        for span in inline.spans() {
            assert_eq!(
                span.source_range(),
                TextRange::new(span.opening().start(), span.closing().end())
                    .expect("span range should be ordered")
            );
            assert!(span.content().start() >= span.opening().end());
            assert!(span.content().end() <= span.closing().start());
        }
    }

    #[test]
    fn semantic_spans_do_not_pair_delimiters_inside_code() {
        let source = "`code **not strong**` **yes**";
        let snapshot = TextBuffer::new(source).snapshot();
        let range = TextRange::new(ByteOffset::ZERO, snapshot.len_bytes())
            .expect("source range should be ordered");
        let inline = parse_inline(&snapshot, range).expect("inline parse should succeed");

        assert_eq!(
            inline
                .spans()
                .iter()
                .filter(|span| span.kind() == InlineSpanKind::CodeSpan)
                .count(),
            1
        );
        assert_eq!(
            inline
                .spans()
                .iter()
                .filter(|span| span.kind() == InlineSpanKind::Strong)
                .count(),
            1
        );
    }
}
