use std::mem;
use std::ops::Range;
use std::sync::Arc;

use super::{AllocationCollector, StorageBackend, StorageStats};

type Link = Option<Arc<Node>>;

#[derive(Clone, Debug)]
struct Piece {
    buffer: Arc<str>,
    start: usize,
    end: usize,
}

impl Piece {
    fn whole(buffer: Arc<str>) -> Self {
        Self {
            start: 0,
            end: buffer.len(),
            buffer,
        }
    }

    fn len(&self) -> usize {
        self.end - self.start
    }

    fn text(&self) -> &str {
        &self.buffer[self.start..self.end]
    }

    fn subpiece(&self, start: usize, end: usize) -> Option<Self> {
        (start < end).then(|| Self {
            buffer: Arc::clone(&self.buffer),
            start: self.start + start,
            end: self.start + end,
        })
    }
}

#[derive(Debug)]
struct Node {
    left: Link,
    piece: Piece,
    right: Link,
    priority: u64,
    bytes: usize,
    pieces: usize,
    height: usize,
}

fn make_node(left: Link, piece: Piece, right: Link, priority: u64) -> Arc<Node> {
    Arc::new(Node {
        bytes: link_bytes(&left) + piece.len() + link_bytes(&right),
        pieces: link_pieces(&left) + 1 + link_pieces(&right),
        height: 1 + link_height(&left).max(link_height(&right)),
        left,
        piece,
        right,
        priority,
    })
}

fn link_bytes(link: &Link) -> usize {
    link.as_ref().map_or(0, |node| node.bytes)
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
    if offset == 0 || offset == node.bytes {
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
            .is_char_boundary(node.piece.start + offset - left_bytes);
    }
    is_char_boundary(&node.right, offset - piece_end)
}

fn collect_allocations(root: &Link, collector: &mut AllocationCollector) {
    let Some(node) = root else { return };
    if !collector.add_node(node, mem::size_of::<Node>()) {
        return;
    }
    collector.add_text(&node.piece.buffer);
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

    pub(super) fn stats(&self) -> StorageStats {
        StorageStats::new(
            StorageBackend::PieceTree,
            link_bytes(&self.root),
            link_pieces(&self.root),
            link_pieces(&self.root),
            link_height(&self.root),
        )
    }

    pub(super) fn collect_allocations(&self, collector: &mut AllocationCollector) {
        collect_allocations(&self.root, collector);
    }
}
