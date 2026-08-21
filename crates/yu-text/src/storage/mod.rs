mod ropey_backend;

use std::collections::HashSet;
use std::sync::Arc;

use ropey_backend::RopeyChunkCursor;
use yu_core::ByteOffset;

pub(crate) use ropey_backend::{RopeySnapshot as StorageSnapshot, RopeyStore as Storage};

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
    inner: RopeyChunkCursor<'a>,
}

impl<'a> ChunkCursor<'a> {
    pub(crate) const fn new(inner: RopeyChunkCursor<'a>) -> Self {
        Self { inner }
    }
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
    pub(crate) start: usize,
    pub(crate) text: &'a str,
}

/// Structural metrics used by tests and benchmarks.
///
/// `chunks` 是一段连续 `&str` 的个数。节点数与树高不在这里：那是 rope 实现
/// 内部的事，Yu 既拿不到也不该依赖。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StorageStats {
    bytes: usize,
    chunks: usize,
}

impl StorageStats {
    pub(crate) const fn new(bytes: usize, chunks: usize) -> Self {
        Self { bytes, chunks }
    }

    #[must_use]
    pub const fn bytes(self) -> usize {
        self.bytes
    }

    #[must_use]
    pub const fn chunks(self) -> usize {
        self.chunks
    }
}

/// De-duplicated logical allocations retained by a set of snapshots.
///
/// The byte estimate excludes allocator/`Arc` headers and container capacity,
/// so it is intended for relative comparisons rather than RSS claims.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SnapshotRetentionStats {
    snapshots: usize,
    snapshot_bytes: usize,
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
        self.snapshot_bytes + self.text_bytes
    }
}

#[derive(Default)]
pub(crate) struct AllocationCollector {
    snapshot_ids: HashSet<usize>,
    snapshot_bytes: usize,
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

    /// 按数据指针去重一片 chunk 文本。
    ///
    /// rope 的叶子节点是私有的，拿不到 `Arc`；但 chunk 的数据指针就是那片
    /// 叶子分配的地址，两个快照共享同一片叶子时拿到同一个指针。
    pub(crate) fn add_chunk_text(&mut self, text: &str) {
        if text.is_empty() {
            return;
        }
        let id = text.as_ptr() as usize;
        if self.text_ids.insert(id) {
            self.text_bytes += text.len();
        }
    }

    pub(crate) fn add_materialized(&mut self, text: &Arc<str>) {
        let id = Arc::as_ptr(text) as *const () as usize;
        if self.materialized_ids.insert(id) {
            self.materialized_bytes += text.len();
        }
        self.add_chunk_text(text);
    }

    pub(crate) fn finish(self) -> SnapshotRetentionStats {
        SnapshotRetentionStats {
            snapshots: self.snapshot_ids.len(),
            snapshot_bytes: self.snapshot_bytes,
            text_buffers: self.text_ids.len(),
            text_bytes: self.text_bytes,
            materialized_buffers: self.materialized_ids.len(),
            materialized_bytes: self.materialized_bytes,
        }
    }
}
