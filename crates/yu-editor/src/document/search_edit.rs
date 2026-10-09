//! Find/replace edits the canonical Markdown snapshot, never rendered text.
use super::*;
use yu_text::Edit;

impl EditorDocument {
    /// Bounded canonical text for the explicit Use Selection for Find action.
    /// Multi-cursor/grid selections and active composition are not query seeds.
    #[must_use]
    pub fn search_query_from_selection(&self) -> Option<String> {
        if self.composition().is_some()
            || self.selections().is_multiple()
            || self.selections().table_columns().is_some()
        {
            return None;
        }
        let range = self.selection().ordered_range();
        let length = range.end().get() - range.start().get();
        if length == 0 || length > 4096 {
            return None;
        }
        let snapshot = self.snapshot();
        let text = snapshot
            .as_str()
            .get(range.start().get() as usize..range.end().get() as usize)?;
        (!text.contains(['\r', '\n', '\0'])).then(|| text.to_owned())
    }

    /// Replace the exact current match, or all non-overlapping matches.
    ///
    /// An unrelated selection is never a replacement target. All replacements
    /// are planned against one revision and committed as one isolated undo step.
    /// Empty queries, missing matches and identical replacement text are no-ops.
    ///
    /// # Errors
    /// Rejects active IME composition or an invalid source transaction.
    pub fn replace_search(
        &mut self,
        replacement: &str,
        all: bool,
    ) -> Result<CommandResult, EditorDocumentError> {
        if self.composition().is_some() {
            return Err(EditorDocumentError::CompositionActive);
        }
        let Some(search) = self.search() else {
            return Ok(self.command_result(false));
        };
        // Derived results must belong to this source snapshot, even if a future
        // asynchronous search implementation changes when they are published.
        if search.revision() != self.revision() || search.is_empty() {
            return Ok(self.command_result(false));
        }
        let ranges = if all {
            search.matches().to_vec()
        } else if !self.selections().is_multiple() && self.selections().table_columns().is_none() {
            search
                .current(self.selection().ordered_range())
                .map(|index| vec![search.matches()[index]])
                .unwrap_or_default()
        } else {
            Vec::new()
        };
        // Adjacent deletions must share one inverse insertion point. Separate
        // empty edits would produce overlapping insertions during undo.
        let ranges = if replacement.is_empty() {
            let mut merged: Vec<TextRange> = Vec::new();
            for range in ranges {
                if let Some(previous) = merged.last_mut()
                    && previous.end() == range.start()
                {
                    *previous = TextRange::new(previous.start(), range.end())
                        .expect("ordered adjacent matches");
                } else {
                    merged.push(range);
                }
            }
            merged
        } else {
            ranges
        };
        let snapshot = self.snapshot();
        let edits: Vec<_> = ranges
            .into_iter()
            .filter(|range| {
                snapshot
                    .as_str()
                    .get(range.start().get() as usize..range.end().get() as usize)
                    != Some(replacement)
            })
            .map(|range| Edit::new(range, replacement))
            .collect();
        if edits.is_empty() {
            return Ok(self.command_result(false));
        }
        self.state.history.break_group();
        let result = self.apply_transaction(&Transaction::new(self.revision(), edits));
        self.state.history.break_group();
        result?;
        Ok(self.command_result(true))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::CaretAffinity;

    fn select(document: &mut EditorDocument, start: u64, end: u64) {
        let selection = EditorSelection::range(
            &document.snapshot(),
            ByteOffset::new(start),
            ByteOffset::new(end),
            CaretAffinity::Downstream,
        )
        .expect("valid selection");
        document.set_selection(selection).expect("select");
    }

    #[test]
    fn selected_query_is_bounded_unicode_source_and_never_an_edit() {
        let mut document = EditorDocument::new("**羽🙂**\nnext");
        assert_eq!(document.search_query_from_selection(), None);
        select(&mut document, 0, 11);
        let revision = document.revision();
        assert_eq!(
            document.search_query_from_selection().as_deref(),
            Some("**羽🙂**")
        );
        assert_eq!(document.revision(), revision);
        assert_eq!(document.history_stats().undo_entries(), 0);
        select(&mut document, 0, 12);
        assert_eq!(document.search_query_from_selection(), None);
        let mut long = EditorDocument::new("x".repeat(4097));
        select(&mut long, 0, 4096);
        assert_eq!(
            long.search_query_from_selection().expect("bounded").len(),
            4096
        );
        select(&mut long, 0, 4097);
        assert_eq!(long.search_query_from_selection(), None);
        long.begin_composition(
            TextRange::empty(ByteOffset::ZERO),
            "候选",
            yu_core::Utf16Range::empty(yu_core::Utf16Offset::ZERO),
        )
        .expect("composition");
        assert_eq!(long.search_query_from_selection(), None);
    }

    #[test]
    fn replace_all_is_one_undo_step_and_preserves_markdown_and_unicode() {
        let source = "# 羽🙂\n\n**羽🙂** [羽🙂](local.md)\n\n```text\n羽🙂\n```\n";
        let mut document = EditorDocument::new(source);
        document.set_search_query("羽🙂");
        select(&mut document, 2, 9);
        let before_selection = document.selections().clone();
        let revision = document.revision();
        assert!(
            document
                .replace_search("Yu🪶", true)
                .expect("replace all")
                .changed()
        );
        assert_ne!(document.revision(), revision);
        assert_eq!(document.snapshot().as_str(), source.replace("羽🙂", "Yu🪶"));
        assert_eq!(document.history_stats().undo_entries(), 1);
        assert!(document.search().expect("search retained").is_empty());
        document.undo().expect("undo");
        assert_eq!(document.snapshot().as_str(), source);
        assert_eq!(
            document.selection().ordered_range(),
            before_selection.primary().ordered_range()
        );
        assert_eq!(
            document.selections().primary_index(),
            before_selection.primary_index()
        );
        assert_eq!(
            document
                .search()
                .expect("refreshed matches")
                .matches()
                .len(),
            4
        );
        document.redo().expect("redo");
        assert_eq!(document.snapshot().as_str(), source.replace("羽🙂", "Yu🪶"));
    }

    #[test]
    fn replace_current_requires_an_exact_single_match() {
        let mut document = EditorDocument::new("one one");
        document.set_search_query("one");
        select(&mut document, 0, 7);
        assert!(
            !document
                .replace_search("two", false)
                .expect("unrelated selection")
                .changed()
        );
        select(&mut document, 0, 0);
        assert!(
            !document
                .replace_search("two", false)
                .expect("cursor")
                .changed()
        );
        select(&mut document, 4, 7);
        assert!(
            document
                .replace_search("two", false)
                .expect("current match")
                .changed()
        );
        assert_eq!(document.snapshot().as_str(), "one two");
        assert_eq!(document.search().expect("refreshed").matches().len(), 1);
        document.undo().expect("undo");
        assert_eq!(
            document.selection().ordered_range(),
            TextRange::new(ByteOffset::new(4), ByteOffset::new(7)).unwrap()
        );
    }

    #[test]
    fn no_op_replacements_do_not_dirty_source_or_history() {
        let mut document = EditorDocument::new("one");
        let revision = document.revision();
        assert!(
            !document
                .replace_search("x", true)
                .expect("no query")
                .changed()
        );
        for query in ["", "missing", "one"] {
            document.set_search_query(query);
            assert!(
                !document
                    .replace_search("one", true)
                    .expect("no-op")
                    .changed()
            );
        }
        assert_eq!(document.revision(), revision);
        assert_eq!(document.history_stats().undo_entries(), 0);
    }

    #[test]
    fn empty_replacement_deletes_all_non_overlapping_matches() {
        let mut document = EditorDocument::new("aaaaa");
        document.set_search_query("aa");
        document.replace_search("", true).expect("delete");
        assert_eq!(document.snapshot().as_str(), "a");
        document.undo().expect("undo");
        assert_eq!(document.snapshot().as_str(), "aaaaa");
    }

    #[test]
    fn replacement_containing_query_does_not_recursively_replace_new_matches() {
        let mut document = EditorDocument::new("a a");
        document.set_search_query("a");
        document
            .replace_search("aaa", true)
            .expect("bounded replace");
        assert_eq!(document.snapshot().as_str(), "aaa aaa");
        assert_eq!(document.search().expect("updated").matches().len(), 6);
        assert_eq!(document.history_stats().undo_entries(), 1);
    }

    #[test]
    fn replacement_supports_multiline_text_without_rich_text_roundtrip() {
        let source = "| a | b |\n|---|---|\n| key | `key` |\n";
        let mut document = EditorDocument::new(source);
        document.set_search_query("key");
        document
            .replace_search("$1\\path\n羽", true)
            .expect("literal replacement");
        assert_eq!(
            document.snapshot().as_str(),
            source.replace("key", "$1\\path\n羽")
        );
        document.undo().expect("undo");
        assert_eq!(document.snapshot().as_str(), source);
    }

    #[test]
    fn search_replace_is_isolated_from_adjacent_typing() {
        let mut document = EditorDocument::new("a a");
        document
            .execute(EditorCommand::insert_text("x"))
            .expect("typing before");
        let before = document.snapshot().as_str().to_owned();
        document.set_search_query("a");
        document.replace_search("b", true).expect("replace");
        let replaced = document.snapshot().as_str().to_owned();
        document
            .execute(EditorCommand::insert_text("y"))
            .expect("typing after");
        document.undo().expect("undo typing");
        assert_eq!(document.snapshot().as_str(), replaced);
        document.undo().expect("undo replacements only");
        assert_eq!(document.snapshot().as_str(), before);
        document.undo().expect("undo earlier typing");
        assert_eq!(document.snapshot().as_str(), "a a");
    }

    #[test]
    fn composition_blocks_current_and_all_replacements_without_mutation() {
        let mut document = EditorDocument::new("a a");
        document.set_search_query("a");
        let revision = document.revision();
        document
            .begin_composition(
                TextRange::empty(ByteOffset::ZERO),
                "候选",
                Utf16Range::empty(yu_core::Utf16Offset::ZERO),
            )
            .expect("begin composition");
        for all in [false, true] {
            assert!(matches!(
                document.replace_search("x", all),
                Err(EditorDocumentError::CompositionActive)
            ));
        }
        assert_eq!(document.snapshot().as_str(), "a a");
        assert_eq!(document.revision(), revision);
        assert!(document.composition().is_some());
    }

    #[test]
    fn large_replace_all_uses_one_transaction() {
        let source = "羽🙂 ".repeat(2000);
        let mut document = EditorDocument::new(source.clone());
        document.set_search_query("羽🙂");
        document.replace_search("x", true).expect("replace all");
        assert_eq!(document.snapshot().as_str(), "x ".repeat(2000));
        assert_eq!(document.history_stats().undo_entries(), 1);
        document.undo().expect("undo");
        assert_eq!(document.snapshot().as_str(), source);
    }
}
