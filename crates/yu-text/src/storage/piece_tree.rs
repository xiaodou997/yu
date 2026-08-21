use std::mem;
use std::ops::Range;
use std::sync::Arc;

use super::{AllocationCollector, StorageBackend, StorageChunk, StorageStats};
use crate::TextSummary;
use crate::summary::{
    byte_after_line_break, byte_offset_for_utf16 as byte_offset_for_utf16_in_text,
};

const SUMMARY_CHECKPOINT_BYTES: usize = 4 * 1024;
type Link = Option<Arc<Node>>;

#[derive(Clone, Copy, Debug)]
struct SummaryCheckpoint {
    byte: usize,
    summary: TextSummary,
}

#[derive(Debug)]
struct PieceBuffer {
    text: Arc<str>,
    checkpoints: Box<[SummaryCheckpoint]>,
    summary: TextSummary,
}

impl PieceBuffer {
    fn new(text: Arc<str>) -> Arc<Self> {
        let mut checkpoints = vec![SummaryCheckpoint {
            byte: 0,
            summary: TextSummary::EMPTY,
        }];
        let mut summary = TextSummary::EMPTY;
        let mut checkpoint_after = SUMMARY_CHECKPOINT_BYTES;

        for (byte, character) in text.char_indices() {
            if byte >= checkpoint_after {
                checkpoints.push(SummaryCheckpoint { byte, summary });
                checkpoint_after = byte.saturating_add(SUMMARY_CHECKPOINT_BYTES);
            }
            summary = summary.plus(TextSummary::from_char(character));
        }

        Arc::new(Self {
            text,
            checkpoints: checkpoints.into_boxed_slice(),
            summary,
        })
    }

    fn prefix_summary(&self, offset: usize) -> TextSummary {
        debug_assert!(offset <= self.text.len());
        debug_assert!(self.text.is_char_boundary(offset));
        if offset == self.text.len() {
            return self.summary;
        }
        let index = self
            .checkpoints
            .partition_point(|checkpoint| checkpoint.byte <= offset)
            .saturating_sub(1);
        let checkpoint = self.checkpoints[index];
        checkpoint
            .summary
            .plus(TextSummary::from_text(&self.text[checkpoint.byte..offset]))
    }

    fn summary(&self, range: Range<usize>) -> TextSummary {
        self.prefix_summary(range.end)
            .minus(self.prefix_summary(range.start))
    }

    fn byte_offset_for_utf16(&self, range: Range<usize>, target: u64) -> Option<usize> {
        let range_start = self.prefix_summary(range.start).utf16_u64();
        let absolute_target = range_start + target;
        let checkpoint_index = self
            .checkpoints
            .partition_point(|checkpoint| checkpoint.summary.utf16_u64() <= absolute_target)
            .saturating_sub(1);
        let checkpoint = self.checkpoints[checkpoint_index];
        let relative = byte_offset_for_utf16_in_text(
            &self.text[checkpoint.byte..range.end],
            absolute_target - checkpoint.summary.utf16_u64(),
        )?;
        let absolute = checkpoint.byte + relative;
        (absolute >= range.start).then_some(absolute - range.start)
    }

    fn byte_offset_for_line(&self, range: Range<usize>, target: u64) -> Option<usize> {
        if target == 0 {
            return Some(0);
        }
        let range_start = self.prefix_summary(range.start).line_breaks();
        let absolute_target = range_start + target;
        let checkpoint_index = self
            .checkpoints
            .partition_point(|checkpoint| checkpoint.summary.line_breaks() < absolute_target)
            .saturating_sub(1);
        let checkpoint = self.checkpoints[checkpoint_index];
        let relative = byte_after_line_break(
            &self.text[checkpoint.byte..range.end],
            absolute_target - checkpoint.summary.line_breaks(),
        )?;
        let absolute = checkpoint.byte + relative;
        (absolute >= range.start).then_some(absolute - range.start)
    }
}

#[derive(Clone, Debug)]
struct Piece {
    buffer: Arc<PieceBuffer>,
    start: usize,
    end: usize,
    summary: TextSummary,
}

impl Piece {
    fn whole(text: Arc<str>) -> Self {
        let buffer = PieceBuffer::new(text);
        Self {
            start: 0,
            end: buffer.text.len(),
            summary: buffer.summary,
            buffer,
        }
    }

    fn len(&self) -> usize {
        self.end - self.start
    }

    fn text(&self) -> &str {
        &self.buffer.text[self.start..self.end]
    }

    fn subpiece(&self, start: usize, end: usize) -> Option<Self> {
        (start < end).then(|| {
            let absolute = self.start + start..self.start + end;
            Self {
                buffer: Arc::clone(&self.buffer),
                start: absolute.start,
                end: absolute.end,
                summary: self.buffer.summary(absolute),
            }
        })
    }
}

#[derive(Debug)]
struct Node {
    left: Link,
    piece: Piece,
    right: Link,
    priority: u64,
    summary: TextSummary,
    pieces: usize,
    height: usize,
}

fn make_node(left: Link, piece: Piece, right: Link, priority: u64) -> Arc<Node> {
    Arc::new(Node {
        summary: link_summary(&left)
            .plus(piece.summary)
            .plus(link_summary(&right)),
        pieces: link_pieces(&left) + 1 + link_pieces(&right),
        height: 1 + link_height(&left).max(link_height(&right)),
        left,
        piece,
        right,
        priority,
    })
}

fn link_bytes(link: &Link) -> usize {
    link_summary(link).bytes_usize()
}

fn link_summary(link: &Link) -> TextSummary {
    link.as_ref()
        .map_or(TextSummary::EMPTY, |node| node.summary)
}

fn link_pieces(link: &Link) -> usize {
    link.as_ref().map_or(0, |node| node.pieces)
}

fn link_height(link: &Link) -> usize {
    link.as_ref().map_or(0, |node| node.height)
}

fn merge(left: Link, right: Link) -> Link {
    match (left, right) {
        (None, right) => right,
        (left, None) => left,
        (Some(left), Some(right)) if left.priority >= right.priority => {
            let merged = merge(left.right.clone(), Some(right));
            Some(make_node(
                left.left.clone(),
                left.piece.clone(),
                merged,
                left.priority,
            ))
        }
        (Some(left), Some(right)) => {
            let merged = merge(Some(left), right.left.clone());
            Some(make_node(
                merged,
                right.piece.clone(),
                right.right.clone(),
                right.priority,
            ))
        }
    }
}

fn first_piece(root: &Link) -> Option<&Piece> {
    let mut node = root.as_deref()?;
    while let Some(left) = node.left.as_deref() {
        node = left;
    }
    Some(&node.piece)
}

fn last_piece(root: &Link) -> Option<&Piece> {
    let mut node = root.as_deref()?;
    while let Some(right) = node.right.as_deref() {
        node = right;
    }
    Some(&node.piece)
}

fn pop_first(node: Arc<Node>) -> (Piece, Link) {
    let Some(left) = node.left.clone() else {
        return (node.piece.clone(), node.right.clone());
    };
    let (piece, remainder) = pop_first(left);
    (
        piece,
        Some(make_node(
            remainder,
            node.piece.clone(),
            node.right.clone(),
            node.priority,
        )),
    )
}

fn pop_last(node: Arc<Node>) -> (Link, Piece) {
    let Some(right) = node.right.clone() else {
        return (node.left.clone(), node.piece.clone());
    };
    let (remainder, piece) = pop_last(right);
    (
        Some(make_node(
            node.left.clone(),
            node.piece.clone(),
            remainder,
            node.priority,
        )),
        piece,
    )
}

fn concat_coalescing(left: Link, right: Link, sequence: &mut u64) -> Link {
    let (Some(left_boundary), Some(right_boundary)) = (last_piece(&left), first_piece(&right))
    else {
        return merge(left, right);
    };
    if !Arc::ptr_eq(&left_boundary.buffer, &right_boundary.buffer)
        || left_boundary.end != right_boundary.start
    {
        return merge(left, right);
    }

    let (left_remainder, left_piece) = pop_last(left.expect("boundary requires a left tree"));
    let (right_piece, right_remainder) = pop_first(right.expect("boundary requires a right tree"));
    let merged_piece = Piece {
        buffer: left_piece.buffer,
        start: left_piece.start,
        end: right_piece.end,
        summary: left_piece.summary.plus(right_piece.summary),
    };
    let merged = Some(make_node(None, merged_piece, None, next_priority(sequence)));
    merge(merge(left_remainder, merged), right_remainder)
}

fn split(root: Link, offset: usize, sequence: &mut u64) -> (Link, Link) {
    let Some(node) = root else {
        return (None, None);
    };
    let left_bytes = link_bytes(&node.left);
    let piece_end = left_bytes + node.piece.len();

    if offset < left_bytes {
        let (before, after) = split(node.left.clone(), offset, sequence);
        let right = Some(make_node(
            after,
            node.piece.clone(),
            node.right.clone(),
            node.priority,
        ));
        return (before, right);
    }
    if offset > piece_end {
        let (before, after) = split(node.right.clone(), offset - piece_end, sequence);
        let left = Some(make_node(
            node.left.clone(),
            node.piece.clone(),
            before,
            node.priority,
        ));
        return (left, after);
    }

    let local = offset - left_bytes;
    let left_piece = node.piece.subpiece(0, local).map(|piece| {
        let priority = next_priority(sequence);
        make_node(None, piece, None, priority)
    });
    let right_piece = node.piece.subpiece(local, node.piece.len()).map(|piece| {
        let priority = next_priority(sequence);
        make_node(None, piece, None, priority)
    });
    (
        merge(node.left.clone(), left_piece),
        merge(right_piece, node.right.clone()),
    )
}

fn next_priority(sequence: &mut u64) -> u64 {
    *sequence = sequence.wrapping_add(0x9e37_79b9_7f4a_7c15);
    let mut value = *sequence;
    value = (value ^ (value >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    value = (value ^ (value >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    value ^ (value >> 31)
}

fn append_range(root: &Link, range: Range<usize>, output: &mut String) {
    let Some(node) = root else { return };
    let left_bytes = link_bytes(&node.left);
    let piece_end = left_bytes + node.piece.len();

    if range.start < left_bytes {
        append_range(&node.left, range.start..range.end.min(left_bytes), output);
    }

    let piece_start = range.start.saturating_sub(left_bytes).min(node.piece.len());
    let piece_limit = range.end.saturating_sub(left_bytes).min(node.piece.len());
    if piece_start < piece_limit {
        output.push_str(&node.piece.text()[piece_start..piece_limit]);
    }

    if range.end > piece_end {
        append_range(
            &node.right,
            range.start.saturating_sub(piece_end)..range.end - piece_end,
            output,
        );
    }
}

fn write_all(root: &Link, output: &mut String) {
    let Some(node) = root else { return };
    write_all(&node.left, output);
    output.push_str(node.piece.text());
    write_all(&node.right, output);
}

fn is_char_boundary(root: &Link, offset: usize) -> bool {
    let Some(node) = root else { return offset == 0 };
    if offset == 0 || offset == node.summary.bytes_usize() {
        return true;
    }
    let left_bytes = link_bytes(&node.left);
    let piece_end = left_bytes + node.piece.len();
    if offset < left_bytes {
        return is_char_boundary(&node.left, offset);
    }
    if offset == left_bytes || offset == piece_end {
        return true;
    }
    if offset < piece_end {
        return node
            .piece
            .buffer
            .text
            .is_char_boundary(node.piece.start + offset - left_bytes);
    }
    is_char_boundary(&node.right, offset - piece_end)
}

fn prefix_summary(root: &Link, offset: usize) -> TextSummary {
    let Some(node) = root else {
        return TextSummary::EMPTY;
    };
    let left_bytes = link_bytes(&node.left);
    if offset <= left_bytes {
        return prefix_summary(&node.left, offset);
    }

    let before_piece = link_summary(&node.left);
    let local = offset - left_bytes;
    if local <= node.piece.len() {
        return before_piece.plus(
            node.piece
                .buffer
                .summary(node.piece.start..node.piece.start + local),
        );
    }

    before_piece
        .plus(node.piece.summary)
        .plus(prefix_summary(&node.right, local - node.piece.len()))
}

fn byte_offset_for_utf16(root: &Link, target: u64) -> Option<usize> {
    let node = root.as_ref()?;
    let left_summary = link_summary(&node.left);
    if target <= left_summary.utf16_u64() {
        return byte_offset_for_utf16(&node.left, target);
    }

    let after_left = target - left_summary.utf16_u64();
    if after_left <= node.piece.summary.utf16_u64() {
        let local = node
            .piece
            .buffer
            .byte_offset_for_utf16(node.piece.start..node.piece.end, after_left)?;
        return Some(link_bytes(&node.left) + local);
    }

    byte_offset_for_utf16(&node.right, after_left - node.piece.summary.utf16_u64())
        .map(|offset| link_bytes(&node.left) + node.piece.len() + offset)
}

fn byte_offset_for_line(root: &Link, target: u64) -> Option<usize> {
    if target == 0 {
        return Some(0);
    }
    let node = root.as_ref()?;
    let left_summary = link_summary(&node.left);
    if target <= left_summary.line_breaks() {
        return byte_offset_for_line(&node.left, target);
    }

    let after_left = target - left_summary.line_breaks();
    if after_left <= node.piece.summary.line_breaks() {
        let local = node
            .piece
            .buffer
            .byte_offset_for_line(node.piece.start..node.piece.end, after_left)?;
        return Some(link_bytes(&node.left) + local);
    }

    byte_offset_for_line(&node.right, after_left - node.piece.summary.line_breaks())
        .map(|offset| link_bytes(&node.left) + node.piece.len() + offset)
}

fn collect_allocations(root: &Link, collector: &mut AllocationCollector) {
    let Some(node) = root else { return };
    if !collector.add_node(node, mem::size_of::<Node>()) {
        return;
    }
    let buffer = &node.piece.buffer;
    let auxiliary_bytes = mem::size_of::<PieceBuffer>()
        + buffer.checkpoints.len() * mem::size_of::<SummaryCheckpoint>();
    collector.add_auxiliary(buffer, auxiliary_bytes);
    collector.add_text(&buffer.text);
    collect_allocations(&node.left, collector);
    collect_allocations(&node.right, collector);
}

#[derive(Debug)]
pub(crate) struct PieceTreeStore {
    root: Link,
    sequence: u64,
}

impl PieceTreeStore {
    pub(super) fn new(text: String) -> Self {
        let mut sequence = 0;
        let root = if text.is_empty() {
            None
        } else {
            let piece = Piece::whole(Arc::from(text));
            Some(make_node(None, piece, None, next_priority(&mut sequence)))
        };
        Self { root, sequence }
    }

    pub(super) fn len_bytes(&self) -> usize {
        link_bytes(&self.root)
    }

    pub(super) fn is_char_boundary(&self, offset: usize) -> bool {
        offset <= self.len_bytes() && is_char_boundary(&self.root, offset)
    }

    pub(super) fn slice(&self, range: Range<usize>) -> String {
        let mut output = String::with_capacity(range.len());
        append_range(&self.root, range, &mut output);
        output
    }

    pub(super) fn replace_range(&mut self, range: Range<usize>, inserted: Arc<str>) {
        let (through_end, after) = split(self.root.clone(), range.end, &mut self.sequence);
        let (before, _) = split(through_end, range.start, &mut self.sequence);
        let inserted = if inserted.is_empty() {
            None
        } else {
            let piece = Piece::whole(inserted);
            Some(make_node(
                None,
                piece,
                None,
                next_priority(&mut self.sequence),
            ))
        };
        let before_inserted = concat_coalescing(before, inserted, &mut self.sequence);
        self.root = concat_coalescing(before_inserted, after, &mut self.sequence);
    }

    pub(super) fn snapshot(&self) -> PieceTreeSnapshot {
        PieceTreeSnapshot {
            root: self.root.clone(),
        }
    }

    pub(super) fn stats(&self) -> StorageStats {
        self.snapshot().stats()
    }
}

#[derive(Clone, Debug)]
pub(crate) struct PieceTreeSnapshot {
    root: Link,
}

impl PieceTreeSnapshot {
    pub(super) fn write_to(&self, output: &mut String) {
        write_all(&self.root, output);
    }

    pub(super) fn len_bytes(&self) -> usize {
        link_bytes(&self.root)
    }

    pub(super) fn stats(&self) -> StorageStats {
        StorageStats::new(
            StorageBackend::PieceTree,
            link_bytes(&self.root),
            link_pieces(&self.root),
        )
    }

    pub(super) fn summary(&self) -> TextSummary {
        link_summary(&self.root)
    }

    pub(super) fn is_char_boundary(&self, offset: usize) -> bool {
        offset <= link_bytes(&self.root) && is_char_boundary(&self.root, offset)
    }

    pub(super) fn prefix_summary(&self, offset: usize) -> TextSummary {
        prefix_summary(&self.root, offset)
    }

    pub(super) fn byte_offset_for_utf16(&self, offset: u64) -> Option<usize> {
        if offset == 0 {
            return Some(0);
        }
        byte_offset_for_utf16(&self.root, offset)
    }

    pub(super) fn byte_offset_for_line(&self, line: u64) -> Option<usize> {
        byte_offset_for_line(&self.root, line)
    }

    pub(super) fn collect_allocations(&self, collector: &mut AllocationCollector) {
        collect_allocations(&self.root, collector);
    }

    pub(super) fn chunks_from(&self, offset: usize) -> PieceTreeChunkCursor<'_> {
        PieceTreeChunkCursor::new(&self.root, offset)
    }

    pub(super) fn chunk_before(&self, offset: usize) -> Option<StorageChunk<'_>> {
        previous_chunk(&self.root, offset, 0)
    }
}

fn previous_chunk<'a>(node: &'a Link, offset: usize, base: usize) -> Option<StorageChunk<'a>> {
    let node = node.as_deref()?;
    let start = base + link_bytes(&node.left);
    let end = start + node.piece.len();

    if offset <= start {
        return previous_chunk(&node.left, offset, base);
    }
    if offset < end {
        return previous_chunk(&node.left, offset, base);
    }

    previous_chunk(&node.right, offset, end).or_else(|| {
        Some(StorageChunk {
            start,
            text: node.piece.text(),
        })
    })
}

pub(super) struct PieceTreeChunkCursor<'a> {
    stack: Vec<(&'a Node, usize)>,
}

impl<'a> PieceTreeChunkCursor<'a> {
    fn new(root: &'a Link, offset: usize) -> Self {
        let mut cursor = Self { stack: Vec::new() };
        let mut node = root.as_deref();
        let mut base = 0;

        while let Some(current) = node {
            let piece_start = base + link_bytes(&current.left);
            let piece_end = piece_start + current.piece.len();
            if offset < piece_end {
                cursor.stack.push((current, base));
                if offset < piece_start {
                    node = current.left.as_deref();
                } else {
                    break;
                }
            } else {
                base = piece_end;
                node = current.right.as_deref();
            }
        }

        cursor
    }

    fn push_left(&mut self, mut node: Option<&'a Node>, base: usize) {
        while let Some(current) = node {
            self.stack.push((current, base));
            node = current.left.as_deref();
        }
    }
}

impl<'a> Iterator for PieceTreeChunkCursor<'a> {
    type Item = StorageChunk<'a>;

    fn next(&mut self) -> Option<Self::Item> {
        let (node, base) = self.stack.pop()?;
        let start = base + link_bytes(&node.left);
        self.push_left(node.right.as_deref(), start + node.piece.len());
        Some(StorageChunk {
            start,
            text: node.piece.text(),
        })
    }
}
