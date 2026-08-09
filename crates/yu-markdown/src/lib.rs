#![forbid(unsafe_code)]

//! Lossless Markdown syntax experiments.
//!
//! Phase 1 provides a deliberately small, chunk-aware block scanner. It
//! preserves every source byte through ranges, but it is not yet CommonMark.

use std::error::Error;
use std::fmt;

use yu_core::{Affinity, ByteOffset, Revision, TextAnchor, TextRange};
use yu_text::{AnchorMapError, ChangeSet, TextSnapshot};

/// The block shapes recognized by the Phase 1 scanner.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BlockKind {
    BlankLine,
    Paragraph,
    AtxHeading { level: u8 },
    FencedCodeBlock { marker: char, closed: bool },
}

/// A block that refers to source without owning or normalizing its text.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Block {
    kind: BlockKind,
    range: TextRange,
}

impl Block {
    #[must_use]
    pub fn kind(self) -> BlockKind {
        self.kind
    }

    #[must_use]
    pub fn range(self) -> TextRange {
        self.range
    }
}

/// A lossless block view of one immutable text revision.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MarkdownDocument {
    revision: Revision,
    source_len: ByteOffset,
    blocks: Vec<Block>,
}

impl MarkdownDocument {
    #[must_use]
    pub fn revision(&self) -> Revision {
        self.revision
    }

    #[must_use]
    pub fn source_len(&self) -> ByteOffset {
        self.source_len
    }

    #[must_use]
    pub fn blocks(&self) -> &[Block] {
        &self.blocks
    }

    /// Confirms that ordered block ranges cover the source exactly once.
    #[must_use]
    pub fn has_lossless_coverage(&self) -> bool {
        let mut expected_start = ByteOffset::ZERO;
        for block in &self.blocks {
            if block.range.start() != expected_start {
                return false;
            }
            expected_start = block.range.end();
        }
        expected_start == self.source_len
    }
}

/// The observable result of one conservative incremental block parse.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IncrementalParse {
    document: MarkdownDocument,
    reparsed_range: TextRange,
    reused_prefix_blocks: usize,
}

impl IncrementalParse {
    #[must_use]
    pub fn document(&self) -> &MarkdownDocument {
        &self.document
    }

    #[must_use]
    pub fn into_document(self) -> MarkdownDocument {
        self.document
    }

    #[must_use]
    pub fn reparsed_range(&self) -> TextRange {
        self.reparsed_range
    }

    #[must_use]
    pub fn reused_prefix_blocks(&self) -> usize {
        self.reused_prefix_blocks
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum IncrementalParseError {
    PreviousRevision {
        document: Revision,
        change_set: Revision,
    },
    SnapshotRevision {
        snapshot: Revision,
        change_set: Revision,
    },
    AnchorMap(AnchorMapError),
}

impl fmt::Display for IncrementalParseError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::PreviousRevision {
                document,
                change_set,
            } => write!(
                formatter,
                "previous document revision {document:?} does not match change set {change_set:?}"
            ),
            Self::SnapshotRevision {
                snapshot,
                change_set,
            } => write!(
                formatter,
                "snapshot revision {snapshot:?} does not match change set {change_set:?}"
            ),
            Self::AnchorMap(error) => write!(formatter, "cannot map reparse boundary: {error}"),
        }
    }
}

impl Error for IncrementalParseError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::AnchorMap(error) => Some(error),
            Self::PreviousRevision { .. } | Self::SnapshotRevision { .. } => None,
        }
    }
}

impl From<AnchorMapError> for IncrementalParseError {
    fn from(error: AnchorMapError) -> Self {
        Self::AnchorMap(error)
    }
}

#[derive(Clone, Copy, Debug)]
struct Line {
    start: usize,
    end: usize,
    analysis: LineAnalysis,
}

impl Line {
    fn is_blank(self) -> bool {
        self.analysis.blank
    }

    fn opening_fence(self) -> Option<Fence> {
        let analysis = self.analysis;
        if !analysis.indent_valid || !matches!(analysis.prefix, Some('`' | '~')) {
            return None;
        }
        Some(Fence {
            marker: analysis.prefix.expect("validated fence must have a marker"),
            count: analysis.prefix_count,
        })
        .filter(|fence| fence.count >= 3)
    }

    fn is_closing_fence(self, opening: Fence) -> bool {
        let analysis = self.analysis;
        analysis.indent_valid
            && analysis.prefix == Some(opening.marker)
            && analysis.prefix_count >= opening.count
            && analysis.tail_whitespace
    }

    fn atx_heading_level(self) -> Option<u8> {
        let analysis = self.analysis;
        if !analysis.indent_valid
            || analysis.prefix != Some('#')
            || !(1..=6).contains(&analysis.prefix_count)
            || !matches!(analysis.after_prefix, None | Some(' ' | '\t'))
        {
            return None;
        }
        u8::try_from(analysis.prefix_count).ok()
    }
}

#[derive(Clone, Copy, Debug)]
struct LineAnalysis {
    blank: bool,
    indent_valid: bool,
    syntax_done: bool,
    leading_spaces: usize,
    prefix: Option<char>,
    prefix_count: usize,
    after_prefix: Option<char>,
    tail_whitespace: bool,
}

impl Default for LineAnalysis {
    fn default() -> Self {
        Self {
            blank: true,
            indent_valid: true,
            syntax_done: false,
            leading_spaces: 0,
            prefix: None,
            prefix_count: 0,
            after_prefix: None,
            tail_whitespace: true,
        }
    }
}

impl LineAnalysis {
    fn wants_input(self) -> bool {
        self.blank || !self.syntax_done
    }

    fn push(&mut self, character: char) {
        self.blank &= character.is_whitespace();
        if !self.indent_valid || self.syntax_done {
            return;
        }

        let Some(prefix) = self.prefix else {
            if character == ' ' {
                self.leading_spaces += 1;
                if self.leading_spaces > 3 {
                    self.indent_valid = false;
                }
            } else {
                self.prefix = Some(character);
                self.prefix_count = 1;
                if !matches!(character, '#' | '`' | '~') {
                    self.syntax_done = true;
                }
            }
            return;
        };

        if self.after_prefix.is_none() && character == prefix {
            self.prefix_count += 1;
            return;
        }
        if self.after_prefix.is_none() {
            self.after_prefix = Some(character);
        }
        self.tail_whitespace &= character.is_whitespace();
        if prefix == '#' || !self.tail_whitespace {
            self.syntax_done = true;
        }
    }
}

#[derive(Clone, Copy, Debug)]
struct Fence {
    marker: char,
    count: usize,
}

/// Scans an immutable Snapshot without materializing a contiguous source copy.
#[must_use]
pub fn parse(snapshot: &TextSnapshot) -> MarkdownDocument {
    MarkdownDocument {
        revision: snapshot.revision(),
        source_len: snapshot.len_bytes(),
        blocks: parse_blocks(snapshot, 0),
    }
}

/// Reuses a stable block prefix and reparses from the earliest affected block.
///
/// The Phase 1 algorithm intentionally reparses through EOF. This is safe for
/// state that can propagate forward, such as an opened or deleted code fence.
pub fn parse_incremental(
    previous: &MarkdownDocument,
    snapshot: &TextSnapshot,
    changes: &ChangeSet,
) -> Result<IncrementalParse, IncrementalParseError> {
    if previous.revision != changes.before() {
        return Err(IncrementalParseError::PreviousRevision {
            document: previous.revision,
            change_set: changes.before(),
        });
    }
    if snapshot.revision() != changes.after() {
        return Err(IncrementalParseError::SnapshotRevision {
            snapshot: snapshot.revision(),
            change_set: changes.after(),
        });
    }

    if changes.changes().is_empty() {
        let document = MarkdownDocument {
            revision: snapshot.revision(),
            source_len: snapshot.len_bytes(),
            blocks: previous.blocks.clone(),
        };
        return Ok(IncrementalParse {
            reparsed_range: TextRange::empty(snapshot.len_bytes()),
            reused_prefix_blocks: document.blocks.len(),
            document,
        });
    }

    let earliest = changes
        .changes()
        .iter()
        .map(|change| change.old_range().start())
        .min()
        .expect("non-empty changes must have an earliest offset");
    let affected = previous
        .blocks
        .iter()
        .position(|block| block.range.end() > earliest)
        .unwrap_or(previous.blocks.len());
    let reparse_index = affected.saturating_sub(1);
    let old_start = previous
        .blocks
        .get(reparse_index)
        .map_or(ByteOffset::ZERO, |block| block.range.start());
    let mapped = changes.map_anchor(TextAnchor::new(
        previous.revision,
        old_start,
        Affinity::Before,
    ))?;
    let new_start = mapped.offset();
    let new_start_usize = usize::try_from(new_start)
        .expect("a mapped document offset must fit the platform address space");

    let mut blocks = Vec::with_capacity(previous.blocks.len());
    blocks.extend_from_slice(&previous.blocks[..reparse_index]);
    blocks.extend(parse_blocks(snapshot, new_start_usize));
    let document = MarkdownDocument {
        revision: snapshot.revision(),
        source_len: snapshot.len_bytes(),
        blocks,
    };
    let reparsed_range = TextRange::new(new_start, snapshot.len_bytes())
        .expect("reparse start must not exceed the Snapshot length");

    Ok(IncrementalParse {
        document,
        reparsed_range,
        reused_prefix_blocks: reparse_index,
    })
}

fn parse_blocks(snapshot: &TextSnapshot, start: usize) -> Vec<Block> {
    let lines = lines(snapshot, start);
    let mut blocks = Vec::new();
    let mut index = 0;

    while index < lines.len() {
        let line = lines[index];

        if line.is_blank() {
            blocks.push(block(BlockKind::BlankLine, line.start, line.end));
            index += 1;
            continue;
        }

        if let Some(fence) = line.opening_fence() {
            let block_start = line.start;
            index += 1;
            let mut closed = false;
            while index < lines.len() {
                let candidate = lines[index];
                index += 1;
                if candidate.is_closing_fence(fence) {
                    closed = true;
                    break;
                }
            }
            let end = lines[index - 1].end;
            blocks.push(block(
                BlockKind::FencedCodeBlock {
                    marker: fence.marker,
                    closed,
                },
                block_start,
                end,
            ));
            continue;
        }

        if let Some(level) = line.atx_heading_level() {
            blocks.push(block(BlockKind::AtxHeading { level }, line.start, line.end));
            index += 1;
            continue;
        }

        let block_start = line.start;
        index += 1;
        while index < lines.len() {
            let candidate = lines[index];
            if candidate.is_blank()
                || candidate.opening_fence().is_some()
                || candidate.atx_heading_level().is_some()
            {
                break;
            }
            index += 1;
        }
        let end = lines[index - 1].end;
        blocks.push(block(BlockKind::Paragraph, block_start, end));
    }

    blocks
}

fn block(kind: BlockKind, start: usize, end: usize) -> Block {
    let start = ByteOffset::try_from(start).unwrap_or(ByteOffset::new(u64::MAX));
    let end = ByteOffset::try_from(end).unwrap_or(ByteOffset::new(u64::MAX));
    let range = TextRange::new(start, end).unwrap_or_else(|| TextRange::empty(start));
    Block { kind, range }
}

fn lines(snapshot: &TextSnapshot, start: usize) -> Vec<Line> {
    let source_len = usize::try_from(snapshot.len_bytes())
        .expect("Snapshot length must fit the platform address space");
    if start >= source_len {
        return Vec::new();
    }

    let start_offset = ByteOffset::try_from(start).expect("line start must fit u64");
    let cursor = snapshot
        .chunk_cursor(start_offset)
        .expect("block boundary must be a valid UTF-8 offset");
    let mut result = Vec::new();
    let mut line_start = start;
    let mut pending_cr = false;
    let mut analysis = LineAnalysis::default();

    for chunk in cursor {
        let chunk_start = usize::try_from(chunk.start()).expect("chunk offset must fit usize");
        let local_start = start.saturating_sub(chunk_start).min(chunk.text().len());
        let text = &chunk.text()[local_start..];
        let mut local = 0;
        while local < text.len() {
            if !analysis.wants_input() {
                let Some(newline) = text.as_bytes()[local..]
                    .iter()
                    .position(|value| *value == b'\n')
                else {
                    break;
                };
                let absolute = chunk_start + local_start + local + newline;
                result.push(Line {
                    start: line_start,
                    end: absolute + 1,
                    analysis,
                });
                line_start = absolute + 1;
                analysis = LineAnalysis::default();
                pending_cr = false;
                local += newline + 1;
                continue;
            }

            let first = text.as_bytes()[local];
            let character = if first.is_ascii() {
                char::from(first)
            } else {
                text[local..]
                    .chars()
                    .next()
                    .expect("non-empty UTF-8 tail must contain a character")
            };
            let absolute = chunk_start + local_start + local;
            if character == '\n' {
                result.push(Line {
                    start: line_start,
                    end: absolute + 1,
                    analysis,
                });
                line_start = absolute + 1;
                analysis = LineAnalysis::default();
                pending_cr = false;
            } else {
                if pending_cr {
                    analysis.push('\r');
                    pending_cr = false;
                }
                if character == '\r' {
                    pending_cr = true;
                } else {
                    analysis.push(character);
                }
            }
            local += character.len_utf8();
        }
    }

    if line_start < source_len {
        result.push(Line {
            start: line_start,
            end: source_len,
            analysis,
        });
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use yu_text::{Edit, TextBuffer, Transaction, retained_snapshot_stats};

    #[test]
    fn scanner_covers_source_without_gaps() {
        let source = "# 羽\n\nparagraph\ncontinued\n\n```rust\nfn main() {}\n```\n";
        let buffer = TextBuffer::new(source);
        let document = parse(&buffer.snapshot());

        assert!(document.has_lossless_coverage());
        assert_eq!(document.source_len().get(), source.len() as u64);
        assert_eq!(document.blocks().len(), 5);
        assert_eq!(
            document.blocks()[0].kind(),
            BlockKind::AtxHeading { level: 1 }
        );
        assert_eq!(document.blocks()[1].kind(), BlockKind::BlankLine);
        assert_eq!(document.blocks()[2].kind(), BlockKind::Paragraph);
        assert_eq!(
            document.blocks()[4].kind(),
            BlockKind::FencedCodeBlock {
                marker: '`',
                closed: true
            }
        );
    }

    #[test]
    fn scanner_preserves_phase_one_line_classification_rules() {
        let cases = [
            ("   # title\n", BlockKind::AtxHeading { level: 1 }),
            ("    # title\n", BlockKind::Paragraph),
            ("####### title\n", BlockKind::Paragraph),
            ("\u{00a0}\n", BlockKind::BlankLine),
            (
                "```\r\nbody\r\n```\r\n",
                BlockKind::FencedCodeBlock {
                    marker: '`',
                    closed: true,
                },
            ),
            (
                "```\nbody\n``` trailing\n",
                BlockKind::FencedCodeBlock {
                    marker: '`',
                    closed: false,
                },
            ),
        ];

        for (source, expected) in cases {
            let document = parse(&TextBuffer::new(source).snapshot());
            assert_eq!(document.blocks().len(), 1, "source {source:?}");
            assert_eq!(document.blocks()[0].kind(), expected, "source {source:?}");
            assert!(document.has_lossless_coverage());
        }
    }

    #[test]
    fn scanner_reads_syntax_across_piece_boundaries_without_materializing() {
        let parts = [
            "#", " 羽", "\r", "\n", "\r\n", "```", "rust\n", "body", "\n`", "``\n",
        ];
        let mut buffer = TextBuffer::new("");
        for part in parts {
            let at = buffer.snapshot().len_bytes();
            let transaction =
                Transaction::new(buffer.revision(), [Edit::new(TextRange::empty(at), part)]);
            buffer
                .apply(&transaction)
                .expect("append transaction should apply");
        }
        let snapshot = buffer.snapshot();
        assert_eq!(
            retained_snapshot_stats(std::slice::from_ref(&snapshot)).materialized_buffers(),
            0
        );

        let document = parse(&snapshot);

        assert!(document.has_lossless_coverage());
        assert_eq!(document.blocks().len(), 3);
        assert_eq!(
            document.blocks()[0].kind(),
            BlockKind::AtxHeading { level: 1 }
        );
        assert_eq!(document.blocks()[1].kind(), BlockKind::BlankLine);
        assert_eq!(
            document.blocks()[2].kind(),
            BlockKind::FencedCodeBlock {
                marker: '`',
                closed: true
            }
        );
        assert_eq!(
            retained_snapshot_stats(&[snapshot]).materialized_buffers(),
            0
        );
    }

    #[test]
    fn unclosed_fence_owns_the_remaining_source() {
        let buffer = TextBuffer::new("before\n\n~~~\ninside\n");
        let document = parse(&buffer.snapshot());

        assert!(document.has_lossless_coverage());
        assert_eq!(
            document.blocks()[2].kind(),
            BlockKind::FencedCodeBlock {
                marker: '~',
                closed: false
            }
        );
    }

    #[test]
    fn empty_source_has_lossless_coverage() {
        let buffer = TextBuffer::new("");
        let document = parse(&buffer.snapshot());

        assert!(document.blocks().is_empty());
        assert!(document.has_lossless_coverage());
    }

    #[test]
    fn crlf_bytes_are_preserved_in_ranges() {
        let source = "# title\r\n\r\ntext\r\n";
        let buffer = TextBuffer::new(source);
        let snapshot = buffer.snapshot();
        let document = parse(&snapshot);
        let reconstructed: String = document
            .blocks()
            .iter()
            .map(|block| {
                let start =
                    usize::try_from(block.range().start()).expect("test offset should fit usize");
                let end =
                    usize::try_from(block.range().end()).expect("test offset should fit usize");
                &snapshot.as_str()[start..end]
            })
            .collect();

        assert_eq!(reconstructed, source);
    }

    #[test]
    fn empty_change_set_reuses_the_entire_document() {
        let mut buffer = TextBuffer::new("# title\n\nbody\n");
        let previous = parse(&buffer.snapshot());
        let transaction = Transaction::new(buffer.revision(), std::iter::empty::<Edit>());
        let applied = buffer
            .apply(&transaction)
            .expect("empty transaction should still advance the revision");

        let incremental =
            parse_incremental(&previous, applied.result_snapshot(), applied.change_set())
                .expect("matching revisions should parse incrementally");

        assert_eq!(incremental.document(), &parse(applied.result_snapshot()));
        assert_eq!(incremental.reused_prefix_blocks(), previous.blocks().len());
        assert!(incremental.reparsed_range().is_empty());
    }

    #[test]
    fn incremental_parse_rejects_revision_mismatches() {
        let mut buffer = TextBuffer::new("body\n");
        let old_snapshot = buffer.snapshot();
        let previous = parse(&old_snapshot);
        let transaction = Transaction::new(
            buffer.revision(),
            [Edit::new(TextRange::empty(ByteOffset::ZERO), "# ")],
        );
        let applied = buffer
            .apply(&transaction)
            .expect("valid transaction should apply");

        assert!(matches!(
            parse_incremental(&previous, &old_snapshot, applied.change_set()),
            Err(IncrementalParseError::SnapshotRevision { .. })
        ));
        let wrong_previous = parse(applied.result_snapshot());
        assert!(matches!(
            parse_incremental(
                &wrong_previous,
                applied.result_snapshot(),
                applied.change_set()
            ),
            Err(IncrementalParseError::PreviousRevision { .. })
        ));
    }
}
