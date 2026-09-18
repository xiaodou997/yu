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
//! # 每一格自己排一次
//!
//! 从 v1 搬过来时算法一个字没改：整块排成一条线性文字流，再把排好的簇搬进
//! 格子。那条流按**整块**的宽度断行，所以格子里放不下的内容不会重排——
//! `| long header | x |` 排在 12pt 宽里，第二列的 content_x 是 11.75，而第
//! 一列的内容一路铺到 24，后一列的内容压在前一列上。
//!
//! 现在每一格按**自己那一列的宽度**排一次 [`BlockLayout`]：断行、bidi、
//! widget 都是同一套，只是零基。行高是那一行各格高度的最大值，不再是常数。
//! caret 与命中测试也随之交给 `BlockLayout`——表格此前各有一份手写的按 x
//! 扫描，与文字流那一份是两套规则。
//!
//! # 为什么表格不是 `Decoration::Widget`
//!
//! 第 3 节的对照表把表格列在「一个 block widget」那一格。**它不能是**，
//! 至少不能是图片那种：非空 range 的 `Decoration::Widget` 会隐藏它覆盖的
//! source（`Decoration::hides_source`），而整张表的单元格内容一旦从视觉字节
//! 流里消失，光标就进不了任何一格——不变量 A2 说编辑走的是源码，而源码位置
//! 要靠视觉偏移找回来。图片可以，因为图片本来就没有「内部位置」。
//!
//! 对照表真正要解决的是 §2.1 那条泄漏（一种语法一条全链路），而那件事已经
//! 做完了：`yu-scene` 里没有 `TablePrimitive`（网格现在是渲染中立的
//! `OrnamentPrimitive`），FFI 里没有表格几何，`yu-layout` 里没有 `table.rs`。
//! 剩下的是几何，而几何要的是**内部布局**，不是换一种装饰变体。

use std::{error::Error, fmt};

use yu_core::{
    ByteOffset, ClusterMetrics, Revision, ShapingProvider, TextRange, VisualOffset, VisualRange,
};
use yu_decoration::Bias;
use yu_markdown::{TableAlignment, TableBlock, TableCellRange};

use yu_layout::{
    BlockLayout, LayoutConfig, LayoutError, LayoutInput, LayoutPoint, LayoutRect, LineAttrs,
    LineSpan, LineStyleTable,
};

use crate::blockinput::BlockStyleTable;
use crate::widget::BlockWidgets;

use crate::blockinput::BlockLayoutInput;
use crate::blockview::shift_range;
use crate::geometry::{source_range_contains, upstream};
use crate::visual::VisualText;

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
    content_y: f32,
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

    #[must_use]
    pub const fn content_y(self) -> f32 {
        self.content_y
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
    border_width: f32,
    /// 每一行的 y 与高。**行高不再是常数**：一格里的内容换行之后，那一行
    /// 就比 `line_height` 高。
    rows: Vec<TableRowGeometry>,
    bounds: LayoutRect,
    cells: Vec<TableCellLayout>,
    /// 每一格自己那一份布局，与 `cells` 同序、等长。零基视觉空间。
    cell_layouts: Vec<BlockLayout>,
    row_sources: Vec<TextRange>,
}

/// 一行在表格局部坐标里的位置与高度。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TableRowGeometry {
    y: f32,
    height: f32,
}

impl TableRowGeometry {
    #[must_use]
    pub const fn y(self) -> f32 {
        self.y
    }

    #[must_use]
    pub const fn height(self) -> f32 {
        self.height
    }
}

/// 给一格排一次版。度量与 shaping 两条路各一个实现。
///
/// 两条路只在「一段文字有多宽 / 排成什么字形」上分开，断行、bidi、widget
/// 摆放全部由 [`BlockLayout`] 一份代码做——共用代码路径的差分是自证的，
/// 所以这里只留一个可替换的点。
pub(crate) trait CellBackend {
    fn layout(
        &self,
        input: LayoutInput<'_>,
        config: LayoutConfig,
        styles: &BlockStyleTable,
        widgets: &BlockWidgets<'_>,
    ) -> Result<BlockLayout, LayoutError>;
}

struct CellLineStyles(f32, yu_core::FontStrutMode);
impl LineStyleTable for CellLineStyles {
    fn attrs(&self, _: yu_core::LineStyleId) -> Option<LineAttrs> {
        LineAttrs::new(0.0, self.0)
            .ok()
            .map(|attrs| attrs.with_font_struts(self.1))
    }
}

fn cell_lines(input: LayoutInput<'_>) -> Result<[LineSpan; 1], LayoutError> {
    let end = u64::try_from(input.text().len()).map_err(|_| LayoutError::OffsetOverflow)?;
    let range = VisualRange::new(VisualOffset::new(0), VisualOffset::new(end))
        .ok_or(LayoutError::OffsetOverflow)?;
    Ok([LineSpan::new(range, yu_core::LineStyleId(0))])
}

/// 按 [`ClusterMetrics`] 排。
pub(crate) struct MetricsCells<'a, M>(pub(crate) &'a M);

impl<M: ClusterMetrics> CellBackend for MetricsCells<'_, M> {
    fn layout(
        &self,
        input: LayoutInput<'_>,
        config: LayoutConfig,
        styles: &BlockStyleTable,
        widgets: &BlockWidgets<'_>,
    ) -> Result<BlockLayout, LayoutError> {
        let lines = cell_lines(input)?;
        BlockLayout::build_all(
            input.with_line_styles(&lines),
            config,
            styles,
            widgets,
            // Cells and body paragraphs share the theme's font-strut policy.
            &CellLineStyles(
                config.theme().spec().body_line_ratio,
                config.theme().body_font_struts(),
            ),
            self.0,
        )
    }
}

/// 按 [`ShapingProvider`] 排。
pub(crate) struct ShapedCells<'a, S>(pub(crate) &'a S);

impl<S: ShapingProvider> CellBackend for ShapedCells<'_, S> {
    fn layout(
        &self,
        input: LayoutInput<'_>,
        config: LayoutConfig,
        styles: &BlockStyleTable,
        widgets: &BlockWidgets<'_>,
    ) -> Result<BlockLayout, LayoutError> {
        let lines = cell_lines(input)?;
        BlockLayout::build_shaped(
            input.with_line_styles(&lines),
            config,
            styles,
            widgets,
            // Cells and body paragraphs share the theme's font-strut policy.
            &CellLineStyles(
                config.theme().spec().body_line_ratio,
                config.theme().body_font_struts(),
            ),
            self.0,
        )
    }
}

impl TableLayout {
    #[must_use]
    pub const fn border_width(&self) -> f32 {
        self.border_width
    }

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
    pub(crate) fn from_table<B: CellBackend>(
        table: &TableBlock,
        visual: &VisualText,
        input: &BlockLayoutInput,
        widgets: &BlockWidgets<'_>,
        config: LayoutConfig,
        backend: &B,
    ) -> Result<Self, LayoutError> {
        Self::from_table_with_widths(table, visual, input, widgets, config, backend, None)
    }

    pub(crate) fn from_table_with_widths<B: CellBackend>(
        table: &TableBlock,
        visual: &VisualText,
        input: &BlockLayoutInput,
        widgets: &BlockWidgets<'_>,
        config: LayoutConfig,
        backend: &B,
        widths: Option<&[f32]>,
    ) -> Result<Self, LayoutError> {
        let origin_x = input.content_left();
        let config = config.with_max_width(input.available_width());
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

        // 第一步：每一格不断行有多宽，列取那一列的最大值。
        let theme = config.theme().spec();
        let zoom = config.line_height() / theme.body_size;
        let padding = theme.table_padding_x * zoom;
        let padding_y = theme.table_padding_y * zoom;
        let border_width = yu_core::ThemeSpec::TABLE_BORDER_WIDTH * zoom;
        let mut natural_widths = vec![padding * 2.0; column_count];
        let mut measured_rows = Vec::with_capacity(rows.len());
        for row in &rows {
            let mut measured_row = Vec::with_capacity(column_count);
            for (column, cell) in row.iter().copied().enumerate() {
                let source_range = table_cell_range(cell)?;
                let (cell_visual, content_width) =
                    measure_cell_content(visual, input, widgets, config, source_range, backend)?;
                if !content_width.is_finite() || content_width < 0.0 {
                    return Err(LayoutError::InvalidMetrics(content_width.to_bits()));
                }
                natural_widths[column] = natural_widths[column]
                    .max(
                        content_width
                            + padding * 2.0
                            + border_width
                                * (1.0
                                    + if column == 0 { 0.5 } else { 0.0 }
                                    + if column + 1 == column_count { 0.5 } else { 0.0 }),
                    )
                    .max(padding * 2.0);
                measured_row.push((source_range, cell_visual, content_width));
            }
            measured_rows.push(measured_row);
        }

        let natural_total = natural_widths.iter().sum::<f32>();
        if !natural_total.is_finite() || natural_total <= 0.0 {
            return Err(LayoutError::Upstream("table width is not finite".into()));
        }
        let scale = config.max_width() / natural_total;
        let column_widths = natural_widths
            .into_iter()
            .map(|width| width * scale)
            .collect::<Vec<_>>();
        let column_widths = widths.map_or(column_widths, <[f32]>::to_vec);
        if column_widths.len() != column_count
            || column_widths.iter().any(|w| !w.is_finite() || *w <= 0.0)
        {
            return Err(LayoutError::Upstream(
                "invalid table column constraints".into(),
            ));
        }
        let total_width = column_widths.iter().sum::<f32>();

        // 第二步：每一格按**自己那一列**的最终宽度排一次。压缩过的列在这里
        // 断行，所以内容不会铺出格子外面去。
        let alignments = table.alignments();
        let row_count = rows.len();
        let mut cells = Vec::with_capacity(row_count.saturating_mul(column_count));
        let mut cell_layouts = Vec::with_capacity(cells.capacity());
        let mut geometry = Vec::with_capacity(row_count);
        let mut y = 0.0_f32;
        for (row_index, measured_cells) in measured_rows.iter().enumerate() {
            let border_top = border_width * if row_index == 0 { 1.0 } else { 0.5 };
            let border_bottom = border_width * if row_index + 1 == row_count { 1.0 } else { 0.5 };
            let mut x = origin_x;
            let mut height = config.line_height();
            let first_cell = cells.len();
            for (column, (source, cell_visual, content_width)) in
                measured_cells.iter().copied().enumerate()
            {
                let width = column_widths[column];
                // In very narrow containers preserve a usable text box before
                // padding; padding must never place text outside its column.
                let border_left = border_width * if column == 0 { 1.0 } else { 0.5 };
                let border_right =
                    border_width * if column + 1 == column_count { 1.0 } else { 0.5 };
                let inner_width = (width - border_left - border_right).max(f32::EPSILON);
                let cell_padding =
                    padding.min(((inner_width - config.default_advance()) * 0.5).max(0.0));
                let available = (inner_width - cell_padding * 2.0).max(f32::EPSILON);
                let alignment = match alignments[column] {
                    TableAlignment::Center => yu_layout::LineAlignment::Center,
                    TableAlignment::Right => yu_layout::LineAlignment::Right,
                    TableAlignment::Default | TableAlignment::Left => {
                        yu_layout::LineAlignment::Left
                    }
                };
                let slice = input.slice(cell_visual, source)?;
                let layout = backend
                    .layout(
                        slice.layout_input(),
                        config.with_max_width(available),
                        input.styles(),
                        widgets,
                    )?
                    .align_lines(alignment)?;
                height = height.max(layout.height() + padding_y * 2.0 + border_top + border_bottom);
                cells.push(TableCellLayout {
                    row: geometry.len(),
                    column,
                    source,
                    visual: cell_visual,
                    // 高度先记这一行的下限，整行量完再统一补齐。
                    bounds: LayoutRect::new(x, y, width, config.line_height())?,
                    alignment: alignments[column],
                    content_x: x + border_left.min(width * 0.5) + cell_padding,
                    content_y: y + border_top + padding_y,
                    content_width,
                });
                cell_layouts.push(layout);
                x += width;
            }
            if !height.is_finite() {
                return Err(LayoutError::InvalidMetrics(height.to_bits()));
            }
            for cell in &mut cells[first_cell..] {
                cell.bounds = LayoutRect::new(cell.bounds.x(), y, cell.bounds.width(), height)?;
            }
            geometry.push(TableRowGeometry { y, height });
            y += height;
        }
        let bounds = LayoutRect::new(origin_x, 0.0, total_width, y)?;

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
            border_width,
            rows: geometry,
            bounds,
            cells,
            cell_layouts,
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

    /// 每一行的 y 与高。**行高不是常数**——一格里的内容换行之后那一行更高。
    #[must_use]
    pub fn rows(&self) -> &[TableRowGeometry] {
        &self.rows
    }

    /// 每一格自己那一份布局，与 [`TableLayout::cells`] 同序、等长。
    ///
    /// 视觉偏移是**零基**的：加上那一格的 `visual().start()` 才是块空间。
    #[must_use]
    pub fn cell_layouts(&self) -> &[BlockLayout] {
        &self.cell_layouts
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

    /// `point` 落在第几行。表格外的点夹到最近的一行。
    #[must_use]
    pub fn row_at(&self, y: f32) -> usize {
        for (index, row) in self.rows.iter().enumerate() {
            if y < row.y + row.height {
                return index;
            }
        }
        self.rows.len().saturating_sub(1)
    }

    /// `point` 落在哪一格，连同那一格的布局。
    ///
    /// 表格外的点夹到最近的一格：点在网格右边要落到那一行的最后一格里，
    /// 落空的话光标会跳到整块的末尾去。
    #[must_use]
    pub(crate) fn cell_at(&self, point: LayoutPoint) -> Option<(TableCellLayout, &BlockLayout)> {
        let row = self.row_at(point.y() - self.bounds.y());
        let mut best: Option<usize> = None;
        for (index, cell) in self.cells.iter().enumerate() {
            if cell.row != row {
                continue;
            }
            let inside =
                point.x() >= cell.bounds.x() && point.x() < cell.bounds.x() + cell.bounds.width();
            if inside {
                best = Some(index);
                break;
            }
            // 夹到最近的一格：越过右边缘就一路记到最后一格。
            if best.is_none() || point.x() >= cell.bounds.x() {
                best = Some(index);
            }
        }
        let index = best?;
        Some((self.cells[index], self.cell_layouts.get(index)?))
    }

    /// Source offsets retain the owning cell even when several empty cells
    /// share the same projected visual boundary.
    pub(crate) fn cell_for_source(
        &self,
        source: ByteOffset,
    ) -> Option<(TableCellLayout, &BlockLayout)> {
        let index = self
            .cells
            .iter()
            .position(|cell| cell.source.start() <= source && source <= cell.source.end())?;
        Some((self.cells[index], self.cell_layouts.get(index)?))
    }

    /// 视觉偏移落在哪一格，连同那一格的布局。
    ///
    /// 空单元格的视觉区间是空的，几格可以塌在同一个偏移上。`bias` 决定取
    /// 哪一边：`Before` 取塌在这里的第一格，`After` 取最后一格。
    #[must_use]
    pub(crate) fn cell_for_visual(
        &self,
        visual: VisualOffset,
        bias: Bias,
    ) -> Option<(TableCellLayout, &BlockLayout)> {
        let mut found: Option<usize> = None;
        for (index, cell) in self.cells.iter().enumerate() {
            if cell.visual.start() > visual || visual > cell.visual.end() {
                continue;
            }
            match bias {
                Bias::Before if found.is_some() => {}
                _ => found = Some(index),
            }
        }
        let index = found?;
        Some((self.cells[index], self.cell_layouts.get(index)?))
    }

    /// Resolve a divider constraint. The caller must reflow the cells using
    /// these widths before publishing any geometry.
    pub(crate) fn column_widths_for_resize(
        &self,
        index: usize,
        delta: f32,
    ) -> Result<Vec<f32>, LayoutError> {
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
        let minimum = (self.padding * 2.0 + self.border_width)
            .min(pair_width * 0.5)
            .max(f32::EPSILON);
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
        Ok(widths)
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
        let row = self.row_at(point.y() - self.bounds.y());
        let column = self
            .cells
            .iter()
            .find(|cell| {
                cell.row == row
                    && point.x() >= cell.bounds.x()
                    && point.x() < cell.bounds.x() + cell.bounds.width()
            })
            .or_else(|| {
                self.cells.iter().rev().find(|cell| {
                    cell.row == row && point.x() == self.bounds.x() + self.bounds.width()
                })
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

        for row in 0..self.rows.len().saturating_sub(1) {
            let geometry = self.rows[row];
            let y = self.bounds.y() + geometry.y + geometry.height;
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
            border_width: self.border_width,
            rows: self.rows.clone(),
            bounds: self.bounds,
            cell_layouts: self.cell_layouts.clone(),
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
///
/// 锚在这一段里的 widget 也算宽度。它在视觉字节流里不占位，样式段一个字节
/// 都切不到它——不算的话，一格里只有一张图的那一列会被压成一条缝，而
/// 图片照样按自己的宽度画出去，压在下一列上。
fn measure_cell_content<B: CellBackend>(
    text: &VisualText,
    input: &BlockLayoutInput,
    widgets: &BlockWidgets<'_>,
    config: LayoutConfig,
    source: TextRange,
    backend: &B,
) -> Result<(VisualRange, f32), LayoutError> {
    let visual_start = text
        .source_to_visual(source.start(), Bias::After)
        .map_err(upstream)?;
    let visual_end = text
        .source_to_visual(source.end(), Bias::Before)
        .map_err(upstream)?;
    let visual = VisualRange::new(visual_start, visual_end.max(visual_start))
        .ok_or(LayoutError::OffsetOverflow)?;
    // Natural width uses the same resolved fonts, inline sizes and shaping as
    // constrained layout; style-only measurement can disagree with CoreText.
    let slice = input.slice(visual, source)?;
    let layout = backend.layout(
        slice.layout_input(),
        config.with_max_width(1_000_000.0),
        input.styles(),
        widgets,
    )?;
    let width = layout
        .lines()
        .iter()
        .map(|line| line.width())
        .fold(0.0_f32, f32::max);
    Ok((visual, width))
}
