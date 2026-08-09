use std::sync::{Arc, OnceLock};

use yu_core::{ByteOffset, LineIndex, Revision, Utf16Offset};

use crate::storage::{Storage, StorageSnapshot};
use crate::{
    AppliedTransaction, ChunkCursor, EditError, SnapshotRetentionStats, StorageBackend,
    StorageStats, TextPositionError, Transaction, transaction::PreparedTransaction,
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

    #[must_use]
    pub fn summary(&self) -> crate::TextSummary {
        self.inner.storage.summary()
    }

    #[must_use]
    pub fn chunks(&self) -> ChunkCursor<'_> {
        self.inner.storage.chunks_from(0)
    }

    pub fn chunk_cursor(&self, offset: ByteOffset) -> Result<ChunkCursor<'_>, TextPositionError> {
        let offset_usize = self.validate_byte_offset(offset)?;
        Ok(self.inner.storage.chunks_from(offset_usize))
    }

    pub fn utf16_offset(&self, offset: ByteOffset) -> Result<Utf16Offset, TextPositionError> {
        let offset = self.validate_byte_offset(offset)?;
        Ok(self.inner.storage.prefix_summary(offset).utf16_units())
    }

    pub fn byte_offset_for_utf16(
        &self,
        offset: Utf16Offset,
    ) -> Result<ByteOffset, TextPositionError> {
        let len = self.summary().utf16_units();
        if offset > len {
            return Err(TextPositionError::Utf16OutOfBounds { offset, len });
        }
        let byte = self
            .inner
            .storage
            .byte_offset_for_utf16(offset.get())
            .ok_or(TextPositionError::Utf16InsideScalar(offset))?;
        ByteOffset::try_from(byte).map_err(|_| TextPositionError::Utf16OutOfBounds { offset, len })
    }

    pub fn line_index(&self, offset: ByteOffset) -> Result<LineIndex, TextPositionError> {
        let offset = self.validate_byte_offset(offset)?;
        Ok(LineIndex::new(
            self.inner.storage.prefix_summary(offset).line_breaks(),
        ))
    }

    pub fn line_start(&self, line: LineIndex) -> Result<ByteOffset, TextPositionError> {
        let line_count = self.summary().line_count();
        if line.get() >= line_count {
            return Err(TextPositionError::LineOutOfBounds { line, line_count });
        }
        let byte = self
            .inner
            .storage
            .byte_offset_for_line(line.get())
            .expect("a validated line index must have a source offset");
        ByteOffset::try_from(byte)
            .map_err(|_| TextPositionError::LineOutOfBounds { line, line_count })
    }

    fn validate_byte_offset(&self, offset: ByteOffset) -> Result<usize, TextPositionError> {
        let len = self.len_bytes();
        if offset > len {
            return Err(TextPositionError::ByteOutOfBounds { offset, len });
        }
        let offset_usize = usize::try_from(offset)
            .map_err(|_| TextPositionError::ByteOutOfBounds { offset, len })?;
        if !self.inner.storage.is_char_boundary(offset_usize) {
            return Err(TextPositionError::NotUtf8Boundary(offset));
        }
        Ok(offset_usize)
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
    use yu_core::{ByteOffset, LineIndex, TextRange, Utf16Offset};

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

    #[test]
    fn summaries_match_the_flat_text_after_edits() {
        for backend in StorageBackend::ALL {
            let mut model = String::from("first\r\n羽🙂\nlast");
            let mut buffer = TextBuffer::with_backend(model.clone(), backend);
            let range = TextRange::new(ByteOffset::new(7), ByteOffset::new(10))
                .expect("ordered offsets should form a range");
            let transaction = Transaction::new(buffer.revision(), [Edit::new(range, "two\nlines")]);
            model.replace_range(7..10, "two\nlines");
            buffer
                .apply(&transaction)
                .expect("valid transaction should apply");

            assert_eq!(
                buffer.snapshot().summary(),
                crate::TextSummary::from_text(&model),
                "backend {backend}"
            );
        }
    }

    #[test]
    fn chunk_cursor_reconstructs_source_and_seeks_to_containing_chunk() {
        let source = "a羽🙂\n".repeat(2_000);
        for backend in StorageBackend::ALL {
            let mut buffer = TextBuffer::with_backend(source.clone(), backend);
            let insertion = TextRange::empty(ByteOffset::new(1));
            let transaction =
                Transaction::new(buffer.revision(), [Edit::new(insertion, "inserted")]);
            buffer
                .apply(&transaction)
                .expect("valid transaction should apply");
            let snapshot = buffer.snapshot();
            let expected = snapshot.as_str().to_owned();

            let reconstructed: String = snapshot.chunks().map(|chunk| chunk.text()).collect();
            assert_eq!(reconstructed, expected, "backend {backend}");

            let seek = ByteOffset::new(3);
            let mut cursor = snapshot
                .chunk_cursor(seek)
                .expect("seek offset is a UTF-8 boundary");
            let first = cursor.next().expect("seek before EOF should find a chunk");
            assert!(first.start() <= seek, "backend {backend}");
            assert!(seek < first.end(), "backend {backend}");
            let suffix: String = std::iter::once(first)
                .chain(cursor)
                .map(|chunk| chunk.text())
                .collect();
            let first_start = usize::try_from(first.start()).expect("offset should fit usize");
            assert_eq!(suffix, expected[first_start..], "backend {backend}");

            assert!(
                snapshot
                    .chunk_cursor(snapshot.len_bytes())
                    .expect("EOF is a valid cursor boundary")
                    .next()
                    .is_none()
            );
            let invalid = ByteOffset::new(10);
            assert!(
                matches!(
                    snapshot.chunk_cursor(invalid),
                    Err(TextPositionError::NotUtf8Boundary(offset)) if offset == invalid
                ),
                "backend {backend}"
            );
        }
    }

    #[test]
    fn byte_utf16_and_line_queries_match_string_model() {
        for backend in StorageBackend::ALL {
            let mut model = String::from("# title\r\n羽🙂\nlast");
            let mut buffer = TextBuffer::with_backend(model.clone(), backend);
            apply_model_edit(&mut buffer, &mut model, 0..0, "前\n");
            let middle = model.find('羽').expect("fixture contains 羽");
            apply_model_edit(&mut buffer, &mut model, middle..middle, "mid🙂\n");
            let last = model.find("last").expect("fixture contains last");
            apply_model_edit(&mut buffer, &mut model, last..last + 4, "尾");

            let snapshot = buffer.snapshot();
            let boundaries = model
                .char_indices()
                .map(|(byte, _)| byte)
                .chain(std::iter::once(model.len()));
            for byte in boundaries {
                let byte_offset =
                    ByteOffset::try_from(byte).expect("test byte offset should fit u64");
                let expected_utf16 = u64::try_from(model[..byte].encode_utf16().count())
                    .expect("test UTF-16 offset should fit u64");
                let expected_line = u64::try_from(
                    model[..byte]
                        .bytes()
                        .filter(|value| *value == b'\n')
                        .count(),
                )
                .expect("test line index should fit u64");

                assert_eq!(
                    snapshot.utf16_offset(byte_offset),
                    Ok(Utf16Offset::new(expected_utf16)),
                    "backend {backend}, byte {byte}"
                );
                assert_eq!(
                    snapshot.byte_offset_for_utf16(Utf16Offset::new(expected_utf16)),
                    Ok(byte_offset),
                    "backend {backend}, UTF-16 {expected_utf16}"
                );
                assert_eq!(
                    snapshot.line_index(byte_offset),
                    Ok(LineIndex::new(expected_line)),
                    "backend {backend}, byte {byte}"
                );
            }

            let line_starts = std::iter::once(0)
                .chain(
                    model
                        .bytes()
                        .enumerate()
                        .filter_map(|(byte, value)| (value == b'\n').then_some(byte + 1)),
                )
                .collect::<Vec<_>>();
            assert_eq!(
                snapshot.summary().line_count(),
                line_starts.len() as u64,
                "backend {backend}"
            );
            for (line, expected_start) in line_starts.into_iter().enumerate() {
                assert_eq!(
                    snapshot.line_start(LineIndex::new(line as u64)),
                    Ok(ByteOffset::try_from(expected_start).expect("offset should fit u64")),
                    "backend {backend}, line {line}"
                );
            }

            let emoji_byte = model.find('🙂').expect("fixture contains emoji");
            let emoji_utf16 = model[..emoji_byte].encode_utf16().count() as u64;
            let split = Utf16Offset::new(emoji_utf16 + 1);
            assert_eq!(
                snapshot.byte_offset_for_utf16(split),
                Err(TextPositionError::Utf16InsideScalar(split)),
                "backend {backend}"
            );
        }
    }

    fn apply_model_edit(
        buffer: &mut TextBuffer,
        model: &mut String,
        range: std::ops::Range<usize>,
        inserted: &str,
    ) {
        let text_range = TextRange::new(
            ByteOffset::try_from(range.start).expect("test offset should fit u64"),
            ByteOffset::try_from(range.end).expect("test offset should fit u64"),
        )
        .expect("ordered model offsets should form a range");
        let transaction = Transaction::new(buffer.revision(), [Edit::new(text_range, inserted)]);
        buffer
            .apply(&transaction)
            .expect("model transaction should apply");
        model.replace_range(range, inserted);
    }
}
