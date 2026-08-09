use std::sync::{Arc, OnceLock};

use yu_core::{ByteOffset, Revision};

use crate::storage::{Storage, StorageSnapshot};
use crate::{
    AppliedTransaction, EditError, SnapshotRetentionStats, StorageBackend, StorageStats,
    Transaction, transaction::PreparedTransaction,
};

/// An immutable, cheaply cloneable view of one document revision.
///
/// Tree roots are shared immediately. `as_str()` materializes non-contiguous
/// backends once and caches the result for parsers that still need a flat view.
#[derive(Clone, Debug)]
pub struct TextSnapshot {
    inner: Arc<SnapshotInner>,
}

/// Computes de-duplicated logical allocations retained by the supplied snapshots.
#[must_use]
pub fn retained_snapshot_stats(snapshots: &[TextSnapshot]) -> SnapshotRetentionStats {
    let mut collector = crate::storage::AllocationCollector::default();
    for snapshot in snapshots {
        if !collector.add_snapshot(&snapshot.inner, std::mem::size_of::<SnapshotInner>()) {
            continue;
        }
        snapshot.inner.storage.collect_allocations(&mut collector);
        if let Some(materialized) = snapshot.inner.contiguous.get() {
            collector.add_materialized(materialized);
        }
    }
    collector.finish()
}

#[derive(Debug)]
struct SnapshotInner {
    revision: Revision,
    storage: StorageSnapshot,
    contiguous: OnceLock<Arc<str>>,
}

impl TextSnapshot {
    fn new(revision: Revision, storage: StorageSnapshot) -> Self {
        let contiguous = OnceLock::new();
        if let Some(text) = storage.contiguous_arc() {
            let _ = contiguous.set(text);
        }
        Self {
            inner: Arc::new(SnapshotInner {
                revision,
                storage,
                contiguous,
            }),
        }
    }

    #[must_use]
    pub fn revision(&self) -> Revision {
        self.inner.revision
    }

    /// Returns a contiguous view, materializing Piece Tree/Rope chunks once.
    #[must_use]
    pub fn as_str(&self) -> &str {
        self.inner
            .contiguous
            .get_or_init(|| {
                let stats = self.inner.storage.stats();
                let mut text = String::with_capacity(stats.bytes());
                self.inner.storage.write_to(&mut text);
                Arc::from(text)
            })
            .as_ref()
    }

    #[must_use]
    pub fn len_bytes(&self) -> ByteOffset {
        ByteOffset::try_from(self.inner.storage.stats().bytes())
            .unwrap_or(ByteOffset::new(u64::MAX))
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.inner.storage.stats().bytes() == 0
    }

    #[must_use]
    pub fn storage_stats(&self) -> StorageStats {
        self.inner.storage.stats()
    }
}

/// A mutable text facade with selectable storage backends.
#[derive(Debug)]
pub struct TextBuffer {
    revision: Revision,
    storage: Storage,
}

impl TextBuffer {
    /// Creates a buffer using Yu's selected Piece Tree backend.
    #[must_use]
    pub fn new(text: impl Into<String>) -> Self {
        Self::with_backend(text, StorageBackend::default())
    }

    #[must_use]
    pub fn with_backend(text: impl Into<String>, backend: StorageBackend) -> Self {
        Self {
            revision: Revision::INITIAL,
            storage: Storage::new(text.into(), backend),
        }
    }

    #[must_use]
    pub fn revision(&self) -> Revision {
        self.revision
    }

    #[must_use]
    pub fn backend(&self) -> StorageBackend {
        self.storage.backend()
    }

    #[must_use]
    pub fn storage_stats(&self) -> StorageStats {
        self.storage.stats()
    }

    #[must_use]
    pub fn snapshot(&self) -> TextSnapshot {
        TextSnapshot::new(self.revision, self.storage.snapshot())
    }

    pub fn apply(&mut self, transaction: &Transaction) -> Result<AppliedTransaction, EditError> {
        let PreparedTransaction {
            edits,
            change_set,
            inverse,
        } = transaction.prepare(self.revision, &self.storage)?;

        for edit in edits.iter().rev() {
            let start =
                usize::try_from(edit.range().start()).map_err(|_| EditError::OffsetOverflow)?;
            let end = usize::try_from(edit.range().end()).map_err(|_| EditError::OffsetOverflow)?;
            self.storage.replace_range(start..end, edit.inserted_arc());
        }

        self.revision = change_set.after();
        Ok(AppliedTransaction::new(
            self.snapshot(),
            change_set,
            inverse,
        ))
    }
}

#[cfg(test)]
mod tests {
    use yu_core::{ByteOffset, TextRange};

    use super::*;
    use crate::Edit;

    #[test]
    fn snapshots_remain_stable_after_an_edit_for_every_backend() {
        for backend in StorageBackend::ALL {
            let mut buffer = TextBuffer::with_backend("羽", backend);
            let old = buffer.snapshot();
            let insert_at_end = TextRange::empty(ByteOffset::new(3));
            let transaction =
                Transaction::new(buffer.revision(), [Edit::new(insert_at_end, " Yu")]);

            buffer
                .apply(&transaction)
                .expect("valid transaction should apply");

            assert_eq!(old.as_str(), "羽", "backend {backend}");
            assert_eq!(old.revision(), Revision::INITIAL);
            assert_eq!(buffer.snapshot().as_str(), "羽 Yu", "backend {backend}");
            assert_eq!(buffer.revision(), Revision::new(1));
        }
    }

    #[test]
    fn cloned_snapshots_are_counted_once() {
        for backend in StorageBackend::ALL {
            let buffer = TextBuffer::with_backend("# 羽\n", backend);
            let snapshot = buffer.snapshot();
            let cloned = snapshot.clone();
            let stats = retained_snapshot_stats(&[snapshot, cloned]);

            assert_eq!(stats.snapshots(), 1, "backend {backend}");
            assert_eq!(stats.text_bytes(), "# 羽\n".len(), "backend {backend}");
        }
    }

    #[test]
    fn persistent_snapshots_share_text_allocations() {
        let source = "Yu 羽🙂\n".repeat(1_024);
        for backend in [StorageBackend::PieceTree, StorageBackend::PersistentRope] {
            let mut buffer = TextBuffer::with_backend(source.clone(), backend);
            let before = buffer.snapshot();
            let middle = source.len() / 2;
            let range = TextRange::empty(
                ByteOffset::try_from(middle).expect("test source offset should fit u64"),
            );
            let transaction = Transaction::new(buffer.revision(), [Edit::new(range, "edit")]);
            let after = buffer
                .apply(&transaction)
                .expect("valid transaction should apply")
                .result_snapshot()
                .clone();

            let before_only = retained_snapshot_stats(std::slice::from_ref(&before));
            let after_only = retained_snapshot_stats(std::slice::from_ref(&after));
            let combined = retained_snapshot_stats(&[before, after]);

            assert_eq!(combined.snapshots(), 2, "backend {backend}");
            assert!(
                combined.estimated_bytes()
                    < before_only.estimated_bytes() + after_only.estimated_bytes(),
                "backend {backend} should de-duplicate shared allocations"
            );
        }
    }

    #[test]
    fn insert_inverse_round_trips_do_not_fragment_tree_backends() {
        let source = "0123456789abcdef".repeat(1_024);
        for backend in [StorageBackend::PieceTree, StorageBackend::PersistentRope] {
            let mut buffer = TextBuffer::with_backend(source.clone(), backend);
            let initial_chunks = buffer.storage_stats().chunks();
            let middle = source.len() / 2;
            let range = TextRange::empty(
                ByteOffset::try_from(middle).expect("test source offset should fit u64"),
            );

            for _ in 0..100 {
                let transaction =
                    Transaction::new(buffer.revision(), [Edit::new(range, "temporary")]);
                let applied = buffer
                    .apply(&transaction)
                    .expect("valid transaction should apply");
                buffer
                    .apply(applied.inverse())
                    .expect("inverse should restore the document");
            }

            assert_eq!(buffer.snapshot().as_str(), source, "backend {backend}");
            assert_eq!(
                buffer.storage_stats().chunks(),
                initial_chunks,
                "backend {backend} accumulated fragments"
            );
        }
    }
}
