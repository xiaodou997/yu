use std::collections::HashSet;
use std::ops::Range;
use std::sync::Arc;

use yu_core::{ByteOffset, TextRange};

/// The block shapes recognized by the Phase 1 scanner.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BlockKind {
    BlankLine,
    Paragraph,
    AtxHeading { level: u8 },
    FencedCodeBlock { marker: char, closed: bool },
}

/// Parser state at a reusable block boundary.
///
/// Phase 1 materializes a complete fenced block as one block, so reusable
/// boundaries are normally `Normal`. The fenced state records an unterminated
/// block at EOF and keeps the synchronization contract explicit for future
/// container and inline states.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum BlockState {
    #[default]
    Normal,
    Fenced {
        marker: char,
        minimum: usize,
    },
}

/// A block that refers to source without owning or normalizing its text.
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
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct BlockStorageStats {
    blocks: usize,
    segments: usize,
    allocations: usize,
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
        let allocations = self
            .segments
            .iter()
            .map(|segment| Arc::as_ptr(&segment.allocation) as *const () as usize)
            .collect::<HashSet<_>>()
            .len();
        BlockStorageStats {
            blocks: self.len,
            segments: self.segments.len(),
            allocations,
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
