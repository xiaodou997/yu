use unicode_segmentation::UnicodeSegmentation;
use yu_core::{ByteOffset, Revision, TextRange};
use yu_markdown::{TableAlignment, TableCellRange};
use yu_projection::{ProjectionError, TableProjection, VisualRunStyle};
use yu_text::{ChangeSet, TextSnapshot};

use crate::{
    ClusterMetrics, LayoutConfig, LayoutError, LayoutPoint, LayoutRect, ShapingProvider,
    map_source_range,
};

/// Geometry for one visible GFM table cell. The row index is visual-table
/// based: `0` is the header and body rows start at `1`; the Markdown
/// delimiter row is intentionally not a visible cell row.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TableCellLayout {
    row: usize,
    column: usize,
    source: TextRange,
    bounds: LayoutRect,
    alignment: TableAlignment,
}

impl TableCellLayout {
    #[must_use]
    pub const fn row(self) -> usize {
        self.row
    }

    #[must_use]
    pub const fn column(self) -> usize {
        self.column
    }

    #[must_use]
    pub const fn source(self) -> TextRange {
        self.source
    }

    #[must_use]
    pub const fn bounds(self) -> LayoutRect {
        self.bounds
    }

    #[must_use]
    pub const fn alignment(self) -> TableAlignment {
        self.alignment
    }
}

/// A source-backed table cell hit. The cell source range can be selected or
/// edited through the normal editor transaction path; the layout never owns
/// cell text.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TableLayoutHit {
    row: usize,
    column: usize,
    source: TextRange,
    bounds: LayoutRect,
    point: LayoutPoint,
}

impl TableLayoutHit {
    #[must_use]
    pub const fn row(self) -> usize {
        self.row
    }

    #[must_use]
    pub const fn column(self) -> usize {
        self.column
    }

    #[must_use]
    pub const fn source(self) -> TextRange {
        self.source
    }

    #[must_use]
    pub const fn bounds(self) -> LayoutRect {
        self.bounds
    }

    #[must_use]
    pub const fn point(self) -> LayoutPoint {
        self.point
    }
}

/// A revision-bound table layout independent of scene/GPU painting.
///
/// The delimiter row is a parser-owned source range but has no visible cell.
/// Column widths are measured from the source-backed cell contents and capped
/// to the supplied block width. A later scene layer can consume the same cell
/// rectangles to draw borders and selection overlays.
#[derive(Clone, Debug, PartialEq)]
pub struct TableLayoutSnapshot {
    revision: Revision,
    source_range: TextRange,
    delimiter_source: Option<TextRange>,
    column_widths: Vec<f32>,
    row_height: f32,
    bounds: LayoutRect,
    cells: Vec<TableCellLayout>,
}

impl TableLayoutSnapshot {
    pub fn from_projection<M: ClusterMetrics>(
        projection: &TableProjection,
        config: LayoutConfig,
        metrics: &M,
    ) -> Result<Self, LayoutError> {
        Self::from_projection_with_measure(projection, config, |text, _source| {
            measure_text(text, metrics)
        })
    }

    pub fn from_projection_with_shaper<S: ShapingProvider>(
        projection: &TableProjection,
        config: LayoutConfig,
        shaper: &S,
    ) -> Result<Self, LayoutError> {
        Self::from_projection_with_measure(projection, config, |text, source| {
            shaper
                .shape(text, source, VisualRunStyle::Plain)
                .map(|shaped| shaped.advance())
                .map_err(|error| LayoutError::Shaping(error.to_string()))
        })
    }

    fn from_projection_with_measure<F>(
        projection: &TableProjection,
        config: LayoutConfig,
        mut measure: F,
    ) -> Result<Self, LayoutError>
    where
        F: FnMut(&str, TextRange) -> Result<f32, LayoutError>,
    {
        config.validate()?;
        let table = projection.table();
        let column_count = table.column_count();
        if column_count == 0 || table.delimiter().len() != column_count {
            return Err(LayoutError::InvalidTable("table columns are inconsistent"));
        }

        let mut rows = Vec::with_capacity(table.body_row_count().saturating_add(1));
        rows.push(table.header().to_vec());
        rows.extend(table.rows().iter().cloned());
        if rows.iter().any(|row| row.len() != column_count) {
            return Err(LayoutError::InvalidTable("table body row is inconsistent"));
        }

        let source = projection.visual().source();
        let padding = config.default_advance();
        let mut natural_widths = vec![padding * 2.0; column_count];
        for row in &rows {
            for (column, cell) in row.iter().copied().enumerate() {
                let source_range = table_cell_range(cell)?;
                let text = read_source_range(source, source_range)?;
                let content_width = measure(&text, source_range)?;
                if !content_width.is_finite() || content_width < 0.0 {
                    return Err(LayoutError::InvalidMetrics(content_width.to_bits()));
                }
                natural_widths[column] = natural_widths[column]
                    .max(content_width + padding * 2.0)
                    .max(padding * 2.0);
            }
        }

        let natural_total = natural_widths.iter().sum::<f32>();
        if !natural_total.is_finite() || natural_total <= 0.0 {
            return Err(LayoutError::InvalidTable("table width is not finite"));
        }
        let scale = (config.max_width() / natural_total).min(1.0);
        let column_widths = natural_widths
            .into_iter()
            .map(|width| width * scale)
            .collect::<Vec<_>>();
        let total_width = column_widths.iter().sum::<f32>();
        let row_height = config.line_height();
        let row_count = rows.len();
        let total_height = row_height * row_count as f32;
        let bounds = LayoutRect::new(0.0, 0.0, total_width, total_height)?;

        let alignments = table.alignments();
        let mut cells = Vec::with_capacity(row_count.saturating_mul(column_count));
        for (row, source_cells) in rows.iter().enumerate() {
            let y = row_height * row as f32;
            let mut x = 0.0_f32;
            for (column, cell) in source_cells.iter().copied().enumerate() {
                let width = column_widths[column];
                cells.push(TableCellLayout {
                    row,
                    column,
                    source: table_cell_range(cell)?,
                    bounds: LayoutRect::new(x, y, width, row_height)?,
                    alignment: alignments[column],
                });
                x += width;
            }
        }

        Ok(Self {
            revision: projection.revision(),
            source_range: projection.source_range(),
            delimiter_source: table
                .delimiter_source_range()
                .map(table_cell_range)
                .transpose()?,
            column_widths,
            row_height,
            bounds,
            cells,
        })
    }

    #[must_use]
    pub const fn revision(&self) -> Revision {
        self.revision
    }

    #[must_use]
    pub const fn source_range(&self) -> TextRange {
        self.source_range
    }

    #[must_use]
    pub const fn delimiter_source(&self) -> Option<TextRange> {
        self.delimiter_source
    }

    #[must_use]
    pub fn column_widths(&self) -> &[f32] {
        &self.column_widths
    }

    #[must_use]
    pub const fn row_height(&self) -> f32 {
        self.row_height
    }

    #[must_use]
    pub const fn bounds(&self) -> LayoutRect {
        self.bounds
    }

    #[must_use]
    pub fn cells(&self) -> &[TableCellLayout] {
        &self.cells
    }

    /// Returns the visible cell containing `point`, or `None` outside the
    /// table bounds. The returned source range is the original Markdown cell
    /// content, not a copied visual string.
    pub fn hit_test(&self, point: LayoutPoint) -> Result<Option<TableLayoutHit>, LayoutError> {
        point.validate()?;
        if point.x() < self.bounds.x()
            || point.y() < self.bounds.y()
            || point.x() > self.bounds.x() + self.bounds.width()
            || point.y() >= self.bounds.y() + self.bounds.height()
        {
            return Ok(None);
        }
        let row = ((point.y() - self.bounds.y()) / self.row_height).floor() as usize;
        let row = row.min(self.cells.len().saturating_sub(1));
        let column = self
            .cells
            .iter()
            .find(|cell| {
                cell.row == row
                    && point.x() >= cell.bounds.x()
                    && point.x() < cell.bounds.x() + cell.bounds.width()
            })
            .or_else(|| {
                self.cells
                    .iter()
                    .rev()
                    .find(|cell| cell.row == row && point.x() == self.bounds.width())
            });
        Ok(column.map(|cell| TableLayoutHit {
            row: cell.row,
            column: cell.column,
            source: cell.source,
            bounds: cell.bounds,
            point,
        }))
    }

    pub fn map_through(
        &self,
        changes: &ChangeSet,
        snapshot: &TextSnapshot,
    ) -> Result<Self, LayoutError> {
        let source_range = map_source_range(self.source_range, changes)?;
        let delimiter_source = self
            .delimiter_source
            .map(|range| map_source_range(range, changes))
            .transpose()?;
        let cells = self
            .cells
            .iter()
            .map(|cell| {
                Ok(TableCellLayout {
                    row: cell.row,
                    column: cell.column,
                    source: map_source_range(cell.source, changes)?,
                    bounds: cell.bounds,
                    alignment: cell.alignment,
                })
            })
            .collect::<Result<Vec<_>, LayoutError>>()?;
        snapshot
            .utf16_offset(source_range.start())
            .map_err(|error| LayoutError::Projection(ProjectionError::SourcePosition(error)))?;
        Ok(Self {
            revision: snapshot.revision(),
            source_range,
            delimiter_source,
            column_widths: self.column_widths.clone(),
            row_height: self.row_height,
            bounds: self.bounds,
            cells,
        })
    }
}

fn table_cell_range(range: TableCellRange) -> Result<TextRange, LayoutError> {
    let start = ByteOffset::try_from(range.start()).map_err(|_| LayoutError::OffsetOverflow)?;
    let end = ByteOffset::try_from(range.end()).map_err(|_| LayoutError::OffsetOverflow)?;
    TextRange::new(start, end).ok_or(LayoutError::OffsetOverflow)
}

fn read_source_range(source: &TextSnapshot, range: TextRange) -> Result<String, LayoutError> {
    let start = usize::try_from(range.start()).map_err(|_| LayoutError::OffsetOverflow)?;
    let end = usize::try_from(range.end()).map_err(|_| LayoutError::OffsetOverflow)?;
    let mut bytes = Vec::with_capacity(end.saturating_sub(start));
    let mut chunks = source
        .chunk_cursor(range.start())
        .map_err(|error| LayoutError::Projection(ProjectionError::SourcePosition(error)))?;
    for chunk in &mut chunks {
        let chunk_start =
            usize::try_from(chunk.start()).map_err(|_| LayoutError::OffsetOverflow)?;
        let chunk_end = chunk_start
            .checked_add(chunk.text().len())
            .ok_or(LayoutError::OffsetOverflow)?;
        if chunk_start >= end {
            break;
        }
        let local_start = start.max(chunk_start).saturating_sub(chunk_start);
        let local_end = end.min(chunk_end).saturating_sub(chunk_start);
        if local_start < local_end {
            bytes.extend_from_slice(&chunk.text().as_bytes()[local_start..local_end]);
        }
    }
    String::from_utf8(bytes).map_err(|_| LayoutError::OffsetOverflow)
}

fn measure_text<M: ClusterMetrics>(text: &str, metrics: &M) -> Result<f32, LayoutError> {
    let mut width = 0.0_f32;
    for cluster in text.graphemes(true) {
        let advance = metrics.advance(cluster, VisualRunStyle::Plain);
        if !advance.is_finite() || advance < 0.0 {
            return Err(LayoutError::InvalidMetrics(advance.to_bits()));
        }
        width += advance;
    }
    Ok(width)
}
