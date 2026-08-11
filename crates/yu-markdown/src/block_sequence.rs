use std::collections::HashSet;
use std::ops::Range;
use std::sync::Arc;

use yu_core::{ByteOffset, TextRange};

/// The block shapes recognized by the lossless block parser.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BlockKind {
    BlankLine,
    /// A source-backed link definition such as `[project]: /docs`.
    ReferenceDefinition,
    Paragraph,
    AtxHeading {
        level: u8,
    },
    FencedCodeBlock {
        marker: char,
        closed: bool,
    },
    /// A contiguous blockquote segment. `depth` is the number of leading
    /// quote markers recognized at the block boundary.
    BlockQuote {
        depth: u8,
    },
    /// One list item, including any indented continuation lines that belong to
    /// it. Nested items are represented as separate records with a larger
    /// `depth`; source ranges remain the canonical structure for now.
    ListItem {
        ordered: bool,
        depth: u8,
        marker: char,
        start: u32,
    },
}

/// Parser state at a reusable block boundary.
///
/// A parser state at a reusable block boundary. Phase 1 materializes complete
/// fenced/container segments as blocks, so reusable boundaries are normally
/// `Normal`. Non-normal states remain explicit so incremental convergence can
/// grow to nested containers without changing the record contract.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum BlockState {
    #[default]
    Normal,
    Fenced {
        marker: char,
        minimum: usize,
    },
}

/// A root-level lossless CST node that refers to source without owning or
/// normalizing its text. Container nodes currently use `BlockKind` metadata
/// and source ranges; a nested child arena is intentionally deferred until a
/// second consumer needs stable node identity.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Block {
    pub(crate) kind: BlockKind,
    pub(crate) range: TextRange,
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct SourceHash(pub(crate) u64);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct BlockRecord {
    pub(crate) block: Block,
    pub(crate) start_state: BlockState,
    pub(crate) end_state: BlockState,
    pub(crate) source_hash: SourceHash,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ResolvedBlockRecord {
    pub(crate) block: Block,
    pub(crate) start_state: BlockState,
    pub(crate) end_state: BlockState,
    pub(crate) source_hash: SourceHash,
}

impl BlockRecord {
    fn resolved(self, delta: i128) -> ResolvedBlockRecord {
        ResolvedBlockRecord {
            block: Block {
                kind: self.block.kind,
                range: shift_range(self.block.range, delta),
            },
            start_state: self.start_state,
            end_state: self.end_state,
            source_hash: self.source_hash,
        }
    }
}

#[derive(Clone, Debug)]
struct BlockSegment {
    allocation: Arc<[BlockRecord]>,
    records: Range<usize>,
    byte_delta: i128,
}

impl BlockSegment {
    fn len(&self) -> usize {
        self.records.len()
    }
}

/// Structural metrics for the immutable block sequence.
///
/// Byte estimates include record and segment payloads, but not allocator or
/// `Arc` headers.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct BlockStorageStats {
    blocks: usize,
    segments: usize,
    allocations: usize,
    retained_records: usize,
    segment_bytes: usize,
    record_bytes: usize,
}

impl BlockStorageStats {
    #[must_use]
    pub const fn blocks(self) -> usize {
        self.blocks
    }

    #[must_use]
    pub const fn segments(self) -> usize {
        self.segments
    }

    #[must_use]
    pub const fn allocations(self) -> usize {
        self.allocations
    }

    #[must_use]
    pub const fn retained_records(self) -> usize {
        self.retained_records
    }

    #[must_use]
    pub const fn reclaimable_records(self) -> usize {
        self.retained_records.saturating_sub(self.blocks)
    }

    #[must_use]
    pub const fn segment_bytes(self) -> usize {
        self.segment_bytes
    }

    #[must_use]
    pub const fn record_bytes(self) -> usize {
        self.record_bytes
    }

    #[must_use]
    pub const fn estimated_bytes(self) -> usize {
        self.segment_bytes.saturating_add(self.record_bytes)
    }
}

/// De-duplicated allocations retained by one or more block sequences.
///
/// Byte estimates include allocation payloads, but not allocator or `Arc`
/// headers.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RetainedBlockStats {
    sequences: usize,
    block_references: usize,
    segment_tables: usize,
    segments: usize,
    segment_bytes: usize,
    block_allocations: usize,
    block_records: usize,
    block_record_bytes: usize,
}

impl RetainedBlockStats {
    #[must_use]
    pub const fn sequences(self) -> usize {
        self.sequences
    }

    #[must_use]
    pub const fn block_references(self) -> usize {
        self.block_references
    }

    #[must_use]
    pub const fn segment_tables(self) -> usize {
        self.segment_tables
    }

    #[must_use]
    pub const fn segments(self) -> usize {
        self.segments
    }

    #[must_use]
    pub const fn segment_bytes(self) -> usize {
        self.segment_bytes
    }

    #[must_use]
    pub const fn block_allocations(self) -> usize {
        self.block_allocations
    }

    #[must_use]
    pub const fn block_records(self) -> usize {
        self.block_records
    }

    #[must_use]
    pub const fn block_record_bytes(self) -> usize {
        self.block_record_bytes
    }

    #[must_use]
    pub const fn estimated_bytes(self) -> usize {
        self.segment_bytes.saturating_add(self.block_record_bytes)
    }
}

/// Policy used by an idle task to decide when block records should be packed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BlockCompactionPolicy {
    max_segments: usize,
    max_retained_ratio: usize,
    min_reclaimable_records: usize,
}

impl BlockCompactionPolicy {
    #[must_use]
    pub const fn new(
        max_segments: usize,
        max_retained_ratio: usize,
        min_reclaimable_records: usize,
    ) -> Option<Self> {
        if max_segments == 0 || max_retained_ratio == 0 {
            return None;
        }
        Some(Self {
            max_segments,
            max_retained_ratio,
            min_reclaimable_records,
        })
    }

    #[must_use]
    pub const fn max_segments(self) -> usize {
        self.max_segments
    }

    #[must_use]
    pub const fn max_retained_ratio(self) -> usize {
        self.max_retained_ratio
    }

    #[must_use]
    pub const fn min_reclaimable_records(self) -> usize {
        self.min_reclaimable_records
    }

    #[must_use]
    pub const fn should_compact(self, stats: BlockStorageStats) -> bool {
        if stats.blocks == 0 {
            return false;
        }
        if stats.segments > self.max_segments {
            return true;
        }
        stats.reclaimable_records() >= self.min_reclaimable_records
            && stats.retained_records > stats.blocks.saturating_mul(self.max_retained_ratio)
    }
}

impl Default for BlockCompactionPolicy {
    fn default() -> Self {
        Self {
            max_segments: 4_096,
            max_retained_ratio: 4,
            min_reclaimable_records: 8_192,
        }
    }
}

/// A persistent sequence assembled from shared immutable block allocations.
///
/// Segments can apply a lazy byte delta, allowing an unchanged suffix to keep
/// its old block allocation after an edit shifts every absolute source range.
#[derive(Clone, Debug, Default)]
pub struct BlockSequence {
    segments: Arc<[BlockSegment]>,
    len: usize,
}

impl BlockSequence {
    pub(crate) fn from_records(records: Vec<BlockRecord>) -> Self {
        if records.is_empty() {
            return Self::default();
        }
        let allocation: Arc<[BlockRecord]> = records.into();
        let len = allocation.len();
        Self {
            segments: Arc::from([BlockSegment {
                allocation,
                records: 0..len,
                byte_delta: 0,
            }]),
            len,
        }
    }

    pub(crate) fn assemble(
        prefix: (&Self, Range<usize>),
        middle: Vec<BlockRecord>,
        suffix: (&Self, Range<usize>, i128),
    ) -> Self {
        let mut segments = Vec::new();
        prefix.0.append_slice(&mut segments, prefix.1, 0);
        if !middle.is_empty() {
            let allocation: Arc<[BlockRecord]> = middle.into();
            let len = allocation.len();
            push_segment(
                &mut segments,
                BlockSegment {
                    allocation,
                    records: 0..len,
                    byte_delta: 0,
                },
            );
        }
        suffix.0.append_slice(&mut segments, suffix.1, suffix.2);
        let len = segments.iter().map(BlockSegment::len).sum();
        Self {
            segments: segments.into(),
            len,
        }
    }

    fn append_slice(
        &self,
        destination: &mut Vec<BlockSegment>,
        requested: Range<usize>,
        additional_delta: i128,
    ) {
        assert!(requested.start <= requested.end && requested.end <= self.len);
        if requested.is_empty() {
            return;
        }

        let mut sequence_start = 0;
        for segment in self.segments.iter() {
            let sequence_end = sequence_start + segment.len();
            let overlap_start = requested.start.max(sequence_start);
            let overlap_end = requested.end.min(sequence_end);
            if overlap_start < overlap_end {
                let local_start = segment.records.start + overlap_start - sequence_start;
                let local_end = segment.records.start + overlap_end - sequence_start;
                push_segment(
                    destination,
                    BlockSegment {
                        allocation: Arc::clone(&segment.allocation),
                        records: local_start..local_end,
                        byte_delta: segment.byte_delta + additional_delta,
                    },
                );
            }
            if sequence_end >= requested.end {
                break;
            }
            sequence_start = sequence_end;
        }
    }

    #[must_use]
    pub const fn len(&self) -> usize {
        self.len
    }

    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.len == 0
    }

    #[must_use]
    pub fn get(&self, index: usize) -> Option<Block> {
        self.resolved_record(index).map(|record| record.block)
    }

    #[must_use]
    pub fn iter(&self) -> BlockIter<'_> {
        BlockIter {
            records: self.resolved_records_from(0),
        }
    }

    #[must_use]
    pub fn storage_stats(&self) -> BlockStorageStats {
        let mut allocation_ids = HashSet::new();
        let mut retained_records = 0_usize;
        for segment in self.segments.iter() {
            let allocation_id = Arc::as_ptr(&segment.allocation) as *const () as usize;
            if allocation_ids.insert(allocation_id) {
                retained_records = retained_records.saturating_add(segment.allocation.len());
            }
        }
        let segment_bytes = self
            .segments
            .len()
            .saturating_mul(std::mem::size_of::<BlockSegment>());
        let record_bytes = retained_records.saturating_mul(std::mem::size_of::<BlockRecord>());
        BlockStorageStats {
            blocks: self.len,
            segments: self.segments.len(),
            allocations: allocation_ids.len(),
            retained_records,
            segment_bytes,
            record_bytes,
        }
    }

    /// Counts records whose immutable allocation is retained by both sequences.
    #[must_use]
    pub fn shared_blocks_with(&self, other: &Self) -> usize {
        let mut shared = 0;
        for left in self.segments.iter() {
            for right in other.segments.iter() {
                if !Arc::ptr_eq(&left.allocation, &right.allocation) {
                    continue;
                }
                let start = left.records.start.max(right.records.start);
                let end = left.records.end.min(right.records.end);
                shared += end.saturating_sub(start);
            }
        }
        shared
    }

    pub(crate) fn resolved_record(&self, index: usize) -> Option<ResolvedBlockRecord> {
        if index >= self.len {
            return None;
        }
        let mut sequence_start = 0;
        for segment in self.segments.iter() {
            let sequence_end = sequence_start + segment.len();
            if index < sequence_end {
                let local = segment.records.start + index - sequence_start;
                return Some(segment.allocation[local].resolved(segment.byte_delta));
            }
            sequence_start = sequence_end;
        }
        None
    }

    pub(crate) fn first_ending_after(&self, offset: ByteOffset) -> usize {
        let mut sequence_start = 0;
        for segment in self.segments.iter() {
            let records = &segment.allocation[segment.records.clone()];
            let local = records.partition_point(|record| {
                record.resolved(segment.byte_delta).block.range.end() <= offset
            });
            if local < records.len() {
                return sequence_start + local;
            }
            sequence_start += records.len();
        }
        self.len
    }

    pub(crate) fn first_starting_at_or_after(&self, offset: ByteOffset) -> usize {
        let mut sequence_start = 0;
        for segment in self.segments.iter() {
            let records = &segment.allocation[segment.records.clone()];
            let local = records.partition_point(|record| {
                record.resolved(segment.byte_delta).block.range.start() < offset
            });
            if local < records.len() {
                return sequence_start + local;
            }
            sequence_start += records.len();
        }
        self.len
    }

    pub(crate) fn resolved_records_from(&self, index: usize) -> ResolvedBlockIter<'_> {
        assert!(index <= self.len);
        let mut remaining = index;
        let mut segment_index = 0;
        while segment_index < self.segments.len() {
            let len = self.segments[segment_index].len();
            if remaining < len {
                break;
            }
            remaining -= len;
            segment_index += 1;
        }
        let local_index = self
            .segments
            .get(segment_index)
            .map_or(0, |segment| segment.records.start + remaining);
        ResolvedBlockIter {
            segments: &self.segments,
            segment_index,
            local_index,
        }
    }

    pub(crate) fn compacted(&self) -> Self {
        let records = self
            .resolved_records_from(0)
            .map(|record| BlockRecord {
                block: record.block,
                start_state: record.start_state,
                end_state: record.end_state,
                source_hash: record.source_hash,
            })
            .collect();
        Self::from_records(records)
    }
}

pub(crate) fn retained_block_stats<'a>(
    sequences: impl IntoIterator<Item = &'a BlockSequence>,
) -> RetainedBlockStats {
    let mut stats = RetainedBlockStats::default();
    let mut segment_table_ids = HashSet::new();
    let mut block_allocation_ids = HashSet::new();

    for sequence in sequences {
        stats.sequences += 1;
        stats.block_references = stats.block_references.saturating_add(sequence.len);

        if !sequence.segments.is_empty() {
            let table_id = Arc::as_ptr(&sequence.segments) as *const () as usize;
            if segment_table_ids.insert(table_id) {
                stats.segment_tables += 1;
                stats.segments = stats.segments.saturating_add(sequence.segments.len());
                stats.segment_bytes = stats.segment_bytes.saturating_add(
                    sequence
                        .segments
                        .len()
                        .saturating_mul(std::mem::size_of::<BlockSegment>()),
                );
            }
        }

        for segment in sequence.segments.iter() {
            let allocation_id = Arc::as_ptr(&segment.allocation) as *const () as usize;
            if block_allocation_ids.insert(allocation_id) {
                stats.block_allocations += 1;
                stats.block_records = stats.block_records.saturating_add(segment.allocation.len());
                stats.block_record_bytes = stats.block_record_bytes.saturating_add(
                    segment
                        .allocation
                        .len()
                        .saturating_mul(std::mem::size_of::<BlockRecord>()),
                );
            }
        }
    }

    stats
}

impl PartialEq for BlockSequence {
    fn eq(&self, other: &Self) -> bool {
        self.len == other.len
            && self
                .resolved_records_from(0)
                .eq(other.resolved_records_from(0))
    }
}

impl Eq for BlockSequence {}

impl<'a> IntoIterator for &'a BlockSequence {
    type Item = Block;
    type IntoIter = BlockIter<'a>;

    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

pub struct BlockIter<'a> {
    records: ResolvedBlockIter<'a>,
}

impl Iterator for BlockIter<'_> {
    type Item = Block;

    fn next(&mut self) -> Option<Self::Item> {
        self.records.next().map(|record| record.block)
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        self.records.size_hint()
    }
}

pub(crate) struct ResolvedBlockIter<'a> {
    segments: &'a [BlockSegment],
    segment_index: usize,
    local_index: usize,
}

impl Iterator for ResolvedBlockIter<'_> {
    type Item = ResolvedBlockRecord;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            let segment = self.segments.get(self.segment_index)?;
            if self.local_index < segment.records.end {
                let record = segment.allocation[self.local_index].resolved(segment.byte_delta);
                self.local_index += 1;
                return Some(record);
            }
            self.segment_index += 1;
            let next = self.segments.get(self.segment_index)?;
            self.local_index = next.records.start;
        }
    }
}

fn push_segment(segments: &mut Vec<BlockSegment>, segment: BlockSegment) {
    if segment.records.is_empty() {
        return;
    }
    if let Some(previous) = segments.last_mut()
        && Arc::ptr_eq(&previous.allocation, &segment.allocation)
        && previous.byte_delta == segment.byte_delta
        && previous.records.end == segment.records.start
    {
        previous.records.end = segment.records.end;
        return;
    }
    segments.push(segment);
}

fn shift_range(range: TextRange, delta: i128) -> TextRange {
    let start = shift_offset(range.start(), delta);
    let end = shift_offset(range.end(), delta);
    TextRange::new(start, end).expect("shifting a valid range must preserve ordering")
}

fn shift_offset(offset: ByteOffset, delta: i128) -> ByteOffset {
    let shifted = i128::from(offset.get()) + delta;
    let shifted = u64::try_from(shifted).expect("shared block offset must remain in u64 range");
    ByteOffset::new(shifted)
}
