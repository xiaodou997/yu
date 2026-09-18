//! Confirmed column proportions follow source anchors, not transient drag revisions.
use super::*;
use yu_core::{Affinity, TextAnchor};
use yu_text::ChangeSet;

#[derive(Clone, Debug, PartialEq)]
pub(super) struct SavedTableWidths {
    delimiter: TextRange,
    proportions: Arc<[f32]>,
}

/// Portable presentation metadata. The storage owner must bind these records
/// to the exact saved document identity and source fingerprint before restoring.
#[derive(Clone, Debug, PartialEq)]
pub struct TableColumnWidthRecord {
    pub delimiter: TextRange,
    pub proportions: Vec<f32>,
}

impl LayoutContext {
    fn table_width_anchor_is_current(&self, delimiter: TextRange, columns: usize) -> bool {
        let blocks = self.markdown.blocks();
        blocks
            .block_index_for_offset(delimiter.start())
            .and_then(|index| blocks.get(index))
            .and_then(|block| yu_markdown::table_for_block(&self.markdown, block))
            .is_some_and(|table| {
                table.column_count() == columns
                    && table.delimiter_source_range().is_some_and(|range| {
                        range.start() as u64 == delimiter.start().get()
                            && range.end() as u64 == delimiter.end().get()
                    })
            })
    }

    /// Export confirmed proportions only; active drag geometry is not persisted.
    #[must_use]
    pub fn table_column_width_records(&self) -> Vec<TableColumnWidthRecord> {
        self.table_widths
            .iter()
            .filter(|entry| {
                self.table_width_anchor_is_current(entry.delimiter, entry.proportions.len())
            })
            .map(|entry| TableColumnWidthRecord {
                delimiter: entry.delimiter,
                proportions: entry.proportions.to_vec(),
            })
            .collect()
    }

    /// Replace presentation metadata after the storage owner validates document
    /// identity. Validate every record before mutation; malformed or stale
    /// records never partially alter layout, source, selection, or undo history.
    pub fn restore_table_column_width_records(
        &mut self,
        records: &[TableColumnWidthRecord],
    ) -> Result<(), EditorDocumentError> {
        let mut restored = Vec::with_capacity(records.len());
        let mut seen = std::collections::BTreeSet::new();
        for record in records {
            let sum: f32 = record.proportions.iter().sum();
            let valid_numbers = !record.proportions.is_empty()
                && record
                    .proportions
                    .iter()
                    .all(|p| p.is_finite() && *p > 0.0 && *p <= 1.0)
                && sum.is_finite()
                && (sum - 1.0).abs() <= 0.0001;
            let key = (record.delimiter.start().get(), record.delimiter.end().get());
            let valid_table =
                self.table_width_anchor_is_current(record.delimiter, record.proportions.len());
            if !valid_numbers || !seen.insert(key) || !valid_table {
                return Err(
                    LayoutError::Upstream("invalid saved table column widths".into()).into(),
                );
            }
            restored.push(SavedTableWidths {
                delimiter: record.delimiter,
                proportions: record.proportions.clone().into(),
            });
        }
        if *self.table_widths != restored {
            let mut previous = self
                .table_widths
                .iter()
                .map(|entry| ((entry.delimiter.start(), entry.delimiter.end()), entry))
                .collect::<std::collections::BTreeMap<_, _>>();
            let mut changed = Vec::new();
            for entry in &restored {
                let old = previous.remove(&(entry.delimiter.start(), entry.delimiter.end()));
                if old.is_none_or(|old| old.proportions != entry.proportions) {
                    changed.push(entry.delimiter);
                }
            }
            changed.extend(previous.into_values().map(|entry| entry.delimiter));
            for delimiter in changed {
                if let Some(index) = self
                    .markdown
                    .blocks()
                    .block_index_for_offset(delimiter.start())
                {
                    self.viewport.invalidate_block_measurement(index);
                }
            }
            self.table_widths = Arc::new(restored);
            self.table_width_generation = self.table_width_generation.wrapping_add(1);
            self.render_identity = Arc::new(());
            self.layout_snapshot = None;
        }
        Ok(())
    }

    /// Geometry identity for confirmed column widths, independent of source edits.
    #[must_use]
    pub fn table_width_generation(&self) -> u64 {
        self.table_width_generation
    }

    /// Accept only geometry belonging to this document revision and table.
    /// This changes presentation state without rewriting Markdown source.
    pub fn confirm_table_column_widths(
        &mut self,
        index: usize,
        table: &crate::TableLayout,
    ) -> Result<(), EditorDocumentError> {
        let block = self.block_at(index)?;
        let source_table = yu_markdown::table_for_block(&self.markdown, block)
            .ok_or_else(|| LayoutError::Upstream("missing table".into()))?;
        let delimiter = source_table
            .delimiter_source_range()
            .and_then(|r| {
                TextRange::new(
                    ByteOffset::new(r.start() as u64),
                    ByteOffset::new(r.end() as u64),
                )
            })
            .ok_or_else(|| LayoutError::Upstream("missing table delimiter".into()))?;
        if table.revision() != self.revision()
            || table.delimiter_source() != Some(delimiter)
            || table.column_widths().len() != source_table.column_count()
        {
            return Err(LayoutError::Upstream("stale table width geometry".into()).into());
        }
        let total: f32 = table.column_widths().iter().sum();
        if !total.is_finite()
            || total <= 0.0
            || table
                .column_widths()
                .iter()
                .any(|w| !w.is_finite() || *w <= 0.0)
        {
            return Err(LayoutError::Upstream("invalid column widths".into()).into());
        }
        let saved = SavedTableWidths {
            delimiter,
            proportions: table.column_widths().iter().map(|w| *w / total).collect(),
        };
        let widths = Arc::make_mut(&mut self.table_widths);
        widths.retain(|entry| entry.delimiter != delimiter);
        widths.push(saved);
        self.table_width_generation = self.table_width_generation.wrapping_add(1);
        self.render_identity = Arc::new(());
        self.layout_snapshot = None;
        // A column resize changes only this table's wrapping. Keep the measured
        // prefix so an immediate pointer query still resolves the same block;
        // clearing it would replace preceding paragraphs with height estimates.
        self.viewport.invalidate_block_measurement(index);
        Ok(())
    }

    pub(super) fn has_saved_table_widths(&self, index: usize) -> bool {
        self.markdown.blocks().get(index).is_some_and(|block| {
            self.table_widths.iter().any(|entry| {
                block.range().start() <= entry.delimiter.start()
                    && entry.delimiter.end() <= block.range().end()
            })
        })
    }

    pub(super) fn measured_table_aware_height<S: ShapingProvider>(
        &mut self,
        index: usize,
        config: LayoutConfig,
        shaper: &S,
        sizes: &[ImageSize],
    ) -> Result<f32, EditorDocumentError> {
        if self.has_saved_table_widths(index) {
            Ok(self
                .block_layout_for_visual_state_with_shaper_and_images(index, config, shaper, sizes)?
                .height())
        } else {
            Ok(self
                .block_layout_with_shaper_and_images(index, config, shaper, sizes)?
                .height())
        }
    }

    pub(super) fn saved_table_widths(&self, _index: usize, layout: &BlockView) -> Option<Vec<f32>> {
        let table = layout.table()?;
        let delimiter = table.delimiter_source()?;
        let saved = self
            .table_widths
            .iter()
            .find(|entry| entry.delimiter == delimiter)?;
        if saved.proportions.len() != table.column_widths().len() {
            return None;
        }
        let total = table.bounds().width();
        Some(
            saved
                .proportions
                .iter()
                .map(|ratio| ratio * total)
                .collect(),
        )
    }

    pub(super) fn map_table_widths(&mut self, changes: &ChangeSet) {
        if self.table_widths.is_empty() {
            return;
        }
        let old_source = self.markdown.source();
        let new_source = &self.source;
        Arc::make_mut(&mut self.table_widths).retain_mut(|saved| {
            let mut aligned_at_start = false;
            for change in changes.changes() {
                let old = change.old_range();
                let touches = if old.is_empty() {
                    saved.delimiter.start() < old.start() && old.start() < saved.delimiter.end()
                } else {
                    old.start() < saved.delimiter.end() && saved.delimiter.start() < old.end()
                };
                if !touches {
                    continue;
                }
                // Column alignment replaces one delimiter cell, including on
                // undo/redo. It retains the table's column identity. Replacing
                // a row, a column boundary, or the table itself does not.
                if old.start() < saved.delimiter.start() || old.end() > saved.delimiter.end() {
                    return false;
                }
                let Ok(before) = read_source_range(old_source, old) else {
                    return false;
                };
                let Ok(after) = read_source_range(new_source, change.new_range()) else {
                    return false;
                };
                let dashes = before.trim_matches(':');
                if dashes.is_empty()
                    || !dashes.bytes().all(|b| b == b'-')
                    || after.trim_matches(':') != dashes
                {
                    return false;
                }
                aligned_at_start |= old.start() == saved.delimiter.start();
            }
            let map = |at, affinity| {
                changes
                    .map_anchor(TextAnchor::new(changes.before(), at, affinity))
                    .ok()
                    .map(|a| a.offset())
            };
            let Some(start) = map(
                saved.delimiter.start(),
                if aligned_at_start {
                    Affinity::Before
                } else {
                    Affinity::After
                },
            ) else {
                return false;
            };
            let Some(end) = map(saved.delimiter.end(), Affinity::Before) else {
                return false;
            };
            let Some(range) = TextRange::new(start, end) else {
                return false;
            };
            saved.delimiter = range;
            true
        });
    }
}

/// Checkpoints share immutable proportions and are retained only while history
/// references them. Token zero represents the common, allocation-free empty state.
#[derive(Debug, Default)]
pub(super) struct WidthHistory {
    next: u64,
    checkpoints: std::collections::BTreeMap<u64, Arc<Vec<SavedTableWidths>>>,
}

impl WidthHistory {
    pub(super) fn capture(&mut self, widths: Arc<Vec<SavedTableWidths>>) -> u64 {
        if widths.is_empty() {
            return 0;
        }
        self.next = self
            .next
            .checked_add(1)
            .expect("width history token exhausted");
        self.checkpoints.insert(self.next, widths);
        self.next
    }
}

impl EditorDocument {
    pub(super) fn prune_width_history(&mut self) {
        let retained: std::collections::BTreeSet<_> =
            self.state.history.presentation_tokens().collect();
        self.state
            .width_history
            .checkpoints
            .retain(|id, _| retained.contains(id));
    }

    pub(super) fn restore_width_history(&mut self, token: Option<u64>) {
        let Some(saved) = token
            .and_then(|id| self.state.width_history.checkpoints.get(&id))
            .cloned()
        else {
            return;
        };
        let mut restored = Vec::new();
        let mut changed_blocks = Vec::new();
        for entry in saved.iter() {
            // Existing associations already followed the inverse ChangeSet. Keep
            // later manual resizes instead of replacing them with an old snapshot.
            if self
                .table_widths
                .iter()
                .any(|current| current.delimiter == entry.delimiter)
            {
                continue;
            }
            let blocks = self.markdown.blocks();
            let Some(index) = blocks.block_index_for_offset(entry.delimiter.start()) else {
                continue;
            };
            let Some(table) = blocks
                .get(index)
                .and_then(|block| yu_markdown::table_for_block(&self.markdown, block))
            else {
                continue;
            };
            let valid = table.column_count() == entry.proportions.len()
                && table.delimiter_source_range().is_some_and(|range| {
                    range.start() as u64 == entry.delimiter.start().get()
                        && range.end() as u64 == entry.delimiter.end().get()
                });
            if valid {
                restored.push(entry.clone());
                changed_blocks.push(index);
            }
        }
        if !restored.is_empty() {
            Arc::make_mut(&mut self.presentation.table_widths).extend(restored);
            self.presentation.table_width_generation = self.table_width_generation.wrapping_add(1);
            self.presentation.render_identity = Arc::new(());
            self.presentation.layout_snapshot = None;
            for index in changed_blocks {
                self.presentation
                    .viewport
                    .invalidate_block_measurement(index);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use yu_text::Edit;
    const TABLE: &str = "| H | B |\n| --- | --- |\n| one | two |\n";

    fn resize(document: &mut EditorDocument, index: usize, delta: f32) -> Vec<f32> {
        let mut layout = document
            .block_layout_for_visual_state(index, LayoutConfig::new(500.0, 16.0))
            .expect("layout");
        layout.apply_table_column_resize(0, delta).expect("resize");
        document
            .confirm_table_column_widths(index, layout.table().expect("table"))
            .expect("confirm");
        document
            .table_widths
            .iter()
            .find(|w| Some(w.delimiter) == layout.table().expect("table").delimiter_source())
            .expect("saved")
            .proportions
            .to_vec()
    }

    #[test]
    fn confirming_widths_retains_measured_prefix_and_table_height_estimate() {
        let source = format!("# Heading\n\nA paragraph before the table.\n\n{TABLE}");
        let mut document = EditorDocument::new(&source);
        let table_index = document
            .markdown
            .blocks()
            .iter()
            .position(|block| yu_markdown::table_for_block(&document.markdown, block).is_some())
            .expect("table block");
        let markdown = document.markdown.clone();
        document.viewport.sync(&markdown).expect("sync");
        let count = document.viewport.stats().entries();
        for index in 0..count {
            document
                .viewport
                .set_block_height(index, 50.0 + index as f32)
                .expect("measured height");
        }
        let total = document.viewport.height_index().total_height();
        resize(&mut document, table_index, 1.0);
        assert_eq!(document.viewport.stats().entries(), count);
        assert_eq!(document.viewport.stats().measured(), count - 1);
        assert_eq!(document.viewport.height_index().total_height(), total);
        assert_eq!(document.snapshot().as_str(), source);
    }

    fn measure_all_blocks(document: &mut EditorDocument) -> (usize, f32) {
        let markdown = document.markdown.clone();
        document.viewport.sync(&markdown).expect("sync");
        let count = document.viewport.stats().entries();
        for index in 0..count {
            document
                .viewport
                .set_block_height(index, 80.0 + index as f32)
                .expect("height");
        }
        (count, document.viewport.height_index().total_height())
    }

    #[test]
    fn restoring_widths_invalidates_only_changed_or_removed_tables() {
        let source = format!("# Intro\n\n{TABLE}\nUnchanged paragraph.\n\n{TABLE}");
        let mut document = EditorDocument::new(&source);
        let tables = document
            .markdown
            .blocks()
            .iter()
            .enumerate()
            .filter_map(|(index, block)| {
                yu_markdown::table_for_block(&document.markdown, block).map(|_| index)
            })
            .collect::<Vec<_>>();
        resize(&mut document, tables[0], 10.0);
        resize(&mut document, tables[1], 20.0);
        let mut records = document.table_column_width_records();
        records[0].proportions = vec![0.4, 0.6];
        for replacement in [records.clone(), records[1..].to_vec(), Vec::new()] {
            let (count, height) = measure_all_blocks(&mut document);
            document
                .restore_table_column_width_records(&replacement)
                .expect("restore");
            assert_eq!(document.viewport.stats().entries(), count);
            assert_eq!(document.viewport.stats().measured(), count - 1);
            assert_eq!(document.viewport.height_index().total_height(), height);
            assert_eq!(document.snapshot().as_str(), source);
        }
    }

    #[test]
    fn history_width_restore_keeps_unrelated_measured_geometry() {
        let source = format!("# Intro\n\n{TABLE}\nTail paragraph.\n");
        let mut document = EditorDocument::new(&source);
        let table = document
            .markdown
            .blocks()
            .iter()
            .position(|block| yu_markdown::table_for_block(&document.markdown, block).is_some())
            .expect("table");
        resize(&mut document, table, 10.0);
        let widths = document.table_widths.clone();
        let token = document.state.width_history.capture(widths);
        document
            .restore_table_column_width_records(&[])
            .expect("remove widths");
        let (count, height) = measure_all_blocks(&mut document);
        document.restore_width_history(Some(token));
        assert_eq!(document.table_column_width_records().len(), 1);
        assert_eq!(document.viewport.stats().entries(), count);
        assert_eq!(document.viewport.stats().measured(), count - 1);
        assert_eq!(document.viewport.height_index().total_height(), height);
        assert_eq!(document.snapshot().as_str(), source);
    }

    #[test]
    fn width_records_round_trip_without_touching_source_or_history() {
        let mut original = EditorDocument::new(TABLE);
        resize(&mut original, 0, -60.0);
        let records = original.table_column_width_records();
        let mut reopened = EditorDocument::new(TABLE);
        let revision = reopened.revision();
        reopened
            .restore_table_column_width_records(&records)
            .expect("restore");
        assert_eq!(reopened.snapshot().as_str(), TABLE);
        assert_eq!(reopened.revision(), revision);
        assert_eq!(reopened.table_column_width_records(), records);
        let before = original
            .block_layout_for_visual_state(0, LayoutConfig::new(500.0, 16.0))
            .expect("before");
        let after = reopened
            .block_layout_for_visual_state(0, LayoutConfig::new(500.0, 16.0))
            .expect("after");
        assert_eq!(
            before.table().expect("table").column_widths(),
            after.table().expect("table").column_widths()
        );
        let generation = reopened.table_width_generation();
        reopened
            .restore_table_column_width_records(&records)
            .expect("idempotent");
        assert_eq!(reopened.table_width_generation(), generation);
        assert!(!reopened.undo().expect("undo").changed());
    }

    #[test]
    fn invalid_width_records_reject_atomically() {
        let mut document = EditorDocument::new(TABLE);
        resize(&mut document, 0, 20.0);
        let records = document.table_column_width_records();
        let generation = document.table_width_generation();
        let mut variants = vec![vec![records[0].clone(), records[0].clone()]];
        for widths in [
            vec![f32::NAN, 0.5],
            vec![f32::INFINITY, 0.5],
            vec![0.0, 1.0],
            vec![0.2, 0.2],
            vec![1.0],
        ] {
            let mut record = records[0].clone();
            record.proportions = widths;
            variants.push(vec![record]);
        }
        let mut stale = records[0].clone();
        stale.delimiter = TextRange::new(ByteOffset::ZERO, ByteOffset::new(3)).expect("range");
        variants.push(vec![records[0].clone(), stale]);
        for invalid in variants {
            assert!(
                document
                    .restore_table_column_width_records(&invalid)
                    .is_err()
            );
            assert_eq!(document.table_column_width_records(), records);
            assert_eq!(document.table_width_generation(), generation);
            assert_eq!(document.snapshot().as_str(), TABLE);
        }
        document
            .restore_table_column_width_records(&[])
            .expect("clear");
        assert!(document.table_column_width_records().is_empty());
        assert!(document.table_width_generation() > generation);
    }

    #[test]
    fn deleted_table_widths_restore_without_overwriting_later_resize() {
        let original = format!("{TABLE}\n{TABLE}");
        let mut document = EditorDocument::new(original.clone());
        let first = resize(&mut document, 0, -40.0);
        let second_index = document
            .block_index_for_source(ByteOffset::new(TABLE.len() as u64 + 1))
            .expect("second table");
        resize(&mut document, second_index, 25.0);
        let range = TextRange::new(ByteOffset::ZERO, ByteOffset::new(TABLE.len() as u64 + 1))
            .expect("range");
        document
            .apply_transaction(&Transaction::new(
                document.revision(),
                [Edit::new(range, "")],
            ))
            .expect("delete first");
        let second = resize(&mut document, 0, -30.0);
        for _ in 0..3 {
            document.undo().expect("undo");
            assert_eq!(document.snapshot().as_str(), original);
            assert_eq!(document.table_widths.len(), 2);
            let mut widths = document.table_widths.iter().collect::<Vec<_>>();
            widths.sort_by_key(|w| w.delimiter.start());
            assert_eq!(&*widths[0].proportions, first);
            assert_eq!(&*widths[1].proportions, second);
            document.redo().expect("redo");
            assert_eq!(document.snapshot().as_str(), TABLE);
            assert_eq!(&*document.table_widths[0].proportions, second);
        }
    }

    #[test]
    fn structural_edit_replay_captures_later_manual_widths() {
        use crate::TableEdit;
        for edit in [TableEdit::InsertColumnAfter, TableEdit::DeleteColumn] {
            let mut document = EditorDocument::new(TABLE);
            let old = resize(&mut document, 0, -40.0);
            let at = ByteOffset::new(TABLE.find("one").expect("cell") as u64);
            document
                .set_selection(
                    EditorSelection::cursor(
                        &document.snapshot(),
                        at,
                        crate::CaretAffinity::Downstream,
                    )
                    .expect("cursor"),
                )
                .expect("select");
            document
                .execute(EditorCommand::EditTable(edit))
                .expect("structure");
            let edited = document.snapshot().as_str().to_owned();
            let after = if edit == TableEdit::InsertColumnAfter {
                Some(resize(&mut document, 0, 20.0))
            } else {
                None
            };
            for _ in 0..3 {
                document.undo().expect("undo");
                assert_eq!(document.snapshot().as_str(), TABLE);
                assert_eq!(&*document.table_widths[0].proportions, old);
                document.redo().expect("redo");
                assert_eq!(document.snapshot().as_str(), edited);
                if let Some(after) = &after {
                    assert_eq!(&*document.table_widths[0].proportions, after);
                }
            }
        }
    }

    #[test]
    fn grouped_table_deletion_and_prefix_edits_restore_in_order() {
        let mut document = EditorDocument::new(TABLE);
        let old = resize(&mut document, 0, -40.0);
        let whole =
            TextRange::new(ByteOffset::ZERO, document.snapshot().len_bytes()).expect("whole");
        document
            .apply_transaction(&Transaction::new(
                document.revision(),
                [Edit::new(whole, "tail")],
            ))
            .expect("replace table");
        document
            .apply_transaction(&Transaction::new(
                document.revision(),
                [Edit::new(TextRange::empty(ByteOffset::ZERO), "prefix ")],
            ))
            .expect("prefix same group");
        for _ in 0..3 {
            document.undo().expect("group undo");
            assert_eq!(document.snapshot().as_str(), TABLE);
            assert_eq!(&*document.table_widths[0].proportions, old);
            document.redo().expect("group redo");
            assert_eq!(document.snapshot().as_str(), "prefix tail");
            assert!(document.table_widths.is_empty());
        }
    }

    #[test]
    fn checkpoints_are_bounded_and_reset_cannot_leak_widths() {
        let mut document = EditorDocument::new(TABLE);
        resize(&mut document, 0, -40.0);
        for _ in 0..530 {
            document.state.history.break_group();
            let end = document.snapshot().len_bytes();
            document
                .apply_transaction(&Transaction::new(
                    document.revision(),
                    [Edit::new(TextRange::empty(end), "\n")],
                ))
                .expect("append");
        }
        assert_eq!(
            document.state.width_history.checkpoints.len(),
            document.history_stats().undo_entries()
        );
        assert!(document.state.width_history.checkpoints.len() <= 512);
        document.undo().expect("undo");
        document.state.history.break_group();
        let end = document.snapshot().len_bytes();
        document
            .apply_transaction(&Transaction::new(
                document.revision(),
                [Edit::new(TextRange::empty(end), "different")],
            ))
            .expect("discard redo");
        assert_eq!(
            document.state.width_history.checkpoints.len(),
            document.history_stats().undo_entries()
        );
        document.reset_source(TABLE).expect("reset");
        assert!(document.table_widths.is_empty());
        assert!(document.state.width_history.checkpoints.is_empty());
    }
}
