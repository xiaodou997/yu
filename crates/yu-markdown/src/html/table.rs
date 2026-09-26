//! Source-backed HTML grid metadata. No synthetic Markdown delimiter rows.
use super::{HtmlAlignment, HtmlBlockModel, HtmlElementKind as Kind, HtmlNodeKind};
use yu_core::TextRange;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HtmlTableSection {
    Direct,
    Head,
    Body,
    Foot,
}

#[derive(Clone, Debug, PartialEq)]
pub struct HtmlTableCell {
    pub node: usize,
    pub source: TextRange,
    pub content: TextRange,
    pub header: bool,
    pub span: super::HtmlCellSpan,
    pub alignment: Option<HtmlAlignment>,
    pub paragraphs: Vec<HtmlCellParagraph>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct HtmlCellParagraph {
    pub source: TextRange,
    pub alignment: Option<HtmlAlignment>,
    pub heading: Option<u8>,
    pub disclosure_content: Option<TextRange>,
    pub list: Vec<HtmlCellListItem>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HtmlCellListItem {
    pub container: TextRange,
    pub opening: Option<TextRange>,
    pub number: Option<u64>,
    pub tight: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct HtmlTableRow {
    pub source: TextRange,
    pub section: HtmlTableSection,
    /// Source parent identifies distinct adjacent row groups of the same kind.
    pub group: usize,
    pub content_end: yu_core::ByteOffset,
    pub cells: Vec<HtmlTableCell>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct HtmlTable {
    pub source: TextRange,
    pub columns: usize,
    pub grid: super::HtmlTableGrid,
    pub rows: Vec<HtmlTableRow>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HtmlTableError {
    InvalidStructure,
    EmptyTable,
    NestedTable,
    UnsupportedCellLayout,
    /// One incoming owner would cross an existing target row-group boundary.
    CrossRowGroupSpan,
}

impl HtmlTable {
    /// Plan against the target's groups, not the clipboard's group names.
    /// Growth appends only to the final group; it never moves a boundary to
    /// make an otherwise illegal span fit. No source edits occur in this gate.
    fn validate_paste_groups(
        &self,
        incoming: &Self,
        start: (usize, usize),
    ) -> Result<(), HtmlTableError> {
        if start.0 >= self.rows.len() || start.1 >= self.columns {
            return Err(HtmlTableError::InvalidStructure);
        }
        let end_row = start
            .0
            .checked_add(incoming.rows.len())
            .ok_or(HtmlTableError::InvalidStructure)?;
        let end_column = start
            .1
            .checked_add(incoming.columns)
            .ok_or(HtmlTableError::InvalidStructure)?;
        let rows = end_row.max(self.rows.len());
        let columns = end_column.max(self.columns);
        if rows
            .checked_mul(columns)
            .is_none_or(|slots| slots > 1_048_576)
        {
            return Err(HtmlTableError::InvalidStructure);
        }
        // Record the end of each contiguous target group in linear time.
        // Direct rows separated by an explicit group are distinct runs too.
        let mut ends = vec![rows; self.rows.len()];
        let mut end = rows;
        for row in (0..self.rows.len()).rev() {
            if row + 1 < self.rows.len() && self.rows[row].group != self.rows[row + 1].group {
                end = row + 1;
            }
            ends[row] = end;
        }
        for cell in &incoming.grid.cells {
            let row = start.0 + cell.source_row;
            if row + cell.rows > ends.get(row).copied().unwrap_or(rows) {
                return Err(HtmlTableError::CrossRowGroupSpan);
            }
        }
        Ok(())
    }
}

impl HtmlBlockModel {
    fn cell_list_context(
        &self,
        part: &super::HtmlFlowPartition,
        cell: usize,
    ) -> Vec<HtmlCellListItem> {
        let mut result = Vec::new();
        let mut owner = part.content.owner;
        while let Some(id) = owner {
            if id == cell {
                break;
            }
            let node = &self.fragment.nodes[id];
            if self.resolution.elements[id]
                .as_ref()
                .is_some_and(|e| e.kind == Kind::ListItem)
                && let Some(parent) = node.parent
                && self.resolution.elements[parent].is_some()
                && let HtmlNodeKind::Element { opening, .. } = &node.kind
            {
                let Some(&(number, tight)) = self.list_items.get(&id) else {
                    break;
                };
                result.push(HtmlCellListItem {
                    container: self.fragment.nodes[parent].source,
                    opening: (part.source.start() <= opening.source.start()
                        && opening.source.end() <= part.source.end())
                    .then_some(opening.source),
                    number,
                    tight,
                });
            }
            owner = node.parent;
        }
        result.reverse();
        result
    }

    fn cell_paragraph_breaks(&self, cell: usize) -> Option<Vec<TextRange>> {
        let parts = self.cell_partitions.get(&cell)?;
        let mut breaks = Vec::new();
        for pair in parts.windows(2) {
            let left = pair[0].content.source;
            let right = pair[1].content.source;
            let tags = &self.boundary_tags;
            let ending = tags.partition_point(|(range, _)| range.end() < left.end());
            let closing = tags.get(ending).filter(|(range, closing)| {
                *closing && range.end() == left.end() && range.start() >= left.start()
            });
            let following = tags.partition_point(|(range, _)| range.start() < left.end());
            let boundary = closing
                .or_else(|| {
                    tags.get(following).filter(|(range, closing)| {
                        range.end() <= right.start() || (!closing && range.start() == right.start())
                    })
                })
                .map(|(range, _)| *range);
            let boundary = boundary?;
            breaks.push(boundary);
        }
        Some(breaks)
    }

    /// The same capability gate is used by projection and structural editing.
    /// A source-only table must never acquire hidden grid-editing behavior.
    pub fn native_table(&self, owner: usize) -> Result<HtmlTable, HtmlTableError> {
        let table = self.table(owner)?;
        // Paragraphs, headings, lists and div containers share composed paragraph geometry.
        // Richer block types remain source-backed until their cell layout exists.
        for cell in table.rows.iter().flat_map(|row| &row.cells) {
            if self.cell_paragraph_breaks(cell.node).is_none() {
                return Err(HtmlTableError::UnsupportedCellLayout);
            }
            let mut descendants = self.fragment.nodes[cell.node].children.clone();
            while let Some(id) = descendants.pop() {
                if matches!(self.fragment.nodes[id].kind, HtmlNodeKind::Element { .. })
                    && self.resolution.elements[id].is_none()
                {
                    return Err(HtmlTableError::UnsupportedCellLayout);
                }
                if self.resolution.elements[id]
                    .as_ref()
                    .is_some_and(|element| matches!(element.kind, Kind::Table))
                {
                    return Err(HtmlTableError::UnsupportedCellLayout);
                }
                descendants.extend(&self.fragment.nodes[id].children);
            }
        }
        Ok(table)
    }

    /// Grow at the right/bottom edges, preserving source row groups and spans.
    pub fn grow_table_edit(
        &self,
        source: &str,
        owner: usize,
        rows: usize,
        columns: usize,
    ) -> Result<(TextRange, String), HtmlTableError> {
        let table = self.table(owner)?;
        if rows < table.rows.len()
            || columns < table.columns
            || rows
                .checked_mul(columns)
                .is_none_or(|slots| slots > 1_048_576)
        {
            return Err(HtmlTableError::InvalidStructure);
        }
        let base = table.source.start().get() as usize;
        let mut patched = source
            .get(base..table.source.end().get() as usize)
            .ok_or(HtmlTableError::InvalidStructure)?
            .to_owned();
        let ending = if patched.contains("\r\n") {
            "\r\n"
        } else if patched.contains('\n') {
            "\n"
        } else {
            ""
        };
        let mut edits = Vec::new();
        if columns > table.columns {
            for (y, row) in table.rows.iter().enumerate() {
                let mut extra = String::new();
                for x in 0..columns {
                    if x < table.columns && table.grid.slots[y * table.columns + x].is_some() {
                        continue;
                    }
                    extra.push_str(if row.cells.last().is_some_and(|cell| cell.header) {
                        "<th></th>"
                    } else {
                        "<td></td>"
                    });
                }
                if !extra.is_empty() {
                    edits.push((TextRange::empty(row.content_end), extra));
                }
            }
        }
        let last = table.rows.last().ok_or(HtmlTableError::InvalidStructure)?;
        let group_start = table.rows.len()
            - table
                .rows
                .iter()
                .rev()
                .take_while(|row| row.group == last.group)
                .count();
        let extending: Vec<_> = table
            .grid
            .cells
            .iter()
            .filter_map(|cell| {
                let declared = table.rows[cell.source_row].cells[cell.source_cell]
                    .span
                    .rows;
                (cell.source_row >= group_start
                    && (declared == 0 || cell.source_row + declared > table.rows.len()))
                .then_some((cell, declared))
            })
            .collect();
        let mut extra = String::new();
        let mut covered = vec![false; columns];
        for y in table.rows.len()..rows {
            covered.fill(false);
            for (cell, declared) in &extending {
                if *declared == 0 || cell.source_row + *declared > y {
                    covered[cell.column..cell.column + cell.columns].fill(true);
                }
            }
            extra.push_str(ending);
            extra.push_str("<tr>");
            for (x, occupied) in covered.iter().enumerate() {
                if *occupied {
                    continue;
                }
                let header = if x < table.columns {
                    table.grid.slots[(table.rows.len() - 1) * table.columns + x]
                        .map(|id| table.grid.cells[id])
                        .is_some_and(|cell| {
                            table.rows[cell.source_row].cells[cell.source_cell].header
                        })
                } else {
                    last.cells.last().is_some_and(|cell| cell.header)
                };
                extra.push_str(if header { "<th></th>" } else { "<td></td>" });
            }
            extra.push_str("</tr>");
        }
        if !extra.is_empty() {
            edits.push((TextRange::empty(last.source.end()), extra));
        }
        edits.sort_by_key(|(range, _)| range.start());
        for (range, text) in edits.into_iter().rev() {
            patched.replace_range(
                range.start().get() as usize - base..range.end().get() as usize - base,
                &text,
            );
        }
        Ok((table.source, patched))
    }

    /// Overlay a finite HTML table while preserving its merged owner structure.
    pub fn paste_table_edit(
        &self,
        source: &str,
        owner: usize,
        start: (usize, usize),
        payload: &str,
    ) -> Result<(TextRange, String), HtmlTableError> {
        let parsed = crate::parse(&yu_text::TextBuffer::new(payload).snapshot());
        let index = parsed.html_regions();
        let model = index
            .regions
            .first()
            .and_then(|r| r.model.as_ref().ok())
            .ok_or(HtmlTableError::InvalidStructure)?;
        if index.regions.len() != 1 || model.partitions.len() != 1 {
            return Err(HtmlTableError::InvalidStructure);
        }
        let incoming = model.native_table(
            model.partitions[0]
                .content
                .owner
                .ok_or(HtmlTableError::InvalidStructure)?,
        )?;
        if payload[incoming.source.start().get() as usize..incoming.source.end().get() as usize]
            .trim()
            != payload.trim()
        {
            return Err(HtmlTableError::InvalidStructure);
        }
        // Reject boundary conflicts before even preparing a split/grown copy.
        // A multi-group rectangle is legal when each incoming owner fits.
        self.native_table(owner)?
            .validate_paste_groups(&incoming, start)?;
        let empty = vec![
            String::new();
            incoming
                .rows
                .len()
                .checked_mul(incoming.columns)
                .ok_or(HtmlTableError::InvalidStructure)?
        ];
        let (range, mut prepared) =
            self.paste_cells_edit(source, owner, start, incoming.columns, &empty)?;
        let target_parsed = crate::parse(&yu_text::TextBuffer::new(&prepared).snapshot());
        let target_index = target_parsed.html_regions();
        let target_model = target_index
            .regions
            .first()
            .and_then(|r| r.model.as_ref().ok())
            .ok_or(HtmlTableError::InvalidStructure)?;
        let target = target_model.native_table(
            target_model.partitions[0]
                .content
                .owner
                .ok_or(HtmlTableError::InvalidStructure)?,
        )?;
        let mut edits = Vec::new();
        for row in 0..incoming.rows.len() {
            for column in 0..incoming.columns {
                let id = target.grid.slots[(start.0 + row) * target.columns + start.1 + column]
                    .ok_or(HtmlTableError::InvalidStructure)?;
                let cell = target.grid.cells[id];
                let destination = &target.rows[cell.source_row].cells[cell.source_cell];
                let Some(id) = incoming.grid.slots[row * incoming.columns + column] else {
                    continue;
                };
                let cell = incoming.grid.cells[id];
                if cell.source_row != row || cell.column != column {
                    edits.push((destination.source, String::new()));
                    continue;
                }
                if target.rows[start.0 + row..start.0 + row + cell.rows]
                    .iter()
                    .any(|candidate| candidate.group != target.rows[start.0 + row].group)
                {
                    return Err(HtmlTableError::CrossRowGroupSpan);
                }
                let original = &incoming.rows[cell.source_row].cells[cell.source_cell];
                let mut text = payload
                    [original.source.start().get() as usize..original.source.end().get() as usize]
                    .to_owned();
                // A copied zero/oversized span must not reach beyond its payload.
                if original.span.rows != cell.rows {
                    let HtmlNodeKind::Element { opening, .. } =
                        &model.fragment.nodes[original.node].kind
                    else {
                        return Err(HtmlTableError::InvalidStructure);
                    };
                    let value = opening
                        .attributes
                        .iter()
                        .find(|a| a.name == "rowspan")
                        .and_then(|a| a.value)
                        .ok_or(HtmlTableError::InvalidStructure)?;
                    text.replace_range(
                        (value.start().get() - original.source.start().get()) as usize
                            ..(value.end().get() - original.source.start().get()) as usize,
                        &cell.rows.to_string(),
                    );
                }
                edits.push((destination.source, text));
            }
        }
        edits.sort_by_key(|(range, _)| range.start());
        for (range, text) in edits.into_iter().rev() {
            prepared.replace_range(
                range.start().get() as usize..range.end().get() as usize,
                &text,
            );
        }
        let verified = crate::parse(&yu_text::TextBuffer::new(&prepared).snapshot());
        let index = verified.html_regions();
        let model = index
            .regions
            .first()
            .and_then(|r| r.model.as_ref().ok())
            .ok_or(HtmlTableError::InvalidStructure)?;
        let result = model.native_table(
            model.partitions[0]
                .content
                .owner
                .ok_or(HtmlTableError::InvalidStructure)?,
        )?;
        if result.columns != target.columns || result.rows.len() != target.rows.len() {
            return Err(HtmlTableError::InvalidStructure);
        }
        // Dimensions alone do not prove preservation: a syntactically valid
        // result could still have clipped a span or collapsed a row boundary.
        if target
            .rows
            .iter()
            .zip(&result.rows)
            .any(|(before, after)| before.section != after.section)
            || target
                .rows
                .windows(2)
                .zip(result.rows.windows(2))
                .any(|(before, after)| {
                    (before[0].group == before[1].group) != (after[0].group == after[1].group)
                })
        {
            return Err(HtmlTableError::InvalidStructure);
        }
        for cell in &incoming.grid.cells {
            let row = start.0 + cell.source_row;
            let column = start.1 + cell.column;
            let id = result.grid.slots[row * result.columns + column]
                .ok_or(HtmlTableError::InvalidStructure)?;
            let actual = result.grid.cells[id];
            if actual.source_row != row
                || actual.column != column
                || actual.rows != cell.rows
                || actual.columns != cell.columns
            {
                return Err(HtmlTableError::InvalidStructure);
            }
        }
        Ok((range, prepared))
    }

    /// Paste an encoded rectangular payload, growing the table when needed.
    /// Splitting and replacement are returned as one original-source edit.
    pub fn paste_cells_edit(
        &self,
        source: &str,
        owner: usize,
        start: (usize, usize),
        columns: usize,
        contents: &[String],
    ) -> Result<(TextRange, String), HtmlTableError> {
        self.replace_cells_edit(source, owner, start, columns, contents, None)
    }

    /// Fill an inclusive rectangle while retaining merged-cell attributes.
    pub fn fill_cells_edit(
        &self,
        source: &str,
        owner: usize,
        rectangle: (usize, usize, usize, usize),
        text: &str,
    ) -> Result<(TextRange, String), HtmlTableError> {
        let (top, left, bottom, right) = rectangle;
        if bottom < top || right < left {
            return Err(HtmlTableError::InvalidStructure);
        }
        let rows = bottom
            .checked_sub(top)
            .and_then(|n| n.checked_add(1))
            .ok_or(HtmlTableError::InvalidStructure)?;
        let columns = right
            .checked_sub(left)
            .and_then(|n| n.checked_add(1))
            .ok_or(HtmlTableError::InvalidStructure)?;
        self.replace_cells_edit(
            source,
            owner,
            (top, left),
            columns,
            &[text.to_owned()],
            Some(rows),
        )
    }

    fn replace_cells_edit(
        &self,
        source: &str,
        owner: usize,
        start: (usize, usize),
        columns: usize,
        contents: &[String],
        fill_rows: Option<usize>,
    ) -> Result<(TextRange, String), HtmlTableError> {
        let table = self.table(owner)?;
        if columns == 0
            || contents.is_empty()
            || (fill_rows.is_none() && !contents.len().is_multiple_of(columns))
        {
            return Err(HtmlTableError::InvalidStructure);
        }
        let end_row = start
            .0
            .checked_add(fill_rows.unwrap_or(contents.len() / columns))
            .ok_or(HtmlTableError::InvalidStructure)?;
        let end_column = start
            .1
            .checked_add(columns)
            .ok_or(HtmlTableError::InvalidStructure)?;
        if end_row > table.rows.len() || end_column > table.columns {
            let (range, grown) = self.grow_table_edit(
                source,
                owner,
                end_row.max(table.rows.len()),
                end_column.max(table.columns),
            )?;
            let parsed = crate::parse(&yu_text::TextBuffer::new(&grown).snapshot());
            let index = parsed.html_regions();
            let model = index
                .regions
                .first()
                .and_then(|region| region.model.as_ref().ok())
                .ok_or(HtmlTableError::InvalidStructure)?;
            let owner = model
                .partitions
                .first()
                .and_then(|part| part.content.owner)
                .ok_or(HtmlTableError::InvalidStructure)?;
            let grown_table = model.table(owner)?;
            if grown_table.rows.len() != end_row.max(table.rows.len())
                || grown_table.columns != end_column.max(table.columns)
            {
                return Err(HtmlTableError::InvalidStructure);
            }
            let (_, result) =
                model.replace_cells_edit(&grown, owner, start, columns, contents, fill_rows)?;
            return Ok((range, result));
        }
        let base = table.source.start().get() as usize;
        let mut patched = source
            .get(base..table.source.end().get() as usize)
            .ok_or(HtmlTableError::InvalidStructure)?
            .to_owned();
        let edits = if fill_rows.is_none() {
            self.split_cell_edits(
                source,
                owner,
                (start.0, start.1, end_row - 1, end_column - 1),
            )?
        } else {
            Vec::new()
        };
        for (range, text) in edits.into_iter().rev() {
            patched.replace_range(
                range.start().get() as usize - base..range.end().get() as usize - base,
                &text,
            );
        }
        let parsed = crate::parse(&yu_text::TextBuffer::new(&patched).snapshot());
        let index = parsed.html_regions();
        let model = index
            .regions
            .first()
            .and_then(|r| r.model.as_ref().ok())
            .ok_or(HtmlTableError::InvalidStructure)?;
        let owner = model
            .partitions
            .first()
            .and_then(|p| p.content.owner)
            .ok_or(HtmlTableError::InvalidStructure)?;
        let split = model.table(owner)?;
        let mut replacements = Vec::new();
        let mut insertions = std::collections::BTreeMap::<yu_core::ByteOffset, String>::new();
        for row in start.0..end_row {
            for column in 0..end_column {
                let text = (column >= start.1).then(|| {
                    contents[if fill_rows.is_some() {
                        0
                    } else {
                        (row - start.0) * columns + column - start.1
                    }]
                    .as_str()
                });
                if split.grid.slots[row * split.columns + column].is_some() {
                    if let Some(text) = text {
                        replacements.push((row, column, text));
                    }
                } else {
                    let following = split
                        .grid
                        .cells
                        .iter()
                        .find(|cell| cell.source_row == row && cell.column >= column);
                    let at = following.map_or(split.rows[row].content_end, |cell| {
                        split.rows[row].cells[cell.source_cell].source.start()
                    });
                    insertions
                        .entry(at)
                        .or_default()
                        .push_str(&format!("<td>{}</td>", text.unwrap_or("")));
                }
            }
        }
        let mut edits = model.cell_content_edits(&patched, owner, &replacements)?;
        edits.extend(
            insertions
                .into_iter()
                .map(|(at, text)| (TextRange::empty(at), text)),
        );
        edits.sort_by_key(|(range, _)| (range.start(), range.end()));
        for (range, text) in edits.into_iter().rev() {
            patched.replace_range(
                range.start().get() as usize..range.end().get() as usize,
                &text,
            );
        }
        let parsed = crate::parse(&yu_text::TextBuffer::new(&patched).snapshot());
        let index = parsed.html_regions();
        let model = index
            .regions
            .first()
            .and_then(|r| r.model.as_ref().ok())
            .ok_or(HtmlTableError::InvalidStructure)?;
        let result = model.table(
            model
                .partitions
                .first()
                .and_then(|p| p.content.owner)
                .ok_or(HtmlTableError::InvalidStructure)?,
        )?;
        if result.columns != table.columns || result.rows.len() != table.rows.len() {
            return Err(HtmlTableError::InvalidStructure);
        }
        Ok((table.source, patched))
    }

    /// Split merged owners intersecting an inclusive visual rectangle.
    /// Original content and id remain only at the owner's top-left position.
    pub fn split_cell_edits(
        &self,
        source: &str,
        owner: usize,
        rectangle: (usize, usize, usize, usize),
    ) -> Result<Vec<(TextRange, String)>, HtmlTableError> {
        let table = self.table(owner)?;
        let (top, left, bottom, right) = rectangle;
        if top > bottom || left > right || bottom >= table.rows.len() || right >= table.columns {
            return Err(HtmlTableError::InvalidStructure);
        }
        let slice = |range: TextRange| {
            source
                .get(range.start().get() as usize..range.end().get() as usize)
                .ok_or(HtmlTableError::InvalidStructure)
        };
        let mut edits = Vec::new();
        let mut insertions =
            std::collections::BTreeMap::<yu_core::ByteOffset, Vec<(usize, String)>>::new();
        for cell in &table.grid.cells {
            if (cell.columns == 1 && cell.rows == 1)
                || cell.source_row > bottom
                || cell.source_row + cell.rows <= top
                || cell.column > right
                || cell.column + cell.columns <= left
            {
                continue;
            }
            let original = &table.rows[cell.source_row].cells[cell.source_cell];
            let HtmlNodeKind::Element {
                opening,
                closing: Some(closing),
            } = &self.fragment.nodes[original.node].kind
            else {
                return Err(HtmlTableError::InvalidStructure);
            };
            let opening_text = |clone: bool| -> Result<String, HtmlTableError> {
                let mut text = slice(opening.source)?.to_owned();
                for attribute in opening.attributes.iter().rev().filter(|a| {
                    matches!(a.name.as_str(), "colspan" | "rowspan") || (clone && a.name == "id")
                }) {
                    text.replace_range(
                        (attribute.source.start().get() - opening.source.start().get()) as usize
                            ..(attribute.source.end().get() - opening.source.start().get())
                                as usize,
                        "",
                    );
                }
                Ok(text)
            };
            let close = slice(closing.source)?;
            let empty = format!("{}{close}", opening_text(true)?);
            let mut replacement = format!(
                "{}{}{close}",
                opening_text(false)?,
                slice(original.content)?
            );
            for _ in 1..cell.columns {
                replacement.push_str(&empty);
            }
            edits.push((original.source, replacement));
            for row in cell.source_row + 1..cell.source_row + cell.rows {
                let next = table.grid.cells.iter().find(|candidate| {
                    candidate.source_row == row && candidate.column >= cell.column
                });
                let at = next.map_or(table.rows[row].content_end, |candidate| {
                    table.rows[row].cells[candidate.source_cell].source.start()
                });
                insertions
                    .entry(at)
                    .or_default()
                    .push((cell.column, empty.repeat(cell.columns)));
            }
        }
        for (at, mut cells) in insertions {
            cells.sort_by_key(|(column, _)| *column);
            edits.push((
                TextRange::empty(at),
                cells.into_iter().map(|(_, text)| text).collect(),
            ));
        }
        edits.sort_by_key(|(range, _)| (range.start(), range.end()));
        Ok(edits)
    }

    /// Replace cell bodies addressed by visual slots; one edit per source owner.
    pub fn cell_content_edits(
        &self,
        source: &str,
        owner: usize,
        replacements: &[(usize, usize, &str)],
    ) -> Result<Vec<(TextRange, String)>, HtmlTableError> {
        let table = self.table(owner)?;
        let mut contents = std::collections::BTreeMap::new();
        for &(row, column, text) in replacements {
            if row >= table.grid.rows || column >= table.columns {
                return Err(HtmlTableError::InvalidStructure);
            }
            let owner = table.grid.slots[row * table.columns + column]
                .ok_or(HtmlTableError::InvalidStructure)?;
            if let Some(previous) = contents.insert(owner, text)
                && previous != text
            {
                return Err(HtmlTableError::InvalidStructure);
            }
        }
        let mut edits = Vec::new();
        for (owner, text) in contents {
            let owner = table.grid.cells[owner];
            let cell = &table.rows[owner.source_row].cells[owner.source_cell];
            let current = source
                .get(cell.content.start().get() as usize..cell.content.end().get() as usize)
                .ok_or(HtmlTableError::InvalidStructure)?;
            if current != text {
                edits.push((cell.content, text.to_owned()));
            }
        }
        Ok(edits)
    }

    /// Align each unique owner intersecting a visual column exactly once.
    pub fn align_column_edits(
        &self,
        source: &str,
        owner: usize,
        column: usize,
        alignment: Option<HtmlAlignment>,
    ) -> Result<Vec<(TextRange, String)>, HtmlTableError> {
        let table = self.table(owner)?;
        if column >= table.columns {
            return Err(HtmlTableError::InvalidStructure);
        }
        let value = alignment.map(|alignment| match alignment {
            HtmlAlignment::Left => "left",
            HtmlAlignment::Center => "center",
            HtmlAlignment::Right => "right",
            HtmlAlignment::Justify => "justify",
        });
        let mut edits = Vec::new();
        for owner in &table.grid.cells {
            if column < owner.column || column >= owner.column + owner.columns {
                continue;
            }
            let cell = &table.rows[owner.source_row].cells[owner.source_cell];
            let HtmlNodeKind::Element { opening, .. } = &self.fragment.nodes[cell.node].kind else {
                return Err(HtmlTableError::InvalidStructure);
            };
            let attribute = opening
                .attributes
                .iter()
                .find(|a| a.name == "align" || a.name == "style");
            match (attribute, value) {
                (Some(attribute), None) => edits.push((attribute.source, String::new())),
                (Some(attribute), Some(value)) => {
                    let range = attribute.value.ok_or(HtmlTableError::InvalidStructure)?;
                    let text = if attribute.name == "style" {
                        format!("text-align:{value}")
                    } else {
                        value.to_owned()
                    };
                    if source
                        .get(range.start().get() as usize..range.end().get() as usize)
                        .ok_or(HtmlTableError::InvalidStructure)?
                        != text
                    {
                        edits.push((range, text));
                    }
                }
                (None, Some(value)) => {
                    let at = yu_core::ByteOffset::new(
                        opening
                            .source
                            .end()
                            .get()
                            .checked_sub(1)
                            .ok_or(HtmlTableError::InvalidStructure)?,
                    );
                    edits.push((TextRange::empty(at), format!(" align=\"{value}\"")));
                }
                (None, None) => {}
            }
        }
        Ok(edits)
    }

    /// Insert relative to one source row, staying inside that row's group.
    pub fn insert_row_edits(
        &self,
        source: &str,
        owner: usize,
        row: usize,
        before: bool,
    ) -> Result<Vec<(TextRange, String)>, HtmlTableError> {
        let table = self.table(owner)?;
        let reference = table
            .rows
            .get(row)
            .ok_or(HtmlTableError::InvalidStructure)?;
        let position = row + usize::from(!before);
        let group_start = row
            - table.rows[..row]
                .iter()
                .rev()
                .take_while(|r| r.group == reference.group)
                .count();
        let group_end = row
            + table.rows[row..]
                .iter()
                .take_while(|r| r.group == reference.group)
                .count();
        let mut covered = vec![false; table.columns];
        let mut edits = Vec::new();
        for cell in &table.grid.cells {
            if cell.source_row < group_start || cell.source_row >= position {
                continue;
            }
            let original = &table.rows[cell.source_row].cells[cell.source_cell];
            let end = cell.source_row + cell.rows;
            let interior = position < end;
            let extends_group_end = position == group_end
                && end == group_end
                && (original.span.rows == 0 || original.span.rows > cell.rows);
            if !interior && !extends_group_end {
                continue;
            }
            covered[cell.column..cell.column + cell.columns].fill(true);
            if interior && original.span.rows != 0 {
                if original.span.rows >= 65534 {
                    return Err(HtmlTableError::InvalidStructure);
                }
                let HtmlNodeKind::Element { opening, .. } =
                    &self.fragment.nodes[original.node].kind
                else {
                    return Err(HtmlTableError::InvalidStructure);
                };
                let value = opening
                    .attributes
                    .iter()
                    .find(|attr| attr.name == "rowspan")
                    .and_then(|attr| attr.value)
                    .ok_or(HtmlTableError::InvalidStructure)?;
                edits.push((value, (original.span.rows + 1).to_string()));
            }
        }
        let mut text = String::from("<tr>");
        for (column, covered) in covered.into_iter().enumerate() {
            if covered {
                continue;
            }
            let header = table.grid.slots[row * table.columns + column]
                .map(|id| table.grid.cells[id])
                .is_some_and(|cell| table.rows[cell.source_row].cells[cell.source_cell].header);
            text.push_str(if header { "<th></th>" } else { "<td></td>" });
        }
        text.push_str("</tr>");
        let raw = source
            .get(table.source.start().get() as usize..table.source.end().get() as usize)
            .ok_or(HtmlTableError::InvalidStructure)?;
        let ending = if raw.contains("\r\n") {
            "\r\n"
        } else if raw.contains('\n') {
            "\n"
        } else {
            ""
        };
        let (at, text) = if before {
            (reference.source.start(), format!("{text}{ending}"))
        } else {
            (reference.source.end(), format!("{ending}{text}"))
        };
        edits.push((TextRange::empty(at), text));
        edits.sort_by_key(|(range, _)| range.start());
        Ok(edits)
    }

    /// Delete a visual row without deleting surviving rowspan owners.
    pub fn delete_row_edits(
        &self,
        source: &str,
        owner: usize,
        row: usize,
    ) -> Result<Vec<(TextRange, String)>, HtmlTableError> {
        let table = self.table(owner)?;
        let removed = table
            .rows
            .get(row)
            .ok_or(HtmlTableError::InvalidStructure)?;
        if table.rows.len() == 1 {
            return Ok(vec![(table.source, String::new())]);
        }
        let mut edits = vec![(removed.source, String::new())];
        let mut insertions = std::collections::BTreeMap::<yu_core::ByteOffset, String>::new();
        for cell in &table.grid.cells {
            if cell.source_row > row || cell.source_row + cell.rows <= row || cell.rows == 1 {
                continue;
            }
            let original = &table.rows[cell.source_row].cells[cell.source_cell];
            let HtmlNodeKind::Element { opening, .. } = &self.fragment.nodes[original.node].kind
            else {
                return Err(HtmlTableError::InvalidStructure);
            };
            let value = opening
                .attributes
                .iter()
                .find(|attr| attr.name == "rowspan")
                .and_then(|attr| attr.value)
                .ok_or(HtmlTableError::InvalidStructure)?;
            let declared = original.span.rows;
            if cell.source_row < row {
                if declared != 0 {
                    edits.push((value, (declared - 1).to_string()));
                }
                continue;
            }
            let next = table
                .rows
                .get(row + 1)
                .ok_or(HtmlTableError::InvalidStructure)?;
            if next.group != removed.group {
                return Err(HtmlTableError::InvalidStructure);
            }
            let mut text = source
                .get(original.source.start().get() as usize..original.source.end().get() as usize)
                .ok_or(HtmlTableError::InvalidStructure)?
                .to_owned();
            if declared != 0 {
                text.replace_range(
                    (value.start().get() - original.source.start().get()) as usize
                        ..(value.end().get() - original.source.start().get()) as usize,
                    &(declared - 1).to_string(),
                );
            }
            let next_cell = table.grid.cells.iter().find(|candidate| {
                candidate.source_row == row + 1 && candidate.column >= cell.column
            });
            let at = next_cell.map_or(next.content_end, |candidate| {
                next.cells[candidate.source_cell].source.start()
            });
            insertions.entry(at).or_default().push_str(&text);
        }
        edits.extend(
            insertions
                .into_iter()
                .map(|(at, text)| (TextRange::empty(at), text)),
        );
        edits.sort_by_key(|(range, _)| range.start());
        Ok(edits)
    }

    /// Source edits for a visual column. Owners spanning several rows are edited once.
    pub fn column_edits(
        &self,
        source: &str,
        owner: usize,
        column: usize,
        insert: bool,
    ) -> Result<Vec<(TextRange, String)>, HtmlTableError> {
        let table = self.table(owner)?;
        if column > table.columns || (!insert && column == table.columns) {
            return Err(HtmlTableError::InvalidStructure);
        }
        if !insert && table.columns == 1 {
            return Ok(vec![(table.source, String::new())]);
        }
        let mut edits = Vec::new();
        for cell in &table.grid.cells {
            let overlaps = if insert {
                cell.column < column && column < cell.column + cell.columns
            } else {
                cell.column <= column && column < cell.column + cell.columns
            };
            if !overlaps {
                continue;
            }
            let original = &table.rows[cell.source_row].cells[cell.source_cell];
            if !insert && cell.columns == 1 {
                edits.push((original.source, String::new()));
                continue;
            }
            let HtmlNodeKind::Element { opening, .. } = &self.fragment.nodes[original.node].kind
            else {
                return Err(HtmlTableError::InvalidStructure);
            };
            if insert && cell.columns >= 1000 {
                return Err(HtmlTableError::InvalidStructure);
            }
            let span = if insert {
                cell.columns + 1
            } else {
                cell.columns - 1
            };
            let attribute = opening
                .attributes
                .iter()
                .find(|attr| attr.name == "colspan")
                .and_then(|attr| attr.value)
                .ok_or(HtmlTableError::InvalidStructure)?;
            if source
                .get(attribute.start().get() as usize..attribute.end().get() as usize)
                .is_none()
            {
                return Err(HtmlTableError::InvalidStructure);
            }
            edits.push((attribute, span.to_string()));
        }
        if insert {
            for (row_index, row) in table.rows.iter().enumerate() {
                if table.grid.cells.iter().any(|cell| {
                    cell.source_row <= row_index
                        && row_index < cell.source_row + cell.rows
                        && cell.column < column
                        && column < cell.column + cell.columns
                }) {
                    continue;
                }
                let following = table
                    .grid
                    .cells
                    .iter()
                    .find(|cell| cell.source_row == row_index && cell.column >= column);
                let at = following.map_or(row.content_end, |cell| {
                    row.cells[cell.source_cell].source.start()
                });
                let header = following.is_some_and(|cell| row.cells[cell.source_cell].header)
                    || (following.is_none() && row.cells.last().is_some_and(|cell| cell.header));
                let ending = table
                    .grid
                    .cells
                    .iter()
                    .filter(|cell| {
                        cell.source_row <= row_index && row_index < cell.source_row + cell.rows
                    })
                    .map(|cell| cell.column + cell.columns)
                    .max()
                    .unwrap_or(0);
                let count = column.saturating_sub(ending) + 1;
                edits.push((
                    TextRange::empty(at),
                    if header { "<th></th>" } else { "<td></td>" }.repeat(count),
                ));
            }
        }
        edits.sort_by_key(|(range, _)| range.start());
        Ok(edits)
    }

    /// Native table decoration input shared by presentation and cell editing.
    pub fn decorate_table(
        &self,
        source: &str,
        part: &super::HtmlFlowPartition,
        out: &mut crate::ExtensionOutput,
    ) -> bool {
        if part.content.kind != super::HtmlFlowKind::Table || !self.partitions.contains(part) {
            return false;
        }
        let Some(owner) = part.content.owner else {
            return false;
        };
        let Ok(table) = self.native_table(owner) else {
            return false;
        };
        let Some(grid) = crate::TableBlock::from_html(&table) else {
            return false;
        };
        if source
            .get(part.source.start().get() as usize..part.source.end().get() as usize)
            .is_none()
        {
            return false;
        }
        let mut cursor = part.source.start();
        for cell in table.rows.iter().flat_map(|row| &row.cells) {
            out.replace(TextRange::new(cursor, cell.content.start()).expect("ordered cells"));
            let parts = self.cell_partitions.get(&cell.node).expect("cell flow");
            let breaks = self
                .cell_paragraph_breaks(cell.node)
                .expect("validated cell boundaries");
            for part in parts {
                self.decorate_text_part_with_breaks(source, part, out, &breaks);
            }
            if cell.header {
                let style = out.style(yu_core::TextAttrs::new(yu_core::TextStyle::Strong));
                out.mark(cell.content, style);
            }
            cursor = cell.content.end();
        }
        out.replace(TextRange::new(cursor, part.source.end()).expect("table suffix"));
        let hidden_bodies: Vec<_> = self
            .disclosures()
            .into_iter()
            .filter(|details| !details.open)
            .map(|details| details.body)
            .collect();
        for image in self.image_spans(source) {
            if hidden_bodies.iter().any(|body| {
                body.start() <= image.source().start() && image.source().end() <= body.end()
            }) {
                continue;
            }
            if table.rows.iter().flat_map(|row| &row.cells).any(|cell| {
                image.source().start() >= cell.content.start()
                    && image.source().end() <= cell.content.end()
            }) {
                let widget = out.widget(crate::BlockWidget::Image(image));
                out.place_widget(image.source(), widget, yu_core::WidgetSide::Before);
            }
        }
        let style = out.line_style(crate::BlockOrnament::Table(grid));
        out.line(part.source, style);
        true
    }

    pub fn table(&self, owner: usize) -> Result<HtmlTable, HtmlTableError> {
        let table = self
            .fragment
            .nodes
            .get(owner)
            .ok_or(HtmlTableError::InvalidStructure)?;
        if self
            .resolution
            .elements
            .get(owner)
            .and_then(Option::as_ref)
            .map(|e| e.kind)
            != Some(Kind::Table)
        {
            return Err(HtmlTableError::InvalidStructure);
        }
        let disclosures = self.disclosures_ref();
        let disclosure_headers: std::collections::BTreeMap<_, _> = disclosures
            .iter()
            .flat_map(|details| {
                std::iter::once((details.opening.start(), details))
                    .chain(details.summary.map(|summary| (summary.start(), details)))
            })
            .collect();
        let mut rows = Vec::new();
        let mut pending = vec![(table.children.as_slice(), HtmlTableSection::Direct)];
        while let Some((children, section)) = pending.pop() {
            for &id in children {
                let node = &self.fragment.nodes[id];
                if !matches!(node.kind, HtmlNodeKind::Element { .. }) {
                    continue;
                }
                let element = self.resolution.elements[id]
                    .as_ref()
                    .ok_or(HtmlTableError::InvalidStructure)?;
                match element.kind {
                    Kind::TableHead | Kind::TableBody | Kind::TableFoot => {
                        let group = match element.kind {
                            Kind::TableHead => HtmlTableSection::Head,
                            Kind::TableFoot => HtmlTableSection::Foot,
                            _ => HtmlTableSection::Body,
                        };
                        pending.push((node.children.as_slice(), group));
                    }
                    Kind::TableRow => {
                        let mut cells = Vec::new();
                        for &cell_id in &node.children {
                            let cell = &self.fragment.nodes[cell_id];
                            if !matches!(cell.kind, HtmlNodeKind::Element { .. }) {
                                continue;
                            }
                            let resolved = self.resolution.elements[cell_id]
                                .as_ref()
                                .ok_or(HtmlTableError::InvalidStructure)?;
                            if !matches!(resolved.kind, Kind::HeaderCell | Kind::DataCell) {
                                return Err(HtmlTableError::InvalidStructure);
                            }
                            let HtmlNodeKind::Element {
                                opening,
                                closing: Some(closing),
                            } = &cell.kind
                            else {
                                return Err(HtmlTableError::InvalidStructure);
                            };
                            let mut descendants = cell.children.clone();
                            while let Some(child) = descendants.pop() {
                                if self.resolution.elements[child]
                                    .as_ref()
                                    .is_some_and(|e| e.kind == Kind::Table)
                                {
                                    return Err(HtmlTableError::NestedTable);
                                }
                                descendants.extend(&self.fragment.nodes[child].children);
                            }
                            cells.push(HtmlTableCell {
                                node: cell_id,
                                source: cell.source,
                                content: TextRange::new(
                                    opening.source.end(),
                                    closing.source.start(),
                                )
                                .ok_or(HtmlTableError::InvalidStructure)?,
                                header: resolved.kind == Kind::HeaderCell,
                                span: super::HtmlCellSpan {
                                    columns: resolved.attributes.column_span.unwrap_or(1),
                                    rows: resolved.attributes.row_span.unwrap_or(1),
                                },
                                alignment: self.alignment_for_node(cell_id),
                                paragraphs: self
                                    .cell_partitions
                                    .get(&cell_id)
                                    .ok_or(HtmlTableError::InvalidStructure)?
                                    .iter()
                                    .map(|part| {
                                        let content = part
                                            .content
                                            .owner
                                            .and_then(|owner| {
                                                let node = &self.fragment.nodes[owner];
                                                let HtmlNodeKind::Element {
                                                    opening,
                                                    closing: Some(closing),
                                                } = &node.kind
                                                else {
                                                    return None;
                                                };
                                                (node.source == part.content.source).then(|| {
                                                    TextRange::new(
                                                        opening.source.end(),
                                                        closing.source.start(),
                                                    )
                                                    .expect("element body")
                                                })
                                            })
                                            .unwrap_or(part.content.source);
                                        let disclosure = disclosure_headers
                                            .get(&part.content.source.start())
                                            .filter(|details| {
                                                details.source == part.content.source
                                                    || details.summary == Some(part.content.source)
                                                    || details.opening == part.content.source
                                            });
                                        let content = disclosure
                                            .and_then(|details| {
                                                TextRange::new(
                                                    details.opening.start(),
                                                    details
                                                        .summary_content
                                                        .map_or(details.opening.end(), |range| {
                                                            range.end()
                                                        }),
                                                )
                                            })
                                            .unwrap_or(content);
                                        HtmlCellParagraph {
                                            source: content,
                                            disclosure_content: disclosure
                                                .and_then(|details| details.summary_content),
                                            alignment: self.alignment(part),
                                            list: self.cell_list_context(part, cell_id),
                                            heading: match part.content.kind {
                                                super::HtmlFlowKind::Heading(level) => Some(level),
                                                _ => None,
                                            },
                                        }
                                    })
                                    .collect(),
                            });
                        }
                        let HtmlNodeKind::Element {
                            closing: Some(closing),
                            ..
                        } = &node.kind
                        else {
                            return Err(HtmlTableError::InvalidStructure);
                        };
                        rows.push(HtmlTableRow {
                            source: node.source,
                            content_end: closing.source.start(),
                            section,
                            group: node.parent.unwrap_or(owner),
                            cells,
                        });
                    }
                    _ => return Err(HtmlTableError::InvalidStructure),
                }
            }
        }
        rows.sort_by_key(|row| row.source.start());
        let mut groups: Vec<Vec<Vec<super::HtmlCellSpan>>> = Vec::new();
        let mut previous_group = None;
        for row in &rows {
            if previous_group != Some(row.group) {
                groups.push(Vec::new());
                previous_group = Some(row.group);
            }
            groups
                .last_mut()
                .expect("row group")
                .push(row.cells.iter().map(|cell| cell.span).collect());
        }
        let grid =
            super::HtmlTableGrid::build(&groups).map_err(|_| HtmlTableError::InvalidStructure)?;
        let columns = grid.columns;
        if columns == 0 {
            return Err(HtmlTableError::EmptyTable);
        }
        Ok(HtmlTable {
            source: table.source,
            columns,
            grid,
            rows,
        })
    }
}
