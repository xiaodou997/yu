#![forbid(unsafe_code)]

//! Lossless Markdown syntax experiments.
//!
//! Phase 1 provides deliberately small, chunk-aware block and inline token
//! scanners. They preserve every source byte through ranges, but are not yet
//! CommonMark semantic parsers.

use std::error::Error;
use std::fmt;
use std::iter::Peekable;

use yu_core::{Affinity, ByteOffset, Revision, TextAnchor, TextRange};
use yu_text::{
    AnchorMapError, ChangeSet, ChunkCursor, SnapshotRetentionStats, TextSnapshot,
    retained_snapshot_stats,
};

mod block_sequence;
mod inline;
mod reference;
mod table;
mod task;

pub use block_sequence::{
    Block, BlockCompactionPolicy, BlockKind, BlockSequence, BlockState, BlockStorageStats,
    RetainedBlockStats, TaskState,
};
use block_sequence::{BlockRecord, ResolvedBlockRecord, SourceHash, retained_block_stats};
pub use inline::{
    InlineDelimiter, InlineDocument, InlineNode, InlineNodeKind, InlineParseError,
    InlinePunctuation, InlineSpan, InlineSpanKind, parse_inline, parse_inline_with_definitions,
};
pub use reference::{ReferenceDefinition, ReferenceDefinitionIndex};
pub use table::{
    TableAlignment, TableBlock, TableCellAddress, TableCellRange, TableRowRange, parse_table,
    parse_table_in_snapshot,
};
pub use task::TaskMarker;

/// A lossless block view of one immutable text revision.
#[derive(Clone, Debug)]
pub struct MarkdownDocument {
    revision: Revision,
    source_len: ByteOffset,
    source: TextSnapshot,
    blocks: BlockSequence,
    references: ReferenceDefinitionIndex,
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
    pub fn blocks(&self) -> &BlockSequence {
        &self.blocks
    }

    /// Returns the source-backed link definitions for this document revision.
    #[must_use]
    pub fn reference_definitions(&self) -> &ReferenceDefinitionIndex {
        &self.references
    }

    #[must_use]
    pub fn block_storage_stats(&self) -> BlockStorageStats {
        self.blocks.storage_stats()
    }

    #[must_use]
    pub fn needs_block_compaction(&self, policy: BlockCompactionPolicy) -> bool {
        policy.should_compact(self.block_storage_stats())
    }

    /// Packs all active block records into one allocation.
    ///
    /// This is intentionally explicit because its cost is linear in the number
    /// of blocks. Product code should call it from an idle/background task.
    pub fn compact_blocks(&mut self) -> bool {
        let stats = self.block_storage_stats();
        if stats.blocks() == 0 || (stats.segments() == 1 && stats.reclaimable_records() == 0) {
            return false;
        }
        self.blocks = self.blocks.compacted();
        true
    }

    pub fn compact_blocks_if_needed(&mut self, policy: BlockCompactionPolicy) -> bool {
        if !self.needs_block_compaction(policy) {
            return false;
        }
        self.compact_blocks()
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

impl PartialEq for MarkdownDocument {
    fn eq(&self, other: &Self) -> bool {
        self.revision == other.revision
            && self.source_len == other.source_len
            && self.blocks == other.blocks
            && self.references == other.references
    }
}

impl Eq for MarkdownDocument {}

/// De-duplicated storage retained by a set of immutable Markdown revisions.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MarkdownRetentionStats {
    documents: usize,
    document_bytes: usize,
    text: SnapshotRetentionStats,
    blocks: RetainedBlockStats,
}

impl MarkdownRetentionStats {
    #[must_use]
    pub const fn documents(self) -> usize {
        self.documents
    }

    #[must_use]
    pub const fn document_bytes(self) -> usize {
        self.document_bytes
    }

    #[must_use]
    pub const fn text(self) -> SnapshotRetentionStats {
        self.text
    }

    #[must_use]
    pub const fn blocks(self) -> RetainedBlockStats {
        self.blocks
    }

    #[must_use]
    pub const fn estimated_bytes(self) -> usize {
        self.document_bytes
            .saturating_add(self.text.estimated_bytes())
            .saturating_add(self.blocks.estimated_bytes())
    }
}

#[must_use]
pub fn retained_markdown_stats(documents: &[MarkdownDocument]) -> MarkdownRetentionStats {
    let snapshots = documents
        .iter()
        .map(|document| document.source.clone())
        .collect::<Vec<_>>();
    MarkdownRetentionStats {
        documents: documents.len(),
        document_bytes: documents
            .len()
            .saturating_mul(std::mem::size_of::<MarkdownDocument>()),
        text: retained_snapshot_stats(&snapshots),
        blocks: retained_block_stats(documents.iter().map(|document| &document.blocks)),
    }
}

/// The observable result of one conservative incremental block parse.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IncrementalParse {
    document: MarkdownDocument,
    reparsed_range: TextRange,
    reused_prefix_blocks: usize,
    reused_suffix_blocks: usize,
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

    #[must_use]
    pub fn reused_suffix_blocks(&self) -> usize {
        self.reused_suffix_blocks
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
    source_hash: SourceHash,
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

    fn block_marker(self) -> Option<LineMarker> {
        if !self.analysis.indent_valid {
            return None;
        }
        let depth = u8::try_from(self.analysis.leading_spaces / 2).ok()?;
        match self.analysis.prefix {
            Some('>') => Some(LineMarker::BlockQuote { depth: 1 }),
            Some(marker @ ('-' | '+' | '*'))
                if self.analysis.prefix_count == 1
                    && self.analysis.after_prefix.is_none_or(is_markdown_space) =>
            {
                Some(LineMarker::List {
                    ordered: false,
                    depth,
                    marker,
                    start: 1,
                })
            }
            Some('0'..='9')
                if self.analysis.ordered_digits > 0
                    && self.analysis.ordered_digits <= 9
                    && matches!(self.analysis.after_prefix, Some('.' | ')'))
                    && self.analysis.marker_following.is_none_or(is_markdown_space) =>
            {
                Some(LineMarker::List {
                    ordered: true,
                    depth,
                    marker: self.analysis.after_prefix.unwrap_or('.'),
                    start: self.analysis.ordered_value.unwrap_or(1),
                })
            }
            _ => None,
        }
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
    marker_following: Option<char>,
    ordered_value: Option<u32>,
    ordered_digits: usize,
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
            marker_following: None,
            ordered_value: None,
            ordered_digits: 0,
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
                if !matches!(character, '#' | '`' | '~' | '>' | '-' | '+' | '*')
                    && !character.is_ascii_digit()
                {
                    self.syntax_done = true;
                }
                if character.is_ascii_digit() {
                    self.ordered_value = character.to_digit(10);
                    self.ordered_digits = 1;
                }
            }
            return;
        };

        if self.after_prefix.is_none() && character == prefix && matches!(prefix, '#' | '`' | '~') {
            self.prefix_count += 1;
            return;
        }
        if self.after_prefix.is_none()
            && prefix.is_ascii_digit()
            && character.is_ascii_digit()
            && self.ordered_digits < 9
        {
            self.ordered_value = self.ordered_value.and_then(|value| {
                value
                    .checked_mul(10)
                    .and_then(|value| value.checked_add(character.to_digit(10)?))
            });
            self.ordered_digits += 1;
            return;
        }
        if self.after_prefix.is_none() {
            self.after_prefix = Some(character);
        } else if prefix.is_ascii_digit() && self.marker_following.is_none() {
            self.marker_following = Some(character);
        }
        self.tail_whitespace &= character.is_whitespace();
        let ordered_delimiter_pending = prefix.is_ascii_digit()
            && matches!(self.after_prefix, Some('.' | ')'))
            && self.marker_following.is_none();
        if prefix == '#' || (!self.tail_whitespace && !ordered_delimiter_pending) {
            self.syntax_done = true;
        }
    }
}

#[derive(Clone, Copy, Debug)]
struct Fence {
    marker: char,
    count: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum LineMarker {
    BlockQuote {
        depth: u8,
    },
    List {
        ordered: bool,
        depth: u8,
        marker: char,
        start: u32,
    },
}

fn is_markdown_space(character: char) -> bool {
    matches!(character, ' ' | '\t')
}

/// Scans an immutable Snapshot without materializing a contiguous source copy.
#[must_use]
pub fn parse(snapshot: &TextSnapshot) -> MarkdownDocument {
    let blocks = BlockSequence::from_records(BlockParser::new(snapshot, 0).collect());
    MarkdownDocument {
        revision: snapshot.revision(),
        source_len: snapshot.len_bytes(),
        source: snapshot.clone(),
        references: ReferenceDefinitionIndex::from_blocks(snapshot, &blocks),
        blocks,
    }
}

/// Reparses from a conservative boundary until source, state, and block shape
/// converge with an unaffected old block, then shares the remaining suffix.
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
            source: snapshot.clone(),
            blocks: previous.blocks.clone(),
            references: ReferenceDefinitionIndex::from_blocks(snapshot, &previous.blocks),
        };
        return Ok(IncrementalParse {
            reparsed_range: TextRange::empty(snapshot.len_bytes()),
            reused_prefix_blocks: document.blocks.len(),
            reused_suffix_blocks: 0,
            document,
        });
    }

    let earliest = changes
        .changes()
        .iter()
        .map(|change| change.old_range().start())
        .min()
        .expect("non-empty changes must have an earliest offset");
    let affected = previous.blocks.first_ending_after(earliest);
    let reparse_index = affected.saturating_sub(1);
    let old_start = previous
        .blocks
        .get(reparse_index)
        .map_or(ByteOffset::ZERO, |block| block.range().start());
    let mapped = changes.map_anchor(TextAnchor::new(
        previous.revision,
        old_start,
        Affinity::Before,
    ))?;
    let new_start = mapped.offset();
    let new_start_usize = usize::try_from(new_start)
        .expect("a mapped document offset must fit the platform address space");

    let latest_changed_end = changes
        .changes()
        .iter()
        .map(|change| change.old_range().end())
        .max()
        .expect("non-empty changes must have a latest offset");
    let mut candidate_index = previous
        .blocks
        .first_starting_at_or_after(latest_changed_end)
        .max(reparse_index);

    let mut parser = BlockParser::new(snapshot, new_start_usize);
    let mut middle = Vec::new();
    let mut scanned_end = new_start;
    let mut reused_suffix_start = previous.blocks.len();
    let mut suffix_delta = 0_i128;

    for new_record in &mut parser {
        scanned_end = new_record.block.range.end();

        let mut candidate = None;
        while candidate_index < previous.blocks.len() {
            let old_record = previous
                .blocks
                .resolved_record(candidate_index)
                .expect("candidate index must identify an old block");
            let mapped_range =
                map_unchanged_range(previous.revision, old_record.block.range, changes)?;
            if mapped_range.start() < new_record.block.range.start() {
                candidate_index += 1;
                continue;
            }
            candidate = Some((old_record, mapped_range));
            break;
        }

        if let Some((old_record, mapped_range)) = candidate
            && records_converge(
                old_record,
                mapped_range,
                &new_record,
                &previous.source,
                snapshot,
            )
        {
            reused_suffix_start = candidate_index;
            suffix_delta = i128::from(new_record.block.range.start().get())
                - i128::from(old_record.block.range.start().get());
            break;
        }

        middle.push(new_record);
    }

    let blocks = BlockSequence::assemble(
        (&previous.blocks, 0..reparse_index),
        middle,
        (
            &previous.blocks,
            reused_suffix_start..previous.blocks.len(),
            suffix_delta,
        ),
    );
    let document = MarkdownDocument {
        revision: snapshot.revision(),
        source_len: snapshot.len_bytes(),
        source: snapshot.clone(),
        references: ReferenceDefinitionIndex::from_blocks(snapshot, &blocks),
        blocks,
    };
    let reparsed_range = TextRange::new(new_start, scanned_end)
        .expect("the scanner cannot finish before its reparse boundary");

    Ok(IncrementalParse {
        document,
        reparsed_range,
        reused_prefix_blocks: reparse_index,
        reused_suffix_blocks: previous.blocks.len() - reused_suffix_start,
    })
}

struct BlockParser<'a> {
    snapshot: &'a TextSnapshot,
    lines: Peekable<LineCursor<'a>>,
}

impl<'a> BlockParser<'a> {
    fn new(snapshot: &'a TextSnapshot, start: usize) -> Self {
        Self {
            snapshot,
            lines: LineCursor::new(snapshot, start).peekable(),
        }
    }

    fn is_reference_definition(&self, line: Line) -> bool {
        let Some(range) = TextRange::new(
            ByteOffset::try_from(line.start).expect("line start must fit u64"),
            ByteOffset::try_from(line.end).expect("line end must fit u64"),
        ) else {
            return false;
        };
        reference::is_reference_definition_line(self.snapshot, range)
    }

    fn record(
        &self,
        kind: BlockKind,
        start: usize,
        end: usize,
        end_state: BlockState,
        source_hash: SourceHash,
    ) -> BlockRecord {
        let block = block(kind, start, end);
        BlockRecord {
            block,
            start_state: BlockState::Normal,
            end_state,
            source_hash,
        }
    }
}

impl Iterator for BlockParser<'_> {
    type Item = BlockRecord;

    fn next(&mut self) -> Option<Self::Item> {
        let line = self.lines.next()?;
        if line.is_blank() {
            return Some(self.record(
                BlockKind::BlankLine,
                line.start,
                line.end,
                BlockState::Normal,
                line.source_hash,
            ));
        }

        if let Some(fence) = line.opening_fence() {
            let block_start = line.start;
            let mut closed = false;
            let mut end = line.end;
            let mut source_hash = line.source_hash;
            for candidate in self.lines.by_ref() {
                end = candidate.end;
                source_hash = concatenate_hash(
                    source_hash,
                    candidate.source_hash,
                    candidate.end - candidate.start,
                );
                if candidate.is_closing_fence(fence) {
                    closed = true;
                    break;
                }
            }
            let end_state = if closed {
                BlockState::Normal
            } else {
                BlockState::Fenced {
                    marker: fence.marker,
                    minimum: fence.count,
                }
            };
            return Some(self.record(
                BlockKind::FencedCodeBlock {
                    marker: fence.marker,
                    closed,
                },
                block_start,
                end,
                end_state,
                source_hash,
            ));
        }

        if let Some(level) = line.atx_heading_level() {
            return Some(self.record(
                BlockKind::AtxHeading { level },
                line.start,
                line.end,
                BlockState::Normal,
                line.source_hash,
            ));
        }

        if let Some(marker) = line.block_marker() {
            return Some(self.parse_container(line, marker));
        }

        if self.is_reference_definition(line) {
            return Some(self.record(
                BlockKind::ReferenceDefinition,
                line.start,
                line.end,
                BlockState::Normal,
                line.source_hash,
            ));
        }

        let block_start = line.start;
        let mut end = line.end;
        let mut source_hash = line.source_hash;
        while let Some(candidate) = self.lines.peek().copied() {
            if candidate.is_blank()
                || candidate.opening_fence().is_some()
                || candidate.atx_heading_level().is_some()
                || candidate.block_marker().is_some()
                || self.is_reference_definition(candidate)
            {
                break;
            }
            let line = self
                .lines
                .next()
                .expect("a peeked paragraph line must remain available");
            end = line.end;
            source_hash = concatenate_hash(source_hash, line.source_hash, line.end - line.start);
        }
        Some(self.record(
            BlockKind::Paragraph,
            block_start,
            end,
            BlockState::Normal,
            source_hash,
        ))
    }
}

impl BlockParser<'_> {
    fn parse_container(&mut self, first: Line, marker: LineMarker) -> BlockRecord {
        let block_start = first.start;
        let mut end = first.end;
        let mut source_hash = first.source_hash;

        while let Some(candidate) = self.lines.peek().copied() {
            if candidate.is_blank()
                || candidate.opening_fence().is_some()
                || candidate.atx_heading_level().is_some()
            {
                break;
            }

            match (marker, candidate.block_marker()) {
                (
                    LineMarker::BlockQuote { depth },
                    Some(LineMarker::BlockQuote { depth: next }),
                ) if depth == next => {
                    self.consume_line(&mut end, &mut source_hash);
                }
                (LineMarker::BlockQuote { .. }, Some(_)) | (LineMarker::List { .. }, Some(_)) => {
                    break;
                }
                (_, None) => {
                    // A non-marked line is a lazy continuation of the current
                    // container. Keeping it in the same source range avoids
                    // inventing a second canonical text representation.
                    self.consume_line(&mut end, &mut source_hash);
                }
            }
        }

        let kind = match marker {
            LineMarker::BlockQuote { depth } => BlockKind::BlockQuote { depth },
            LineMarker::List {
                ordered,
                depth,
                marker,
                start,
            } => match task::parse_task_marker(self.snapshot, line_range(first), ordered) {
                Some(task) => BlockKind::TaskListItem {
                    ordered,
                    depth,
                    marker,
                    start,
                    state: task.state(),
                },
                None => BlockKind::ListItem {
                    ordered,
                    depth,
                    marker,
                    start,
                },
            },
        };
        self.record(kind, block_start, end, BlockState::Normal, source_hash)
    }

    fn consume_line(&mut self, end: &mut usize, source_hash: &mut SourceHash) {
        let line = self
            .lines
            .next()
            .expect("a peeked container continuation must remain available");
        *end = line.end;
        *source_hash = concatenate_hash(*source_hash, line.source_hash, line.end - line.start);
    }
}

fn line_range(line: Line) -> TextRange {
    TextRange::new(
        ByteOffset::try_from(line.start).expect("line start must fit u64"),
        ByteOffset::try_from(line.end).expect("line end must fit u64"),
    )
    .expect("line range must be ordered")
}

/// Returns the parser-owned task marker range for a task-list block.
#[must_use]
pub fn task_marker(source: &TextSnapshot, block: Block) -> Option<TaskMarker> {
    let BlockKind::TaskListItem { ordered, .. } = block.kind() else {
        return None;
    };
    task::parse_task_marker(source, block.range(), ordered)
}

/// Returns parser-owned block syntax ranges that the visual projection may
/// hide without re-parsing Markdown in a consumer.
///
/// The ranges deliberately cover only structural prefixes. List bullets and
/// task text remain source-visible; a later layout/scene layer can render the
/// list marker from the stable `BlockKind` metadata. ATX heading and blockquote
/// prefixes are hidden here because their visual counterparts are block-level
/// style/indentation rather than editable text.
#[must_use]
pub fn block_syntax_hidden_ranges(source: &TextSnapshot, block: Block) -> Vec<TextRange> {
    let lines = block_line_ranges(source, block.range());
    match block.kind() {
        BlockKind::AtxHeading { level } => lines
            .first()
            .and_then(|line| heading_prefix_range(source, *line, level))
            .into_iter()
            .collect(),
        BlockKind::BlockQuote { .. } => lines
            .iter()
            .filter_map(|line| blockquote_prefix_range(source, *line))
            .collect(),
        BlockKind::BlankLine
        | BlockKind::ReferenceDefinition
        | BlockKind::Paragraph
        | BlockKind::FencedCodeBlock { .. }
        | BlockKind::ListItem { .. }
        | BlockKind::TaskListItem { .. } => Vec::new(),
    }
}

fn block_line_ranges(source: &TextSnapshot, range: TextRange) -> Vec<TextRange> {
    let Ok(start) = usize::try_from(range.start()) else {
        return Vec::new();
    };
    let Ok(end) = usize::try_from(range.end()) else {
        return Vec::new();
    };
    let Ok(mut chunks) = source.chunk_cursor(range.start()) else {
        return Vec::new();
    };
    let mut line_start = start;
    let mut lines = Vec::new();
    for chunk in &mut chunks {
        let Ok(chunk_start) = usize::try_from(chunk.start()) else {
            return Vec::new();
        };
        let chunk_end = chunk_start.saturating_add(chunk.text().len());
        let local_start = start.max(chunk_start).saturating_sub(chunk_start);
        let local_end = end.min(chunk_end).saturating_sub(chunk_start);
        for (index, byte) in chunk.text().as_bytes()[local_start..local_end]
            .iter()
            .enumerate()
        {
            if *byte != b'\n' {
                continue;
            }
            let absolute = chunk_start
                .saturating_add(local_start)
                .saturating_add(index + 1);
            let Ok(line_end) = ByteOffset::try_from(absolute) else {
                return Vec::new();
            };
            let Ok(line_start_offset) = ByteOffset::try_from(line_start) else {
                return Vec::new();
            };
            let Some(line) = TextRange::new(line_start_offset, line_end) else {
                return Vec::new();
            };
            lines.push(line);
            line_start = absolute;
        }
    }
    if line_start < end {
        let Ok(line_start_offset) = ByteOffset::try_from(line_start) else {
            return Vec::new();
        };
        let Ok(line_end) = ByteOffset::try_from(end) else {
            return Vec::new();
        };
        if let Some(line) = TextRange::new(line_start_offset, line_end) {
            lines.push(line);
        }
    }
    lines
}

fn heading_prefix_range(source: &TextSnapshot, line: TextRange, level: u8) -> Option<TextRange> {
    let start = usize::try_from(line.start()).ok()?;
    let line_end = usize::try_from(line.end()).ok()?;
    let mut cursor = SourceByteCursor::new(source, line)?;
    let mut next = cursor.next();
    let mut leading_spaces = 0_usize;
    while let Some((_, b' ')) = next {
        leading_spaces = leading_spaces.saturating_add(1);
        if leading_spaces > 3 {
            return None;
        }
        next = cursor.next();
    }
    let mut hashes = 0_u8;
    while hashes < level {
        let (_, byte) = next?;
        if byte != b'#' {
            return None;
        }
        hashes = hashes.saturating_add(1);
        next = cursor.next();
    }
    let prefix_end = match next {
        None => line_end,
        Some((position, b'\n' | b'\r')) => position,
        Some((position, b' ' | b'\t')) => {
            let mut end = position.saturating_add(1);
            for (next_position, byte) in cursor {
                if matches!(byte, b' ' | b'\t') {
                    end = next_position.saturating_add(1);
                } else {
                    end = next_position;
                    break;
                }
            }
            end
        }
        Some(_) => return None,
    };
    let start = ByteOffset::try_from(start).ok()?;
    let end = ByteOffset::try_from(prefix_end.min(line_end)).ok()?;
    (end > start).then(|| TextRange::new(start, end)).flatten()
}

fn blockquote_prefix_range(source: &TextSnapshot, line: TextRange) -> Option<TextRange> {
    let start = usize::try_from(line.start()).ok()?;
    let line_end = usize::try_from(line.end()).ok()?;
    let mut cursor = SourceByteCursor::new(source, line)?;
    let mut next = cursor.next();
    let mut leading_spaces = 0_usize;
    while let Some((_, b' ')) = next {
        leading_spaces = leading_spaces.saturating_add(1);
        if leading_spaces > 3 {
            return None;
        }
        next = cursor.next();
    }
    let Some((_, b'>')) = next else {
        return None;
    };
    next = cursor.next();
    let prefix_end = match next {
        None => line_end,
        Some((position, b'\n' | b'\r')) => position,
        Some((position, b' ' | b'\t')) => {
            let mut end = position.saturating_add(1);
            for (next_position, byte) in cursor {
                if matches!(byte, b' ' | b'\t') {
                    end = next_position.saturating_add(1);
                } else {
                    end = next_position;
                    break;
                }
            }
            end
        }
        Some(_) => return None,
    };
    let start = ByteOffset::try_from(start).ok()?;
    let end = ByteOffset::try_from(prefix_end.min(line_end)).ok()?;
    (end > start).then(|| TextRange::new(start, end)).flatten()
}

struct SourceByteCursor<'a> {
    chunks: ChunkCursor<'a>,
    requested_start: usize,
    end: usize,
    current: Option<&'a str>,
    current_start: usize,
    current_index: usize,
}

impl<'a> SourceByteCursor<'a> {
    fn new(source: &'a TextSnapshot, range: TextRange) -> Option<Self> {
        Some(Self {
            chunks: source.chunk_cursor(range.start()).ok()?,
            requested_start: usize::try_from(range.start()).ok()?,
            end: usize::try_from(range.end()).ok()?,
            current: None,
            current_start: 0,
            current_index: 0,
        })
    }
}

impl Iterator for SourceByteCursor<'_> {
    type Item = (usize, u8);

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            if let Some(current) = self.current {
                if self.current_index < self.current_start + current.len()
                    && self.current_index < self.end
                {
                    let local = self.current_index - self.current_start;
                    let position = self.current_index;
                    let byte = current.as_bytes()[local];
                    self.current_index += 1;
                    return Some((position, byte));
                }
                self.current = None;
            }

            let chunk = self.chunks.next()?;
            self.current_start = usize::try_from(chunk.start()).ok()?;
            self.current_index = self.current_start.max(self.requested_start);
            self.current = Some(chunk.text());
            if self.current_index < self.end {
                continue;
            }
            return None;
        }
    }
}

fn block(kind: BlockKind, start: usize, end: usize) -> Block {
    let start = ByteOffset::try_from(start).unwrap_or(ByteOffset::new(u64::MAX));
    let end = ByteOffset::try_from(end).unwrap_or(ByteOffset::new(u64::MAX));
    let range = TextRange::new(start, end).unwrap_or_else(|| TextRange::empty(start));
    Block { kind, range }
}

struct LineCursor<'a> {
    chunks: ChunkCursor<'a>,
    source_len: usize,
    requested_start: usize,
    current_text: Option<&'a str>,
    current_start: usize,
    current_local: usize,
    line_start: usize,
    pending_cr: bool,
    analysis: LineAnalysis,
    source_hash: SourceHash,
    finished: bool,
}

impl<'a> LineCursor<'a> {
    fn new(snapshot: &'a TextSnapshot, start: usize) -> Self {
        let source_len = usize::try_from(snapshot.len_bytes())
            .expect("Snapshot length must fit the platform address space");
        let start_offset = ByteOffset::try_from(start).expect("line start must fit u64");
        let chunks = snapshot
            .chunk_cursor(start_offset)
            .expect("block boundary must be a valid UTF-8 offset");
        Self {
            chunks,
            source_len,
            requested_start: start,
            current_text: None,
            current_start: 0,
            current_local: 0,
            line_start: start,
            pending_cr: false,
            analysis: LineAnalysis::default(),
            source_hash: SourceHash(0),
            finished: start >= source_len,
        }
    }
}

impl Iterator for LineCursor<'_> {
    type Item = Line;

    fn next(&mut self) -> Option<Self::Item> {
        if self.finished {
            return None;
        }

        loop {
            if self.current_text.is_none() {
                let Some(chunk) = self.chunks.next() else {
                    self.finished = true;
                    return (self.line_start < self.source_len).then_some(Line {
                        start: self.line_start,
                        end: self.source_len,
                        analysis: self.analysis,
                        source_hash: self.source_hash,
                    });
                };
                self.current_start =
                    usize::try_from(chunk.start()).expect("chunk offset must fit usize");
                self.current_local = self
                    .requested_start
                    .saturating_sub(self.current_start)
                    .min(chunk.text().len());
                self.current_text = Some(chunk.text());
            }

            let text = self.current_text.expect("current chunk was initialized");
            while self.current_local < text.len() {
                if !self.analysis.wants_input() {
                    let Some(newline) = text.as_bytes()[self.current_local..]
                        .iter()
                        .position(|value| *value == b'\n')
                    else {
                        self.source_hash =
                            extend_hash(self.source_hash, &text.as_bytes()[self.current_local..]);
                        self.current_local = text.len();
                        break;
                    };
                    let consumed_end = self.current_local + newline + 1;
                    self.source_hash = extend_hash(
                        self.source_hash,
                        &text.as_bytes()[self.current_local..consumed_end],
                    );
                    let absolute = self.current_start + self.current_local + newline;
                    let line = Line {
                        start: self.line_start,
                        end: absolute + 1,
                        analysis: self.analysis,
                        source_hash: self.source_hash,
                    };
                    self.line_start = absolute + 1;
                    self.analysis = LineAnalysis::default();
                    self.source_hash = SourceHash(0);
                    self.pending_cr = false;
                    self.current_local = consumed_end;
                    return Some(line);
                }

                let character_start = self.current_local;
                let first = text.as_bytes()[character_start];
                let character = if first.is_ascii() {
                    char::from(first)
                } else {
                    text[self.current_local..]
                        .chars()
                        .next()
                        .expect("non-empty UTF-8 tail must contain a character")
                };
                let absolute = self.current_start + self.current_local;
                self.current_local += character.len_utf8();
                self.source_hash = extend_hash(
                    self.source_hash,
                    &text.as_bytes()[character_start..self.current_local],
                );
                if character == '\n' {
                    let line = Line {
                        start: self.line_start,
                        end: absolute + 1,
                        analysis: self.analysis,
                        source_hash: self.source_hash,
                    };
                    self.line_start = absolute + 1;
                    self.analysis = LineAnalysis::default();
                    self.source_hash = SourceHash(0);
                    self.pending_cr = false;
                    return Some(line);
                }

                if self.pending_cr {
                    self.analysis.push('\r');
                    self.pending_cr = false;
                }
                if character == '\r' {
                    self.pending_cr = true;
                } else {
                    self.analysis.push(character);
                }
            }
            self.current_text = None;
        }
    }
}

const HASH_BASE: u64 = 0x0000_0100_0000_01b3;

fn extend_hash(mut hash: SourceHash, bytes: &[u8]) -> SourceHash {
    for byte in bytes {
        hash.0 = hash
            .0
            .wrapping_mul(HASH_BASE)
            .wrapping_add(u64::from(*byte) + 1);
    }
    hash
}

fn concatenate_hash(left: SourceHash, right: SourceHash, right_len: usize) -> SourceHash {
    SourceHash(
        left.0
            .wrapping_mul(wrapping_power(HASH_BASE, right_len))
            .wrapping_add(right.0),
    )
}

fn wrapping_power(mut base: u64, mut exponent: usize) -> u64 {
    let mut result = 1_u64;
    while exponent > 0 {
        if exponent & 1 == 1 {
            result = result.wrapping_mul(base);
        }
        base = base.wrapping_mul(base);
        exponent >>= 1;
    }
    result
}

fn map_unchanged_range(
    revision: Revision,
    range: TextRange,
    changes: &ChangeSet,
) -> Result<TextRange, AnchorMapError> {
    let start = changes
        .map_anchor(TextAnchor::new(revision, range.start(), Affinity::After))?
        .offset();
    let end = changes
        .map_anchor(TextAnchor::new(revision, range.end(), Affinity::Before))?
        .offset();
    Ok(TextRange::new(start, end).expect("an unaffected mapped block must remain ordered"))
}

fn records_converge(
    old: ResolvedBlockRecord,
    mapped_old_range: TextRange,
    new: &BlockRecord,
    old_source: &TextSnapshot,
    new_source: &TextSnapshot,
) -> bool {
    mapped_old_range == new.block.range
        && old.block.kind == new.block.kind
        && old.start_state == new.start_state
        && old.end_state == new.end_state
        && old.source_hash == new.source_hash
        && ranges_equal(old_source, old.block.range, new_source, new.block.range)
}

fn ranges_equal(
    left_source: &TextSnapshot,
    left_range: TextRange,
    right_source: &TextSnapshot,
    right_range: TextRange,
) -> bool {
    left_range.len() == right_range.len()
        && RangeSlices::new(left_source, left_range)
            .flat_map(|slice| slice.iter().copied())
            .eq(RangeSlices::new(right_source, right_range).flat_map(|slice| slice.iter().copied()))
}

struct RangeSlices<'a> {
    chunks: ChunkCursor<'a>,
    start: usize,
    end: usize,
}

impl<'a> RangeSlices<'a> {
    fn new(snapshot: &'a TextSnapshot, range: TextRange) -> Self {
        Self {
            chunks: snapshot
                .chunk_cursor(range.start())
                .expect("block ranges must start at valid UTF-8 boundaries"),
            start: usize::try_from(range.start()).expect("block offset must fit usize"),
            end: usize::try_from(range.end()).expect("block offset must fit usize"),
        }
    }
}

impl<'a> Iterator for RangeSlices<'a> {
    type Item = &'a [u8];

    fn next(&mut self) -> Option<Self::Item> {
        for chunk in self.chunks.by_ref() {
            let chunk_start = usize::try_from(chunk.start()).expect("chunk offset must fit usize");
            if chunk_start >= self.end {
                return None;
            }
            let chunk_end = chunk_start + chunk.text().len();
            let start = self.start.max(chunk_start) - chunk_start;
            let end = self.end.min(chunk_end) - chunk_start;
            if start < end {
                return Some(&chunk.text().as_bytes()[start..end]);
            }
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use yu_text::{Edit, StorageBackend, TextBuffer, Transaction, retained_snapshot_stats};

    #[test]
    fn scanner_covers_source_without_gaps() {
        let source = "# 羽\n\nparagraph\ncontinued\n\n```rust\nfn main() {}\n```\n";
        let buffer = TextBuffer::new(source);
        let document = parse(&buffer.snapshot());

        assert!(document.has_lossless_coverage());
        assert_eq!(document.source_len().get(), source.len() as u64);
        assert_eq!(document.blocks().len(), 5);
        assert_eq!(kind_at(&document, 0), BlockKind::AtxHeading { level: 1 });
        assert_eq!(kind_at(&document, 1), BlockKind::BlankLine);
        assert_eq!(kind_at(&document, 2), BlockKind::Paragraph);
        assert_eq!(
            kind_at(&document, 4),
            BlockKind::FencedCodeBlock {
                marker: '`',
                closed: true
            }
        );
    }

    #[test]
    fn block_sequence_resolves_source_caret_boundaries_without_linear_scan() {
        let source = "# title\n\nparagraph\n";
        let buffer = TextBuffer::new(source);
        let document = parse(&buffer.snapshot());
        let blocks = document.blocks();

        assert_eq!(blocks.block_index_for_offset(ByteOffset::new(0)), Some(0));
        assert_eq!(blocks.block_index_for_offset(ByteOffset::new(8)), Some(1));
        assert_eq!(blocks.block_index_for_offset(ByteOffset::new(9)), Some(2));
        assert_eq!(
            blocks.block_index_for_offset(ByteOffset::new(source.len() as u64)),
            Some(2)
        );
        assert_eq!(
            blocks.block_index_for_offset(ByteOffset::new(source.len() as u64 + 1)),
            None
        );
        assert_eq!(
            blocks.block_index_range_for_source_range(
                TextRange::new(ByteOffset::new(8), ByteOffset::new(10))
                    .expect("test range should be valid")
            ),
            Some(1..3)
        );
        assert_eq!(
            blocks.block_index_range_for_source_range(
                TextRange::new(ByteOffset::new(9), ByteOffset::new(18))
                    .expect("test range should be valid")
            ),
            Some(2..3)
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
            assert_eq!(kind_at(&document, 0), expected, "source {source:?}");
            assert!(document.has_lossless_coverage());
        }
    }

    #[test]
    fn scanner_classifies_blockquotes_and_list_items_without_losing_ranges() {
        let source = "> quoted\n> continued\n\n- one\n  continuation\n  - nested\n- two\n\n1. first\n2) second\n";
        let buffer = TextBuffer::new(source);
        let snapshot = buffer.snapshot();
        let document = parse(&snapshot);

        assert!(document.has_lossless_coverage());
        assert_eq!(document.blocks().len(), 8);
        assert_eq!(kind_at(&document, 0), BlockKind::BlockQuote { depth: 1 });
        assert_eq!(kind_at(&document, 1), BlockKind::BlankLine);
        assert_eq!(
            kind_at(&document, 2),
            BlockKind::ListItem {
                ordered: false,
                depth: 0,
                marker: '-',
                start: 1,
            }
        );
        assert_eq!(
            kind_at(&document, 3),
            BlockKind::ListItem {
                ordered: false,
                depth: 1,
                marker: '-',
                start: 1,
            }
        );
        assert_eq!(
            kind_at(&document, 4),
            BlockKind::ListItem {
                ordered: false,
                depth: 0,
                marker: '-',
                start: 1,
            }
        );
        assert_eq!(kind_at(&document, 5), BlockKind::BlankLine);
        assert_eq!(
            kind_at(&document, 6),
            BlockKind::ListItem {
                ordered: true,
                depth: 0,
                marker: '.',
                start: 1,
            }
        );
        assert_eq!(
            kind_at(&document, 7),
            BlockKind::ListItem {
                ordered: true,
                depth: 0,
                marker: ')',
                start: 2,
            }
        );

        let reconstructed: String = document
            .blocks()
            .iter()
            .map(|block| {
                let start = usize::try_from(block.range().start()).expect("offset fits usize");
                let end = usize::try_from(block.range().end()).expect("offset fits usize");
                &snapshot.as_str()[start..end]
            })
            .collect();
        assert_eq!(reconstructed, source);
    }

    #[test]
    fn block_syntax_hidden_ranges_are_parser_owned_and_line_local() {
        let source = "  ## 标题\n\n> 引用\n  延续\n\n- [ ] 任务\n";
        let buffer = TextBuffer::new(source);
        let snapshot = buffer.snapshot();
        let document = parse(&snapshot);

        let heading = document.blocks().get(0).expect("heading block");
        let heading_ranges = block_syntax_hidden_ranges(&snapshot, heading);
        assert_eq!(heading_ranges.len(), 1);
        assert_eq!(
            &snapshot.as_str()
                [heading_ranges[0].start().get() as usize..heading_ranges[0].end().get() as usize],
            "  ## "
        );

        let quote = document.blocks().get(2).expect("quote block");
        let quote_ranges = block_syntax_hidden_ranges(&snapshot, quote);
        assert_eq!(quote_ranges.len(), 1);
        assert_eq!(
            &snapshot.as_str()
                [quote_ranges[0].start().get() as usize..quote_ranges[0].end().get() as usize],
            "> "
        );

        let task = document.blocks().get(4).expect("task block");
        assert!(block_syntax_hidden_ranges(&snapshot, task).is_empty());
    }

    #[test]
    fn scanner_classifies_task_list_items_and_exposes_marker_ranges() {
        let source = "- [ ] todo\n1. [x] done\n- [X] done\n- [x]attached\n";
        let snapshot = TextBuffer::new(source).snapshot();
        let document = parse(&snapshot);

        assert_eq!(document.blocks().len(), 4);
        assert_eq!(
            kind_at(&document, 0),
            BlockKind::TaskListItem {
                ordered: false,
                depth: 0,
                marker: '-',
                start: 1,
                state: TaskState::Todo,
            }
        );
        assert_eq!(
            kind_at(&document, 1),
            BlockKind::TaskListItem {
                ordered: true,
                depth: 0,
                marker: '.',
                start: 1,
                state: TaskState::Done,
            }
        );
        assert_eq!(
            kind_at(&document, 2),
            BlockKind::TaskListItem {
                ordered: false,
                depth: 0,
                marker: '-',
                start: 1,
                state: TaskState::Done,
            }
        );
        assert_eq!(
            kind_at(&document, 3),
            BlockKind::ListItem {
                ordered: false,
                depth: 0,
                marker: '-',
                start: 1,
            }
        );

        let marker = task_marker(
            &snapshot,
            document.blocks().get(0).expect("task block should exist"),
        )
        .expect("task marker should be source-backed");
        assert_eq!(marker.state(), TaskState::Todo);
        assert_eq!(
            &snapshot.as_str()[usize::try_from(marker.range().start()).expect("offset")
                ..usize::try_from(marker.range().end()).expect("offset")],
            "[ ]"
        );
        assert!(document.has_lossless_coverage());
    }

    #[test]
    fn scanner_does_not_treat_attached_markers_as_list_items() {
        let source = "-attached\n1.attached\n*attached\n";
        let document = parse(&TextBuffer::new(source).snapshot());

        assert_eq!(document.blocks().len(), 1);
        assert_eq!(kind_at(&document, 0), BlockKind::Paragraph);
        assert!(document.has_lossless_coverage());
    }

    #[test]
    fn scanner_extracts_source_backed_reference_definitions() {
        let source = "[Project Link]: <https://example.com> \"title\"\n[other]: /docs\n\n[Project Link]\n![other]\n";
        let snapshot = TextBuffer::new(source).snapshot();
        let document = parse(&snapshot);

        assert!(document.has_lossless_coverage());
        assert_eq!(kind_at(&document, 0), BlockKind::ReferenceDefinition);
        assert_eq!(kind_at(&document, 1), BlockKind::ReferenceDefinition);
        assert_eq!(kind_at(&document, 2), BlockKind::BlankLine);
        assert_eq!(kind_at(&document, 3), BlockKind::Paragraph);
        assert_eq!(document.reference_definitions().definitions().len(), 2);

        let paragraph = document.blocks().get(3).expect("paragraph should exist");
        let label_start = source[paragraph.range().start().get() as usize..]
            .find("Project Link")
            .expect("shortcut label should exist") as u64
            + paragraph.range().start().get();
        let label = TextRange::new(
            ByteOffset::new(label_start),
            ByteOffset::new(label_start + "Project Link".len() as u64),
        )
        .expect("label range should be ordered");
        let definition = document
            .reference_definitions()
            .lookup(&snapshot, label)
            .expect("definition should resolve case-insensitively");
        assert_eq!(
            &source[definition.destination().start().get() as usize
                ..definition.destination().end().get() as usize],
            "https://example.com"
        );
    }

    #[test]
    fn four_space_indented_definition_remains_literal_paragraph_text() {
        let document = parse(&TextBuffer::new("    [id]: /docs\n").snapshot());
        assert_eq!(document.blocks().len(), 1);
        assert_eq!(kind_at(&document, 0), BlockKind::Paragraph);
        assert!(document.reference_definitions().definitions().is_empty());
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
        assert_eq!(kind_at(&document, 0), BlockKind::AtxHeading { level: 1 });
        assert_eq!(kind_at(&document, 1), BlockKind::BlankLine);
        assert_eq!(
            kind_at(&document, 2),
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
    fn reference_definition_scan_stays_chunk_aware() {
        let parts = [
            "prefix\n\n[",
            "project",
            "]: <",
            "https://example.com",
            ">\n\n[project]\n",
        ];
        let mut buffer = TextBuffer::with_backend("", StorageBackend::PieceTree);
        for part in parts {
            let at = buffer.snapshot().len_bytes();
            let transaction =
                Transaction::new(buffer.revision(), [Edit::new(TextRange::empty(at), part)]);
            buffer
                .apply(&transaction)
                .expect("append transaction should apply");
        }
        let snapshot = buffer.snapshot();
        let document = parse(&snapshot);

        assert_eq!(document.reference_definitions().definitions().len(), 1);
        assert_eq!(
            retained_snapshot_stats(std::slice::from_ref(&snapshot)).materialized_buffers(),
            0
        );
    }

    #[test]
    fn unclosed_fence_owns_the_remaining_source() {
        let buffer = TextBuffer::new("before\n\n~~~\ninside\n");
        let document = parse(&buffer.snapshot());

        assert!(document.has_lossless_coverage());
        assert_eq!(
            kind_at(&document, 2),
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
    fn incremental_parse_rebuilds_reference_definition_index() {
        let source = "[id]: /docs\n\n[id]\n";
        let mut buffer = TextBuffer::new(source);
        let previous = parse(&buffer.snapshot());
        let label_start = source.find("id").expect("definition label should exist");
        let transaction = Transaction::new(
            buffer.revision(),
            [Edit::new(
                TextRange::new(
                    ByteOffset::new(label_start as u64),
                    ByteOffset::new((label_start + 2) as u64),
                )
                .expect("label range should be ordered"),
                "new",
            )],
        );
        let applied = buffer
            .apply(&transaction)
            .expect("definition edit should apply");
        let incremental =
            parse_incremental(&previous, applied.result_snapshot(), applied.change_set())
                .expect("definition edit should parse incrementally");
        let full = parse(applied.result_snapshot());

        assert_eq!(incremental.document(), &full);
        assert_ne!(
            previous.reference_definitions().fingerprint(),
            full.reference_definitions().fingerprint()
        );
        assert_eq!(full.reference_definitions().definitions().len(), 1);
    }

    #[test]
    fn incremental_parse_reclassifies_task_state_like_full_parse() {
        let source = "- [ ] todo\n\n- [x] done\n";
        let mut buffer = TextBuffer::new(source);
        let previous = parse(&buffer.snapshot());
        let state_offset = source.find("[ ]").expect("todo marker should exist") + 1;
        let transaction = Transaction::new(
            buffer.revision(),
            [Edit::new(
                TextRange::new(
                    ByteOffset::new(state_offset as u64),
                    ByteOffset::new((state_offset + 1) as u64),
                )
                .expect("task state range should be ordered"),
                "x",
            )],
        );
        let applied = buffer
            .apply(&transaction)
            .expect("task state edit should apply");
        let incremental =
            parse_incremental(&previous, applied.result_snapshot(), applied.change_set())
                .expect("task state edit should parse incrementally");
        let full = parse(applied.result_snapshot());

        assert_eq!(incremental.document(), &full);
        assert!(matches!(
            kind_at(&full, 0),
            BlockKind::TaskListItem {
                state: TaskState::Done,
                ..
            }
        ));
        assert!(matches!(
            kind_at(&full, 2),
            BlockKind::TaskListItem {
                state: TaskState::Done,
                ..
            }
        ));
        assert!(
            incremental
                .reparsed_range()
                .contains(ByteOffset::new(state_offset as u64))
        );
    }

    #[test]
    fn empty_incremental_parse_rebinds_definition_index_revision() {
        let mut buffer = TextBuffer::new("[id]: /docs\n\n[id]\n");
        let previous = parse(&buffer.snapshot());
        let transaction = Transaction::new(buffer.revision(), std::iter::empty::<Edit>());
        let applied = buffer
            .apply(&transaction)
            .expect("empty transaction should apply");
        let incremental =
            parse_incremental(&previous, applied.result_snapshot(), applied.change_set())
                .expect("empty edit should parse incrementally");

        assert_eq!(
            incremental.document().reference_definitions().revision(),
            applied.result_snapshot().revision()
        );
        assert_eq!(
            incremental
                .document()
                .reference_definitions()
                .definitions()
                .len(),
            1
        );
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

    fn kind_at(document: &MarkdownDocument, index: usize) -> BlockKind {
        document
            .blocks()
            .get(index)
            .expect("test block must exist")
            .kind()
    }
}
