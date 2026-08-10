mod flat;
mod piece_tree;
mod rope;

use std::collections::HashSet;
use std::fmt;
use std::ops::Range;
use std::sync::Arc;

use crate::TextSummary;
use flat::{FlatChunkCursor, FlatSnapshot, FlatStore};
use piece_tree::{PieceTreeChunkCursor, PieceTreeSnapshot, PieceTreeStore};
use rope::{RopeChunkCursor, RopeSnapshot, RopeStore};
use yu_core::ByteOffset;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TextChunk<'a> {
    start: ByteOffset,
    text: &'a str,
}

impl<'a> TextChunk<'a> {
    pub(crate) const fn new(start: ByteOffset, text: &'a str) -> Self {
        Self { start, text }
    }

    #[must_use]
    pub const fn start(self) -> ByteOffset {
        self.start
    }

    #[must_use]
    pub fn end(self) -> ByteOffset {
        self.start
            .checked_add(u64::try_from(self.text.len()).unwrap_or(u64::MAX))
            .unwrap_or(ByteOffset::new(u64::MAX))
    }

    #[must_use]
    pub const fn text(self) -> &'a str {
        self.text
    }
}

pub struct ChunkCursor<'a> {
    inner: StorageChunkCursor<'a>,
}

impl<'a> Iterator for ChunkCursor<'a> {
    type Item = TextChunk<'a>;

    fn next(&mut self) -> Option<Self::Item> {
        self.inner.next().map(|chunk| {
            TextChunk::new(
                ByteOffset::try_from(chunk.start).unwrap_or(ByteOffset::new(u64::MAX)),
                chunk.text,
            )
        })
    }
}

pub(crate) struct StorageChunk<'a> {
    start: usize,
    text: &'a str,
}

enum StorageChunkCursor<'a> {
    Flat(FlatChunkCursor<'a>),
    PieceTree(PieceTreeChunkCursor<'a>),
    Rope(RopeChunkCursor<'a>),
}

impl<'a> Iterator for StorageChunkCursor<'a> {
    type Item = StorageChunk<'a>;

    fn next(&mut self) -> Option<Self::Item> {
        match self {
            Self::Flat(cursor) => cursor.next(),
            Self::PieceTree(cursor) => cursor.next(),
            Self::Rope(cursor) => cursor.next(),
        }
    }
}

/// Selects a text storage implementation without changing editor semantics.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum StorageBackend {
    FlatReference,
    #[default]
    PieceTree,
    PersistentRope,
}

impl StorageBackend {
    pub const ALL: [Self; 3] = [Self::FlatReference, Self::PieceTree, Self::PersistentRope];
}

impl fmt::Display for StorageBackend {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::FlatReference => formatter.write_str("flat-reference"),
            Self::PieceTree => formatter.write_str("piece-tree"),
            Self::PersistentRope => formatter.write_str("persistent-rope"),
        }
    }
}

/// Structural metrics used by tests and candidate benchmarks.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StorageStats {
    backend: StorageBackend,
    bytes: usize,
    chunks: usize,
    nodes: usize,
    height: usize,
}

/// De-duplicated logical allocations retained by a set of snapshots.
///
/// The byte estimate excludes allocator/`Arc` headers and container capacity,
/// so it is intended for relative candidate comparisons rather than RSS claims.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SnapshotRetentionStats {
    snapshots: usize,
    snapshot_bytes: usize,
    nodes: usize,
    node_bytes: usize,
    auxiliary_allocations: usize,
    auxiliary_bytes: usize,
    text_buffers: usize,
    text_bytes: usize,
    materialized_buffers: usize,
    materialized_bytes: usize,
}

impl SnapshotRetentionStats {
    #[must_use]
    pub const fn snapshots(self) -> usize {
        self.snapshots
    }

    #[must_use]
    pub const fn snapshot_bytes(self) -> usize {
        self.snapshot_bytes
    }

    #[must_use]
    pub const fn nodes(self) -> usize {
        self.nodes
    }

    #[must_use]
    pub const fn node_bytes(self) -> usize {
        self.node_bytes
    }

    #[must_use]
    pub const fn auxiliary_allocations(self) -> usize {
        self.auxiliary_allocations
    }

    #[must_use]
    pub const fn auxiliary_bytes(self) -> usize {
        self.auxiliary_bytes
    }

    #[must_use]
    pub const fn text_buffers(self) -> usize {
        self.text_buffers
    }

    #[must_use]
    pub const fn text_bytes(self) -> usize {
        self.text_bytes
    }

    #[must_use]
    pub const fn materialized_buffers(self) -> usize {
        self.materialized_buffers
    }

    #[must_use]
    pub const fn materialized_bytes(self) -> usize {
        self.materialized_bytes
    }

    #[must_use]
    pub const fn estimated_bytes(self) -> usize {
        self.snapshot_bytes + self.node_bytes + self.auxiliary_bytes + self.text_bytes
    }
}

#[derive(Default)]
pub(crate) struct AllocationCollector {
    snapshot_ids: HashSet<usize>,
    snapshot_bytes: usize,
    node_ids: HashSet<usize>,
    node_bytes: usize,
    auxiliary_ids: HashSet<usize>,
    auxiliary_bytes: usize,
    text_ids: HashSet<usize>,
    text_bytes: usize,
    materialized_ids: HashSet<usize>,
    materialized_bytes: usize,
}

impl AllocationCollector {
    pub(crate) fn add_snapshot<T>(&mut self, snapshot: &Arc<T>, bytes: usize) -> bool {
        let id = Arc::as_ptr(snapshot) as usize;
        if self.snapshot_ids.insert(id) {
            self.snapshot_bytes += bytes;
            true
        } else {
            false
        }
    }

    pub(crate) fn add_node<T>(&mut self, node: &Arc<T>, bytes: usize) -> bool {
        let id = Arc::as_ptr(node) as usize;
        if self.node_ids.insert(id) {
            self.node_bytes += bytes;
            true
        } else {
            false
        }
    }

    pub(crate) fn add_auxiliary<T>(&mut self, allocation: &Arc<T>, bytes: usize) -> bool {
        let id = Arc::as_ptr(allocation) as usize;
        if self.auxiliary_ids.insert(id) {
            self.auxiliary_bytes += bytes;
            true
        } else {
            false
        }
    }

    pub(crate) fn add_text(&mut self, text: &Arc<str>) {
        let id = Arc::as_ptr(text) as *const () as usize;
        if self.text_ids.insert(id) {
            self.text_bytes += text.len();
        }
    }

    pub(crate) fn add_materialized(&mut self, text: &Arc<str>) {
        let id = Arc::as_ptr(text) as *const () as usize;
        if self.materialized_ids.insert(id) {
            self.materialized_bytes += text.len();
        }
        self.add_text(text);
    }

    pub(crate) fn finish(self) -> SnapshotRetentionStats {
        SnapshotRetentionStats {
            snapshots: self.snapshot_ids.len(),
            snapshot_bytes: self.snapshot_bytes,
            nodes: self.node_ids.len(),
            node_bytes: self.node_bytes,
            auxiliary_allocations: self.auxiliary_ids.len(),
            auxiliary_bytes: self.auxiliary_bytes,
            text_buffers: self.text_ids.len(),
            text_bytes: self.text_bytes,
            materialized_buffers: self.materialized_ids.len(),
            materialized_bytes: self.materialized_bytes,
        }
    }
}

impl StorageStats {
    pub(crate) const fn new(
        backend: StorageBackend,
        bytes: usize,
        chunks: usize,
        nodes: usize,
        height: usize,
    ) -> Self {
        Self {
            backend,
            bytes,
            chunks,
            nodes,
            height,
        }
    }

    #[must_use]
    pub const fn backend(self) -> StorageBackend {
        self.backend
    }

    #[must_use]
    pub const fn bytes(self) -> usize {
        self.bytes
    }

    #[must_use]
    pub const fn chunks(self) -> usize {
        self.chunks
    }

    #[must_use]
    pub const fn nodes(self) -> usize {
        self.nodes
    }

    #[must_use]
    pub const fn height(self) -> usize {
        self.height
    }
}

#[derive(Debug)]
pub(crate) enum Storage {
    Flat(FlatStore),
    PieceTree(PieceTreeStore),
    Rope(RopeStore),
}

impl Storage {
    pub(crate) fn new(text: String, backend: StorageBackend) -> Self {
        match backend {
            StorageBackend::FlatReference => Self::Flat(FlatStore::new(text)),
            StorageBackend::PieceTree => Self::PieceTree(PieceTreeStore::new(text)),
            StorageBackend::PersistentRope => Self::Rope(RopeStore::new(text)),
        }
    }

    pub(crate) fn backend(&self) -> StorageBackend {
        match self {
            Self::Flat(_) => StorageBackend::FlatReference,
            Self::PieceTree(_) => StorageBackend::PieceTree,
            Self::Rope(_) => StorageBackend::PersistentRope,
        }
    }

    pub(crate) fn len_bytes(&self) -> usize {
        match self {
            Self::Flat(store) => store.len_bytes(),
            Self::PieceTree(store) => store.len_bytes(),
            Self::Rope(store) => store.len_bytes(),
        }
    }

    pub(crate) fn is_char_boundary(&self, offset: usize) -> bool {
        match self {
            Self::Flat(store) => store.is_char_boundary(offset),
            Self::PieceTree(store) => store.is_char_boundary(offset),
            Self::Rope(store) => store.is_char_boundary(offset),
        }
    }

    pub(crate) fn slice(&self, range: Range<usize>) -> String {
        match self {
            Self::Flat(store) => store.slice(range),
            Self::PieceTree(store) => store.slice(range),
            Self::Rope(store) => store.slice(range),
        }
    }

    pub(crate) fn replace_range(&mut self, range: Range<usize>, inserted: Arc<str>) {
        match self {
            Self::Flat(store) => store.replace_range(range, &inserted),
            Self::PieceTree(store) => store.replace_range(range, inserted),
            Self::Rope(store) => store.replace_range(range, inserted),
        }
    }

    pub(crate) fn snapshot(&self) -> StorageSnapshot {
        match self {
            Self::Flat(store) => StorageSnapshot::Flat(store.snapshot()),
            Self::PieceTree(store) => StorageSnapshot::PieceTree(store.snapshot()),
            Self::Rope(store) => StorageSnapshot::Rope(store.snapshot()),
        }
    }

    pub(crate) fn stats(&self) -> StorageStats {
        match self {
            Self::Flat(store) => store.stats(),
            Self::PieceTree(store) => store.stats(),
            Self::Rope(store) => store.stats(),
        }
    }
}

#[derive(Clone, Debug)]
pub(crate) enum StorageSnapshot {
    Flat(FlatSnapshot),
    PieceTree(PieceTreeSnapshot),
    Rope(RopeSnapshot),
}

impl StorageSnapshot {
    pub(crate) fn write_to(&self, output: &mut String) {
        match self {
            Self::Flat(snapshot) => snapshot.write_to(output),
            Self::PieceTree(snapshot) => snapshot.write_to(output),
            Self::Rope(snapshot) => snapshot.write_to(output),
        }
    }

    pub(crate) fn contiguous_arc(&self) -> Option<Arc<str>> {
        match self {
            Self::Flat(snapshot) => Some(snapshot.text()),
            Self::PieceTree(_) | Self::Rope(_) => None,
        }
    }

    pub(crate) fn stats(&self) -> StorageStats {
        match self {
            Self::Flat(snapshot) => snapshot.stats(),
            Self::PieceTree(snapshot) => snapshot.stats(),
            Self::Rope(snapshot) => snapshot.stats(),
        }
    }

    pub(crate) fn summary(&self) -> TextSummary {
        match self {
            Self::Flat(snapshot) => snapshot.summary(),
            Self::PieceTree(snapshot) => snapshot.summary(),
            Self::Rope(snapshot) => snapshot.summary(),
        }
    }

    pub(crate) fn chunks_from(&self, offset: usize) -> ChunkCursor<'_> {
        let inner = match self {
            Self::Flat(snapshot) => StorageChunkCursor::Flat(snapshot.chunks_from(offset)),
            Self::PieceTree(snapshot) => {
                StorageChunkCursor::PieceTree(snapshot.chunks_from(offset))
            }
            Self::Rope(snapshot) => StorageChunkCursor::Rope(snapshot.chunks_from(offset)),
        };
        ChunkCursor { inner }
    }

    pub(crate) fn chunk_before(&self, offset: usize) -> Option<(usize, &str)> {
        let chunk = match self {
            Self::Flat(snapshot) => snapshot.chunk_before(offset),
            Self::PieceTree(snapshot) => snapshot.chunk_before(offset),
            Self::Rope(snapshot) => snapshot.chunk_before(offset),
        }?;
        Some((chunk.start, chunk.text))
    }

    pub(crate) fn is_char_boundary(&self, offset: usize) -> bool {
        match self {
            Self::Flat(snapshot) => snapshot.is_char_boundary(offset),
            Self::PieceTree(snapshot) => snapshot.is_char_boundary(offset),
            Self::Rope(snapshot) => snapshot.is_char_boundary(offset),
        }
    }

    pub(crate) fn prefix_summary(&self, offset: usize) -> TextSummary {
        match self {
            Self::Flat(snapshot) => snapshot.prefix_summary(offset),
            Self::PieceTree(snapshot) => snapshot.prefix_summary(offset),
            Self::Rope(snapshot) => snapshot.prefix_summary(offset),
        }
    }

    pub(crate) fn byte_offset_for_utf16(&self, offset: u64) -> Option<usize> {
        match self {
            Self::Flat(snapshot) => snapshot.byte_offset_for_utf16(offset),
            Self::PieceTree(snapshot) => snapshot.byte_offset_for_utf16(offset),
            Self::Rope(snapshot) => snapshot.byte_offset_for_utf16(offset),
        }
    }

    pub(crate) fn byte_offset_for_line(&self, line: u64) -> Option<usize> {
        match self {
            Self::Flat(snapshot) => snapshot.byte_offset_for_line(line),
            Self::PieceTree(snapshot) => snapshot.byte_offset_for_line(line),
            Self::Rope(snapshot) => snapshot.byte_offset_for_line(line),
        }
    }

    pub(crate) fn collect_allocations(&self, collector: &mut AllocationCollector) {
        match self {
            Self::Flat(snapshot) => snapshot.collect_allocations(collector),
            Self::PieceTree(snapshot) => snapshot.collect_allocations(collector),
            Self::Rope(snapshot) => snapshot.collect_allocations(collector),
        }
    }
}
