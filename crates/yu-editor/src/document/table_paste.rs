use super::*;
use yu_markdown::TableCellAddress;

pub(super) struct TableGridPlan {
    pub(super) range: TextRange,
    pub(super) source: String,
    pub(super) edits: Vec<yu_text::Edit>,
    pub(super) anchor: usize,
    pub(super) focus: usize,
}

impl EditorDocument {
    /// Recognize a complete clipboard table using the same source parser as
    /// layout. Ordinary multiline prose must never be inferred to be a grid.
    pub(super) fn markdown_table_grid(source: &str) -> Option<(usize, Vec<Arc<str>>)> {
        let source = source.trim_matches(['\r', '\n']);
        let table = yu_markdown::parse_table(source)?;
        let cells = std::iter::once(table.first_row())
            .chain(table.rows().iter().map(Vec::as_slice))
            .flat_map(|row| row.iter())
            .map(|cell| Arc::from(&source[cell.start()..cell.end()]))
            .collect();
        Some((table.column_count(), cells))
    }

    pub(super) fn grid_paste_is_outside_table(&self) -> bool {
        self.presentation.selections.table_columns().is_none()
            && self
                .presentation
                .selections
                .as_slice()
                .iter()
                .all(|selection| {
                    self.block_index_for_offset(selection.ordered_range().start())
                        .and_then(|index| self.presentation.markdown.blocks().get(index))
                        .and_then(|block| {
                            self.html_table_grid(block).or_else(|| {
                                yu_markdown::table_for_block(&self.presentation.markdown, block)
                            })
                        })
                        .is_none()
                })
    }

    pub(super) fn serialize_grid(
        columns: usize,
        cells: &[Arc<str>],
    ) -> Result<String, EditorDocumentError> {
        if columns == 0
            || cells.is_empty()
            || !cells.len().is_multiple_of(columns)
            || cells.iter().any(|cell| cell.contains(['\n', '\r']))
        {
            return Err(EditorDocumentError::InvalidTablePaste);
        }
        let mut source = String::new();
        for (index, row) in cells.chunks(columns).enumerate() {
            if index > 0 {
                source.push('\n');
            }
            source.push('|');
            for cell in row {
                source.push(' ');
                source.push_str(
                    &yu_markdown::quote_table_cell_input("", 0..0, cell, false)
                        .ok_or(EditorDocumentError::InvalidTablePaste)?,
                );
                source.push_str(" |");
            }
            if index == 0 {
                source.push_str("\n|");
                for _ in 0..columns {
                    source.push_str(" --- |");
                }
            }
        }
        Ok(source)
    }

    pub(super) fn table_grid_plan(
        &self,
        columns: usize,
        cells: &[Arc<str>],
    ) -> Result<TableGridPlan, EditorDocumentError> {
        let invalid = || EditorDocumentError::InvalidTablePaste;
        if columns == 0 || cells.is_empty() || !cells.len().is_multiple_of(columns) {
            return Err(invalid());
        }
        let selections = &self.presentation.selections;
        if selections.is_multiple() && selections.table_columns().is_none() {
            return Err(invalid());
        }
        let first = selections.as_slice()[0].ordered_range();
        let block = self
            .presentation
            .markdown
            .blocks()
            .get(
                self.block_index_for_offset(first.start())
                    .ok_or_else(invalid)?,
            )
            .ok_or_else(invalid)?;
        if self.html_table_grid(block).is_some() {
            return self.html_table_grid_plan(block, columns, cells, false);
        }
        // Physical newlines cannot be inserted into a Markdown pipe-table cell.
        // Reject before mutation; the plain-text importer must encode line breaks.
        if cells.iter().any(|cell| cell.contains(['\n', '\r'])) {
            return Err(invalid());
        }
        let table =
            yu_markdown::table_for_block(&self.presentation.markdown, block).ok_or_else(invalid)?;
        let start = table
            .visible_cell_for_source(first.start().get() as usize)
            .ok_or_else(invalid)?;
        let owner = table.visible_cell(start).ok_or_else(invalid)?;
        if first.end().get() as usize > owner.end() {
            return Err(invalid());
        }
        let fill = cells.len() == 1 && selections.table_columns().is_some();
        let width = if fill {
            selections.table_columns().unwrap_or(1)
        } else {
            columns
        };
        let height = if fill {
            selections.len() / width
        } else {
            cells.len() / columns
        };
        let end_column = start.column().checked_add(width).ok_or_else(invalid)?;
        let end_row = start.row().checked_add(height).ok_or_else(invalid)?;
        let column_count = table.column_count().max(end_column);
        let row_count = table.visible_row_count().max(end_row);
        let snapshot = self.snapshot();
        let source = snapshot.as_str();
        let table_start = table.source_range().start();
        let table_end = table.source_range().end();
        let mut patched = source[table_start..table_end].to_owned();
        let payload = |row: usize, column: usize| -> Option<&str> {
            if row < start.row()
                || row >= end_row
                || column < start.column()
                || column >= end_column
            {
                return None;
            }
            Some(
                &cells[if fill {
                    0
                } else {
                    (row - start.row()) * columns + column - start.column()
                }],
            )
        };
        let encode = |text: &str, separator: bool| {
            yu_markdown::quote_table_cell_input("", 0..0, text, separator).ok_or_else(invalid)
        };
        let mut patches = Vec::new();
        for (physical_index, row) in std::iter::once(table.first_row())
            .chain(std::iter::once(table.delimiter()))
            .chain(table.rows().iter().map(Vec::as_slice))
            .enumerate()
        {
            let delimiter = physical_index == 1;
            let visible_row = if physical_index == 0 {
                0
            } else {
                physical_index - 1
            };
            let physical = table.row_ranges()[physical_index];
            let last = row.last().ok_or_else(invalid)?;
            let grows = column_count > table.column_count();
            for (column, cell) in row.iter().enumerate() {
                let input = if delimiter {
                    None
                } else {
                    payload(visible_row, column)
                };
                let last_cell = column + 1 == row.len();
                if input.is_none() && !(last_cell && grows) {
                    continue;
                }
                let mut replacement = match input {
                    Some(text) => encode(
                        text,
                        source.as_bytes().get(cell.end()) == Some(&b'|') && !(last_cell && grows),
                    )?,
                    None => source[cell.start()..cell.end()].to_owned(),
                };
                if last_cell && grows {
                    for new_column in table.column_count()..column_count {
                        replacement.push_str(" | ");
                        replacement.push_str(&encode(
                            if delimiter {
                                "---"
                            } else {
                                payload(visible_row, new_column).unwrap_or("")
                            },
                            false,
                        )?);
                    }
                    // Explicitly terminate rows which formerly omitted outer pipes;
                    // otherwise an empty appended cell vanishes during parsing.
                    if !source[last.end()..physical.end()].contains('|') {
                        replacement.push_str(" |");
                    }
                }
                patches.push((
                    cell.start() - table_start,
                    cell.end() - table_start,
                    replacement,
                ));
            }
        }
        patches.sort_by_key(|patch| patch.0);
        // Preserve transaction identity for untouched delimiter/header bytes.
        // A whole-table replacement falsely deletes anchors during cell edits.
        let mut edits = patches
            .iter()
            .filter(|(start, end, replacement)| {
                source[table_start + start..table_start + end] != *replacement
            })
            .map(|(start, end, replacement)| {
                yu_text::Edit::new(
                    TextRange::new(
                        ByteOffset::new((table_start + start) as u64),
                        ByteOffset::new((table_start + end) as u64),
                    )
                    .expect("ordered table patch"),
                    replacement.clone(),
                )
            })
            .collect::<Vec<_>>();
        for (start, end, replacement) in patches.into_iter().rev() {
            patched.replace_range(start..end, &replacement);
        }
        let existing_rows_len = patched.len();
        if row_count > table.visible_row_count() {
            let continuation = table.row_ranges()[1];
            let prefix = source[continuation.start()..table.delimiter()[0].start()]
                .split('|')
                .next()
                .ok_or_else(invalid)?;
            let eol = if source[table_start..table_end].contains("\r\n") {
                "\r\n"
            } else {
                "\n"
            };
            let terminal_newline = patched.ends_with('\n');
            for row in table.visible_row_count()..row_count {
                if !patched.ends_with('\n') {
                    patched.push_str(eol);
                }
                patched.push_str(prefix);
                patched.push('|');
                for column in 0..column_count {
                    patched.push(' ');
                    patched.push_str(&encode(payload(row, column).unwrap_or(""), false)?);
                    patched.push_str(" |");
                }
                if terminal_newline {
                    patched.push_str(eol);
                }
            }
        }
        if patched.len() > existing_rows_len {
            edits.push(yu_text::Edit::new(
                TextRange::empty(ByteOffset::new(table_end as u64)),
                patched[existing_rows_len..].to_owned(),
            ));
        }
        // Parse only the proposed table before committing. This catches malformed
        // inline source without constructing a second editor or changing history.
        let scratch = TextBuffer::new(patched.clone());
        let parsed = yu_markdown::parse(&scratch.snapshot());
        let result = parsed
            .blocks()
            .iter()
            .find_map(|block| yu_markdown::table_for_block(&parsed, block))
            .ok_or_else(invalid)?;
        if result.column_count() != column_count || result.visible_row_count() != row_count {
            return Err(invalid());
        }
        let anchor = result.visible_cell(start).ok_or_else(invalid)?.start() + table_start;
        let focus = result
            .visible_cell(TableCellAddress::new(end_row - 1, end_column - 1))
            .ok_or_else(invalid)?
            .start()
            + table_start;
        Ok(TableGridPlan {
            range: TextRange::new(
                ByteOffset::new(table_start as u64),
                ByteOffset::new(table_end as u64),
            )
            .ok_or_else(invalid)?,
            source: patched,
            edits,
            anchor,
            focus,
        })
    }

    pub(super) fn paste_table_grid(
        &mut self,
        columns: usize,
        cells: Vec<Arc<str>>,
    ) -> Result<CommandResult, EditorDocumentError> {
        if self.grid_paste_is_outside_table() {
            return self.paste_fragments(vec![Self::serialize_grid(columns, &cells)?.into()]);
        }
        let plan = self.table_grid_plan(columns, &cells)?;
        self.apply_table_grid_plan(plan)
    }

    pub(super) fn apply_table_grid_plan(
        &mut self,
        plan: TableGridPlan,
    ) -> Result<CommandResult, EditorDocumentError> {
        if self.snapshot().as_str()
            [plan.range.start().get() as usize..plan.range.end().get() as usize]
            == plan.source
        {
            return Ok(self.command_result(false));
        }
        self.state.history.break_group();
        self.apply_transaction_with_group(
            &Transaction::new(self.revision(), plan.edits),
            HistoryGroup::TableEditing,
        )?;
        self.select_table_cells(
            ByteOffset::new(plan.anchor as u64),
            ByteOffset::new(plan.focus as u64),
        )?;
        self.state
            .history
            .finish_selection(self.presentation.selections.clone());
        self.state.history.break_group();
        Ok(self.command_result(true))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn select(document: &mut EditorDocument, first: &str, last: &str) {
        let source = document.snapshot();
        document
            .select_table_cells(
                ByteOffset::new(
                    source
                        .as_str()
                        .find(first)
                        .expect("valid table fixture operation") as u64,
                ),
                ByteOffset::new(
                    source
                        .as_str()
                        .find(last)
                        .expect("valid table fixture operation") as u64,
                ),
            )
            .expect("valid table fixture operation");
    }

    fn paste(document: &mut EditorDocument, columns: usize, cells: &[&str]) {
        document
            .execute(EditorCommand::PasteTableGrid {
                columns,
                cells: cells.iter().copied().map(Arc::from).collect(),
            })
            .expect("valid table fixture operation");
    }

    #[test]
    fn grid_paste_grows_rows_columns_preserving_source_and_undo() {
        for prefix in ["", "> ", "> > "] {
            for eol in ["\n", "\r\n"] {
                for terminal in [false, true] {
                    let source = format!(
                        "{prefix}| A | B |{eol}{prefix}| :---- | ----: |{eol}{prefix}| x | y |{}",
                        if terminal { eol } else { "" }
                    );
                    let mut document = EditorDocument::new(source.clone());
                    select(&mut document, "y", "y");
                    let before = document.selections().clone();
                    paste(&mut document, 2, &["中文|羽", "**bold**", "", "🪶"]);
                    let expected = format!(
                        "{prefix}| A | B |  |{eol}{prefix}| :---- | ----: | --- |{eol}{prefix}| x | 中文\\|羽 | **bold** |{eol}{prefix}|  |  | 🪶 |{}",
                        if terminal { eol } else { "" }
                    );
                    assert_eq!(document.snapshot().as_str(), expected);
                    assert_eq!(document.selections().table_columns(), Some(2));
                    assert_eq!(document.selections().len(), 4);
                    document
                        .execute(EditorCommand::Undo)
                        .expect("valid table fixture operation");
                    assert_eq!(document.snapshot().as_str(), source);
                    assert_eq!(
                        document.selections().table_columns(),
                        before.table_columns()
                    );
                    document
                        .execute(EditorCommand::Redo)
                        .expect("valid table fixture operation");
                    assert_eq!(document.snapshot().as_str(), expected);
                    assert_eq!(document.selections().table_columns(), Some(2));
                }
            }
        }
    }

    #[test]
    fn scalar_fills_rectangle_but_grid_uses_its_own_dimensions() {
        let source = "| A | B | C |\n| --- | --- | --- |\n| x | y | z |\n";
        let mut document = EditorDocument::new(source);
        select(&mut document, "A", "y");
        paste(&mut document, 1, &["q"]);
        assert_eq!(
            document.snapshot().as_str(),
            "| q | q | C |\n| --- | --- | --- |\n| q | q | z |\n"
        );
        document
            .execute(EditorCommand::Undo)
            .expect("valid table fixture operation");
        paste(&mut document, 1, &["one", "two", "three"]);
        assert_eq!(
            document.snapshot().as_str(),
            "| one | B | C |\n| --- | --- | --- |\n| two | y | z |\n| three |  |  |\n"
        );
        assert_eq!(document.selections().table_columns(), Some(1));
        assert_eq!(document.selections().len(), 3);
    }

    #[test]
    fn invalid_grid_never_mutates_source_selection_or_history() {
        let source = "| A | B |\n| --- | --- |\n| x | y |";
        let mut document = EditorDocument::new(source);
        select(&mut document, "A", "y");
        let revision = document.revision();
        let selections = document.selections().clone();
        for (columns, cells) in [
            (0, vec!["x"]),
            (2, vec!["x"]),
            (1, vec![]),
            (1, vec!["a\nb"]),
        ] {
            let command = EditorCommand::PasteTableGrid {
                columns,
                cells: cells.into_iter().map(Arc::from).collect(),
            };
            assert!(!document.command_available(&command));
            assert_eq!(
                document
                    .execute(command)
                    .expect_err("invalid paste rejected"),
                EditorDocumentError::InvalidTablePaste
            );
            assert_eq!(document.snapshot().as_str(), source);
            assert_eq!(document.revision(), revision);
            assert_eq!(document.selections(), &selections);
        }
        assert!(!document.command_available(&EditorCommand::Undo));
    }

    #[test]
    fn grid_preserves_list_continuation_and_compact_delimiters() {
        for source in ["A|B\n---|---\nx|y", "- |A|B|\n  |---|---|\n  |x|y|"] {
            let mut document = EditorDocument::new(source);
            select(&mut document, "y", "y");
            paste(&mut document, 2, &["q", "", "r", "s"]);
            let parsed = document
                .markdown()
                .blocks()
                .iter()
                .find_map(|b| yu_markdown::table_for_block(document.markdown(), b))
                .expect("valid table fixture operation");
            assert_eq!(parsed.column_count(), 3);
            assert_eq!(parsed.visible_row_count(), 3);
            assert_eq!(
                document
                    .snapshot()
                    .as_str()
                    .lines()
                    .filter(|line| line.starts_with("- "))
                    .count(),
                usize::from(source.starts_with("- "))
            );
            document
                .execute(EditorCommand::Undo)
                .expect("valid table fixture operation");
            assert_eq!(document.snapshot().as_str(), source);
        }
    }

    #[test]
    fn grid_from_single_caret_preserves_surroundings_and_compact_escape_boundaries() {
        let source = "before\n\n|A|B|\n|:---|---:|\n|x|y|\n\nafter\n";
        let mut document = EditorDocument::new(source);
        let snapshot = document.snapshot();
        let at = source.find('x').expect("valid table fixture operation") as u64;
        document
            .set_selection(
                EditorSelection::cursor(
                    &snapshot,
                    ByteOffset::new(at),
                    crate::CaretAffinity::Downstream,
                )
                .expect("valid table fixture operation"),
            )
            .expect("valid table fixture operation");
        paste(&mut document, 3, &["\\", "`a|b`", ""]);
        assert_eq!(
            document.snapshot().as_str(),
            "before\n\n|A|B | |\n|:---|---: | ---|\n|\\\\|`a|b` | |\n\nafter\n"
        );
        assert_eq!(document.selections().len(), 3);
        document
            .execute(EditorCommand::Undo)
            .expect("valid table fixture operation");
        assert_eq!(document.snapshot().as_str(), source);
        assert_eq!(document.selection().focus(), ByteOffset::new(at));
        assert_eq!(document.selections().table_columns(), None);
    }

    #[test]
    fn arbitrary_multicaret_and_cross_cell_ranges_are_not_grid_targets() {
        let source = "| A | B |\n| --- | --- |\n| x | y |";
        let mut document = EditorDocument::new(source);
        let snapshot = document.snapshot();
        let a = ByteOffset::new(source.find('A').expect("valid table fixture operation") as u64);
        let b = ByteOffset::new(source.find('B').expect("valid table fixture operation") as u64);
        document
            .set_selections(
                [a, b].map(|at| {
                    EditorSelection::cursor(&snapshot, at, crate::CaretAffinity::Downstream)
                        .expect("valid table fixture operation")
                }),
                0,
            )
            .expect("valid table fixture operation");
        let command = EditorCommand::PasteTableGrid {
            columns: 1,
            cells: vec!["q".into()],
        };
        assert_eq!(
            document
                .execute(command.clone())
                .expect_err("invalid paste rejected"),
            EditorDocumentError::InvalidTablePaste
        );
        document
            .set_selection(
                EditorSelection::range(&snapshot, a, b, crate::CaretAffinity::Downstream)
                    .expect("valid table fixture operation"),
            )
            .expect("valid table fixture operation");
        assert_eq!(
            document
                .execute(command)
                .expect_err("invalid paste rejected"),
            EditorDocumentError::InvalidTablePaste
        );
        assert_eq!(document.snapshot().as_str(), source);
        assert!(!document.command_available(&EditorCommand::Undo));
    }

    #[test]
    fn external_markdown_table_pastes_cells_not_delimiter_source() {
        let source = "> | H |\r\n> | :---- |\r\n> | z |";
        let mut document = EditorDocument::new(source);
        select(&mut document, "z", "z");
        let clipboard = "\n| **A** | `a|b` |\n| :---: | ---: |\n| x\\|y | 🪶 |\n\n";
        document
            .execute(EditorCommand::PasteFragments(vec![clipboard.into()]))
            .expect("external table");
        let expected = "> | H |  |\r\n> | :---- | --- |\r\n> | **A** | `a|b` |\r\n> | x\\|y | 🪶 |";
        assert_eq!(document.snapshot().as_str(), expected);
        assert_eq!(document.selections().table_columns(), Some(2));
        assert_eq!(document.selections().len(), 4);
        document.execute(EditorCommand::Undo).expect("undo");
        assert_eq!(document.snapshot().as_str(), source);
        assert_eq!(document.selections().table_columns(), Some(1));
        document.execute(EditorCommand::Redo).expect("redo");
        assert_eq!(document.snapshot().as_str(), expected);
    }

    #[test]
    fn external_table_outside_cells_keeps_original_source_and_alignment() {
        let mut document = EditorDocument::new("");
        let clipboard = "\nA | B\r\n:----- | ---:\r\nx | y\n";
        document
            .execute(EditorCommand::PasteFragments(vec![clipboard.into()]))
            .expect("ordinary paste");
        assert_eq!(document.snapshot().as_str(), clipboard);
        document.execute(EditorCommand::Undo).expect("undo");
        assert_eq!(document.snapshot().as_str(), "");
    }

    #[test]
    fn clipboard_table_recognition_requires_the_entire_payload() {
        for source in [
            "before\n\n| A | B |\n| --- | --- |",
            "| A | B |\n| --- | --- |\n\nafter",
            "x\ty\na\tb",
            "ordinary\nprose",
            "| A | B |\n| --- | --- |\n| incomplete |",
        ] {
            assert!(
                EditorDocument::markdown_table_grid(source).is_none(),
                "{source:?}"
            );
        }
        let (columns, cells) = EditorDocument::markdown_table_grid("| A |\r\n| --- |\r\n| |\r\n")
            .expect("one-column table");
        assert_eq!(columns, 1);
        assert_eq!(cells, vec![Arc::<str>::from("A"), Arc::<str>::from("")]);
    }
}
