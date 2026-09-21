use super::*;
use crate::TableEdit;
use yu_markdown::TableCellAddress;

pub(super) struct TableEditPlan {
    pub(super) edits: Vec<yu_text::Edit>,
    pub(super) table_start: ByteOffset,
    pub(super) target: TableCellAddress,
}

pub(super) struct TableCellSelection {
    ranges: Vec<EditorSelection>,
    primary: usize,
    columns: usize,
    slots: Vec<Option<usize>>,
}

impl EditorDocument {
    pub(super) fn table_cell_selection(
        &self,
        anchor: ByteOffset,
        focus: ByteOffset,
    ) -> Option<TableCellSelection> {
        if self.source_mode() {
            return None;
        }
        let block = self
            .presentation
            .markdown
            .blocks()
            .get(self.block_index_for_offset(anchor)?)?;
        let table = self
            .html_table_grid(block)
            .or_else(|| yu_markdown::table_for_block(&self.presentation.markdown, block))?;
        let start = table.visible_cell_for_source(anchor.get() as usize)?;
        let end = table.visible_cell_for_source(focus.get() as usize)?;
        self.table_cell_selection_in_grid(&table, start, end)
    }

    fn table_cell_selection_in_grid(
        &self,
        table: &yu_markdown::TableBlock,
        start: TableCellAddress,
        end: TableCellAddress,
    ) -> Option<TableCellSelection> {
        let (first, last) = table.selection_bounds(start, end)?;
        let (first_row, last_row) = (first.row(), last.row());
        let (first_column, last_column) = (first.column(), last.column());
        let end = table.cell_origin(end)?;
        let columns = last_column - first_column + 1;
        let snapshot = self.snapshot();
        let mut ranges = Vec::new();
        let mut primary = 0;
        let mut slots = Vec::new();
        let mut owners = std::collections::HashMap::new();
        for row in first_row..=last_row {
            for column in first_column..=last_column {
                let address = TableCellAddress::new(row, column);
                let Some(cell) = table.visible_cell(address) else {
                    slots.push(None);
                    continue;
                };
                let origin = table.cell_origin(address)?;
                if let Some(owner) = owners.get(&(origin.row(), origin.column())) {
                    slots.push(Some(*owner));
                    continue;
                }
                owners.insert((origin.row(), origin.column()), ranges.len());
                slots.push(Some(ranges.len()));
                if origin == end {
                    primary = ranges.len();
                }
                ranges.push(
                    EditorSelection::range(
                        &snapshot,
                        ByteOffset::new(cell.start() as u64),
                        ByteOffset::new(cell.end() as u64),
                        crate::CaretAffinity::Downstream,
                    )
                    .ok()?,
                );
            }
        }
        Some(TableCellSelection {
            ranges,
            primary,
            columns,
            slots,
        })
    }

    pub(super) fn select_table_cells(
        &mut self,
        anchor: ByteOffset,
        focus: ByteOffset,
    ) -> Result<CommandResult, EditorDocumentError> {
        let TableCellSelection {
            ranges,
            primary,
            columns,
            slots,
        } = self
            .table_cell_selection(anchor, focus)
            .ok_or(EditorDocumentError::Selection(SelectionError::InvalidRange))?;
        let previously_revealed = self.presentation.selection_reveal_block_index();
        self.set_selections(ranges, primary)?;
        if let Some(index) = previously_revealed {
            self.presentation
                .viewport
                .invalidate_block_measurement(index);
        }
        self.presentation.selections = self
            .presentation
            .selections
            .clone()
            .with_table_slots(columns, slots)?;
        Ok(self.command_result(false))
    }

    pub(super) fn clear_table_cells(&mut self) -> Result<CommandResult, EditorDocumentError> {
        let ranges = self.presentation.selections.as_slice();
        let primary = self.presentation.selections.primary_index();
        let slots = self
            .presentation
            .selections
            .table_slots()
            .ok_or(EditorDocumentError::InvalidTablePaste)?
            .to_vec();
        let columns = self
            .presentation
            .selections
            .table_columns()
            .ok_or(EditorDocumentError::InvalidTablePaste)?;
        let anchor = ranges
            .first()
            .ok_or(EditorDocumentError::InvalidTablePaste)?
            .ordered_range()
            .start();
        let invalid = || EditorDocumentError::Selection(SelectionError::InvalidRange);
        let block = self
            .presentation
            .markdown
            .blocks()
            .get(self.block_index_for_offset(anchor).ok_or_else(invalid)?)
            .ok_or_else(invalid)?;
        let table = self
            .html_table_grid(block)
            .or_else(|| yu_markdown::table_for_block(&self.presentation.markdown, block))
            .ok_or_else(invalid)?;
        let addresses = ranges
            .iter()
            .map(|selection| {
                table
                    .visible_cell_for_source(selection.ordered_range().start().get() as usize)
                    .ok_or_else(invalid)
            })
            .collect::<Result<Vec<_>, _>>()?;
        let table_start = ByteOffset::new(table.source_range().start() as u64);
        let ranges = ranges.iter().map(|s| s.ordered_range()).collect();
        self.state.history.break_group();
        let result = self.delete_ranges(ranges, HistoryGroup::Deletion)?;
        if result.changed() {
            let block = self
                .presentation
                .markdown
                .blocks()
                .get(
                    self.block_index_for_offset(table_start)
                        .ok_or_else(invalid)?,
                )
                .ok_or_else(invalid)?;
            let table = self
                .html_table_grid(block)
                .or_else(|| yu_markdown::table_for_block(&self.presentation.markdown, block))
                .ok_or_else(invalid)?;
            let snapshot = self.snapshot();
            let ranges = addresses
                .into_iter()
                .map(|address| {
                    let cell = table.visible_cell(address).ok_or_else(invalid)?;
                    EditorSelection::range(
                        &snapshot,
                        ByteOffset::new(cell.start() as u64),
                        ByteOffset::new(cell.end() as u64),
                        crate::CaretAffinity::Downstream,
                    )
                    .map_err(EditorDocumentError::from)
                })
                .collect::<Result<Vec<_>, _>>()?;
            self.set_selections(ranges, primary)?;
            self.presentation.selections = self
                .presentation
                .selections
                .clone()
                .with_table_slots(columns, slots)?;
            self.state
                .history
                .finish_selection(self.presentation.selections.clone());
        }
        self.state.history.break_group();
        Ok(self.command_result(result.changed()))
    }

    pub(super) fn edit_table(
        &mut self,
        edit: TableEdit,
    ) -> Result<CommandResult, EditorDocumentError> {
        let Some(plan) = self.table_edit_plan(edit) else {
            return Ok(self.command_result(false));
        };
        self.state.history.break_group();
        let transaction = Transaction::new(self.revision(), plan.edits);
        self.apply_transaction_with_group(&transaction, HistoryGroup::TableEditing)?;
        let snapshot = self.snapshot();
        let anchor = plan.table_start.min(snapshot.len_bytes());
        let cell = self
            .block_index_for_offset(anchor)
            .and_then(|index| self.presentation.markdown.blocks().get(index))
            .and_then(|block| {
                self.html_table_grid(block)
                    .or_else(|| yu_markdown::table_for_block(&self.presentation.markdown, block))
            })
            .and_then(|table| table.visible_cell(plan.target));
        let focus = cell.map_or(anchor, |cell| ByteOffset::new(cell.start() as u64));
        self.set_edit_selection(EditorSelection::cursor(
            &snapshot,
            focus,
            crate::CaretAffinity::Downstream,
        )?);
        self.state.history.break_group();
        Ok(self.command_result(true))
    }

    pub(super) fn table_edit_plan(&self, edit: TableEdit) -> Option<TableEditPlan> {
        // A structural action has one owning cell. Do not silently apply it to
        // an arbitrary table when the user has disjoint selections elsewhere.
        if self.presentation.selections.is_multiple() {
            return None;
        }
        let block = self
            .presentation
            .markdown
            .blocks()
            .get(self.block_index_for_offset(self.selection().focus())?)?;
        if self
            .markdown
            .html_regions()
            .region_for(block.range())
            .is_some()
        {
            return self.html_table_edit_plan(block, edit);
        }
        let table = yu_markdown::table_for_block(&self.presentation.markdown, block)?;
        let current = table.visible_cell_for_source(self.selection().focus().get() as usize)?;
        let owner = table.visible_cell(current)?;
        let selection = self.selection().ordered_range();
        if selection.start().get() < owner.start() as u64
            || selection.end().get() > owner.end() as u64
        {
            return None;
        }
        let snapshot = self.snapshot();
        let source = snapshot.as_str();
        let mut plan = TableEditPlan {
            edits: Vec::new(),
            table_start: ByteOffset::new(table.source_range().start() as u64),
            target: current,
        };
        let replace = |start: usize, end: usize, text: String| {
            TextRange::new(ByteOffset::new(start as u64), ByteOffset::new(end as u64))
                .map(|range| yu_text::Edit::new(range, text))
        };
        match edit {
            TableEdit::AlignLeft
            | TableEdit::AlignCenter
            | TableEdit::AlignRight
            | TableEdit::AlignDefault => {
                let cell = table.delimiter().get(current.column())?;
                let dashes = source.get(cell.start()..cell.end())?.trim_matches(':');
                let text = match edit {
                    TableEdit::AlignLeft => format!(":{dashes}"),
                    TableEdit::AlignCenter => format!(":{dashes}:"),
                    TableEdit::AlignRight => format!("{dashes}:"),
                    _ => dashes.to_owned(),
                };
                if text == source.get(cell.start()..cell.end())? {
                    return None;
                }
                plan.edits.push(replace(cell.start(), cell.end(), text)?);
            }
            TableEdit::InsertColumnBefore
            | TableEdit::InsertColumnAfter
            | TableEdit::DeleteColumn => {
                if edit == TableEdit::DeleteColumn && table.column_count() == 1 {
                    plan.edits.push(replace(
                        table.source_range().start(),
                        table.source_range().end(),
                        String::new(),
                    )?);
                } else {
                    for (row_index, row) in std::iter::once(table.first_row())
                        .chain(std::iter::once(table.delimiter()))
                        .chain(table.rows().iter().map(Vec::as_slice))
                        .enumerate()
                    {
                        let cell = row.get(current.column())?;
                        let physical = table.row_ranges().get(row_index)?;
                        let first = row.first()?;
                        let last = row.last()?;
                        // A two-column row may omit both outer pipes. After
                        // deleting one column it needs explicit pipes to remain
                        // a table, rather than becoming a Setext heading/list.
                        if edit == TableEdit::DeleteColumn
                            && row.len() == 2
                            && !source.get(physical.start()..first.start())?.contains('|')
                            && !source.get(last.end()..physical.end())?.contains('|')
                        {
                            let kept = row.get(1 - current.column())?;
                            plan.edits.push(replace(
                                first.start(),
                                last.end(),
                                format!("| {} |", source.get(kept.start()..kept.end())?),
                            )?);
                            continue;
                        }
                        let change = match edit {
                            TableEdit::InsertColumnBefore => replace(
                                cell.start(),
                                cell.start(),
                                if row_index == 1 { "--- | " } else { " | " }.to_owned(),
                            ),
                            TableEdit::InsertColumnAfter => replace(
                                cell.end(),
                                cell.end(),
                                if row_index == 1 { " | ---" } else { " | " }.to_owned(),
                            ),
                            _ if current.column() + 1 < row.len() => replace(
                                cell.start(),
                                row[current.column() + 1].start(),
                                String::new(),
                            ),
                            _ => {
                                replace(row[current.column() - 1].end(), cell.end(), String::new())
                            }
                        }?;
                        plan.edits.push(change);
                    }
                    let column = match edit {
                        TableEdit::InsertColumnAfter => current.column() + 1,
                        TableEdit::DeleteColumn => current.column().min(table.column_count() - 2),
                        _ => current.column(),
                    };
                    plan.target = TableCellAddress::new(current.row(), column);
                }
            }
            TableEdit::DeleteRow => {
                if current.row() == 0 {
                    return None;
                }
                let row = table.row_source_range(current.row() + 1)?;
                plan.edits
                    .push(replace(row.start(), row.end(), String::new())?);
                plan.target = TableCellAddress::new(
                    current.row().min(table.visible_row_count() - 2),
                    current.column(),
                );
            }
            TableEdit::InsertRowBefore | TableEdit::InsertRowAfter => {
                if current.row() == 0 && edit == TableEdit::InsertRowBefore {
                    return None;
                }
                // Always use a continuation prefix, not a list's first marker.
                let continuation = table.row_ranges().get(1)?;
                let first = table.delimiter().first()?;
                let prefix = source
                    .get(continuation.start()..first.start())?
                    .split('|')
                    .next()?;
                let end = table.source_range().end();
                let eol = if source
                    .get(table.source_range().start()..end)?
                    .contains("\r\n")
                {
                    "\r\n"
                } else {
                    "\n"
                };
                let physical = if edit == TableEdit::InsertRowBefore {
                    current.row() + 1
                } else {
                    current.row() + 2
                };
                let at = table
                    .row_ranges()
                    .get(physical)
                    .map_or(end, |row| row.start());
                let has_ending = source.get(..at)?.ends_with('\n');
                let mut text = String::new();
                if at == end && !has_ending {
                    text.push_str(eol);
                }
                text.push_str(prefix);
                text.push('|');
                for _ in 0..table.column_count() {
                    text.push_str("  |");
                }
                if at != end || has_ending {
                    text.push_str(eol);
                }
                plan.edits.push(replace(at, at, text)?);
                plan.target = TableCellAddress::new(physical - 1, current.column());
            }
        }
        Some(plan)
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn clear_table_retains_sparse_slots_and_noncorner_primary_through_history() {
        let source = "<table><tr><td>中文🙂</td><td>B</td></tr><tr><td>C</td></tr></table>\r\n";
        let expected = "<table><tr><td></td><td></td></tr><tr><td></td></tr></table>\r\n";
        for primary in 0..3 {
            let mut document = EditorDocument::new(source);
            let ranges = ["中文🙂", "B", "C"].map(|text| {
                let start = source.find(text).expect("content");
                EditorSelection::range(
                    &document.snapshot(),
                    ByteOffset::new(start as u64),
                    ByteOffset::new((start + text.len()) as u64),
                    crate::CaretAffinity::Downstream,
                )
                .expect("range")
            });
            document.set_selections(ranges, primary).expect("select");
            let slots = vec![Some(0), Some(1), Some(2), None];
            document.presentation.selections = document
                .presentation
                .selections
                .clone()
                .with_table_slots(2, slots.clone())
                .expect("sparse selection");
            let original = document.selections().clone();
            document.clear_table_cells().expect("clear");
            assert_eq!(document.snapshot().as_str(), expected);
            assert_eq!(document.selections().primary_index(), primary);
            assert_eq!(document.selections().table_slots(), Some(slots.as_slice()));
            assert_eq!(document.selections().as_slice().len(), 3);
            assert!(
                document
                    .selections()
                    .as_slice()
                    .iter()
                    .all(|selection| selection.is_empty())
            );
            document.undo().expect("undo");
            assert_eq!(document.snapshot().as_str(), source);
            assert_eq!(
                document.selections().primary_index(),
                original.primary_index()
            );
            assert_eq!(document.selections().table_slots(), original.table_slots());
            for (actual, expected) in document
                .selections()
                .as_slice()
                .iter()
                .zip(original.as_slice())
            {
                assert_eq!(actual.anchor(), expected.anchor());
                assert_eq!(actual.focus(), expected.focus());
            }
            document.redo().expect("redo");
            assert_eq!(document.snapshot().as_str(), expected);
            assert_eq!(document.selections().primary_index(), primary);
            assert_eq!(document.selections().table_slots(), Some(slots.as_slice()));
        }
    }

    #[test]
    fn merged_rectangle_expands_transitively_and_keeps_unique_ranges() {
        let source = "<table><tr><td rowspan='2'>中文🙂</td><td colspan='2'>B</td></tr><tr><td>C</td><td rowspan='2'>D</td></tr><tr><td>E</td><td>F</td></tr></table>\r\n";
        let document = EditorDocument::new(source);
        let index = document.markdown.html_regions();
        let model = index.regions[0].model.as_ref().expect("model");
        let table = model
            .table(model.partitions[0].content.owner.expect("owner"))
            .expect("table");
        let grid = yu_markdown::TableBlock::from_html(&table).expect("adapter");
        let covered = TableCellAddress::new(1, 0);
        let other = TableCellAddress::new(1, 1);
        assert_eq!(
            grid.selection_bounds(covered, other),
            Some((TableCellAddress::new(0, 0), TableCellAddress::new(2, 2)))
        );
        for (start, end, primary) in [(covered, other, 2), (other, covered, 0)] {
            let selected = document
                .table_cell_selection_in_grid(&grid, start, end)
                .expect("selection");
            assert_eq!(selected.columns, 3);
            assert_eq!(selected.primary, primary);
            assert_eq!(selected.ranges.len(), 6);
            assert_eq!(
                selected.slots,
                vec![
                    Some(0),
                    Some(1),
                    Some(1),
                    Some(0),
                    Some(2),
                    Some(3),
                    Some(4),
                    Some(5),
                    Some(3)
                ]
            );
            let state = Selections::new(&document.snapshot(), selected.ranges, selected.primary)
                .expect("unique ranges")
                .with_table_slots(selected.columns, selected.slots)
                .expect("rectangular owners");
            assert_eq!(state.primary_index(), primary);
        }
        assert_eq!(document.snapshot().as_str(), source);
    }

    use super::*;

    fn focus(document: &mut EditorDocument, text: &str) {
        let snapshot = document.snapshot();
        let at = snapshot.as_str().find(text).expect("cell");
        document
            .set_selection(
                EditorSelection::cursor(
                    &snapshot,
                    ByteOffset::new(at as u64),
                    crate::CaretAffinity::Downstream,
                )
                .expect("caret"),
            )
            .expect("selection");
    }

    fn table(document: &EditorDocument) -> yu_markdown::TableBlock {
        document
            .markdown()
            .blocks()
            .iter()
            .find_map(|block| yu_markdown::table_for_block(document.markdown(), block))
            .expect("table remains recognized")
    }

    #[test]
    fn empty_cells_keep_distinct_caret_and_pointer_geometry() {
        for prefix in ["", "> ", "> > "] {
            let source = format!(
                "{prefix}| A | B | C |\n{prefix}| --- | --- | --- |\n{prefix}|  |  |  |\n{prefix}|  |  |  |\n"
            );
            let mut document = EditorDocument::new(source);
            for zoom in [0.75, 1.0, 1.5] {
                let layout = document
                    .block_layout(0, LayoutConfig::new(400.0 * zoom, 16.0 * zoom))
                    .expect("table layout");
                let empty: Vec<_> = layout
                    .table()
                    .expect("table")
                    .cells()
                    .iter()
                    .filter(|cell| cell.source().is_empty())
                    .collect();
                assert_eq!(empty.len(), 6);
                for cell in empty {
                    for bias in [Bias::Before, Bias::After] {
                        let caret = layout
                            .caret_for_source(cell.source().start(), bias)
                            .expect("caret");
                        assert_eq!(caret.source(), cell.source().start());
                        assert!(cell.bounds().contains(caret.point()));
                        let hit = layout
                            .hit_test(LayoutPoint::new(
                                caret.point().x() + 1.0,
                                caret.point().y() + 1.0,
                            ))
                            .expect("hit");
                        assert_eq!(
                            hit.source(),
                            cell.source().start(),
                            "{prefix:?} row={} column={}",
                            cell.row(),
                            cell.column()
                        );
                        assert_eq!(hit.line(), cell.row());
                    }
                }
            }
        }
    }

    #[test]
    fn rectangular_cells_include_empty_cells_and_restore_after_deletion() {
        for prefix in ["", "> ", "> > "] {
            let source = format!(
                "{prefix}| A | B | C |\n{prefix}| --- | --- | --- |\n{prefix}| a |  | 中文 |\n{prefix}| b | x | 🪶 |\n"
            );
            for reversed in [false, true] {
                for deletion in [EditorCommand::DeleteBackward, EditorCommand::DeleteForward] {
                    let mut document = EditorDocument::new(source.clone());
                    let a = ByteOffset::new(source.find('B').expect("header") as u64);
                    let f = ByteOffset::new(source.find('🪶').expect("body") as u64);
                    let (anchor, focus) = if reversed { (f, a) } else { (a, f) };
                    document
                        .execute(EditorCommand::SelectTableCells { anchor, focus })
                        .expect("rectangle");
                    assert_eq!(document.selections().table_columns(), Some(2));
                    assert_eq!(document.selections().len(), 6);
                    assert_eq!(
                        document.selections().primary_index(),
                        if reversed { 0 } else { 5 }
                    );
                    assert_eq!(
                        document
                            .selections()
                            .as_slice()
                            .iter()
                            .filter(|s| s.is_empty())
                            .count(),
                        1
                    );
                    document.execute(deletion).expect("clear selected cells");
                    let expected = source
                        .replace(['B', 'C'], "")
                        .replace("中文", "")
                        .replace(['x', '🪶'], "");
                    assert_eq!(document.snapshot().as_str(), expected);
                    assert_eq!(table(&document).column_count(), 3);
                    assert_eq!(document.selections().table_columns(), Some(2));
                    assert_eq!(
                        document.selections().primary_index(),
                        if reversed { 0 } else { 5 }
                    );
                    assert!(document.selections().as_slice().iter().all(|s| {
                        table(&document)
                            .visible_cell_for_source(s.focus().get() as usize)
                            .is_some()
                    }));
                    document.execute(EditorCommand::undo()).expect("undo");
                    assert_eq!(document.snapshot().as_str(), source);
                    assert_eq!(document.selections().table_columns(), Some(2));
                    assert_eq!(document.selections().len(), 6);
                    document.execute(EditorCommand::redo()).expect("redo");
                    assert_eq!(document.snapshot().as_str(), expected);
                }
            }
        }
    }

    #[test]
    fn rectangular_selection_does_not_reveal_inline_source_delimiters() {
        let source = "| **A** | B |\n| --- | --- |\n| `code` | C |\n";
        let mut document = EditorDocument::new(source);
        document
            .execute(EditorCommand::SelectTableCells {
                anchor: ByteOffset::new(source.find("code").expect("code") as u64),
                focus: ByteOffset::new(source.find('A').expect("header") as u64),
            })
            .expect("reverse rectangle");
        assert_eq!(document.selections().table_columns(), Some(1));
        assert!(document.selection_reveal_block_index().is_none());
        let visual = document.visual_text_for_visual_state().expect("projection");
        assert!(!visual.text().contains("**") && !visual.text().contains('`'));
        assert_eq!(document.snapshot().as_str(), source);
    }

    #[test]
    fn rectangular_selection_rejects_cross_table_endpoints_without_changes() {
        let source = "| A | B |\n| --- | --- |\n\noutside\n";
        let mut document = EditorDocument::new(source);
        let before = document.selections().clone();
        let command = EditorCommand::SelectTableCells {
            anchor: ByteOffset::new(2),
            focus: ByteOffset::new(source.find("outside").expect("outside") as u64),
        };
        assert!(!document.command_available(&command));
        assert!(document.execute(command).is_err());
        assert_eq!(document.selections(), &before);
        assert_eq!(document.snapshot().as_str(), source);
    }

    #[test]
    fn table_column_commands_preserve_other_bytes_and_restore_history() {
        for (first, rest) in [
            ("", ""),
            ("> ", "> "),
            ("> > ", "> > "),
            ("- ", "  "),
            ("> - ", ">   "),
        ] {
            for eol in ["\n", "\r\n"] {
                let source = format!(
                    "{first}| A | B | C |{eol}{rest}| :--- | ---: | :---: |{eol}{rest}| left\\|pipe | middle | 中文🪶 |{eol}"
                );
                for (edit, header, delimiter, body, columns) in [
                    (
                        TableEdit::InsertColumnBefore,
                        "| A |  | B | C |",
                        "| :--- | --- | ---: | :---: |",
                        "| left\\|pipe |  | middle | 中文🪶 |",
                        4,
                    ),
                    (
                        TableEdit::InsertColumnAfter,
                        "| A | B |  | C |",
                        "| :--- | ---: | --- | :---: |",
                        "| left\\|pipe | middle |  | 中文🪶 |",
                        4,
                    ),
                    (
                        TableEdit::DeleteColumn,
                        "| A | C |",
                        "| :--- | :---: |",
                        "| left\\|pipe | 中文🪶 |",
                        2,
                    ),
                    (
                        TableEdit::AlignLeft,
                        "| A | B | C |",
                        "| :--- | :--- | :---: |",
                        "| left\\|pipe | middle | 中文🪶 |",
                        3,
                    ),
                    (
                        TableEdit::AlignCenter,
                        "| A | B | C |",
                        "| :--- | :---: | :---: |",
                        "| left\\|pipe | middle | 中文🪶 |",
                        3,
                    ),
                    (
                        TableEdit::AlignDefault,
                        "| A | B | C |",
                        "| :--- | --- | :---: |",
                        "| left\\|pipe | middle | 中文🪶 |",
                        3,
                    ),
                ] {
                    let mut document = EditorDocument::new(source.clone());
                    focus(&mut document, "middle");
                    let before = document.selection().focus();
                    assert!(document.command_available(&EditorCommand::EditTable(edit)));
                    document
                        .execute(EditorCommand::EditTable(edit))
                        .expect("edit");
                    let expected =
                        format!("{first}{header}{eol}{rest}{delimiter}{eol}{rest}{body}{eol}");
                    assert_eq!(document.snapshot().as_str(), expected, "{first:?} {edit:?}");
                    let parsed = table(&document);
                    assert_eq!(parsed.column_count(), columns);
                    assert!(
                        parsed
                            .visible_cell_for_source(document.selection().focus().get() as usize)
                            .is_some()
                    );
                    document.execute(EditorCommand::undo()).expect("undo");
                    assert_eq!(document.snapshot().as_str(), source);
                    assert_eq!(document.selection().focus(), before);
                    document.execute(EditorCommand::redo()).expect("redo");
                    assert_eq!(document.snapshot().as_str(), expected);
                }
            }
        }
    }

    #[test]
    fn table_row_commands_preserve_containers_line_endings_and_new_cell_focus() {
        for (first, rest) in [
            ("", ""),
            ("> ", "> "),
            ("> > ", "> > "),
            ("- ", "  "),
            ("> - ", ">   "),
        ] {
            for eol in ["\n", "\r\n"] {
                for ending in ["", eol] {
                    let source = format!(
                        "{first}| A | B |{eol}{rest}| --- | --- |{eol}{rest}| x | y |{ending}"
                    );
                    for edit in [
                        TableEdit::InsertRowBefore,
                        TableEdit::InsertRowAfter,
                        TableEdit::DeleteRow,
                    ] {
                        let mut document = EditorDocument::new(source.clone());
                        focus(&mut document, "y");
                        document
                            .execute(EditorCommand::EditTable(edit))
                            .expect("edit");
                        let header = format!("{first}| A | B |{eol}{rest}| --- | --- |{eol}");
                        let expected = match edit {
                            TableEdit::InsertRowBefore => {
                                format!("{header}{rest}|  |  |{eol}{rest}| x | y |{ending}")
                            }
                            TableEdit::InsertRowAfter => {
                                format!("{header}{rest}| x | y |{eol}{rest}|  |  |{ending}")
                            }
                            _ => header,
                        };
                        assert_eq!(document.snapshot().as_str(), expected, "{first:?} {edit:?}");
                        let parsed = table(&document);
                        assert_eq!(
                            parsed.body_row_count(),
                            if edit == TableEdit::DeleteRow { 0 } else { 2 }
                        );
                        let address = parsed
                            .visible_cell_for_source(document.selection().focus().get() as usize)
                            .expect("caret inside table");
                        assert_eq!(address.column(), 1);
                        document
                            .execute(EditorCommand::insert_text("新"))
                            .expect("input follows new caret");
                        document
                            .execute(EditorCommand::undo())
                            .expect("undo input separately");
                        assert_eq!(document.snapshot().as_str(), expected);
                        document
                            .execute(EditorCommand::undo())
                            .expect("undo structural command");
                        assert_eq!(document.snapshot().as_str(), source);
                    }
                }
            }
        }
    }

    #[test]
    fn deleting_edge_columns_keeps_implicit_outer_pipe_tables_recognizable() {
        for prefix in ["", "> "] {
            let source = format!("{prefix}A | B\n{prefix}:--- | ---:\n{prefix}`x|y` | 中文\n");
            for (focus_text, header, delimiter, body) in [
                ("中文", "A", ":---", "`x|y`"),
                ("`x|y`", "B", "---:", "中文"),
            ] {
                let mut document = EditorDocument::new(source.clone());
                focus(&mut document, focus_text);
                document
                    .execute(EditorCommand::EditTable(TableEdit::DeleteColumn))
                    .expect("delete edge");
                assert_eq!(
                    document.snapshot().as_str(),
                    format!("{prefix}| {header} |\n{prefix}| {delimiter} |\n{prefix}| {body} |\n")
                );
                assert_eq!(table(&document).column_count(), 1);
                document.execute(EditorCommand::undo()).expect("undo");
                assert_eq!(document.snapshot().as_str(), source);
            }
        }
    }

    #[test]
    fn table_single_column_and_header_guards_remain_editable() {
        let source = "| A | B |\n| --- | --- |\n| `a|b` | 中文 |\n";
        let mut document = EditorDocument::new(source);
        focus(&mut document, "中文");
        document
            .execute(EditorCommand::EditTable(TableEdit::DeleteColumn))
            .expect("delete second column");
        assert_eq!(document.snapshot().as_str(), "| A |\n| --- |\n| `a|b` |\n");
        assert_eq!(table(&document).column_count(), 1);
        let layout = document
            .block_layout(0, LayoutConfig::new(320.0, 16.0))
            .expect("single column layout");
        assert_eq!(
            layout
                .table()
                .expect("single column presentation")
                .column_widths()
                .len(),
            1
        );
        focus(&mut document, "A");
        for edit in [TableEdit::DeleteRow, TableEdit::InsertRowBefore] {
            assert!(!document.command_available(&EditorCommand::EditTable(edit)));
            assert!(
                !document
                    .execute(EditorCommand::EditTable(edit))
                    .expect("header protected")
                    .changed()
            );
        }
        document
            .execute(EditorCommand::EditTable(TableEdit::InsertRowAfter))
            .expect("insert below header");
        assert_eq!(
            document.snapshot().as_str(),
            "| A |\n| --- |\n|  |\n| `a|b` |\n"
        );
        document
            .execute(EditorCommand::EditTable(TableEdit::DeleteColumn))
            .expect("delete final column");
        assert_eq!(document.snapshot().as_str(), "");
        document
            .execute(EditorCommand::undo())
            .expect("restore table");
        assert_eq!(table(&document).column_count(), 1);
    }
}
