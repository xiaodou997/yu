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
    /// Bracket/parenthesis punctuation used by links and images.
    Punctuation { kind: InlinePunctuation },
    /// A source line ending. Hard breaks include the two trailing spaces or
    /// the backslash that makes the break explicit.
    LineBreak { hard: bool },
}

/// Punctuation retained as individual source-backed nodes for link and
/// autolink parsing.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum InlinePunctuation {
    Bang,
    OpenBracket,
    CloseBracket,
    OpenParen,
    CloseParen,
    AngleOpen,
    AngleClose,
}

/// A semantic inline span recognized by the lossless inline parser.
///
/// The span keeps every source range needed by projection and editing. It does
/// not own or normalize the text, and unmatched delimiters intentionally do
/// not produce a span.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum InlineSpanKind {
    Emphasis,
    Strong,
    CodeSpan,
    Link,
    Image,
    ReferenceLink,
    ReferenceImage,
    Autolink,
}

/// Source ranges for one parser-owned semantic inline span.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct InlineSpan {
    kind: InlineSpanKind,
    source_range: TextRange,
    opening: TextRange,
    content: TextRange,
    closing: TextRange,
    destination: Option<TextRange>,
    reference: Option<TextRange>,
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

    /// Returns an inline link or autolink destination, excluding surrounding
    /// angle brackets and optional whitespace/title syntax.
    #[must_use]
    pub const fn destination(self) -> Option<TextRange> {
        self.destination
    }

    /// Returns the source range inside the brackets of a reference link.
    /// Empty ranges represent collapsed references such as `[label][]`.
    #[must_use]
    pub const fn reference(self) -> Option<TextRange> {
        self.reference
    }
}

/// A source-backed inline token.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct InlineNode {
    kind: InlineNodeKind,
    range: TextRange,
    can_open: bool,
    can_close: bool,
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

    #[must_use]
    const fn can_open(self) -> bool {
        self.can_open
    }

    #[must_use]
    const fn can_close(self) -> bool {
        self.can_close
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
    let mut trailing_space_start = None;
    let mut trailing_space_count = 0_usize;
    let mut previous_byte = None;

    while let Some((start, byte)) = cursor.next() {
        if byte == b'\\' {
            flush_text(&mut nodes, &mut text_start, start)?;
            trailing_space_start = None;
            trailing_space_count = 0;
            if cursor.peek().is_some_and(|(_, next)| is_line_ending(*next)) {
                let end = consume_backslash_line_break(&mut cursor, start)?;
                nodes.push(InlineNode {
                    kind: InlineNodeKind::LineBreak { hard: true },
                    range: byte_range(start, end)?,
                    can_open: false,
                    can_close: false,
                });
                previous_byte = None;
                continue;
            }
            let (end, last_byte) = consume_escaped_scalar(&mut cursor, start)?;
            nodes.push(InlineNode {
                kind: InlineNodeKind::Escaped,
                range: byte_range(start, end)?,
                can_open: false,
                can_close: false,
            });
            previous_byte = Some(last_byte);
            continue;
        }

        if is_line_ending(byte) {
            let end = consume_line_ending(&mut cursor, start, byte)?;
            let break_start = if trailing_space_count >= 2 {
                trailing_space_start.unwrap_or(start)
            } else {
                start
            };
            flush_text(&mut nodes, &mut text_start, break_start)?;
            nodes.push(InlineNode {
                kind: InlineNodeKind::LineBreak {
                    hard: trailing_space_count >= 2,
                },
                range: byte_range(break_start, end)?,
                can_open: false,
                can_close: false,
            });
            trailing_space_start = None;
            trailing_space_count = 0;
            previous_byte = None;
            continue;
        }

        if let Some(kind) = punctuation_for(byte) {
            flush_text(&mut nodes, &mut text_start, start)?;
            nodes.push(InlineNode {
                kind: InlineNodeKind::Punctuation { kind },
                range: byte_range(
                    start,
                    start
                        .checked_add(1)
                        .ok_or(InlineParseError::OffsetOverflow)?,
                )?,
                can_open: false,
                can_close: false,
            });
            trailing_space_start = None;
            trailing_space_count = 0;
            previous_byte = Some(byte);
            continue;
        }

        let Some(marker) = delimiter_for(byte) else {
            text_start.get_or_insert(start);
            if byte == b' ' {
                trailing_space_start.get_or_insert(start);
                trailing_space_count = trailing_space_count.saturating_add(1);
            } else {
                trailing_space_start = None;
                trailing_space_count = 0;
            }
            previous_byte = Some(byte);
            continue;
        };

        flush_text(&mut nodes, &mut text_start, start)?;
        trailing_space_start = None;
        trailing_space_count = 0;
        let mut end = start
            .checked_add(1)
            .ok_or(InlineParseError::OffsetOverflow)?;
        while cursor.peek().is_some_and(|(_, next)| *next == byte) {
            let (next_start, _) = cursor.next().expect("peeked delimiter must be available");
            end = next_start
                .checked_add(1)
                .ok_or(InlineParseError::OffsetOverflow)?;
        }
        let next_byte = cursor.peek().map(|(_, next)| *next);
        let (can_open, can_close) = delimiter_flanking(marker, previous_byte, next_byte);
        nodes.push(InlineNode {
            kind: InlineNodeKind::Delimiter { marker },
            range: byte_range(start, end)?,
            can_open,
            can_close,
        });
        previous_byte = Some(byte);
    }

    flush_text(
        &mut nodes,
        &mut text_start,
        usize::try_from(source_range.end()).map_err(|_| InlineParseError::OffsetOverflow)?,
    )?;

    let spans = build_spans(source, &nodes)?;

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

fn punctuation_for(byte: u8) -> Option<InlinePunctuation> {
    match byte {
        b'!' => Some(InlinePunctuation::Bang),
        b'[' => Some(InlinePunctuation::OpenBracket),
        b']' => Some(InlinePunctuation::CloseBracket),
        b'(' => Some(InlinePunctuation::OpenParen),
        b')' => Some(InlinePunctuation::CloseParen),
        b'<' => Some(InlinePunctuation::AngleOpen),
        b'>' => Some(InlinePunctuation::AngleClose),
        _ => None,
    }
}

fn is_line_ending(byte: u8) -> bool {
    matches!(byte, b'\r' | b'\n')
}

fn delimiter_flanking(
    marker: InlineDelimiter,
    previous: Option<u8>,
    next: Option<u8>,
) -> (bool, bool) {
    if marker == InlineDelimiter::Code {
        return (true, true);
    }
    let previous_whitespace = previous.is_none_or(is_ascii_whitespace);
    let next_whitespace = next.is_none_or(is_ascii_whitespace);
    let previous_punctuation = previous.is_some_and(is_ascii_punctuation);
    let next_punctuation = next.is_some_and(is_ascii_punctuation);
    let left_flanking =
        !next_whitespace && (!next_punctuation || previous_whitespace || previous_punctuation);
    let right_flanking =
        !previous_whitespace && (!previous_punctuation || next_whitespace || next_punctuation);
    if marker == InlineDelimiter::Underscore
        && !previous_whitespace
        && !next_whitespace
        && !previous_punctuation
        && !next_punctuation
    {
        return (false, false);
    }
    (left_flanking, right_flanking)
}

fn is_ascii_whitespace(byte: u8) -> bool {
    matches!(byte, b' ' | b'\t' | b'\r' | b'\n')
}

fn is_ascii_punctuation(byte: u8) -> bool {
    byte.is_ascii_punctuation()
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
            can_open: false,
            can_close: false,
        });
    }
    Ok(())
}

fn consume_escaped_scalar(
    cursor: &mut Peekable<InlineByteCursor<'_>>,
    start: usize,
) -> Result<(usize, u8), InlineParseError> {
    let Some((next_start, next_byte)) = cursor.next() else {
        return Ok((
            start
                .checked_add(1)
                .ok_or(InlineParseError::OffsetOverflow)?,
            b'\\',
        ));
    };
    let mut end = next_start
        .checked_add(1)
        .ok_or(InlineParseError::OffsetOverflow)?;
    let mut last_byte = next_byte;
    if next_byte >= 0x80 {
        while cursor.peek().is_some_and(|(_, byte)| (byte & 0xc0) == 0x80) {
            let (continuation_start, continuation_byte) = cursor
                .next()
                .expect("peeked UTF-8 continuation must be available");
            end = continuation_start
                .checked_add(1)
                .ok_or(InlineParseError::OffsetOverflow)?;
            last_byte = continuation_byte;
        }
    }
    Ok((end, last_byte))
}

fn consume_backslash_line_break(
    cursor: &mut Peekable<InlineByteCursor<'_>>,
    start: usize,
) -> Result<usize, InlineParseError> {
    let (newline_start, newline) = cursor
        .next()
        .expect("peeked line ending must remain available");
    let mut end = newline_start
        .checked_add(1)
        .ok_or(InlineParseError::OffsetOverflow)?;
    if newline == b'\r' && cursor.peek().is_some_and(|(_, next)| *next == b'\n') {
        let (lf_start, _) = cursor
            .next()
            .expect("peeked CRLF line ending must remain available");
        end = lf_start
            .checked_add(1)
            .ok_or(InlineParseError::OffsetOverflow)?;
    }
    if end <= start {
        return Err(InlineParseError::OffsetOverflow);
    }
    Ok(end)
}

fn consume_line_ending(
    cursor: &mut Peekable<InlineByteCursor<'_>>,
    start: usize,
    first: u8,
) -> Result<usize, InlineParseError> {
    let mut end = start
        .checked_add(1)
        .ok_or(InlineParseError::OffsetOverflow)?;
    if first == b'\r' && cursor.peek().is_some_and(|(_, next)| *next == b'\n') {
        let (lf_start, _) = cursor
            .next()
            .expect("peeked CRLF line ending must remain available");
        end = lf_start
            .checked_add(1)
            .ok_or(InlineParseError::OffsetOverflow)?;
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
    can_open: bool,
    can_close: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct DelimiterPair {
    opening: Delimiter,
    closing: Delimiter,
}

fn build_spans(
    source: &TextSnapshot,
    nodes: &[InlineNode],
) -> Result<Vec<InlineSpan>, InlineParseError> {
    let delimiters = nodes
        .iter()
        .filter_map(|node| match node.kind() {
            InlineNodeKind::Delimiter { marker } => {
                Some((marker, node.range(), node.can_open(), node.can_close()))
            }
            InlineNodeKind::Text
            | InlineNodeKind::Escaped
            | InlineNodeKind::Punctuation { .. }
            | InlineNodeKind::LineBreak { .. } => None,
        })
        .map(|(marker, range, can_open, can_close)| {
            Ok(Delimiter {
                marker,
                len: usize::try_from(range.len()).map_err(|_| InlineParseError::OffsetOverflow)?,
                range,
                can_open,
                can_close,
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
    spans.extend(build_link_spans(source, nodes, &code_pairs)?);
    spans.extend(build_reference_link_spans(nodes, &code_pairs)?);
    spans.extend(build_autolink_spans(source, nodes, &code_pairs)?);

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
        destination: None,
        reference: None,
    })
}

fn pair_delimiters(delimiters: &[Delimiter], marker: InlineDelimiter) -> Vec<DelimiterPair> {
    let mut openings = Vec::new();
    let mut pairs = Vec::new();
    for delimiter in delimiters
        .iter()
        .copied()
        .filter(|item| item.marker == marker)
        .filter(|item| marker == InlineDelimiter::Code || item.can_open || item.can_close)
    {
        if delimiter.can_close
            && let Some(opening_index) = openings
                .iter()
                .rposition(|opening: &Delimiter| opening.len == delimiter.len && opening.can_open)
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

fn build_link_spans(
    source: &TextSnapshot,
    nodes: &[InlineNode],
    code_pairs: &[DelimiterPair],
) -> Result<Vec<InlineSpan>, InlineParseError> {
    let mut spans = Vec::new();
    for (open_index, open_node) in nodes.iter().enumerate() {
        if !matches!(
            open_node.kind(),
            InlineNodeKind::Punctuation {
                kind: InlinePunctuation::OpenBracket
            }
        ) || inside_code(open_node.range(), code_pairs)
        {
            continue;
        }
        let Some(close_index) = matching_bracket(nodes, open_index) else {
            continue;
        };
        let Some(open_paren_index) = close_index.checked_add(1) else {
            continue;
        };
        if !matches!(
            nodes.get(open_paren_index).map(|node| node.kind()),
            Some(InlineNodeKind::Punctuation {
                kind: InlinePunctuation::OpenParen
            })
        ) {
            continue;
        }
        let Some(close_paren_index) = matching_parenthesis(nodes, open_paren_index) else {
            continue;
        };
        let close_node = nodes[close_index];
        let open_paren = nodes[open_paren_index];
        let close_paren = nodes[close_paren_index];
        let image = open_index > 0
            && matches!(
                nodes[open_index - 1].kind(),
                InlineNodeKind::Punctuation {
                    kind: InlinePunctuation::Bang
                }
            )
            && nodes[open_index - 1].range().end() == open_node.range().start();
        let opening_start = if image {
            nodes[open_index - 1].range().start()
        } else {
            open_node.range().start()
        };
        let source_range = TextRange::new(opening_start, close_paren.range().end())
            .ok_or(InlineParseError::OffsetOverflow)?;
        let content = TextRange::new(open_node.range().end(), close_node.range().start())
            .ok_or(InlineParseError::OffsetOverflow)?;
        let closing = TextRange::new(close_node.range().start(), close_paren.range().end())
            .ok_or(InlineParseError::OffsetOverflow)?;
        let destination_range =
            TextRange::new(open_paren.range().end(), close_paren.range().start())
                .ok_or(InlineParseError::OffsetOverflow)?;
        let destination = trim_destination(source, destination_range)?;
        spans.push(InlineSpan {
            kind: if image {
                InlineSpanKind::Image
            } else {
                InlineSpanKind::Link
            },
            source_range,
            opening: TextRange::new(opening_start, open_node.range().end())
                .ok_or(InlineParseError::OffsetOverflow)?,
            content,
            closing,
            destination: Some(destination),
            reference: None,
        });
    }
    Ok(spans)
}

fn build_reference_link_spans(
    nodes: &[InlineNode],
    code_pairs: &[DelimiterPair],
) -> Result<Vec<InlineSpan>, InlineParseError> {
    let mut spans = Vec::new();
    for (open_index, open_node) in nodes.iter().enumerate() {
        if !matches!(
            open_node.kind(),
            InlineNodeKind::Punctuation {
                kind: InlinePunctuation::OpenBracket
            }
        ) || inside_code(open_node.range(), code_pairs)
        {
            continue;
        }
        let Some(close_index) = matching_bracket(nodes, open_index) else {
            continue;
        };
        let Some(reference_open_index) = close_index.checked_add(1) else {
            continue;
        };
        if !matches!(
            nodes.get(reference_open_index).map(|node| node.kind()),
            Some(InlineNodeKind::Punctuation {
                kind: InlinePunctuation::OpenBracket
            })
        ) {
            continue;
        }
        let reference_open = nodes[reference_open_index];
        if inside_code(reference_open.range(), code_pairs) {
            continue;
        }
        let Some(reference_close_index) = matching_bracket(nodes, reference_open_index) else {
            continue;
        };
        let reference_close = nodes[reference_close_index];
        if inside_code(reference_close.range(), code_pairs) {
            continue;
        }
        let close_node = nodes[close_index];
        let image = open_index > 0
            && matches!(
                nodes[open_index - 1].kind(),
                InlineNodeKind::Punctuation {
                    kind: InlinePunctuation::Bang
                }
            )
            && nodes[open_index - 1].range().end() == open_node.range().start();
        let opening_start = if image {
            nodes[open_index - 1].range().start()
        } else {
            open_node.range().start()
        };
        let source_range = TextRange::new(opening_start, reference_close.range().end())
            .ok_or(InlineParseError::OffsetOverflow)?;
        let content = TextRange::new(open_node.range().end(), close_node.range().start())
            .ok_or(InlineParseError::OffsetOverflow)?;
        let closing = TextRange::new(close_node.range().start(), reference_close.range().end())
            .ok_or(InlineParseError::OffsetOverflow)?;
        let reference = TextRange::new(
            reference_open.range().end(),
            reference_close.range().start(),
        )
        .ok_or(InlineParseError::OffsetOverflow)?;
        spans.push(InlineSpan {
            kind: if image {
                InlineSpanKind::ReferenceImage
            } else {
                InlineSpanKind::ReferenceLink
            },
            source_range,
            opening: TextRange::new(opening_start, open_node.range().end())
                .ok_or(InlineParseError::OffsetOverflow)?,
            content,
            closing,
            destination: None,
            reference: Some(reference),
        });
    }
    Ok(spans)
}

fn build_autolink_spans(
    source: &TextSnapshot,
    nodes: &[InlineNode],
    code_pairs: &[DelimiterPair],
) -> Result<Vec<InlineSpan>, InlineParseError> {
    let mut spans = Vec::new();
    for (open_index, open_node) in nodes.iter().enumerate() {
        if !matches!(
            open_node.kind(),
            InlineNodeKind::Punctuation {
                kind: InlinePunctuation::AngleOpen
            }
        ) || inside_code(open_node.range(), code_pairs)
        {
            continue;
        }
        let mut close_index = None;
        for (index, node) in nodes.iter().enumerate().skip(open_index + 1) {
            match node.kind() {
                InlineNodeKind::Punctuation {
                    kind: InlinePunctuation::AngleClose,
                } => {
                    close_index = Some(index);
                    break;
                }
                InlineNodeKind::Punctuation {
                    kind: InlinePunctuation::AngleOpen,
                }
                | InlineNodeKind::LineBreak { .. } => break,
                _ => {}
            }
        }
        let Some(close_index) = close_index else {
            continue;
        };
        let close_node = nodes[close_index];
        if inside_code(close_node.range(), code_pairs) {
            continue;
        }
        let content = TextRange::new(open_node.range().end(), close_node.range().start())
            .ok_or(InlineParseError::OffsetOverflow)?;
        if content.is_empty() || !is_valid_autolink(source, content)? {
            continue;
        }
        let source_range = TextRange::new(open_node.range().start(), close_node.range().end())
            .ok_or(InlineParseError::OffsetOverflow)?;
        spans.push(InlineSpan {
            kind: InlineSpanKind::Autolink,
            source_range,
            opening: open_node.range(),
            content,
            closing: close_node.range(),
            destination: Some(content),
            reference: None,
        });
    }
    Ok(spans)
}

fn is_valid_autolink(source: &TextSnapshot, range: TextRange) -> Result<bool, InlineParseError> {
    let cursor = InlineByteCursor::new(source, range)?;
    let mut bytes = Vec::new();
    for (_, byte) in cursor {
        bytes.push(byte);
    }
    if bytes
        .iter()
        .any(|byte| byte.is_ascii_whitespace() || matches!(*byte, b'<' | b'>') || *byte >= 0x80)
    {
        return Ok(false);
    }
    if let Some(colon) = bytes.iter().position(|byte| *byte == b':') {
        let scheme = &bytes[..colon];
        if (2..=32).contains(&scheme.len())
            && scheme[0].is_ascii_alphabetic()
            && scheme[1..]
                .iter()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(*byte, b'+' | b'.' | b'-'))
            && colon + 1 < bytes.len()
        {
            return Ok(true);
        }
    }
    let Some(at) = bytes.iter().position(|byte| *byte == b'@') else {
        return Ok(false);
    };
    if at == 0 || at + 1 >= bytes.len() || bytes[at + 1..].contains(&b'@') {
        return Ok(false);
    }
    let local = &bytes[..at];
    let domain = &bytes[at + 1..];
    Ok(local.iter().all(|byte| {
        byte.is_ascii_alphanumeric()
            || matches!(
                *byte,
                b'!' | b'#'
                    | b'$'
                    | b'%'
                    | b'&'
                    | b'\''
                    | b'*'
                    | b'+'
                    | b'-'
                    | b'/'
                    | b'='
                    | b'?'
                    | b'^'
                    | b'_'
                    | b'`'
                    | b'{'
                    | b'|'
                    | b'}'
                    | b'~'
                    | b'.'
            ) && domain
                .iter()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(*byte, b'.' | b'-'))
                && !domain.starts_with(b".")
                && !domain.ends_with(b".")
    }))
}

fn matching_bracket(nodes: &[InlineNode], open_index: usize) -> Option<usize> {
    let mut depth = 0_usize;
    for (index, node) in nodes.iter().enumerate().skip(open_index) {
        match node.kind() {
            InlineNodeKind::Punctuation {
                kind: InlinePunctuation::OpenBracket,
            } => depth = depth.saturating_add(1),
            InlineNodeKind::Punctuation {
                kind: InlinePunctuation::CloseBracket,
            } => {
                depth = depth.checked_sub(1)?;
                if depth == 0 {
                    return Some(index);
                }
            }
            _ => {}
        }
    }
    None
}

fn matching_parenthesis(nodes: &[InlineNode], open_index: usize) -> Option<usize> {
    let mut depth = 0_usize;
    for (index, node) in nodes.iter().enumerate().skip(open_index) {
        match node.kind() {
            InlineNodeKind::Punctuation {
                kind: InlinePunctuation::OpenParen,
            } => depth = depth.saturating_add(1),
            InlineNodeKind::Punctuation {
                kind: InlinePunctuation::CloseParen,
            } => {
                depth = depth.checked_sub(1)?;
                if depth == 0 {
                    return Some(index);
                }
            }
            _ => {}
        }
    }
    None
}

fn inside_code(range: TextRange, code_pairs: &[DelimiterPair]) -> bool {
    code_pairs.iter().any(|pair| {
        pair.opening.range.start() < range.start() && range.end() < pair.closing.range.end()
    })
}

fn trim_destination(
    source: &TextSnapshot,
    range: TextRange,
) -> Result<TextRange, InlineParseError> {
    let cursor = InlineByteCursor::new(source, range)?;
    let mut first = None;
    let mut end = usize::try_from(range.end()).map_err(|_| InlineParseError::OffsetOverflow)?;
    let mut angle_wrapped = false;
    for (position, byte) in cursor {
        if first.is_none() {
            if matches!(byte, b' ' | b'\t' | b'\r' | b'\n') {
                continue;
            }
            if byte == b'<' {
                angle_wrapped = true;
                first = Some(
                    position
                        .checked_add(1)
                        .ok_or(InlineParseError::OffsetOverflow)?,
                );
                continue;
            }
            first = Some(position);
            continue;
        }
        if angle_wrapped {
            if byte == b'>' {
                end = position;
                break;
            }
        } else if matches!(byte, b' ' | b'\t' | b'\r' | b'\n') {
            end = position;
            break;
        }
    }
    let start = first
        .unwrap_or(usize::try_from(range.start()).map_err(|_| InlineParseError::OffsetOverflow)?);
    byte_range(start, end)
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

    #[test]
    fn links_and_images_keep_label_and_destination_ranges() {
        let source = r#"[Yu](https://example.com "title") ![logo](img.png)"#;
        let snapshot = TextBuffer::new(source).snapshot();
        let range = TextRange::new(ByteOffset::ZERO, snapshot.len_bytes())
            .expect("source range should be ordered");
        let inline = parse_inline(&snapshot, range).expect("inline parse should succeed");

        let links = inline
            .spans()
            .iter()
            .filter(|span| span.kind() == InlineSpanKind::Link)
            .collect::<Vec<_>>();
        let images = inline
            .spans()
            .iter()
            .filter(|span| span.kind() == InlineSpanKind::Image)
            .collect::<Vec<_>>();
        assert_eq!(links.len(), 1);
        assert_eq!(images.len(), 1);

        let link = links[0];
        assert_eq!(slice(source, link.content()), "Yu");
        assert_eq!(
            slice(source, link.destination().expect("link destination")),
            "https://example.com"
        );
        assert_eq!(slice(source, link.opening()), "[");
        assert_eq!(
            slice(source, link.closing()),
            "](https://example.com \"title\")"
        );

        let image = images[0];
        assert_eq!(slice(source, image.opening()), "![");
        assert_eq!(slice(source, image.content()), "logo");
        assert_eq!(
            slice(source, image.destination().expect("image destination")),
            "img.png"
        );
        assert!(inline.has_lossless_coverage());
    }

    #[test]
    fn reference_links_and_autolinks_keep_parser_owned_ranges() {
        let source = "[Yu][project] ![logo][] <https://example.com> <dev@example.com> <div>";
        let snapshot = TextBuffer::new(source).snapshot();
        let range = TextRange::new(ByteOffset::ZERO, snapshot.len_bytes())
            .expect("source range should be ordered");
        let inline = parse_inline(&snapshot, range).expect("inline parse should succeed");

        let references = inline
            .spans()
            .iter()
            .filter(|span| {
                matches!(
                    span.kind(),
                    InlineSpanKind::ReferenceLink | InlineSpanKind::ReferenceImage
                )
            })
            .collect::<Vec<_>>();
        assert_eq!(references.len(), 2);
        assert_eq!(references[0].kind(), InlineSpanKind::ReferenceLink);
        assert_eq!(slice(source, references[0].content()), "Yu");
        assert_eq!(
            slice(source, references[0].reference().expect("reference label")),
            "project"
        );
        assert_eq!(slice(source, references[0].opening()), "[");
        assert_eq!(slice(source, references[0].closing()), "][project]");
        assert_eq!(references[1].kind(), InlineSpanKind::ReferenceImage);
        assert_eq!(slice(source, references[1].content()), "logo");
        assert_eq!(
            references[1]
                .reference()
                .expect("collapsed reference")
                .len(),
            0
        );

        let autolinks = inline
            .spans()
            .iter()
            .filter(|span| span.kind() == InlineSpanKind::Autolink)
            .collect::<Vec<_>>();
        assert_eq!(autolinks.len(), 2);
        assert_eq!(slice(source, autolinks[0].content()), "https://example.com");
        assert_eq!(
            slice(source, autolinks[0].destination().expect("URL destination")),
            "https://example.com"
        );
        assert_eq!(slice(source, autolinks[1].content()), "dev@example.com");
        assert!(
            inline
                .spans()
                .iter()
                .all(|span| { slice(source, span.source_range()) != "<div>" })
        );
        assert!(inline.has_lossless_coverage());
    }

    #[test]
    fn reference_and_autolink_syntax_stays_literal_inside_code() {
        let source = "`[Yu][id] <https://example.com>` [Yu][id]";
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
                .filter(|span| span.kind() == InlineSpanKind::ReferenceLink)
                .count(),
            1
        );
        assert_eq!(
            inline
                .spans()
                .iter()
                .filter(|span| span.kind() == InlineSpanKind::Autolink)
                .count(),
            0
        );
    }

    #[test]
    fn line_break_nodes_distinguish_soft_and_hard_breaks() {
        let source = "soft\nspaces  \nslash\\\nend\r\nlast";
        let snapshot = TextBuffer::new(source).snapshot();
        let range = TextRange::new(ByteOffset::ZERO, snapshot.len_bytes())
            .expect("source range should be ordered");
        let inline = parse_inline(&snapshot, range).expect("inline parse should succeed");
        let breaks = inline
            .nodes()
            .iter()
            .filter_map(|node| match node.kind() {
                InlineNodeKind::LineBreak { hard } => Some((hard, node.range())),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(breaks.len(), 4);
        assert!(!breaks[0].0);
        assert!(breaks[1].0);
        assert_eq!(slice(source, breaks[1].1), "  \n");
        assert!(breaks[2].0);
        assert_eq!(slice(source, breaks[2].1), "\\\n");
        assert!(!breaks[3].0);
        assert_eq!(slice(source, breaks[3].1), "\r\n");
        assert!(inline.has_lossless_coverage());
    }

    #[test]
    fn emphasis_flanking_rejects_intraword_underscores() {
        let source = "foo_bar_baz **strong** *emphasis*";
        let snapshot = TextBuffer::new(source).snapshot();
        let range = TextRange::new(ByteOffset::ZERO, snapshot.len_bytes())
            .expect("source range should be ordered");
        let inline = parse_inline(&snapshot, range).expect("inline parse should succeed");

        assert_eq!(
            inline
                .spans()
                .iter()
                .filter(|span| span.kind() == InlineSpanKind::Emphasis)
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
        assert!(
            inline
                .spans()
                .iter()
                .all(|span| slice(source, span.content()) != "bar")
        );
    }

    fn slice(source: &str, range: TextRange) -> &str {
        &source[usize::try_from(range.start()).expect("offset fits usize")
            ..usize::try_from(range.end()).expect("offset fits usize")]
    }
}
