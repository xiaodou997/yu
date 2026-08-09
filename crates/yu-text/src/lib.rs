#![forbid(unsafe_code)]

//! Immutable text snapshots and transactional editing contracts.
//!
//! Piece Tree is the selected product backend. Flat UTF-8 and Persistent Rope
//! remain available as explicit correctness and performance comparison stores.

mod buffer;
mod position;
mod storage;
mod summary;
mod transaction;

pub use buffer::{TextBuffer, TextSnapshot, retained_snapshot_stats};
pub use position::TextPositionError;
pub use storage::{ChunkCursor, SnapshotRetentionStats, StorageBackend, StorageStats, TextChunk};
pub use summary::TextSummary;
pub use transaction::{
    AnchorMapError, AppliedTransaction, ChangeSet, Edit, EditError, TextChange, Transaction,
};
