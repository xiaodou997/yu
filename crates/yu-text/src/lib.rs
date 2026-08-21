#![forbid(unsafe_code)]

//! Immutable text snapshots and transactional editing contracts.
//!
//! Text lives in a ropey `Rope`, reached only through `storage::ropey_backend`.
//! Everything this crate exposes is addressed in `ByteOffset` (invariant E4).

mod buffer;
mod position;
mod storage;
mod summary;
mod transaction;

pub use buffer::{TextBuffer, TextSnapshot, retained_snapshot_stats};
pub use position::TextPositionError;
pub use storage::{ChunkCursor, SnapshotRetentionStats, StorageStats, TextChunk};
pub use summary::TextSummary;
pub use transaction::{
    AnchorMapError, AppliedTransaction, ChangeSet, Edit, EditError, TextChange, Transaction,
};
