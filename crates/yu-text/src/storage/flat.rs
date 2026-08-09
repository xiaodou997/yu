use std::ops::Range;
use std::sync::Arc;

use super::{AllocationCollector, StorageBackend, StorageStats};

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

    pub(super) fn write_to(&self, output: &mut String) {
        output.push_str(&self.text);
    }

    pub(super) fn stats(&self) -> StorageStats {
        let present = usize::from(!self.text.is_empty());
        StorageStats::new(
            StorageBackend::FlatReference,
            self.text.len(),
            present,
            present,
            present,
        )
    }

    pub(super) fn collect_allocations(&self, collector: &mut AllocationCollector) {
        collector.add_text(&self.text);
    }
}
