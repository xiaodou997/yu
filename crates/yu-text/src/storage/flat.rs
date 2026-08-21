use std::ops::Range;
use std::sync::Arc;

use super::{AllocationCollector, StorageBackend, StorageChunk, StorageStats};
use crate::TextSummary;
use crate::summary::{byte_after_line_break, byte_offset_for_utf16};

#[derive(Debug)]
pub(crate) struct FlatStore {
    text: Arc<str>,
}

impl FlatStore {
    pub(super) fn new(text: String) -> Self {
        Self {
            text: Arc::from(text),
        }
    }

    pub(super) fn len_bytes(&self) -> usize {
        self.text.len()
    }

    pub(super) fn is_char_boundary(&self, offset: usize) -> bool {
        self.text.is_char_boundary(offset)
    }

    pub(super) fn slice(&self, range: Range<usize>) -> String {
        self.text[range].to_owned()
    }

    pub(super) fn replace_range(&mut self, range: Range<usize>, inserted: &str) {
        let mut text = self.text.to_string();
        text.replace_range(range, inserted);
        self.text = Arc::from(text);
    }

    pub(super) fn snapshot(&self) -> FlatSnapshot {
        FlatSnapshot {
            text: Arc::clone(&self.text),
        }
    }

    pub(super) fn stats(&self) -> StorageStats {
        FlatSnapshot {
            text: Arc::clone(&self.text),
        }
        .stats()
    }
}

#[derive(Clone, Debug)]
pub(crate) struct FlatSnapshot {
    text: Arc<str>,
}

impl FlatSnapshot {
    pub(super) fn text(&self) -> Arc<str> {
        Arc::clone(&self.text)
    }

    pub(super) fn len_bytes(&self) -> usize {
        self.text.len()
    }

    pub(super) fn write_to(&self, output: &mut String) {
        output.push_str(&self.text);
    }

    pub(super) fn stats(&self) -> StorageStats {
        StorageStats::new(
            StorageBackend::FlatReference,
            self.text.len(),
            usize::from(!self.text.is_empty()),
        )
    }

    pub(super) fn summary(&self) -> TextSummary {
        TextSummary::from_text(&self.text)
    }

    pub(super) fn is_char_boundary(&self, offset: usize) -> bool {
        self.text.is_char_boundary(offset)
    }

    pub(super) fn prefix_summary(&self, offset: usize) -> TextSummary {
        TextSummary::from_text(&self.text[..offset])
    }

    pub(super) fn byte_offset_for_utf16(&self, offset: u64) -> Option<usize> {
        byte_offset_for_utf16(&self.text, offset)
    }

    pub(super) fn byte_offset_for_line(&self, line: u64) -> Option<usize> {
        byte_after_line_break(&self.text, line)
    }

    pub(super) fn collect_allocations(&self, collector: &mut AllocationCollector) {
        collector.add_text(&self.text);
    }

    pub(super) fn chunks_from(&self, offset: usize) -> FlatChunkCursor<'_> {
        FlatChunkCursor {
            snapshot: self,
            yielded: offset >= self.text.len(),
        }
    }

    pub(super) fn chunk_before(&self, offset: usize) -> Option<StorageChunk<'_>> {
        (offset >= self.text.len() && !self.text.is_empty()).then_some(StorageChunk {
            start: 0,
            text: &self.text,
        })
    }
}

pub(super) struct FlatChunkCursor<'a> {
    snapshot: &'a FlatSnapshot,
    yielded: bool,
}

impl<'a> Iterator for FlatChunkCursor<'a> {
    type Item = StorageChunk<'a>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.yielded {
            return None;
        }
        self.yielded = true;
        Some(StorageChunk {
            start: 0,
            text: &self.snapshot.text,
        })
    }
}
