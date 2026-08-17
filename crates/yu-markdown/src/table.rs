//! Conservative GFM table recognition over one paragraph source range.
//!
//! The block scanner intentionally keeps tables as paragraphs for now.  Table
//! recognition is therefore a projection concern, but all metadata remains
//! source-backed: the parser returns byte ranges into the canonical snapshot
//! instead of copying cell text into an editor document model.

use yu_core::TextRange;
use yu_text::TextSnapshot;

/// Alignment requested by a GFM table delimiter cell.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum TableAlignment {
    Default,
    Left,
    Center,
    Right,
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

/// A recognized table whose cells refer to the supplied source string.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TableBlock {
    source_range: TableCellRange,
    header: Vec<TableCellRange>,
    delimiter: Vec<TableCellRange>,
    alignments: Vec<TableAlignment>,
    rows: Vec<Vec<TableCellRange>>,
    row_ranges: Vec<TableRowRange>,
}

impl TableBlock {
    #[must_use]
    pub const fn source_range(&self) -> TableCellRange {
        self.source_range
    }

    #[must_use]
    pub fn header(&self) -> &[TableCellRange] {
        &self.header
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
        let end = self
            .row_ranges
            .get(row.saturating_add(1))
            .map_or(self.source_range.end(), |next| next.start());
        Some(TableCellRange::new(current.start(), end))
    }

    #[must_use]
    pub fn delimiter_source_range(&self) -> Option<TableCellRange> {
        self.row_source_range(1)
    }

    #[must_use]
    pub fn column_count(&self) -> usize {
        self.header.len()
    }

    #[must_use]
    pub fn body_row_count(&self) -> usize {
        self.rows.len()
    }

    #[must_use]
    pub(crate) fn translated(self, offset: usize) -> Self {
        Self {
            source_range: self.source_range.translated(offset),
            header: self
                .header
                .into_iter()
                .map(|range| range.translated(offset))
                .collect(),
            delimiter: self
                .delimiter
                .into_iter()
                .map(|range| range.translated(offset))
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

    /// Rebuilds table metadata after a source-only range mapping.  The
    /// projection crate uses this to retain table identity across edits that
    /// occur before the table; cell text is still never copied here.
    #[must_use]
    pub fn from_mapped_ranges(
        source_range: TableCellRange,
        header: Vec<TableCellRange>,
        delimiter: Vec<TableCellRange>,
        alignments: Vec<TableAlignment>,
        rows: Vec<Vec<TableCellRange>>,
        row_ranges: Vec<TableRowRange>,
    ) -> Self {
        Self {
            source_range,
            header,
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
    if header.len() < 2 || delimiter.len() != header.len() {
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
        source_range: TableCellRange::new(0, source.len()),
        header,
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
    let mut in_code = false;
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
        if character == '`' {
            in_code = !in_code;
            continue;
        }
        if character == '|' && !in_code {
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
    TableCellRange::new(line_start + start + left, line_start + start + right)
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

#[cfg(test)]
mod tests {
    use super::*;
    use yu_text::TextBuffer;

    #[test]
    fn recognizes_ranges_alignment_and_escaped_pipes() {
        let source = "| A | `x|y` | C\\|D |\n| :--- | :---: | ---: |\n| 1 | **2** | 3 |\n";
        let table = parse_table(source).expect("table should parse");

        assert_eq!(table.header().len(), 3);
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
            .header()
            .iter()
            .map(|range| &source[range.start()..range.end()])
            .collect::<Vec<_>>();
        assert_eq!(header, ["A", "`x|y`", "C\\|D"]);
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
        assert_eq!(table.header()[0], TableCellRange::new(start + 2, start + 3));
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
}
