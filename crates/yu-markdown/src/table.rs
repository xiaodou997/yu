//! Conservative GFM table recognition over one paragraph source range.
//!
//! The block scanner intentionally keeps tables as paragraphs for now so the
//! editor/projection contracts do not acquire a new block kind prematurely.
//! This module still centralizes the table grammar and returns source-relative
//! cell ranges for consumers such as semantic HTML export.

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
}

/// A recognized table whose cells refer to the supplied source string.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TableBlock {
    header: Vec<TableCellRange>,
    alignments: Vec<TableAlignment>,
    rows: Vec<Vec<TableCellRange>>,
}

impl TableBlock {
    #[must_use]
    pub fn header(&self) -> &[TableCellRange] {
        &self.header
    }

    #[must_use]
    pub fn alignments(&self) -> &[TableAlignment] {
        &self.alignments
    }

    #[must_use]
    pub fn rows(&self) -> &[Vec<TableCellRange>] {
        &self.rows
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

    Some(TableBlock {
        header,
        alignments,
        rows,
    })
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
}
