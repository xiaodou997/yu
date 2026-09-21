//! Conservative GFM table recognition over one paragraph source range.
//!
//! The block scanner intentionally keeps tables as paragraphs for now.  Table
//! recognition is therefore a projection concern, but all metadata remains
//! source-backed: the parser returns byte ranges into the canonical snapshot
//! instead of copying cell text into an editor document model.

use yu_core::TextRange;
use yu_text::TextSnapshot;

/// Alignment requested by a GFM delimiter or HTML cell.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum TableAlignment {
    Default,
    Left,
    Center,
    Right,
    /// HTML-only; GFM delimiter syntax cannot represent justification.
    Justify,
}

/// A source-relative byte range for one table cell.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct TableCellRange {
    start: usize,
    end: usize,
}

impl TableCellRange {
    #[must_use]
    pub const fn new(start: usize, end: usize) -> Self {
        Self { start, end }
    }

    #[must_use]
    pub const fn start(self) -> usize {
        self.start
    }

    #[must_use]
    pub const fn end(self) -> usize {
        self.end
    }

    #[must_use]
    const fn translated(self, offset: usize) -> Self {
        Self {
            start: self.start.saturating_add(offset),
            end: self.end.saturating_add(offset),
        }
    }
}

/// A source-relative byte range for one physical table row.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct TableRowRange {
    start: usize,
    end: usize,
}

/// A visible table cell address. Row `0` is the header; body rows start at
/// `1`. The delimiter row is intentionally absent from this coordinate space.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct TableCellAddress {
    row: usize,
    column: usize,
}

impl TableCellAddress {
    #[must_use]
    pub const fn new(row: usize, column: usize) -> Self {
        Self { row, column }
    }

    #[must_use]
    pub const fn row(self) -> usize {
        self.row
    }

    #[must_use]
    pub const fn column(self) -> usize {
        self.column
    }
}

impl TableRowRange {
    #[must_use]
    pub const fn new(start: usize, end: usize) -> Self {
        Self { start, end }
    }

    #[must_use]
    pub const fn start(self) -> usize {
        self.start
    }

    #[must_use]
    pub const fn end(self) -> usize {
        self.end
    }

    #[must_use]
    const fn translated(self, offset: usize) -> Self {
        Self {
            start: self.start.saturating_add(offset),
            end: self.end.saturating_add(offset),
        }
    }
}

/// A source-backed paragraph inside an HTML cell.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TableCellParagraph {
    pub source: TableCellRange,
    pub alignment: TableAlignment,
    pub heading: Option<u8>,
    pub disclosure_content: Option<TableCellRange>,
    pub list: Vec<TableCellListItem>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TableCellListItem {
    pub container: TableCellRange,
    pub opening: Option<TableCellRange>,
    pub number: Option<u64>,
    pub tight: bool,
}

/// A recognized table whose cells refer to the supplied source string.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TableBlock {
    html: bool,
    html_grid: Option<std::sync::Arc<crate::html::HtmlTableGrid>>,
    cell_styles: Vec<Vec<(bool, TableAlignment)>>,
    cell_paragraphs: Vec<Vec<Vec<TableCellParagraph>>>,
    source_range: TableCellRange,
    first_row: Vec<TableCellRange>,
    delimiter: Vec<TableCellRange>,
    alignments: Vec<TableAlignment>,
    rows: Vec<Vec<TableCellRange>>,
    row_ranges: Vec<TableRowRange>,
}

impl TableBlock {
    pub fn is_html(&self) -> bool {
        self.html
    }

    /// Logical span and source owner for an HTML slot, including covered slots.
    pub fn html_cell_owner(&self, address: TableCellAddress) -> Option<crate::html::HtmlGridCell> {
        let grid = self.html_grid.as_ref()?;
        if address.row() >= grid.rows || address.column() >= grid.columns {
            return None;
        }
        let owner = grid.slots[address.row() * grid.columns + address.column()]?;
        grid.cells.get(owner).copied()
    }

    fn source_cell_address(&self, address: TableCellAddress) -> Option<TableCellAddress> {
        if self.html {
            let owner = self.html_cell_owner(address)?;
            Some(TableCellAddress::new(owner.source_row, owner.source_cell))
        } else {
            Some(address)
        }
    }

    /// Canonical visual origin. Covered slots return the same origin.
    pub fn cell_origin(&self, address: TableCellAddress) -> Option<TableCellAddress> {
        if self.html {
            let owner = self.html_cell_owner(address)?;
            Some(TableCellAddress::new(owner.source_row, owner.column))
        } else {
            self.visible_cell(address).map(|_| address)
        }
    }

    /// Expand a rectangular selection until every intersected merged cell is whole.
    pub fn selection_bounds(
        &self,
        start: TableCellAddress,
        end: TableCellAddress,
    ) -> Option<(TableCellAddress, TableCellAddress)> {
        self.visible_cell(start)?;
        self.visible_cell(end)?;
        let (mut top, mut bottom) = (start.row().min(end.row()), start.row().max(end.row()));
        let (mut left, mut right) = (
            start.column().min(end.column()),
            start.column().max(end.column()),
        );
        if let Some(grid) = &self.html_grid {
            loop {
                let previous = (top, bottom, left, right);
                for cell in &grid.cells {
                    let cell_bottom = cell.source_row + cell.rows - 1;
                    let cell_right = cell.column + cell.columns - 1;
                    if cell.source_row <= bottom
                        && cell_bottom >= top
                        && cell.column <= right
                        && cell_right >= left
                    {
                        top = top.min(cell.source_row);
                        bottom = bottom.max(cell_bottom);
                        left = left.min(cell.column);
                        right = right.max(cell_right);
                    }
                }
                if previous == (top, bottom, left, right) {
                    break;
                }
            }
        }
        Some((
            TableCellAddress::new(top, left),
            TableCellAddress::new(bottom, right),
        ))
    }

    pub fn cell_is_header(&self, address: TableCellAddress) -> bool {
        if !self.html {
            return address.row() == 0 && self.visible_cell(address).is_some();
        }
        let Some(address) = self.source_cell_address(address) else {
            return false;
        };
        self.cell_styles
            .get(address.row())
            .and_then(|row| row.get(address.column()))
            .is_some_and(|style| style.0)
    }

    pub fn cell_paragraphs(&self, address: TableCellAddress) -> &[TableCellParagraph] {
        let Some(address) = self.source_cell_address(address) else {
            return &[];
        };
        self.cell_paragraphs
            .get(address.row())
            .and_then(|row| row.get(address.column()))
            .map_or(&[], Vec::as_slice)
    }

    pub fn cell_alignment(&self, address: TableCellAddress) -> TableAlignment {
        let Some(address) = self.source_cell_address(address) else {
            return TableAlignment::Default;
        };
        self.cell_styles
            .get(address.row())
            .and_then(|row| row.get(address.column()))
            .map_or_else(
                || {
                    self.alignments
                        .get(address.column())
                        .copied()
                        .unwrap_or(TableAlignment::Default)
                },
                |style| style.1,
            )
    }

    /// Preserve only real HTML cells. Missing trailing cells have no source
    /// range and are not synthesized by rendering.
    pub fn from_html(table: &crate::html::HtmlTable) -> Option<Self> {
        if table.columns == 0 || table.rows.is_empty() {
            return None;
        }
        let range =
            |r: TextRange| TableCellRange::new(r.start().get() as usize, r.end().get() as usize);
        let cell_styles = table
            .rows
            .iter()
            .map(|row| {
                row.cells
                    .iter()
                    .map(|cell| {
                        let alignment = match cell.alignment {
                            None => TableAlignment::Default,
                            Some(crate::html::HtmlAlignment::Left) => TableAlignment::Left,
                            Some(crate::html::HtmlAlignment::Center) => TableAlignment::Center,
                            Some(crate::html::HtmlAlignment::Right) => TableAlignment::Right,
                            Some(crate::html::HtmlAlignment::Justify) => TableAlignment::Justify,
                        };
                        Some((cell.header, alignment))
                    })
                    .collect::<Option<Vec<_>>>()
            })
            .collect::<Option<Vec<_>>>()?;
        Some(Self {
            html: true,
            html_grid: Some(std::sync::Arc::new(table.grid.clone())),
            cell_styles,
            cell_paragraphs: table
                .rows
                .iter()
                .map(|row| {
                    row.cells
                        .iter()
                        .map(|cell| {
                            cell.paragraphs
                                .iter()
                                .map(|part| TableCellParagraph {
                                    source: range(part.source),
                                    heading: part.heading,
                                    disclosure_content: part.disclosure_content.map(range),
                                    list: part
                                        .list
                                        .iter()
                                        .map(|item| TableCellListItem {
                                            container: range(item.container),
                                            opening: item.opening.map(range),
                                            number: item.number,
                                            tight: item.tight,
                                        })
                                        .collect(),
                                    alignment: match part.alignment {
                                        Some(crate::html::HtmlAlignment::Left) => {
                                            TableAlignment::Left
                                        }
                                        Some(crate::html::HtmlAlignment::Center) => {
                                            TableAlignment::Center
                                        }
                                        Some(crate::html::HtmlAlignment::Right) => {
                                            TableAlignment::Right
                                        }
                                        Some(crate::html::HtmlAlignment::Justify) => {
                                            TableAlignment::Justify
                                        }
                                        None => TableAlignment::Default,
                                    },
                                })
                                .collect()
                        })
                        .collect()
                })
                .collect(),
            source_range: range(table.source),
            first_row: table.rows[0]
                .cells
                .iter()
                .map(|cell| range(cell.content))
                .collect(),
            delimiter: Vec::new(),
            alignments: vec![TableAlignment::Default; table.columns],
            rows: table.rows[1..]
                .iter()
                .map(|row| row.cells.iter().map(|cell| range(cell.content)).collect())
                .collect(),
            row_ranges: table
                .rows
                .iter()
                .map(|row| {
                    TableRowRange::new(
                        row.source.start().get() as usize,
                        row.source.end().get() as usize,
                    )
                })
                .collect(),
        })
    }
    #[must_use]
    pub const fn source_range(&self) -> TableCellRange {
        self.source_range
    }

    #[must_use]
    pub fn first_row(&self) -> &[TableCellRange] {
        &self.first_row
    }

    /// Returns the parser-owned delimiter row. Delimiter cells remain in the
    /// source model even though projection/layout suppress their visible row.
    #[must_use]
    pub fn delimiter(&self) -> &[TableCellRange] {
        &self.delimiter
    }

    #[must_use]
    pub fn alignments(&self) -> &[TableAlignment] {
        &self.alignments
    }

    #[must_use]
    pub fn rows(&self) -> &[Vec<TableCellRange>] {
        &self.rows
    }

    /// Returns physical row ranges in source order: header, delimiter, then
    /// body rows.
    #[must_use]
    pub fn row_ranges(&self) -> &[TableRowRange] {
        &self.row_ranges
    }

    /// Returns the complete source range for one physical row, including its
    /// line ending when the next row is present.  This is useful when a
    /// visual layout replaces a row with semantic geometry and must suppress
    /// the parser-owned line break as well as the row's text.
    #[must_use]
    pub fn row_source_range(&self, row: usize) -> Option<TableCellRange> {
        let current = *self.row_ranges.get(row)?;
        if self.html {
            return Some(TableCellRange::new(current.start(), current.end()));
        }
        let end = self
            .row_ranges
            .get(row.saturating_add(1))
            .map_or(self.source_range.end(), |next| next.start());
        Some(TableCellRange::new(current.start(), end))
    }

    #[must_use]
    pub fn delimiter_source_range(&self) -> Option<TableCellRange> {
        if self.html {
            None
        } else {
            self.row_source_range(1)
        }
    }

    /// Stable structural source for column proportions. HTML has no delimiter
    /// row, so use its opening table/section prefix before the first row.
    #[must_use]
    pub fn width_anchor_source_range(&self) -> Option<TableCellRange> {
        if self.html {
            let end = self.row_ranges.first()?.start();
            (end > self.source_range.start())
                .then(|| TableCellRange::new(self.source_range.start(), end))
        } else {
            self.delimiter_source_range()
        }
    }

    #[must_use]
    pub fn column_count(&self) -> usize {
        if self.html {
            self.alignments.len()
        } else {
            self.first_row.len()
        }
    }

    #[must_use]
    pub fn body_row_count(&self) -> usize {
        self.rows.len()
    }

    /// Returns the number of visible rows. The parser-owned delimiter row is
    /// not part of this coordinate space.
    #[must_use]
    pub fn visible_row_count(&self) -> usize {
        usize::from(self.html || !self.first_row.is_empty()).saturating_add(self.rows.len())
    }

    /// Returns a source-backed visible cell for a row/column address.
    #[must_use]
    pub fn visible_cell(&self, address: TableCellAddress) -> Option<TableCellRange> {
        let address = self.source_cell_address(address)?;
        if address.column >= self.column_count() {
            return None;
        }
        if address.row == 0 {
            return self.first_row.get(address.column).copied();
        }
        self.rows
            .get(address.row.saturating_sub(1))
            .and_then(|row| row.get(address.column))
            .copied()
    }

    /// Returns the visible cell containing a source byte boundary. Cell ends
    /// are included so a caret immediately before the structural pipe still
    /// remains owned by the preceding source-backed cell.
    #[must_use]
    pub fn visible_cell_for_source(&self, offset: usize) -> Option<TableCellAddress> {
        let mut address = TableCellAddress::new(0, 0);
        for row in std::iter::once(&self.first_row).chain(self.rows.iter()) {
            for (column, cell) in row.iter().copied().enumerate() {
                if (cell.start() == cell.end() && offset == cell.start())
                    || (cell.start() <= offset && offset <= cell.end())
                {
                    return if let Some(grid) = &self.html_grid {
                        let owner = grid.cells.iter().find(|cell| {
                            cell.source_row == address.row() && cell.source_cell == column
                        })?;
                        Some(TableCellAddress::new(owner.source_row, owner.column))
                    } else {
                        Some(address)
                    };
                }
                address = TableCellAddress::new(address.row(), column.saturating_add(1));
            }
            address = TableCellAddress::new(address.row().saturating_add(1), 0);
        }
        None
    }

    /// Returns the next visible cell in row-major order.
    #[must_use]
    pub fn next_visible_cell(
        &self,
        address: TableCellAddress,
    ) -> Option<(TableCellAddress, TableCellRange)> {
        let address = self.cell_origin(address).unwrap_or(address);
        for row in address.row()..self.visible_row_count() {
            let first = if row == address.row() {
                address.column().saturating_add(1)
            } else {
                0
            };
            for column in first..self.column_count() {
                let next = TableCellAddress::new(row, column);
                if self.cell_origin(next) != Some(next) {
                    continue;
                }
                if let Some(cell) = self.visible_cell(next) {
                    return Some((next, cell));
                }
            }
        }
        None
    }

    /// Returns the previous source-backed cell, skipping absent HTML slots.
    #[must_use]
    pub fn previous_visible_cell(
        &self,
        address: TableCellAddress,
    ) -> Option<(TableCellAddress, TableCellRange)> {
        let address = self.cell_origin(address).unwrap_or(address);
        for row in (0..=address
            .row()
            .min(self.visible_row_count().saturating_sub(1)))
            .rev()
        {
            let end = if row == address.row() {
                address.column().min(self.column_count())
            } else {
                self.column_count()
            };
            for column in (0..end).rev() {
                let previous = TableCellAddress::new(row, column);
                if self.cell_origin(previous) != Some(previous) {
                    continue;
                }
                if let Some(cell) = self.visible_cell(previous) {
                    return Some((previous, cell));
                }
            }
        }
        None
    }

    #[must_use]
    pub(crate) fn translated(self, offset: usize) -> Self {
        Self {
            source_range: self.source_range.translated(offset),
            first_row: self
                .first_row
                .into_iter()
                .map(|range| range.translated(offset))
                .collect(),
            delimiter: self
                .delimiter
                .into_iter()
                .map(|range| range.translated(offset))
                .collect(),
            html: self.html,
            html_grid: self.html_grid,
            cell_styles: self.cell_styles,
            cell_paragraphs: self
                .cell_paragraphs
                .into_iter()
                .map(|row| {
                    row.into_iter()
                        .map(|parts| {
                            parts
                                .into_iter()
                                .map(|part| TableCellParagraph {
                                    source: part.source.translated(offset),
                                    disclosure_content: part
                                        .disclosure_content
                                        .map(|range| range.translated(offset)),
                                    list: part
                                        .list
                                        .iter()
                                        .map(|item| TableCellListItem {
                                            container: item.container.translated(offset),
                                            opening: item.opening.map(|r| r.translated(offset)),
                                            ..*item
                                        })
                                        .collect(),
                                    ..part
                                })
                                .collect()
                        })
                        .collect()
                })
                .collect(),
            alignments: self.alignments,
            rows: self
                .rows
                .into_iter()
                .map(|row| {
                    row.into_iter()
                        .map(|range| range.translated(offset))
                        .collect()
                })
                .collect(),
            row_ranges: self
                .row_ranges
                .into_iter()
                .map(|range| range.translated(offset))
                .collect(),
        }
    }

    /// 整张表平移 `delta` 个字节。
    ///
    /// 一次落在表格**之外**的编辑会让表内每一个偏移挪同样多——所有偏移都
    /// 在每一处改动的同一侧，所以平移量是个常量。逐个区间去问锚点也对，
    /// 只是把一个常量算了几十遍。落到负数或溢出时返回 `None`：那说明「编辑
    /// 在表格之外」这个前提不成立，宁可让调用方重建。
    #[must_use]
    pub fn shifted(self, delta: i64) -> Option<Self> {
        let cell = |range: TableCellRange| -> Option<TableCellRange> {
            Some(TableCellRange::new(
                shift(range.start(), delta)?,
                shift(range.end(), delta)?,
            ))
        };
        let cells = |ranges: &[TableCellRange]| -> Option<Vec<TableCellRange>> {
            ranges.iter().copied().map(cell).collect()
        };
        Some(Self {
            source_range: cell(self.source_range)?,
            first_row: cells(&self.first_row)?,
            delimiter: cells(&self.delimiter)?,
            html: self.html,
            html_grid: self.html_grid,
            cell_styles: self.cell_styles,
            cell_paragraphs: self
                .cell_paragraphs
                .iter()
                .map(|row| {
                    row.iter()
                        .map(|parts| {
                            parts
                                .iter()
                                .map(|part| {
                                    Some(TableCellParagraph {
                                        source: cell(part.source)?,
                                        disclosure_content: match part.disclosure_content {
                                            Some(range) => Some(cell(range)?),
                                            None => None,
                                        },
                                        list: part
                                            .list
                                            .iter()
                                            .map(|item| {
                                                Some(TableCellListItem {
                                                    container: cell(item.container)?,
                                                    opening: match item.opening {
                                                        Some(range) => Some(cell(range)?),
                                                        None => None,
                                                    },
                                                    ..*item
                                                })
                                            })
                                            .collect::<Option<Vec<_>>>()?,
                                        ..part.clone()
                                    })
                                })
                                .collect::<Option<Vec<_>>>()
                        })
                        .collect::<Option<Vec<_>>>()
                })
                .collect::<Option<Vec<_>>>()?,
            alignments: self.alignments,
            rows: self
                .rows
                .iter()
                .map(|row| cells(row))
                .collect::<Option<Vec<_>>>()?,
            row_ranges: self
                .row_ranges
                .iter()
                .copied()
                .map(|range| {
                    Some(TableRowRange::new(
                        shift(range.start(), delta)?,
                        shift(range.end(), delta)?,
                    ))
                })
                .collect::<Option<Vec<_>>>()?,
        })
    }

    /// Rebuilds table metadata after a source-only range mapping.  The
    /// projection crate uses this to retain table identity across edits that
    /// occur before the table; cell text is still never copied here.
    #[must_use]
    pub fn from_mapped_ranges(
        source_range: TableCellRange,
        first_row: Vec<TableCellRange>,
        delimiter: Vec<TableCellRange>,
        alignments: Vec<TableAlignment>,
        rows: Vec<Vec<TableCellRange>>,
        row_ranges: Vec<TableRowRange>,
    ) -> Self {
        Self {
            html: false,
            html_grid: None,
            cell_styles: Vec::new(),
            cell_paragraphs: Vec::new(),
            source_range,
            first_row,
            delimiter,
            alignments,
            rows,
            row_ranges,
        }
    }
}

/// Recognizes a conservative GitHub-flavored Markdown table.
///
/// A table must have a header row, a delimiter row with at least three dashes
/// per cell, and rows with the same number of cells. Pipes inside backtick code
/// spans or escaped with a backslash do not split a cell. Returning `None`
/// leaves the source to the normal paragraph exporter.
#[must_use]
pub fn parse_table(source: &str) -> Option<TableBlock> {
    let lines = source_lines(source);
    if lines.len() < 2 || lines.iter().any(|(_, line)| line.trim().is_empty()) {
        return None;
    }

    let header = parse_row(lines[0].0, lines[0].1)?;
    let delimiter = parse_row(lines[1].0, lines[1].1)?;
    if header.is_empty() || delimiter.len() != header.len() {
        return None;
    }

    let alignments = delimiter
        .iter()
        .map(|cell| parse_alignment(source, *cell))
        .collect::<Option<Vec<_>>>()?;

    let mut rows = Vec::new();
    for (start, line) in lines.iter().skip(2).copied() {
        let row = parse_row(start, line)?;
        if row.len() != header.len() {
            return None;
        }
        rows.push(row);
    }

    let row_ranges = lines
        .iter()
        .map(|(start, line)| TableRowRange::new(*start, start.saturating_add(line.len())))
        .collect();
    Some(TableBlock {
        html: false,
        html_grid: None,
        cell_styles: Vec::new(),
        cell_paragraphs: Vec::new(),
        source_range: TableCellRange::new(0, source.len()),
        first_row: header,
        delimiter,
        alignments,
        rows,
        row_ranges,
    })
}

/// Recognizes a table in one immutable snapshot range and translates every
/// parser result to absolute document byte offsets.
///
/// Only the candidate paragraph is materialized temporarily.  The returned
/// `TableBlock` retains ranges into `source`; it never owns cell text.  This
/// keeps table metadata compatible with the same revision-bound projection
/// and edit mapping contracts used by ordinary Markdown blocks.
#[must_use]
pub fn parse_table_in_snapshot(source: &TextSnapshot, range: TextRange) -> Option<TableBlock> {
    let start = usize::try_from(range.start()).ok()?;
    let end = usize::try_from(range.end()).ok()?;
    let source_len = usize::try_from(source.len_bytes()).ok()?;
    if end < start || end > source_len {
        return None;
    }
    if !looks_like_table_prefix(source, range)? {
        return None;
    }
    let mut candidate = String::with_capacity(end.saturating_sub(start));
    let mut chunks = source.chunk_cursor(range.start()).ok()?;
    for chunk in &mut chunks {
        let chunk_start = usize::try_from(chunk.start()).ok()?;
        if chunk_start >= end {
            break;
        }
        let chunk_end = chunk_start.checked_add(chunk.text().len())?;
        let local_start = start.max(chunk_start).saturating_sub(chunk_start);
        let local_end = end.min(chunk_end).saturating_sub(chunk_start);
        if local_start < local_end {
            candidate.push_str(&chunk.text()[local_start..local_end]);
        }
    }
    parse_table(&candidate).map(|table| table.translated(start))
}

fn looks_like_table_prefix(source: &TextSnapshot, range: TextRange) -> Option<bool> {
    let start = usize::try_from(range.start()).ok()?;
    let end = usize::try_from(range.end()).ok()?;
    let mut first_has_pipe = false;
    let mut second_has_pipe = false;
    let mut line = 0_u8;
    let mut chunks = source.chunk_cursor(range.start()).ok()?;
    for chunk in &mut chunks {
        let chunk_start = usize::try_from(chunk.start()).ok()?;
        if chunk_start >= end {
            break;
        }
        let chunk_end = chunk_start.checked_add(chunk.text().len())?;
        let local_start = start.max(chunk_start).saturating_sub(chunk_start);
        let local_end = end.min(chunk_end).saturating_sub(chunk_start);
        for byte in chunk.text().as_bytes()[local_start..local_end]
            .iter()
            .copied()
        {
            match byte {
                b'|' if line == 0 => first_has_pipe = true,
                b'|' if line == 1 => second_has_pipe = true,
                b'\n' if line < 2 => line = line.saturating_add(1),
                _ => {}
            }
            if line >= 2 {
                return Some(first_has_pipe && second_has_pipe);
            }
        }
    }
    Some(line >= 1 && first_has_pipe && second_has_pipe)
}

fn source_lines(source: &str) -> Vec<(usize, &str)> {
    let mut lines = Vec::new();
    let mut start = 0;
    for segment in source.split_inclusive('\n') {
        let mut line = segment.strip_suffix('\n').unwrap_or(segment);
        line = line.strip_suffix('\r').unwrap_or(line);
        lines.push((start, line));
        start += segment.len();
    }
    if start < source.len() {
        let mut line = &source[start..];
        line = line.strip_suffix('\r').unwrap_or(line);
        lines.push((start, line));
    }
    lines
}

/// Exact inline code ranges shared by table splitting and contextual input.
/// A plain prefix keeps cell text in paragraph context during syntax parsing.
fn inline_code_ranges(text: &str) -> Vec<std::ops::Range<usize>> {
    if !text.contains('`') {
        return Vec::new();
    }
    let candidate = format!("x {text}");
    let Ok(parsed) = yu_syntax::parse(candidate.as_str()) else {
        return Vec::new();
    };
    crate::extension::SyntaxNode::new(parsed.tree(), 0)
        .descendants()
        .filter(|node| node.kind() == yu_syntax::NodeKind::InlineCode)
        .map(|node| {
            (node.start() as usize).saturating_sub(2)..(node.end() as usize).saturating_sub(2)
        })
        .collect()
}

fn in_code(ranges: &[std::ops::Range<usize>], offset: usize) -> bool {
    let index = ranges.partition_point(|range| range.end <= offset);
    ranges
        .get(index)
        .is_some_and(|range| range.contains(&offset))
}

/// Protect new cell pipes while retaining already escaped source and native
/// reference code-span behavior. Only the inserted text is returned/changed.
#[must_use]
pub fn quote_table_cell_input(
    cell: &str,
    replaced: std::ops::Range<usize>,
    input: &str,
    separator_follows: bool,
) -> Option<String> {
    let before = cell.get(..replaced.start)?;
    let after = cell.get(replaced.end..)?;
    if replaced.start > replaced.end {
        return None;
    }
    let candidate = format!("{before}{input}{after}");
    let codes = inline_code_ranges(&candidate);
    let mut escaped = before
        .bytes()
        .rev()
        .take_while(|byte| *byte == b'\\')
        .count()
        % 2
        != 0;
    let mut result = String::with_capacity(input.len());
    for (offset, ch) in input.char_indices() {
        if ch == '|' && !escaped && !in_code(&codes, before.len() + offset) {
            result.push('\\');
        }
        result.push(ch);
        escaped = ch == '\\' && !escaped;
    }
    // Do not let a final odd backslash consume the structural delimiter of
    // a compact row such as |x|y|. The extra source slash represents the same
    // single visible character; existing row bytes remain untouched.
    if separator_follows && after.is_empty() && escaped {
        result.push('\\');
    }
    Some(result)
}

fn parse_row(line_start: usize, line: &str) -> Option<Vec<TableCellRange>> {
    if !line.contains('|') {
        return None;
    }

    let leading = line.len().saturating_sub(line.trim_start().len());
    let trailing = line.trim_end().len();
    let mut start = leading;
    let mut end = trailing;
    if line.as_bytes().get(start) == Some(&b'|') {
        start += 1;
    }
    if end > start && line.as_bytes().get(end - 1) == Some(&b'|') {
        end -= 1;
    }
    if start > end {
        return None;
    }

    let mut cells = Vec::new();
    let mut cell_start = start;
    let mut escaped = false;
    let codes = inline_code_ranges(line);
    for (offset, character) in line[start..end].char_indices() {
        let absolute = start + offset;
        if escaped {
            escaped = false;
            continue;
        }
        if character == '\\' {
            escaped = true;
            continue;
        }
        if character == '|' && !in_code(&codes, absolute) {
            cells.push(trimmed_cell(line_start, line, cell_start, absolute));
            cell_start = absolute + character.len_utf8();
        }
    }
    cells.push(trimmed_cell(line_start, line, cell_start, end));
    Some(cells)
}

fn trimmed_cell(line_start: usize, line: &str, start: usize, end: usize) -> TableCellRange {
    let value = &line[start..end];
    let left = value.len().saturating_sub(value.trim_start().len());
    let right = value.trim_end().len();
    let trimmed_start = line_start + start + left;
    let trimmed_end = line_start + start + right;
    if trimmed_start <= trimmed_end {
        TableCellRange::new(trimmed_start, trimmed_end)
    } else {
        // A whitespace-only cell has no source content. Keep a valid empty
        // range at the start of the trimmed area so projection can hide all
        // structural bytes without inventing cell text.
        TableCellRange::new(trimmed_end, trimmed_end)
    }
}

fn parse_alignment(source: &str, cell: TableCellRange) -> Option<TableAlignment> {
    let value = &source[cell.start()..cell.end()];
    let bytes = value.as_bytes();
    if bytes.len() < 3 {
        return None;
    }
    let left = bytes.first() == Some(&b':');
    let right = bytes.last() == Some(&b':');
    let start = usize::from(left);
    let end = bytes.len().saturating_sub(usize::from(right));
    if end.saturating_sub(start) < 3 || !bytes[start..end].iter().all(|byte| *byte == b'-') {
        return None;
    }
    Some(match (left, right) {
        (true, true) => TableAlignment::Center,
        (true, false) => TableAlignment::Left,
        (false, true) => TableAlignment::Right,
        (false, false) => TableAlignment::Default,
    })
}

/// 一个字节偏移平移 `delta`。越界返回 `None`。
fn shift(offset: usize, delta: i64) -> Option<usize> {
    usize::try_from(i64::try_from(offset).ok()?.checked_add(delta)?).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use yu_text::TextBuffer;

    #[test]
    fn recognizes_ranges_alignment_and_escaped_pipes() {
        let source = "| A | `x|y` | C\\|D |\n| :--- | :---: | ---: |\n| 1 | **2** | 3 |\n";
        let table = parse_table(source).expect("table should parse");

        assert_eq!(table.first_row().len(), 3);
        assert_eq!(
            table.alignments(),
            &[
                TableAlignment::Left,
                TableAlignment::Center,
                TableAlignment::Right
            ]
        );
        assert_eq!(table.rows().len(), 1);
        let header = table
            .first_row()
            .iter()
            .map(|range| &source[range.start()..range.end()])
            .collect::<Vec<_>>();
        assert_eq!(header, ["A", "`x|y`", "C\\|D"]);
    }

    #[test]
    fn whitespace_only_cells_have_valid_empty_ranges() {
        let table =
            parse_table("| A | B |\r\n| --- | --- |\r\n|  | x |\r\n").expect("table should parse");
        assert_eq!(table.rows()[0][0].start(), table.rows()[0][0].end());
    }

    #[test]
    fn rejects_non_table_paragraphs_and_mismatched_rows() {
        assert!(parse_table("one | two\nthree | four\n").is_none());
        assert!(parse_table("| A | B |\n| --- | --- |\n| 1 |\n").is_none());
        assert!(parse_table("| A | B |\n| -- | --- |\n| 1 | 2 |\n").is_none());
    }

    #[test]
    fn snapshot_parser_translates_cell_and_row_ranges_without_copying_document_source() {
        let prefix = "前言\n\n";
        let table_source = "| A | B |\n| --- | :---: |\n| 1 | 2 |\n";
        let suffix = "\n尾部";
        let source = format!("{prefix}{table_source}{suffix}");
        let buffer = TextBuffer::new(&source);
        let start = prefix.len();
        let end = start + table_source.len();
        let range = TextRange::new(
            yu_core::ByteOffset::try_from(start).expect("start fits"),
            yu_core::ByteOffset::try_from(end).expect("end fits"),
        )
        .expect("range should be ordered");
        let table = parse_table_in_snapshot(&buffer.snapshot(), range).expect("table");

        assert_eq!(table.source_range(), TableCellRange::new(start, end));
        assert_eq!(table.row_ranges().len(), 3);
        assert_eq!(
            table.first_row()[0],
            TableCellRange::new(start + 2, start + 3)
        );
        assert_eq!(
            table.delimiter()[1],
            TableCellRange::new(start + 18, start + 23)
        );
        assert_eq!(
            table.rows()[0][1],
            TableCellRange::new(start + 32, start + 33)
        );
        assert_eq!(table.column_count(), 2);
        assert_eq!(table.body_row_count(), 1);
    }

    #[test]
    fn visible_cell_navigation_skips_delimiter_row() {
        let source = "| A | B |\n| --- | --- |\n| 1 | 2 |\n| 3 | 4 |\n";
        let table = parse_table(source).expect("table");

        assert_eq!(table.visible_row_count(), 3);
        let first = TableCellAddress::new(0, 0);
        assert_eq!(table.visible_cell(first), Some(TableCellRange::new(2, 3)));
        let next = table.next_visible_cell(first).expect("next cell");
        assert_eq!(next.0, TableCellAddress::new(0, 1));
        assert_eq!(next.1, TableCellRange::new(6, 7));
        let body = table.next_visible_cell(next.0).expect("first body cell");
        assert_eq!(body.0, TableCellAddress::new(1, 0));
        assert_eq!(body.1, TableCellRange::new(26, 27));
        assert_eq!(
            table.visible_cell_for_source(30),
            Some(TableCellAddress::new(1, 1))
        );
        assert_eq!(
            table.previous_visible_cell(body.0),
            Some((TableCellAddress::new(0, 1), TableCellRange::new(6, 7)))
        );
    }
}
