//! GFM 表格的几何。
//!
//! # 为什么它住在 `yu-editor`
//!
//! 它此前住在 `yu-layout`（`table.rs`，919 行）。表格是 Markdown 的语法，
//! 列宽按单元格内容算、对齐按 `TableAlignment` 走——布局层为此必须认识
//! GFM，那是不变量 E1 禁止的。`table` 不在 E1 那条 grep 的关键词里，但
//! `TableLayout` 与 `yu-scene::TablePrimitive` 是同一种泄漏，按
//! overview-v2 §3 的对照表处理，不按 grep 处理。
//!
//! 搬过来时算法一个字没改：v1 在真实窗口里跑过，重写它不会更对，只会
//! 更容易错。它在 S6 随 `DecorationSet` 变成一个真正的 block widget，
//! 那时列宽算法会成为 widget measurer 的内部实现。

use std::{error::Error, fmt};

use unicode_segmentation::UnicodeSegmentation;
use yu_core::{
    ByteOffset, ClusterMetrics, Revision, ShapingProvider, TextRange, TextStyle, VisualOffset,
    VisualRange,
};
use yu_decoration::Bias;
use yu_markdown::{TableAlignment, TableBlock, TableCellRange};

use yu_layout::{LayoutConfig, LayoutError, LayoutPoint, LayoutRect};

use crate::blockinput::BlockLayoutInput;
use crate::blockview::shift_range;
use crate::geometry::{source_range_contains, upstream};
use crate::visual::VisualText;
use yu_layout::StyleTable;

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

/// The resize divider selected by a table pointer interaction. Indices refer
/// to the column or row immediately before the divider.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum TableResizeTarget {
    Column { index: usize },
    Row { index: usize },
}

/// A Revision-bound table resize hit. The native layer can use the target and
/// axis position to start a drag without reconstructing table geometry.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TableResizeHit {
    target: TableResizeTarget,
    position: f32,
}

impl TableResizeHit {
    #[must_use]
    pub const fn target(self) -> TableResizeTarget {
        self.target
    }

    /// Returns the x coordinate for a column divider or y coordinate for a
    /// row divider, both in the table-local layout space.
    #[must_use]
    pub const fn position(self) -> f32 {
        self.position
    }
}

/// Errors raised when a native table resize gesture no longer matches the
/// Revision that produced its initial hit-test.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TableResizeGestureError {
    StaleRevision {
        expected: Revision,
        actual: Revision,
    },
    NonFinitePointer(u32),
}

impl fmt::Display for TableResizeGestureError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::StaleRevision { expected, actual } => write!(
                formatter,
                "table resize gesture requires {expected:?}, received {actual:?}"
            ),
            Self::NonFinitePointer(bits) => {
                write!(
                    formatter,
                    "table resize pointer is not finite: {}",
                    f32::from_bits(*bits)
                )
            }
        }
    }
}

impl Error for TableResizeGestureError {}

/// A Revision-bound, source-neutral table resize gesture.
///
/// The gesture stores the pointer anchor separately from the divider position
/// returned by layout. This preserves the small tolerance offset from the
/// mouse-down event while the native caller drags. `finish` returns a commit
/// candidate only; it does not change Markdown source, selection or history.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TableResizeGesture {
    revision: Revision,
    block_index: usize,
    target: TableResizeTarget,
    divider_position: f32,
    start_pointer: f32,
    current_pointer: f32,
}

impl TableResizeGesture {
    pub fn begin(
        revision: Revision,
        block_index: usize,
        hit: TableResizeHit,
        pointer_position: f32,
    ) -> Result<Self, TableResizeGestureError> {
        validate_pointer(pointer_position)?;
        validate_pointer(hit.position())?;
        Ok(Self {
            revision,
            block_index,
            target: hit.target(),
            divider_position: hit.position(),
            start_pointer: pointer_position,
            current_pointer: pointer_position,
        })
    }

    #[must_use]
    pub const fn revision(self) -> Revision {
        self.revision
    }

    #[must_use]
    pub const fn block_index(self) -> usize {
        self.block_index
    }

    #[must_use]
    pub const fn target(self) -> TableResizeTarget {
        self.target
    }

    #[must_use]
    pub const fn start_pointer(self) -> f32 {
        self.start_pointer
    }

    #[must_use]
    pub const fn current_pointer(self) -> f32 {
        self.current_pointer
    }

    /// Returns the pointer displacement since mouse-down.
    #[must_use]
    pub fn delta(self) -> f32 {
        self.current_pointer - self.start_pointer
    }

    /// Returns the proposed divider position in table-local layout space.
    #[must_use]
    pub fn proposed_position(self) -> f32 {
        self.divider_position + self.delta()
    }

    pub fn update(
        &mut self,
        revision: Revision,
        pointer_position: f32,
    ) -> Result<(), TableResizeGestureError> {
        self.ensure_revision(revision)?;
        validate_pointer(pointer_position)?;
        validate_pointer(pointer_position - self.start_pointer)?;
        validate_pointer(self.divider_position + (pointer_position - self.start_pointer))?;
        self.current_pointer = pointer_position;
        Ok(())
    }

    /// Completes the gesture and returns a source-neutral commit candidate.
    pub fn finish(self, revision: Revision) -> Result<TableResizeCommit, TableResizeGestureError> {
        self.ensure_revision(revision)?;
        Ok(self.preview())
    }

    /// Returns the current source-neutral geometry candidate without ending
    /// the gesture. Native hosts can use this for each pointer-move frame and
    /// call [`Self::finish`] only when the pointer is released.
    #[must_use]
    pub fn preview(self) -> TableResizeCommit {
        TableResizeCommit {
            revision: self.revision,
            block_index: self.block_index,
            target: self.target,
            initial_position: self.divider_position,
            final_position: self.proposed_position(),
            delta: self.delta(),
        }
    }

    /// Cancels the gesture without producing a source mutation.
    pub fn cancel(self, revision: Revision) -> Result<(), TableResizeGestureError> {
        self.ensure_revision(revision)
    }

    fn ensure_revision(&self, actual: Revision) -> Result<(), TableResizeGestureError> {
        if self.revision == actual {
            Ok(())
        } else {
            Err(TableResizeGestureError::StaleRevision {
                expected: self.revision,
                actual,
            })
        }
    }
}

/// The result of releasing a table resize pointer. It describes geometry only;
/// a later editor transaction must decide whether and how that geometry can
/// be represented in Markdown before changing the canonical source.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TableResizeCommit {
    revision: Revision,
    block_index: usize,
    target: TableResizeTarget,
    initial_position: f32,
    final_position: f32,
    delta: f32,
}

impl TableResizeCommit {
    #[must_use]
    pub const fn revision(self) -> Revision {
        self.revision
    }

    #[must_use]
    pub const fn block_index(self) -> usize {
        self.block_index
    }

    #[must_use]
    pub const fn target(self) -> TableResizeTarget {
        self.target
    }

    #[must_use]
    pub const fn initial_position(self) -> f32 {
        self.initial_position
    }

    #[must_use]
    pub const fn final_position(self) -> f32 {
        self.final_position
    }

    #[must_use]
    pub const fn delta(self) -> f32 {
        self.delta
    }
}

fn validate_pointer(pointer: f32) -> Result<(), TableResizeGestureError> {
    if pointer.is_finite() {
        Ok(())
    } else {
        Err(TableResizeGestureError::NonFinitePointer(pointer.to_bits()))
    }
}

/// A revision-bound table layout independent of scene/GPU painting.
///
/// The delimiter row is a parser-owned source range but has no visible cell.
/// Column widths are measured from the source-backed cell contents and capped
/// to the supplied block width. A later scene layer can consume the same cell
/// rectangles to draw borders and selection overlays.
#[derive(Clone, Debug, PartialEq)]
pub struct TableLayout {
    revision: Revision,
    source_range: TextRange,
    delimiter_source: Option<TextRange>,
    column_widths: Vec<f32>,
    padding: f32,
    row_height: f32,
    bounds: LayoutRect,
    cells: Vec<TableCellLayout>,
    row_sources: Vec<TextRange>,
}

impl TableLayout {
    /// 按一张已经认出来的网格排几何。
    ///
    /// 网格从 `BlockOrnament::Table` 来（`yu-markdown` 的 table extension
    /// 产的），单元格内容的宽度从**已经排好的文字流**来：`input` 里的样式段
    /// 按单元格的视觉区间切一刀就是这一格的内容。自己再读一遍源码、再判一遍
    /// 字型，就是把装配那一层重写一遍。
    ///
    /// # Errors
    ///
    /// 网格的行列对不上、度量非有限值、视觉偏移越界。
    pub fn from_table<F>(
        table: &TableBlock,
        visual: &VisualText,
        input: &BlockLayoutInput,
        config: LayoutConfig,
        mut measure: F,
    ) -> Result<Self, LayoutError>
    where
        F: FnMut(&str, TextRange, TextStyle) -> Result<f32, LayoutError>,
    {
        config.validate()?;
        let column_count = table.column_count();
        if column_count == 0 || table.delimiter().len() != column_count {
            return Err(LayoutError::Upstream(
                "table columns are inconsistent".into(),
            ));
        }

        let mut rows = Vec::with_capacity(table.body_row_count().saturating_add(1));
        rows.push(table.header().to_vec());
        rows.extend(table.rows().iter().cloned());
        if rows.iter().any(|row| row.len() != column_count) {
            return Err(LayoutError::Upstream(
                "table body row is inconsistent".into(),
            ));
        }

        let padding = config.default_advance();
        let mut natural_widths = vec![padding * 2.0; column_count];
        let mut measured_rows = Vec::with_capacity(rows.len());
        for row in &rows {
            let mut measured_row = Vec::with_capacity(column_count);
            for (column, cell) in row.iter().copied().enumerate() {
                let source_range = table_cell_range(cell)?;
                let (cell_visual, content_width) =
                    measure_cell_content(visual, input, source_range, &mut measure)?;
                if !content_width.is_finite() || content_width < 0.0 {
                    return Err(LayoutError::InvalidMetrics(content_width.to_bits()));
                }
                natural_widths[column] = natural_widths[column]
                    .max(content_width + padding * 2.0)
                    .max(padding * 2.0);
                measured_row.push((source_range, cell_visual, content_width));
            }
            measured_rows.push(measured_row);
        }

        let natural_total = natural_widths.iter().sum::<f32>();
        if !natural_total.is_finite() || natural_total <= 0.0 {
            return Err(LayoutError::Upstream("table width is not finite".into()));
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
            revision: visual.revision(),
            source_range: visual.source_range(),
            delimiter_source: table
                .delimiter_source_range()
                .map(table_cell_range)
                .transpose()?,
            column_widths,
            padding,
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

    /// Returns a copy with one internal column divider moved by `delta`.
    ///
    /// This is deliberately a geometry-only operation. It preserves the
    /// table's total width, keeps both adjacent columns usable, and leaves all
    /// source/visual ranges untouched. Row geometry remains unchanged because
    /// the current table layout contract still uses a uniform row height.
    pub fn resized_columns(&self, index: usize, delta: f32) -> Result<Self, LayoutError> {
        if !delta.is_finite() {
            return Err(LayoutError::Upstream(
                "table column resize delta must be finite".into(),
            ));
        }
        let Some(right_index) = index.checked_add(1) else {
            return Err(LayoutError::Upstream(
                "table column resize index is out of bounds".into(),
            ));
        };
        if right_index >= self.column_widths.len() {
            return Err(LayoutError::Upstream(
                "table column resize index is out of bounds".into(),
            ));
        }

        let left = self.column_widths[index];
        let right = self.column_widths[right_index];
        let pair_width = left + right;
        if !left.is_finite() || !right.is_finite() || !pair_width.is_finite() || pair_width <= 0.0 {
            return Err(LayoutError::Upstream(
                "table column widths are not finite".into(),
            ));
        }
        let minimum = (self.padding * 2.0).min(pair_width * 0.5).max(f32::EPSILON);
        let requested_left = left + delta;
        if !requested_left.is_finite() {
            return Err(LayoutError::Upstream(
                "table column resize result is not finite".into(),
            ));
        }
        let new_left = requested_left.clamp(minimum, pair_width - minimum);
        let new_right = pair_width - new_left;
        if !new_right.is_finite() || new_right < minimum {
            return Err(LayoutError::Upstream(
                "table column resize result is invalid".into(),
            ));
        }

        let mut widths = self.column_widths.clone();
        widths[index] = new_left;
        widths[right_index] = new_right;
        self.with_column_widths(widths)
    }

    fn with_column_widths(&self, column_widths: Vec<f32>) -> Result<Self, LayoutError> {
        if column_widths.is_empty()
            || column_widths
                .iter()
                .any(|width| !width.is_finite() || *width <= 0.0)
        {
            return Err(LayoutError::Upstream(
                "table column widths are invalid".into(),
            ));
        }
        let total_width = column_widths.iter().sum::<f32>();
        if !total_width.is_finite() || total_width <= 0.0 {
            return Err(LayoutError::Upstream("table width is not finite".into()));
        }
        let mut starts = Vec::with_capacity(column_widths.len());
        let mut x = self.bounds.x();
        for width in column_widths.iter().copied() {
            starts.push(x);
            x += width;
        }

        let cells = self
            .cells
            .iter()
            .copied()
            .map(|cell| {
                let width =
                    column_widths
                        .get(cell.column)
                        .copied()
                        .ok_or(LayoutError::Upstream(
                            "table cell column is out of bounds".into(),
                        ))?;
                let column_x = starts
                    .get(cell.column)
                    .copied()
                    .ok_or(LayoutError::Upstream(
                        "table cell column is out of bounds".into(),
                    ))?;
                let available = (width - self.padding * 2.0).max(0.0);
                let slack = (available - cell.content_width).max(0.0);
                let alignment_offset = match cell.alignment {
                    TableAlignment::Center => slack * 0.5,
                    TableAlignment::Right => slack,
                    TableAlignment::Default | TableAlignment::Left => 0.0,
                };
                Ok(TableCellLayout {
                    row: cell.row,
                    column: cell.column,
                    source: cell.source,
                    visual: cell.visual,
                    bounds: LayoutRect::new(
                        column_x,
                        cell.bounds.y(),
                        width,
                        cell.bounds.height(),
                    )?,
                    alignment: cell.alignment,
                    content_x: column_x + self.padding + alignment_offset,
                    content_width: cell.content_width,
                })
            })
            .collect::<Result<Vec<_>, LayoutError>>()?;
        let bounds = LayoutRect::new(
            self.bounds.x(),
            self.bounds.y(),
            total_width,
            self.bounds.height(),
        )?;

        Ok(Self {
            revision: self.revision,
            source_range: self.source_range,
            delimiter_source: self.delimiter_source,
            column_widths,
            padding: self.padding,
            row_height: self.row_height,
            bounds,
            cells,
            row_sources: self.row_sources.clone(),
        })
    }

    /// 落在结构性隐藏区间上的视觉边界解析到哪个单元格。
    ///
    /// 单元格**内部**的位置返回 `None`，交给规范映射——那条能保持逐字素的
    /// 精度。竖线、行尾这些结构位置不能返回被隐藏的字节：光标会停在一个
    /// 看不见的 `|` 上。
    #[must_use]
    pub fn source_for_visual_hit(
        &self,
        text: &VisualText,
        visual: VisualOffset,
    ) -> Option<ByteOffset> {
        let structural_hidden = text.decorations().all().iter().any(|entry| {
            entry.decoration.hides_source()
                && !entry.range.is_empty()
                && text.canonical_source_to_visual(entry.range.start()) == visual
                && !self
                    .cells
                    .iter()
                    .any(|cell| source_range_contains(cell.source(), entry.range))
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
        if !point.is_finite() {
            return Err(LayoutError::InvalidPoint);
        }
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

    /// Finds an internal column or row divider within `tolerance` logical
    /// pixels. Outer table edges are deliberately excluded because resizing
    /// the table itself is a separate container/layout concern.
    pub fn resize_hit_test(
        &self,
        point: LayoutPoint,
        tolerance: f32,
    ) -> Result<Option<TableResizeHit>, LayoutError> {
        if !point.is_finite() {
            return Err(LayoutError::InvalidPoint);
        }
        if !tolerance.is_finite() || tolerance < 0.0 {
            return Err(LayoutError::Upstream(
                "resize tolerance must be finite and non-negative".into(),
            ));
        }
        if point.x() < self.bounds.x() - tolerance
            || point.x() > self.bounds.x() + self.bounds.width() + tolerance
            || point.y() < self.bounds.y() - tolerance
            || point.y() > self.bounds.y() + self.bounds.height() + tolerance
        {
            return Ok(None);
        }

        let mut x = self.bounds.x();
        for (index, width) in self
            .column_widths
            .iter()
            .copied()
            .take(self.column_widths.len().saturating_sub(1))
            .enumerate()
        {
            x += width;
            if (point.x() - x).abs() <= tolerance
                && point.y() >= self.bounds.y()
                && point.y() <= self.bounds.y() + self.bounds.height()
            {
                return Ok(Some(TableResizeHit {
                    target: TableResizeTarget::Column { index },
                    position: x,
                }));
            }
        }

        let row_count = self.row_sources.len();
        for row in 0..row_count.saturating_sub(1) {
            let y = self.bounds.y() + self.row_height * (row.saturating_add(1) as f32);
            if (point.y() - y).abs() <= tolerance
                && point.x() >= self.bounds.x()
                && point.x() <= self.bounds.x() + self.bounds.width()
            {
                return Ok(Some(TableResizeHit {
                    target: TableResizeTarget::Row { index: row },
                    position: y,
                }));
            }
        }
        Ok(None)
    }

    /// 整张表的几何平移 `delta` 个字节。
    ///
    /// 只有源码区间会动：编辑落在块外，网格与列宽一个像素都不变。
    pub(crate) fn shifted(&self, delta: i64) -> Result<Self, LayoutError> {
        Ok(Self {
            revision: self.revision,
            source_range: shift_range(self.source_range, delta)?,
            delimiter_source: self
                .delimiter_source
                .map(|range| shift_range(range, delta))
                .transpose()?,
            column_widths: self.column_widths.clone(),
            padding: self.padding,
            row_height: self.row_height,
            bounds: self.bounds,
            cells: self
                .cells
                .iter()
                .map(|cell| {
                    Ok(TableCellLayout {
                        source: shift_range(cell.source, delta)?,
                        ..*cell
                    })
                })
                .collect::<Result<Vec<_>, LayoutError>>()?,
            row_sources: self
                .row_sources
                .iter()
                .copied()
                .map(|range| shift_range(range, delta))
                .collect::<Result<Vec<_>, _>>()?,
        })
    }
}

fn table_cell_range(range: TableCellRange) -> Result<TextRange, LayoutError> {
    let start = ByteOffset::try_from(range.start()).map_err(|_| LayoutError::OffsetOverflow)?;
    let end = ByteOffset::try_from(range.end()).map_err(|_| LayoutError::OffsetOverflow)?;
    TextRange::new(start, end).ok_or(LayoutError::OffsetOverflow)
}

/// 一个单元格的视觉区间与它内容的宽度。
///
/// 内容从**已经排好的样式段**里切：单元格的两端各问一次映射，落在这一段
/// 里的样式段按视觉偏移裁一刀，就是这一格实际画出来的文字与它的字型。
fn measure_cell_content<F>(
    text: &VisualText,
    input: &BlockLayoutInput,
    source: TextRange,
    measure: &mut F,
) -> Result<(VisualRange, f32), LayoutError>
where
    F: FnMut(&str, TextRange, TextStyle) -> Result<f32, LayoutError>,
{
    let visual_start = text
        .source_to_visual(source.start(), Bias::After)
        .map_err(upstream)?;
    let visual_end = text
        .source_to_visual(source.end(), Bias::Before)
        .map_err(upstream)?;
    let visual = VisualRange::new(visual_start, visual_end.max(visual_start))
        .ok_or(LayoutError::OffsetOverflow)?;
    let mut width = 0.0_f32;
    for run in input.layout_input().runs() {
        let from = run.visual().start().max(visual.start());
        let to = run.visual().end().min(visual.end());
        if from >= to {
            continue;
        }
        let (start, end) = (
            usize::try_from(from.get()).map_err(|_| LayoutError::OffsetOverflow)?,
            usize::try_from(to.get()).map_err(|_| LayoutError::OffsetOverflow)?,
        );
        let slice = input
            .text()
            .get(start..end)
            .ok_or(LayoutError::OffsetOverflow)?;
        let style = input
            .styles()
            .attrs(run.style())
            .ok_or(LayoutError::UnknownStyle(run.style()))?
            .style();
        let shape_source = TextRange::new(
            text.visual_to_source(from, Bias::After).map_err(upstream)?,
            text.visual_to_source(to, Bias::Before).map_err(upstream)?,
        )
        .ok_or(LayoutError::OffsetOverflow)?;
        width += measure(slice, shape_source, style)?;
    }
    Ok((visual, width))
}

/// 按度量量一段文字。`from_table` 的度量后端之一。
pub(crate) fn measure_with<M: ClusterMetrics>(
    text: &str,
    _source: TextRange,
    style: TextStyle,
    metrics: &M,
) -> Result<f32, LayoutError> {
    measure_text(text, metrics, style)
}

/// 按 shaping 后端量一段文字。`from_table` 的另一个度量后端。
pub(crate) fn shape_with<S: ShapingProvider>(
    text: &str,
    source: TextRange,
    style: TextStyle,
    shaper: &S,
) -> Result<f32, LayoutError> {
    shaper
        .shape(text, source, style)
        .map(|shaped| shaped.advance())
        .map_err(|error| LayoutError::Shaping(error.to_string()))
}

fn measure_text<M: ClusterMetrics>(
    text: &str,
    metrics: &M,
    style: TextStyle,
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
