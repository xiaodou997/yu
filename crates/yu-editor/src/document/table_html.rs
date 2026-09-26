//! HTML structural edits replace only owned tags; never serialize a GFM grid.
use super::table_edit::TableEditPlan;
use super::*;
use yu_markdown::html::HtmlNodeKind;
use yu_markdown::{Block, TableBlock, TableCellAddress};

struct HtmlParagraphJoin {
    closing: TextRange,
    opening: TextRange,
    right_closing: TextRange,
    same_tag: bool,
}

impl EditorDocument {
    /// Copy a selected HTML grid with original cell tags and normalized spans.
    pub fn copy_html_table_source(&self) -> Result<Option<String>, EditorDocumentError> {
        let selections = self.selections();
        let Some(columns) = selections.table_columns() else {
            return Ok(None);
        };
        if self.table_target_is_html() != Some(true) {
            return Ok(None);
        }
        let invalid = || EditorDocumentError::InvalidTablePaste;
        let first = selections.as_slice()[0].ordered_range().start();
        let block = self
            .block_index_for_offset(first)
            .and_then(|i| self.markdown.blocks().get(i))
            .ok_or_else(invalid)?;
        let index = self.markdown.html_regions();
        let model = index
            .region_for(block.range())
            .and_then(|r| r.model.as_ref().ok())
            .ok_or_else(invalid)?;
        let owner = index
            .partition_for(block.range())
            .and_then(|p| p.content.owner)
            .ok_or_else(invalid)?;
        let table = model.native_table(owner).map_err(|_| invalid())?;
        let cells: std::collections::HashMap<_, _> = table
            .rows
            .iter()
            .flat_map(|row| &row.cells)
            .map(|cell| ((cell.content.start(), cell.content.end()), cell))
            .collect();
        let slots = selections.table_slots().ok_or_else(invalid)?;
        let mut seen = vec![false; selections.len()];
        let source = self.markdown.source().as_str();
        let mut output = String::from("<table>");
        for (row, values) in slots.chunks(columns).enumerate() {
            output.push_str("<tr>");
            for (column, owner) in values.iter().enumerate() {
                let Some(owner) = *owner else {
                    output.push_str("<td></td>");
                    continue;
                };
                if std::mem::replace(&mut seen[owner], true) {
                    continue;
                }
                let range = selections.as_slice()[owner].ordered_range();
                let cell = cells
                    .get(&(range.start(), range.end()))
                    .ok_or_else(invalid)?;
                let HtmlNodeKind::Element {
                    opening,
                    closing: Some(closing),
                } = &model.fragment.nodes[cell.node].kind
                else {
                    return Err(invalid());
                };
                let base = opening.source.start().get() as usize;
                let mut tag = source[base..opening.source.end().get() as usize].to_owned();
                for attribute in opening
                    .attributes
                    .iter()
                    .rev()
                    .filter(|a| matches!(a.name.as_str(), "rowspan" | "colspan"))
                {
                    let mut start = attribute.source.start().get() as usize - base;
                    while start > 0 && tag.as_bytes()[start - 1].is_ascii_whitespace() {
                        start -= 1;
                    }
                    tag.replace_range(start..attribute.source.end().get() as usize - base, "");
                }
                let width = values[column..]
                    .iter()
                    .take_while(|slot| **slot == Some(owner))
                    .count();
                let height = slots[row * columns + column..]
                    .iter()
                    .step_by(columns)
                    .take_while(|slot| **slot == Some(owner))
                    .count();
                let mut spans = String::new();
                if !opening
                    .attributes
                    .iter()
                    .any(|a| matches!(a.name.as_str(), "align" | "style"))
                    && let Some(alignment) = cell.alignment
                {
                    let value = match alignment {
                        yu_markdown::html::HtmlAlignment::Left => "left",
                        yu_markdown::html::HtmlAlignment::Center => "center",
                        yu_markdown::html::HtmlAlignment::Right => "right",
                        yu_markdown::html::HtmlAlignment::Justify => "justify",
                    };
                    spans.push_str(&format!(" align=\"{value}\""));
                }
                if width > 1 {
                    spans.push_str(&format!(" colspan=\"{width}\""));
                }
                if height > 1 {
                    spans.push_str(&format!(" rowspan=\"{height}\""));
                }
                tag.insert_str(tag.len() - 1, &spans);
                output.push_str(&tag);
                output.push_str(&source[range.start().get() as usize..range.end().get() as usize]);
                output.push_str(
                    &source[closing.source.start().get() as usize
                        ..closing.source.end().get() as usize],
                );
            }
            output.push_str("</tr>");
        }
        output.push_str("</table>");
        Ok(Some(output))
    }

    pub(super) fn html_table_source_plan(
        &self,
        source: &str,
    ) -> Result<Option<super::table_paste::TableGridPlan>, EditorDocumentError> {
        let invalid = || EditorDocumentError::InvalidTablePaste;
        let parsed = yu_markdown::parse(&TextBuffer::new(source).snapshot());
        let index = parsed.html_regions();
        let model = index
            .regions
            .first()
            .and_then(|r| r.model.as_ref().ok())
            .ok_or_else(invalid)?;
        if index.regions.len() != 1 || model.partitions.len() != 1 {
            return Err(invalid());
        }
        let table = model
            .native_table(model.partitions[0].content.owner.ok_or_else(invalid)?)
            .map_err(|_| invalid())?;
        let text = source
            .get(table.source.start().get() as usize..table.source.end().get() as usize)
            .ok_or_else(invalid)?;
        if text.trim() != source.trim() {
            return Err(invalid());
        }
        if !self.markdown.html_footnote_targets_resolve(model, source) {
            return Err(invalid());
        }
        if self.grid_paste_is_outside_table() {
            return Ok(None);
        }
        let selection = self.selections();
        let first = selection.as_slice()[0].ordered_range().start();
        let block = self
            .block_index_for_offset(first)
            .and_then(|i| self.markdown.blocks().get(i))
            .ok_or_else(invalid)?;
        if self.html_table_grid(block).is_none() {
            return self.markdown_table_source_plan(block, text, table.rows.len(), table.columns);
        }
        let target = self.html_table_grid(block).ok_or_else(invalid)?;
        let originals: Vec<_> = std::iter::once(target.first_row())
            .chain(target.rows().iter().map(Vec::as_slice))
            .flatten()
            .collect();
        if originals.len() != selection.len()
            || !originals
                .iter()
                .zip(selection.as_slice())
                .all(|(cell, selected)| {
                    cell.start() as u64 == selected.ordered_range().start().get()
                        && cell.end() as u64 == selected.ordered_range().end().get()
                })
        {
            let start = target
                .visible_cell_for_source(first.get() as usize)
                .ok_or_else(invalid)?;
            if selection.as_slice().iter().any(|selected| {
                selected.ordered_range().start().get() < target.source_range().start() as u64
                    || selected.ordered_range().end().get() > target.source_range().end() as u64
            }) {
                return Err(invalid());
            }
            let target_index = self.markdown.html_regions();
            let target_model = target_index
                .region_for(block.range())
                .and_then(|r| r.model.as_ref().ok())
                .ok_or_else(invalid)?;
            let owner = target_index
                .partition_for(block.range())
                .and_then(|p| p.content.owner)
                .ok_or_else(invalid)?;
            let (range, pasted) = target_model
                .paste_table_edit(
                    self.markdown.source().as_str(),
                    owner,
                    (start.row(), start.column()),
                    text,
                )
                .map_err(|_| invalid())?;
            let parsed = yu_markdown::parse(&TextBuffer::new(&pasted).snapshot());
            let index = parsed.html_regions();
            let model = index
                .regions
                .first()
                .and_then(|r| r.model.as_ref().ok())
                .ok_or_else(invalid)?;
            let result = model
                .native_table(model.partitions[0].content.owner.ok_or_else(invalid)?)
                .map_err(|_| invalid())?;
            let grid = TableBlock::from_html(&result).ok_or_else(invalid)?;
            let base = range.start().get() as usize;
            return Ok(Some(super::table_paste::TableGridPlan {
                range,
                anchor: base + grid.visible_cell(start).ok_or_else(invalid)?.start(),
                focus: base
                    + grid
                        .visible_cell(TableCellAddress::new(
                            start.row() + table.rows.len() - 1,
                            start.column() + table.columns - 1,
                        ))
                        .ok_or_else(invalid)?
                        .start(),
                edits: vec![yu_text::Edit::new(range, pasted.clone())],
                source: pasted,
            }));
        }
        let range = TextRange::new(
            ByteOffset::new(target.source_range().start() as u64),
            ByteOffset::new(target.source_range().end() as u64),
        )
        .ok_or_else(invalid)?;
        let start = table
            .rows
            .iter()
            .flat_map(|row| &row.cells)
            .next()
            .ok_or_else(invalid)?
            .content
            .start()
            .get() as usize;
        let end = table
            .rows
            .iter()
            .flat_map(|row| &row.cells)
            .last()
            .ok_or_else(invalid)?
            .content
            .start()
            .get() as usize;
        let base = range.start().get() as usize;
        let offset = table.source.start().get() as usize;
        Ok(Some(super::table_paste::TableGridPlan {
            range,
            source: text.to_owned(),
            edits: vec![yu_text::Edit::new(range, text)],
            anchor: base + start - offset,
            focus: base + end - offset,
        }))
    }

    pub(super) fn paste_html_table_source(
        &mut self,
        source: Arc<str>,
    ) -> Result<CommandResult, EditorDocumentError> {
        match self.html_table_source_plan(&source)? {
            Some(plan) => self.apply_table_grid_plan(plan),
            None => self.paste_fragments(vec![source]),
        }
    }

    fn markdown_table_source_plan(
        &self,
        block: Block,
        payload: &str,
        rows: usize,
        columns: usize,
    ) -> Result<Option<super::table_paste::TableGridPlan>, EditorDocumentError> {
        let invalid = || EditorDocumentError::InvalidTablePaste;
        let target = yu_markdown::table_for_block(&self.markdown, block).ok_or_else(invalid)?;
        let original_range = target.source_range().start()..target.source_range().end();
        let selections = self.selections();
        if selections.is_multiple() && selections.table_columns().is_none() {
            return Err(invalid());
        }
        let first = selections.as_slice()[0].ordered_range();
        let start = target
            .visible_cell_for_source(first.start().get() as usize)
            .ok_or_else(invalid)?;
        if selections.as_slice().iter().any(|selection| {
            let range = selection.ordered_range();
            target
                .visible_cell_for_source(range.start().get() as usize)
                .and_then(|address| target.visible_cell(address))
                .is_none_or(|cell| range.end().get() as usize > cell.end())
        }) {
            return Err(invalid());
        }
        let document = self.markdown.source().as_str();
        // Reconcile each cell's formula leaves with the canonical parser before
        // accepting any conversion. TeX escapes must not be normalized away.
        let converted = yu_export::export_table_html(document, original_range.clone())
            .and_then(|html| self.markdown.preserve_table_math_source(&html, &target))
            .and_then(|html| self.markdown.preserve_table_footnote_source(&html, &target))
            .ok_or_else(invalid)?;
        let parsed = yu_markdown::parse(&TextBuffer::new(&converted).snapshot());
        let index = parsed.html_regions();
        let model = index
            .regions
            .first()
            .and_then(|region| region.model.as_ref().ok())
            .ok_or_else(invalid)?;
        let owner = model
            .partitions
            .first()
            .and_then(|partition| partition.content.owner)
            .ok_or_else(invalid)?;
        let (_, mut pasted) = model
            .paste_table_edit(&converted, owner, (start.row(), start.column()), payload)
            .map_err(|_| invalid())?;
        if document[original_range.clone()].contains("\r\n") {
            pasted = pasted.replace("\r\n", "\n").replace('\n', "\r\n");
        }
        let original = &document[original_range.clone()];
        pasted.push_str(&original[original.trim_end_matches(['\r', '\n']).len()..]);
        let parsed = yu_markdown::parse(&TextBuffer::new(&pasted).snapshot());
        let index = parsed.html_regions();
        let model = index
            .regions
            .first()
            .and_then(|region| region.model.as_ref().ok())
            .ok_or_else(invalid)?;
        let result = model
            .native_table(model.partitions[0].content.owner.ok_or_else(invalid)?)
            .map_err(|_| invalid())?;
        let grid = TableBlock::from_html(&result).ok_or_else(invalid)?;
        let range = TextRange::new(
            ByteOffset::new(original_range.start as u64),
            ByteOffset::new(original_range.end as u64),
        )
        .ok_or_else(invalid)?;
        Ok(Some(super::table_paste::TableGridPlan {
            range,
            anchor: original_range.start + grid.visible_cell(start).ok_or_else(invalid)?.start(),
            focus: original_range.start
                + grid
                    .visible_cell(TableCellAddress::new(
                        start.row() + rows - 1,
                        start.column() + columns - 1,
                    ))
                    .ok_or_else(invalid)?
                    .start(),
            edits: vec![yu_text::Edit::new(range, pasted.clone())],
            source: pasted,
        }))
    }

    /// Destination syntax for typed native clipboard payloads.
    pub fn table_target_is_html(&self) -> Option<bool> {
        let first = self
            .selections()
            .as_slice()
            .first()?
            .ordered_range()
            .start();
        let block = self
            .block_index_for_offset(first)
            .and_then(|i| self.markdown.blocks().get(i))?;
        if self.html_table_grid(block).is_some() {
            Some(true)
        } else {
            yu_markdown::table_for_block(&self.markdown, block).map(|_| false)
        }
    }

    pub fn html_grid_source(
        &self,
        columns: usize,
        cells: &[Arc<str>],
    ) -> Result<String, EditorDocumentError> {
        let invalid = || EditorDocumentError::InvalidTablePaste;
        if columns == 0 || cells.is_empty() || !cells.len().is_multiple_of(columns) {
            return Err(invalid());
        }
        let mut source = String::from("<table>");
        for row in cells.chunks(columns) {
            source.push_str("<tr>");
            for cell in row {
                source.push_str("<td>");
                source.push_str(cell);
                source.push_str("</td>");
            }
            source.push_str("</tr>");
        }
        source.push_str("</table>");
        let parsed = yu_markdown::parse(&TextBuffer::new(&source).snapshot());
        let index = parsed.html_regions();
        let model = index
            .regions
            .first()
            .ok_or_else(invalid)?
            .model
            .as_ref()
            .map_err(|_| invalid())?;
        if index.regions.len() != 1 || model.partitions.len() != 1 {
            return Err(invalid());
        }
        let table = model
            .native_table(model.partitions[0].content.owner.ok_or_else(invalid)?)
            .map_err(|_| invalid())?;
        if table.columns != columns || table.rows.len() != cells.len() / columns {
            return Err(invalid());
        }
        let contents = table.rows.iter().flat_map(|row| &row.cells).map(|cell| {
            &source[cell.content.start().get() as usize..cell.content.end().get() as usize]
        });
        if !contents.eq(cells.iter().map(AsRef::as_ref))
            || !self.markdown.html_footnote_targets_resolve(model, &source)
        {
            return Err(invalid());
        }
        Ok(source)
    }

    pub(super) fn paste_html_table_grid(
        &mut self,
        columns: usize,
        cells: Vec<Arc<str>>,
    ) -> Result<CommandResult, EditorDocumentError> {
        let source = self.html_grid_source(columns, &cells)?;
        if self.grid_paste_is_outside_table() {
            return self.paste_fragments(vec![source.into()]);
        }
        let first = self.selections().as_slice()[0].ordered_range().start();
        let block = self
            .block_index_for_offset(first)
            .and_then(|i| self.markdown.blocks().get(i))
            .ok_or(EditorDocumentError::InvalidTablePaste)?;
        if self.html_table_grid(block).is_none() {
            return Err(EditorDocumentError::InvalidTablePaste);
        }
        let plan = self.html_table_grid_plan(block, columns, &cells, true)?;
        self.apply_table_grid_plan(plan)
    }

    pub(super) fn html_table_grid(&self, block: Block) -> Option<TableBlock> {
        if self.source_mode() {
            return None;
        }
        let index = self.markdown.html_regions();
        let model = index.region_for(block.range())?.model.as_ref().ok()?;
        let owner = index.partition_for(block.range())?.content.owner?;
        TableBlock::from_html(&model.native_table(owner).ok()?)
    }

    pub(super) fn html_table_cell_input(&self, range: TextRange, text: &str) -> Option<String> {
        self.html_table_replacement_text(range, &encode_html_cell_text(text))
            .map(|(text, _)| text)
    }

    pub(super) fn html_table_decorations(&self, block: Block) -> Option<BlockDecorations> {
        self.html_table_decorations_active(block, Some(self.selection().ordered_range()))
    }

    fn html_table_decorations_active(
        &self,
        block: Block,
        active: Option<TextRange>,
    ) -> Option<BlockDecorations> {
        self.markdown
            .html_regions()
            .decorate(&self.markdown, block, active)
    }

    pub(super) fn html_table_edit_plan(
        &self,
        block: Block,
        edit: crate::TableEdit,
    ) -> Option<TableEditPlan> {
        use crate::TableEdit::*;
        if self.source_mode() {
            return None;
        }
        let index = self.markdown.html_regions();
        let model = index.region_for(block.range())?.model.as_ref().ok()?;
        let table = model
            .native_table(index.partition_for(block.range())?.content.owner?)
            .ok()?;
        let grid = TableBlock::from_html(&table)?;
        let current = grid.visible_cell_for_source(self.selection().focus().get() as usize)?;
        let owner = grid.html_cell_owner(current)?;
        let cell = table
            .rows
            .get(owner.source_row)?
            .cells
            .get(owner.source_cell)?;
        let selection = self.selection().ordered_range();
        if selection.start() < cell.content.start() || selection.end() > cell.content.end() {
            return None;
        }
        let source = self.markdown.source().as_str();
        let mut plan = TableEditPlan {
            edits: Vec::new(),
            table_start: table.source.start(),
            target: current,
        };
        let remove = |range| yu_text::Edit::new(range, "");
        match edit {
            DeleteRow if table.rows.len() == 1 => plan.edits.push(remove(table.source)),
            DeleteColumn if table.columns == 1 => plan.edits.push(remove(table.source)),
            DeleteRow => {
                plan.edits.extend(
                    model
                        .delete_row_edits(
                            source,
                            index.partition_for(block.range())?.content.owner?,
                            current.row(),
                        )
                        .ok()?
                        .into_iter()
                        .map(|(range, text)| yu_text::Edit::new(range, text)),
                );
                plan.target = TableCellAddress::new(
                    current.row().min(table.rows.len() - 2),
                    current.column(),
                );
            }
            InsertRowBefore | InsertRowAfter => {
                let before = edit == InsertRowBefore;
                plan.edits.extend(
                    model
                        .insert_row_edits(
                            source,
                            index.partition_for(block.range())?.content.owner?,
                            current.row(),
                            before,
                        )
                        .ok()?
                        .into_iter()
                        .map(|(range, text)| yu_text::Edit::new(range, text)),
                );
                plan.target =
                    TableCellAddress::new(current.row() + usize::from(!before), current.column());
            }
            InsertColumnBefore | InsertColumnAfter | DeleteColumn => {
                let column = current.column() + usize::from(edit == InsertColumnAfter);
                plan.edits.extend(
                    model
                        .column_edits(
                            source,
                            index.partition_for(block.range())?.content.owner?,
                            column,
                            edit != DeleteColumn,
                        )
                        .ok()?
                        .into_iter()
                        .map(|(range, text)| yu_text::Edit::new(range, text)),
                );
                let column = match edit {
                    DeleteColumn => current.column().min(table.columns - 2),
                    InsertColumnAfter => current.column() + 1,
                    _ => current.column(),
                };
                plan.target = TableCellAddress::new(current.row(), column);
            }
            AlignLeft | AlignCenter | AlignRight | AlignDefault => {
                let alignment = match edit {
                    AlignLeft => Some(yu_markdown::html::HtmlAlignment::Left),
                    AlignCenter => Some(yu_markdown::html::HtmlAlignment::Center),
                    AlignRight => Some(yu_markdown::html::HtmlAlignment::Right),
                    _ => None,
                };
                plan.edits.extend(
                    model
                        .align_column_edits(
                            source,
                            index.partition_for(block.range())?.content.owner?,
                            current.column(),
                            alignment,
                        )
                        .ok()?
                        .into_iter()
                        .map(|(range, text)| yu_text::Edit::new(range, text)),
                );
            }
        }
        (!plan.edits.is_empty()).then_some(plan)
    }
}

impl EditorDocument {
    pub(super) fn html_table_grid_plan(
        &self,
        block: Block,
        columns: usize,
        cells: &[Arc<str>],
        html_source: bool,
    ) -> Result<super::table_paste::TableGridPlan, EditorDocumentError> {
        let invalid = || EditorDocumentError::InvalidTablePaste;
        if columns == 0 || cells.is_empty() || !cells.len().is_multiple_of(columns) {
            return Err(invalid());
        }
        if self.source_mode() {
            return Err(invalid());
        }
        let index = self.markdown.html_regions();
        let model = index
            .region_for(block.range())
            .ok_or_else(invalid)?
            .model
            .as_ref()
            .map_err(|_| invalid())?;
        let owner = index
            .partition_for(block.range())
            .and_then(|p| p.content.owner)
            .ok_or_else(invalid)?;
        let table = model.native_table(owner).map_err(|_| invalid())?;
        let grid = TableBlock::from_html(&table).ok_or_else(invalid)?;
        let selections = &self.presentation.selections;
        let first = selections.as_slice()[0].ordered_range();
        let start = grid
            .visible_cell_for_source(first.start().get() as usize)
            .ok_or_else(invalid)?;
        if first.end().get() as usize > grid.visible_cell(start).ok_or_else(invalid)?.end() {
            return Err(invalid());
        }
        let fill = cells.len() == 1 && selections.table_columns().is_some();
        let width = if fill {
            selections.table_columns().unwrap_or(1)
        } else {
            columns
        };
        let height = if fill {
            selections.table_slots().ok_or_else(invalid)?.len() / width
        } else {
            cells.len() / columns
        };
        let end_column = start.column().checked_add(width).ok_or_else(invalid)?;
        let end_row = start.row().checked_add(height).ok_or_else(invalid)?;
        let contents: Vec<_> = cells
            .iter()
            .map(|text| {
                if html_source {
                    text.to_string()
                } else {
                    encode_html_cell_text(text)
                }
            })
            .collect();
        let (range, source) = if fill || cells.len() == 1 {
            model.fill_cells_edit(
                self.markdown.source().as_str(),
                owner,
                (start.row(), start.column(), end_row - 1, end_column - 1),
                &contents[0],
            )
        } else {
            model.paste_cells_edit(
                self.markdown.source().as_str(),
                owner,
                (start.row(), start.column()),
                columns,
                &contents,
            )
        }
        .map_err(|_| invalid())?;
        let parsed = yu_markdown::parse(&TextBuffer::new(&source).snapshot());
        let index = parsed.html_regions();
        let model = index
            .regions
            .first()
            .and_then(|r| r.model.as_ref().ok())
            .ok_or_else(invalid)?;
        let owner = model
            .partitions
            .first()
            .and_then(|p| p.content.owner)
            .ok_or_else(invalid)?;
        let result = TableBlock::from_html(&model.native_table(owner).map_err(|_| invalid())?)
            .ok_or_else(invalid)?;
        let base = range.start().get() as usize;
        Ok(super::table_paste::TableGridPlan {
            range,
            edits: vec![yu_text::Edit::new(range, source.clone())],
            source,
            anchor: base + result.visible_cell(start).ok_or_else(invalid)?.start(),
            focus: base
                + result
                    .visible_cell(TableCellAddress::new(end_row - 1, end_column - 1))
                    .ok_or_else(invalid)?
                    .start(),
        })
    }
}

fn encode_html_cell_text(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace("\r\n", "\n")
        .replace('\r', "\n")
        .replace('\n', "<br>")
}

impl EditorDocument {
    /// Delete a painted grapheme, retaining surrounding HTML structure. At a
    /// cell edge this is an empty edit; Tab owns movement between cells.
    pub(super) fn html_table_grapheme_deletion(
        &self,
        range: TextRange,
        forward: bool,
    ) -> Result<Option<TextRange>, EditorDocumentError> {
        use unicode_segmentation::UnicodeSegmentation;
        let from = if forward { range.start() } else { range.end() };
        let Some(block) = self
            .block_index_for_offset(from)
            .and_then(|i| self.markdown.blocks().get(i))
        else {
            return Ok(None);
        };
        let Some(grid) = self.html_table_grid(block) else {
            return Ok(None);
        };
        let Some(cell) = grid
            .visible_cell_for_source(from.get() as usize)
            .and_then(|a| grid.visible_cell(a))
        else {
            return Ok(None);
        };
        let Some(decorations) = self.html_table_decorations_active(block, Some(range)) else {
            return Ok(None);
        };
        let content = TextRange::new(
            ByteOffset::new(cell.start() as u64),
            ByteOffset::new(cell.end() as u64),
        )
        .expect("ordered HTML cell range");
        let visual = VisualText::new(self.markdown.source(), content, decorations.set().clone())?;
        let position = visual.source_to_visual(from, Bias::After)?.get() as usize;
        // Images and inactive formulas occupy geometry but no visual text bytes.
        // Active formulas were revealed above and use normal grapheme deletion.
        let mut adjacent_objects =
            yu_markdown::image_spans(self.markdown(), self.markdown.source(), Some(content))
                .into_iter()
                .map(|image| image.source())
                .chain(
                    decorations
                        .widgets()
                        .iter()
                        .filter_map(|widget| match widget {
                            yu_markdown::BlockWidget::Embedded(span) => Some(span.source),
                            _ => None,
                        }),
                )
                .filter(|object| object.start() >= content.start() && object.end() <= content.end())
                .filter(|object| {
                    if forward {
                        from < object.end()
                    } else {
                        from > object.start()
                    }
                })
                .filter(|object| {
                    visual
                        .source_to_visual(object.start(), Bias::After)
                        .is_ok_and(|at| at.get() as usize == position)
                })
                .collect::<Vec<_>>();
        adjacent_objects.sort_by_key(|object| object.start());
        if let Some(object) = if forward {
            adjacent_objects.first()
        } else {
            adjacent_objects.last()
        } {
            return Ok(Some(*object));
        }
        let mut graphemes = visual.text().grapheme_indices(true);
        let cluster = if forward {
            graphemes.find(|(start, text)| start + text.len() > position)
        } else {
            graphemes.rev().find(|(start, _)| *start < position)
        };
        let Some((start, text)) = cluster else {
            return Ok(Some(TextRange::empty(from)));
        };
        let mut coverage = visual.source_coverage(
            yu_core::VisualRange::new(
                yu_core::VisualOffset::new(start as u64),
                yu_core::VisualOffset::new((start + text.len()) as u64),
            )
            .expect("ordered HTML grapheme"),
        )?;
        // A collapsed tag can share a visual boundary with the first/last
        // TeX grapheme. Body editing must not delete the formula's identity.
        if let Some(span) = self
            .markdown
            .html_regions()
            .region_for(block.range())
            .and_then(|region| region.model.as_ref().ok())
            .and_then(|model| {
                model.inline_math_spans().into_iter().find(|span| {
                    span.content.start() <= from
                        && from <= span.content.end()
                        && if forward {
                            from < span.content.end()
                        } else {
                            from > span.content.start()
                        }
                })
            })
            && let Some(body) = TextRange::new(
                coverage.start().max(span.content.start()),
                coverage.end().min(span.content.end()),
            )
        {
            coverage = body;
        }
        if text == "\n"
            && let Some(join) = self.html_paragraph_join(coverage)
        {
            return Ok(TextRange::new(join.closing.start(), join.opening.end()));
        }
        Ok(Some(coverage))
    }
}

impl EditorDocument {
    /// A grapheme can cross inline tag boundaries. Preserve those tag bytes
    /// while removing text/entity spans and explicit line breaks in its range.
    pub(super) fn html_table_deletion_text(&self, range: TextRange) -> Option<Arc<str>> {
        self.html_table_replacement_text(range, "")
            .map(|(text, _)| Arc::from(text))
    }

    pub(super) fn html_table_input_cursor(
        &self,
        range: TextRange,
        replacement_len: usize,
    ) -> Option<usize> {
        let (kept, prefix) = self.html_table_replacement_text(range, "")?;
        replacement_len.checked_sub(kept.len())?.checked_add(prefix)
    }

    /// Merge boundary tags and repair surviving closing tags in one history
    /// transaction. Chained multicaret joins inherit the leftmost block type.
    pub(super) fn delete_html_paragraph_boundaries(
        &mut self,
        ranges: &[TextRange],
        group: HistoryGroup,
    ) -> Result<Option<CommandResult>, EditorDocumentError> {
        if self.source_mode() || self.selections().table_columns().is_some() {
            return Ok(None);
        }
        let mut previous_end = ByteOffset::ZERO;
        let ranges: Vec<_> = ranges
            .iter()
            .map(|range| {
                let start = range.start().max(previous_end);
                let end = range.end().max(start);
                previous_end = end;
                TextRange::new(start, end).expect("ordered deletion")
            })
            .collect();
        let source = self.snapshot();
        let mut closings = std::collections::BTreeMap::new();
        let mut found = false;
        for range in &ranges {
            if let Some(join) = self.html_paragraph_join(*range)
                && TextRange::new(join.closing.start(), join.opening.end()) == Some(*range)
            {
                found = true;
                let inherited: String = closings
                    .remove(&join.closing.start())
                    .map(|(_, text)| text)
                    .unwrap_or_else(|| {
                        source.as_str()
                            [join.closing.start().get() as usize..join.closing.end().get() as usize]
                            .to_owned()
                    });
                closings.insert(join.right_closing.start(), (join.right_closing, inherited));
            }
        }
        if !found {
            return Ok(None);
        }
        let closings: Vec<_> = closings
            .into_values()
            .filter(|(range, text)| {
                source.as_str()[range.start().get() as usize..range.end().get() as usize] != *text
            })
            .collect();
        let mut edits = Vec::new();
        for range in &ranges {
            if range.is_empty() {
                continue;
            }
            let replacement = self
                .html_table_replacement_with_closings(*range, "", &closings, true)
                .map(|(text, _)| text)
                .unwrap_or_default();
            edits.push(yu_text::Edit::new(*range, replacement));
        }
        for (range, replacement) in &closings {
            if ranges
                .iter()
                .any(|deleted| deleted.start() <= range.start() && range.end() <= deleted.end())
            {
                continue;
            }
            if ranges
                .iter()
                .any(|deleted| deleted.start() < range.end() && range.start() < deleted.end())
            {
                return Err(EditorDocumentError::Selection(SelectionError::InvalidRange));
            }
            edits.push(yu_text::Edit::new(*range, replacement.clone()));
        }
        edits.sort_by_key(|edit| edit.range().start());
        let primary = self.selections().primary_index();
        self.state.history.break_group();
        let applied =
            self.apply_transaction_with_group(&Transaction::new(self.revision(), edits), group)?;
        let selections = ranges
            .iter()
            .map(|range| {
                let anchor = yu_core::TextAnchor::new(
                    source.revision(),
                    range.start(),
                    yu_core::Affinity::Before,
                );
                let mapped = applied
                    .change_set()
                    .map_anchor(anchor)
                    .map_err(|_| EditorDocumentError::Selection(SelectionError::InvalidRange))?;
                Ok(EditorSelection::cursor(
                    applied.result_snapshot(),
                    mapped.offset(),
                    crate::CaretAffinity::Downstream,
                )?)
            })
            .collect::<Result<Vec<_>, EditorDocumentError>>()?;
        self.presentation.selections =
            Selections::new(applied.result_snapshot(), selections, primary)?;
        self.state
            .history
            .finish_selection(self.presentation.selections.clone());
        self.state.history.break_group();
        Ok(Some(self.command_result(true)))
    }

    fn html_paragraph_join(&self, range: TextRange) -> Option<HtmlParagraphJoin> {
        let block = self
            .block_index_for_offset(range.start())
            .and_then(|i| self.markdown.blocks().get(i))?;
        self.html_table_grid(block)?;
        let index = self.markdown.html_regions();
        let model = index.region_for(block.range())?.model.as_ref().ok()?;
        for parts in model.cell_partitions.values() {
            for pair in parts.windows(2) {
                let (Some(left), Some(right)) = (pair[0].content.owner, pair[1].content.owner)
                else {
                    continue;
                };
                let left = &model.fragment.nodes[left];
                let right = &model.fragment.nodes[right];
                let HtmlNodeKind::Element {
                    opening: left_open,
                    closing: Some(close),
                } = &left.kind
                else {
                    continue;
                };
                let HtmlNodeKind::Element {
                    opening: right_open,
                    closing: Some(right_close),
                } = &right.kind
                else {
                    continue;
                };
                let paragraph =
                    |name: &str| matches!(name, "p" | "h1" | "h2" | "h3" | "h4" | "h5" | "h6");
                if !paragraph(&left_open.name)
                    || !paragraph(&right_open.name)
                    || left.parent != right.parent
                    || left.source != pair[0].content.source
                    || right.source != pair[1].content.source
                {
                    continue;
                }
                if range == close.source
                    || range == right_open.source
                    || TextRange::new(close.source.start(), right_open.source.end()) == Some(range)
                {
                    return Some(HtmlParagraphJoin {
                        closing: close.source,
                        opening: right_open.source,
                        right_closing: right_close.source,
                        same_tag: left_open.name == right_open.name,
                    });
                }
            }
        }
        None
    }

    pub(super) fn html_table_replacement_text(
        &self,
        range: TextRange,
        inserted: &str,
    ) -> Option<(String, usize)> {
        self.html_table_replacement_with_closings(range, inserted, &[], false)
    }

    fn html_table_replacement_with_closings(
        &self,
        range: TextRange,
        inserted: &str,
        closings: &[(TextRange, String)],
        merge_boundary: bool,
    ) -> Option<(String, usize)> {
        let block = self
            .block_index_for_offset(range.start())
            .and_then(|i| self.markdown.blocks().get(i))?;
        let grid = self.html_table_grid(block)?;
        grid.visible_cell_for_source(range.start().get() as usize)?;
        grid.visible_cell_for_source(range.end().get() as usize)?;
        let index = self.markdown.html_regions();
        let model = index.region_for(block.range())?.model.as_ref().ok()?;
        let source = self.markdown.source().as_str();
        let whole_leaves: Vec<_> = model
            .inline_math_spans()
            .into_iter()
            .map(|span| span.source)
            .chain(model.footnote_spans().into_iter().map(|span| span.source))
            .filter(|span| range.start() <= span.start() && span.end() <= range.end())
            .collect();
        let mut removed = model
            .fragment
            .nodes
            .iter()
            .filter_map(|node| {
                // Remove a selected source leaf including its identity tags once.
                // Body-only edits retain those tags, even for an empty body.
                if whole_leaves.iter().any(|span| {
                    *span != node.source
                        && span.start() <= node.source.start()
                        && node.source.end() <= span.end()
                }) {
                    return None;
                }
                let removable = match &node.kind {
                    HtmlNodeKind::Text => true,
                    HtmlNodeKind::Element { opening, .. } => {
                        whole_leaves.contains(&node.source)
                            || opening.name == "br"
                            || (opening.name == "img"
                                && node.source.start() >= range.start()
                                && node.source.end() <= range.end())
                    }
                    HtmlNodeKind::Comment => false,
                };
                let start = node.source.start().max(range.start());
                let end = node.source.end().min(range.end());
                (removable && start < end).then_some((start, end, ""))
            })
            .collect::<Vec<_>>();
        if inserted.is_empty()
            && let Some(join) = self.html_paragraph_join(range)
            && (merge_boundary || join.same_tag)
            && TextRange::new(join.closing.start(), join.opening.end()) == Some(range)
        {
            removed.push((join.closing.start(), join.closing.end(), ""));
            removed.push((join.opening.start(), join.opening.end(), ""));
        }
        for (closing, replacement) in closings {
            if closing.start() >= range.start() && closing.end() <= range.end() {
                removed.push((closing.start(), closing.end(), replacement.as_str()));
            }
        }
        removed.sort_unstable_by_key(|(start, end, _)| (*start, *end));
        let mut kept = String::new();
        let mut cursor = range.start();
        let mut inserted_text = false;
        let mut input_end = 0;
        for (start, end, replacement) in removed {
            if start > cursor {
                kept.push_str(&source[cursor.get() as usize..start.get() as usize]);
            }
            if !inserted_text {
                kept.push_str(inserted);
                input_end = kept.len();
                inserted_text = true;
            }
            kept.push_str(replacement);
            cursor = cursor.max(end);
        }
        if !inserted_text {
            kept.push_str(inserted);
            input_end = kept.len();
        }
        kept.push_str(&source[cursor.get() as usize..range.end().get() as usize]);
        Some((kept, input_end))
    }
}
