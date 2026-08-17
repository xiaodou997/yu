use unicode_segmentation::UnicodeSegmentation;
use yu_core::{ByteOffset, Revision, TextRange};
use yu_markdown::{TableAlignment, TableCellRange};
use yu_projection::{
    Projection, ProjectionBias, ProjectionError, TableProjection, VisualOffset, VisualRange,
    VisualRunKind, VisualRunStyle,
};
use yu_text::{ChangeSet, TextSnapshot};

use crate::{
    ClusterMetrics, LayoutConfig, LayoutError, LayoutPoint, LayoutRect, ShapingProvider,
    map_source_range, source_range_contains,
};

/// Geometry for one visible GFM table cell. The row index is visual-table
/// based: `0` is the header and body rows start at `1`; the Markdown
/// delimiter row is intentionally not a visible cell row.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TableCellLayout {
    row: usize,
    column: usize,
    source: TextRange,
    visual: VisualRange,
    bounds: LayoutRect,
    alignment: TableAlignment,
    content_x: f32,
    content_width: f32,
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
    pub const fn visual(self) -> VisualRange {
        self.visual
    }

    #[must_use]
    pub const fn bounds(self) -> LayoutRect {
        self.bounds
    }

    #[must_use]
    pub const fn alignment(self) -> TableAlignment {
        self.alignment
    }

    /// Returns the x coordinate at which the visible cell content begins.
    /// Padding and Markdown alignment are already applied.
    #[must_use]
    pub const fn content_x(self) -> f32 {
        self.content_x
    }

    /// Returns the measured width of the visible cell content, excluding
    /// padding and alignment slack.
    #[must_use]
    pub const fn content_width(self) -> f32 {
        self.content_width
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
    row_sources: Vec<TextRange>,
}

impl TableLayoutSnapshot {
    pub fn from_projection<M: ClusterMetrics>(
        projection: &TableProjection,
        config: LayoutConfig,
        metrics: &M,
    ) -> Result<Self, LayoutError> {
        Self::from_projection_with_measure(projection, config, |text, _source, style| {
            measure_text(text, metrics, style)
        })
    }

    pub fn from_projection_with_shaper<S: ShapingProvider>(
        projection: &TableProjection,
        config: LayoutConfig,
        shaper: &S,
    ) -> Result<Self, LayoutError> {
        Self::from_projection_with_measure(projection, config, |text, source, style| {
            shaper
                .shape(text, source, style)
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
        F: FnMut(&str, TextRange, VisualRunStyle) -> Result<f32, LayoutError>,
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

        let padding = config.default_advance();
        let mut natural_widths = vec![padding * 2.0; column_count];
        let mut measured_rows = Vec::with_capacity(rows.len());
        for row in &rows {
            let mut measured_row = Vec::with_capacity(column_count);
            for (column, cell) in row.iter().copied().enumerate() {
                let source_range = table_cell_range(cell)?;
                let (visual, content_width) =
                    measure_cell_content(projection, source_range, &mut measure)?;
                if !content_width.is_finite() || content_width < 0.0 {
                    return Err(LayoutError::InvalidMetrics(content_width.to_bits()));
                }
                natural_widths[column] = natural_widths[column]
                    .max(content_width + padding * 2.0)
                    .max(padding * 2.0);
                measured_row.push((source_range, visual, content_width));
            }
            measured_rows.push(measured_row);
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
        for (row, measured_cells) in measured_rows.iter().enumerate() {
            let y = row_height * row as f32;
            let mut x = 0.0_f32;
            for (column, (source, visual, content_width)) in
                measured_cells.iter().copied().enumerate()
            {
                let width = column_widths[column];
                let available = (width - padding * 2.0).max(0.0);
                let slack = (available - content_width).max(0.0);
                let alignment_offset = match alignments[column] {
                    TableAlignment::Center => slack * 0.5,
                    TableAlignment::Right => slack,
                    TableAlignment::Default | TableAlignment::Left => 0.0,
                };
                cells.push(TableCellLayout {
                    row,
                    column,
                    source,
                    visual,
                    bounds: LayoutRect::new(x, y, width, row_height)?,
                    alignment: alignments[column],
                    content_x: x + padding + alignment_offset,
                    content_width,
                });
                x += width;
            }
        }

        let row_sources = (0..row_count)
            .map(|row| {
                let physical_row = row.saturating_add(usize::from(row > 0));
                table
                    .row_source_range(physical_row)
                    .ok_or(LayoutError::OffsetOverflow)
                    .and_then(table_cell_range)
            })
            .collect::<Result<Vec<_>, _>>()?;

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
            row_sources,
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

    #[must_use]
    pub fn row_sources(&self) -> &[TextRange] {
        &self.row_sources
    }

    /// Resolves a visual boundary that lands on a structural table run to a
    /// visible cell source boundary. Interior cell positions return `None` so
    /// the normal projection mapping can preserve per-grapheme precision;
    /// structural pipe/row-ending positions must not return hidden bytes.
    #[must_use]
    pub fn source_for_visual_hit(
        &self,
        projection: &Projection,
        visual: VisualOffset,
        _bias: ProjectionBias,
    ) -> Option<ByteOffset> {
        let structural_hidden = projection.runs().iter().any(|run| {
            run.kind() == VisualRunKind::HiddenSyntax
                && run.visual().is_empty()
                && run.visual().start() == visual
                && !self
                    .cells
                    .iter()
                    .any(|cell| source_range_contains(cell.source(), run.source()))
        });
        if !structural_hidden {
            return None;
        }

        self.cells
            .iter()
            .copied()
            .find(|cell| cell.visual.start() >= visual)
            .map(|cell| cell.source.start())
            .or_else(|| self.cells.last().map(|cell| cell.source.end()))
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
                    visual: cell.visual,
                    bounds: cell.bounds,
                    alignment: cell.alignment,
                    content_x: cell.content_x,
                    content_width: cell.content_width,
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
            row_sources: self
                .row_sources
                .iter()
                .copied()
                .map(|range| map_source_range(range, changes))
                .collect::<Result<Vec<_>, _>>()?,
        })
    }
}

fn table_cell_range(range: TableCellRange) -> Result<TextRange, LayoutError> {
    let start = ByteOffset::try_from(range.start()).map_err(|_| LayoutError::OffsetOverflow)?;
    let end = ByteOffset::try_from(range.end()).map_err(|_| LayoutError::OffsetOverflow)?;
    TextRange::new(start, end).ok_or(LayoutError::OffsetOverflow)
}

fn measure_cell_content<F>(
    projection: &TableProjection,
    source: TextRange,
    measure: &mut F,
) -> Result<(VisualRange, f32), LayoutError>
where
    F: FnMut(&str, TextRange, VisualRunStyle) -> Result<f32, LayoutError>,
{
    let visual_start = projection
        .visual()
        .source_to_visual(source.start(), ProjectionBias::After)?;
    let visual_end = projection
        .visual()
        .source_to_visual(source.end(), ProjectionBias::Before)?;
    let visual = VisualRange::new(visual_start, visual_end).ok_or(LayoutError::OffsetOverflow)?;
    let mut width = 0.0_f32;
    for run in projection.visual().runs().iter().copied() {
        if matches!(
            run.kind(),
            VisualRunKind::HiddenSyntax | VisualRunKind::LineBreak { .. }
        ) {
            continue;
        }
        if !source_range_contains(source, run.source()) {
            continue;
        }
        let overlaps = if visual.is_empty() {
            run.visual().is_empty() && run.visual().start() == visual.start()
        } else {
            run.visual().start() < visual.end() && visual.start() < run.visual().end()
        };
        if !overlaps {
            continue;
        }
        let text = projection.visual().text_for_run(run)?;
        let shape_source = projection.visual().shape_source_range_for_run(run);
        width += measure(&text, shape_source, run.style())?;
    }
    Ok((visual, width))
}

fn measure_text<M: ClusterMetrics>(
    text: &str,
    metrics: &M,
    style: VisualRunStyle,
) -> Result<f32, LayoutError> {
    let mut width = 0.0_f32;
    for cluster in text.graphemes(true) {
        let advance = metrics.advance(cluster, style);
        if !advance.is_finite() || advance < 0.0 {
            return Err(LayoutError::InvalidMetrics(advance.to_bits()));
        }
        width += advance;
    }
    Ok(width)
}
