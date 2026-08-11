use yu_core::Revision;
use yu_text::{AppliedTransaction, Transaction};

const DEFAULT_HISTORY_LIMIT: usize = 512;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum HistoryGroup {
    Typing,
    Deletion,
    ListEditing,
    Composition,
    External,
}

#[derive(Clone, Debug)]
pub(crate) struct HistoryEntry {
    transaction: Transaction,
    group: u64,
}

impl HistoryEntry {
    pub(crate) fn new(transaction: Transaction, group: u64) -> Self {
        Self { transaction, group }
    }

    pub(crate) fn transaction_for(&self, revision: Revision) -> Transaction {
        Transaction::new(revision, self.transaction.edits().iter().cloned())
    }

    pub(crate) fn group(&self) -> u64 {
        self.group
    }
}

/// Bounded inverse-transaction history with lightweight command grouping.
#[derive(Debug)]
pub(crate) struct EditorHistory {
    undo: Vec<HistoryEntry>,
    redo: Vec<HistoryEntry>,
    next_group: u64,
    open_group: Option<(HistoryGroup, u64)>,
    limit: usize,
}

impl Default for EditorHistory {
    fn default() -> Self {
        Self {
            undo: Vec::new(),
            redo: Vec::new(),
            next_group: 0,
            open_group: None,
            limit: DEFAULT_HISTORY_LIMIT,
        }
    }
}

impl EditorHistory {
    pub(crate) fn record(&mut self, applied: &AppliedTransaction, kind: HistoryGroup) {
        self.redo.clear();
        let group = match self.open_group {
            Some((open_kind, group)) if open_kind == kind => group,
            _ => {
                let group = self.next_group;
                self.next_group = self.next_group.saturating_add(1);
                self.open_group = Some((kind, group));
                group
            }
        };
        self.undo
            .push(HistoryEntry::new(applied.inverse().clone(), group));
        Self::trim(self.limit, &mut self.undo);
    }

    pub(crate) fn break_group(&mut self) {
        self.open_group = None;
    }

    pub(crate) fn clear(&mut self) {
        self.undo.clear();
        self.redo.clear();
        self.open_group = None;
    }

    pub(crate) fn stats(&self) -> HistoryStats {
        HistoryStats {
            undo_entries: self.undo.len(),
            redo_entries: self.redo.len(),
            limit: self.limit,
        }
    }

    pub(crate) fn pop_undo_group(&mut self) -> Option<Vec<HistoryEntry>> {
        let group = self.undo.last()?.group;
        let mut entries = Vec::new();
        while self.undo.last().is_some_and(|entry| entry.group == group) {
            entries.push(self.undo.pop().expect("last entry exists"));
        }
        self.open_group = None;
        Some(entries)
    }

    pub(crate) fn pop_redo_group(&mut self) -> Option<Vec<HistoryEntry>> {
        let group = self.redo.last()?.group;
        let mut entries = Vec::new();
        while self.redo.last().is_some_and(|entry| entry.group == group) {
            entries.push(self.redo.pop().expect("last entry exists"));
        }
        self.open_group = None;
        Some(entries)
    }

    pub(crate) fn restore_undo_group(&mut self, entries: &[HistoryEntry]) {
        for entry in entries.iter().rev() {
            self.undo.push(entry.clone());
        }
        Self::trim(self.limit, &mut self.undo);
    }

    pub(crate) fn restore_redo_group(&mut self, entries: &[HistoryEntry]) {
        for entry in entries.iter().rev() {
            self.redo.push(entry.clone());
        }
        Self::trim(self.limit, &mut self.redo);
    }

    pub(crate) fn push_redo_group(&mut self, entries: Vec<HistoryEntry>) {
        self.redo.extend(entries);
        Self::trim(self.limit, &mut self.redo);
    }

    pub(crate) fn push_undo_group(&mut self, entries: Vec<HistoryEntry>) {
        self.undo.extend(entries);
        Self::trim(self.limit, &mut self.undo);
    }

    fn trim(limit: usize, entries: &mut Vec<HistoryEntry>) {
        while entries.len() > limit {
            let oldest_group = entries[0].group;
            let remove = entries
                .iter()
                .take_while(|entry| entry.group == oldest_group)
                .count();
            entries.drain(..remove);
        }
    }
}

/// Observable history depth and capacity.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HistoryStats {
    undo_entries: usize,
    redo_entries: usize,
    limit: usize,
}

impl HistoryStats {
    #[must_use]
    pub const fn undo_entries(self) -> usize {
        self.undo_entries
    }

    #[must_use]
    pub const fn redo_entries(self) -> usize {
        self.redo_entries
    }

    #[must_use]
    pub const fn limit(self) -> usize {
        self.limit
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use yu_core::{ByteOffset, Revision, TextRange};
    use yu_text::{Edit, TextBuffer, Transaction};

    fn edit_at(revision: Revision, offset: u64, text: &str) -> Transaction {
        Transaction::new(
            revision,
            [Edit::new(TextRange::empty(ByteOffset::new(offset)), text)],
        )
    }

    #[test]
    fn inverse_transactions_are_grouped_and_redo_order_is_forward() {
        let mut buffer = TextBuffer::new("");
        let mut history = EditorHistory::default();
        let first = buffer
            .apply(&edit_at(buffer.revision(), 0, "a"))
            .expect("first edit");
        history.record(&first, HistoryGroup::Typing);
        let second = buffer
            .apply(&edit_at(buffer.revision(), 1, "b"))
            .expect("second edit");
        history.record(&second, HistoryGroup::Typing);

        let undo = history.pop_undo_group().expect("undo group");
        assert_eq!(undo.len(), 2);
        let mut redo = Vec::new();
        for entry in &undo {
            let applied = buffer
                .apply(&entry.transaction_for(buffer.revision()))
                .expect("inverse should apply");
            redo.push(HistoryEntry::new(applied.inverse().clone(), entry.group));
        }
        assert_eq!(buffer.snapshot().as_str(), "");
        history.push_redo_group(redo);

        let redo = history.pop_redo_group().expect("redo group");
        for entry in &redo {
            buffer
                .apply(&entry.transaction_for(buffer.revision()))
                .expect("redo should apply");
        }
        assert_eq!(buffer.snapshot().as_str(), "ab");
    }

    #[test]
    fn a_different_group_breaks_consecutive_input() {
        let mut buffer = TextBuffer::new("");
        let mut history = EditorHistory::default();
        let first = buffer
            .apply(&edit_at(buffer.revision(), 0, "a"))
            .expect("edit");
        history.record(&first, HistoryGroup::Typing);
        history.break_group();
        let second = buffer
            .apply(&edit_at(buffer.revision(), 1, "b"))
            .expect("edit");
        history.record(&second, HistoryGroup::Typing);
        assert_eq!(history.pop_undo_group().expect("group").len(), 1);
    }
}
