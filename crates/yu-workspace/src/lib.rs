#![forbid(unsafe_code)]

//! Product-facing integration between the editor model and retained scenes.
//!
//! `yu-editor` owns canonical source, Markdown, viewport measurements and
//! block-local layout caches. This crate is the first layer allowed to combine
//! those results with `yu-scene`/`yu-render`; neither side needs to depend on
//! the other.

use std::error::Error;
use std::fmt;
use std::sync::Arc;

use yu_assets::{
    EmbeddedRenderPayload, EmbeddedRenderPublication, ImageIntrinsicPublication, ImageKey,
    ImagePublication,
};
use yu_core::{Revision, TextRange, TextRole, VisualRange};
use yu_editor::{
    Bias, BlockKind, BlockView, BlockWidget, CaretAffinity, CheckboxPlacement, EditorDocument,
    EditorDocumentError, ImageSpan, LayoutError, Selections, ShapingProvider, TableLayout,
    TableResizeCommit, TableResizeTarget, TaskState, ViewportSpan,
};
use yu_font::GlyphAtlas;
use yu_layout::{ImageIntrinsicSize, LayoutRect};
use yu_render::{RenderError, RenderPlan, RenderPlanBuilder};
use yu_scene::{
    EditorDecorationPrimitive, EditorDecorationPrimitiveRole, EmbeddedSvgPrimitive, ImagePrimitive,
    OrnamentPrimitive, OrnamentRole, Point, Primitive, Rect, Rgba8, Scene, SceneBuilder,
    SceneError, SceneGlyph, ViewportBlockContent, ViewportBlockGeometry, ViewportSceneInput,
    translate_block_rect,
};

mod workspace;

pub use workspace::{
    CloseAction, CloseResult, OpenTabResult, TabId, Workspace, WorkspaceCloseRequest,
    WorkspaceError, WorkspaceTab,
};

/// `BlockKind::TaskListItem` 在视口输入里的标签。见
/// `yu_markdown::BlockKind::viewport_tag`。
const TASK_LIST_ITEM_TAG: u8 = 7;

/// A validated scene together with the viewport metadata that produced it.
#[derive(Clone, Debug, PartialEq)]
pub struct ViewportSceneFrame {
    input: ViewportSceneInput,
    scene: Scene,
}

/// A parser- and Revision-bound task checkbox target from one published scene.
///
/// `source` is the exact `[ ]`/`[x]` marker range rather than the containing
/// list item. Platform shells use `block_index` only to invoke the existing
/// canonical `ToggleTask` editor command after validating this hit.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TaskCheckboxHit {
    revision: Revision,
    block_index: usize,
    source: yu_core::TextRange,
    bounds: Rect,
}

impl TaskCheckboxHit {
    #[must_use]
    pub const fn revision(self) -> Revision {
        self.revision
    }

    #[must_use]
    pub const fn block_index(self) -> usize {
        self.block_index
    }

    #[must_use]
    pub const fn source(self) -> yu_core::TextRange {
        self.source
    }

    #[must_use]
    pub const fn bounds(self) -> Rect {
        self.bounds
    }
}

/// Returns the optional background used by the product visual projection for
/// one parser block. The scene crate receives only the resulting color, so
/// Markdown semantics stay at this editor-to-scene integration boundary.
#[must_use]
pub fn viewport_block_background(kind: BlockKind) -> Option<Rgba8> {
    match kind {
        BlockKind::FencedCodeBlock { .. } => Some(Rgba8::new(245, 246, 248, 255)),
        _ => None,
    }
}

/// 表格网格的颜色与线宽。产品选色住在这一层，不住在场景层。
#[derive(Clone, Copy, Debug)]
struct TableSceneStyle {
    border_width: f32,
    border_color: Rgba8,
    header_fill: Option<Rgba8>,
    selection_fill: Option<Rgba8>,
}

#[must_use]
const fn viewport_table_style() -> TableSceneStyle {
    TableSceneStyle {
        border_width: 1.0,
        border_color: Rgba8::new(190, 195, 205, 255),
        header_fill: Some(Rgba8::new(248, 249, 251, 255)),
        selection_fill: Some(Rgba8::new(210, 225, 255, 255)),
    }
}

/// 代码高亮的调色板：[`TextRole`] → RGBA。
///
/// # 为什么在这一层
///
/// 与 `viewport_table_style` / `viewport_block_quote_color` 同一个理由：
/// **产品选色住在这一层**。装饰产出的是「这是一个关键字」，不是
/// `#0550AE`——`yu-markdown` 里写死颜色就等于把主题焊进解析层，而
/// `yu-layout` / `yu-scene` 按不变量 E1 连角色都不该解释。
///
/// # 为什么是一份写死的浅色
///
/// 整个编辑区现在就是浅色的：`macos_render_host_config` 把背景写死成
/// `Rgba8::white()`，注释里写着「暗色模式需要平台把实际的
/// `textBackgroundColor` 传进来」。**那条欠账在第四刀的人工验收记录里已经
/// 登记**（D2 不通过：深色外观下面板变深而文档区仍是白底）。这里跟着它走，
/// 不在这一刀里单独给高亮开一条深色路——那会造出「深色的代码配白色的底」。
///
/// 底色是 `viewport_block_background` 的 `(245,246,248)`，所以每一种角色都
/// 按那块底挑的对比度，不是按纯白。
#[must_use]
const fn viewport_code_role_color(role: TextRole) -> Option<Rgba8> {
    match role {
        // 正文色由这一帧给（`ViewportRenderConfig::color`），不覆盖。
        TextRole::Plain | TextRole::Variable => None,
        TextRole::Keyword => Some(Rgba8::new(207, 34, 46, 255)),
        TextRole::Literal => Some(Rgba8::new(10, 48, 105, 255)),
        TextRole::Number => Some(Rgba8::new(5, 80, 174, 255)),
        TextRole::Comment => Some(Rgba8::new(110, 119, 129, 255)),
        TextRole::Function => Some(Rgba8::new(130, 80, 223, 255)),
        TextRole::Type => Some(Rgba8::new(149, 63, 25, 255)),
        TextRole::Constant => Some(Rgba8::new(5, 80, 174, 255)),
        TextRole::Operator => Some(Rgba8::new(5, 80, 174, 255)),
        // 括号与分号不着色：全部着上之后代码看着像圣诞树，而它们本来就靠
        // 形状而不是颜色区分。
        TextRole::Punctuation => None,
    }
}

#[must_use]
const fn viewport_block_quote_color() -> Rgba8 {
    Rgba8::new(176, 181, 190, 255)
}

/// 目标解析不出来的图片那个空框。
///
/// 比未解码图片的浅灰再深一点：那一种是「还在加载」，这一种是「加载不了」。
const fn viewport_broken_image_color() -> Rgba8 {
    Rgba8::new(198, 203, 212, 255)
}

/// 表格的底色、选中高亮与网格线。
///
/// 这段几何原来住在 `yu-scene::append_table`——场景层为此认识「表头」
/// 「单元格」「分隔线」，一种语法一条全链路（overview-v2 §2.1）。现在它在
/// 这里算完，交给场景层的只有矩形与一个渲染中立的角色。
fn append_table_ornaments(
    ornaments: &mut Vec<OrnamentPrimitive>,
    table: &TableLayout,
    origin: Point,
    style: TableSceneStyle,
    selection: Option<TextRange>,
) -> Result<(), ViewportSceneError> {
    let table_source = table.source_range();
    if let Some(color) = style.header_fill {
        for cell in table.cells().iter().copied().filter(|cell| cell.row() == 0) {
            ornaments.push(OrnamentPrimitive::new(
                cell.source(),
                translate_block_rect(cell.bounds(), origin)?,
                color,
                OrnamentRole::Background,
            ));
        }
    }
    if let Some(color) = style.selection_fill {
        for cell in table.cells().iter().copied() {
            if selection.is_some_and(|range| ranges_intersect_or_caret(range, cell.source())) {
                ornaments.push(OrnamentPrimitive::new(
                    cell.source(),
                    translate_block_rect(cell.bounds(), origin)?,
                    color,
                    OrnamentRole::Fill,
                ));
            }
        }
    }

    let bounds = table.bounds();
    let thickness_x = style.border_width.min(bounds.width());
    let thickness_y = style.border_width.min(bounds.height());
    if thickness_x <= 0.0 || thickness_y <= 0.0 {
        return Ok(());
    }
    let total_width = bounds.width();
    let total_height = bounds.height();
    let mut x = 0.0_f32;
    for column_width in table.column_widths() {
        ornaments.push(OrnamentPrimitive::new(
            table_source,
            translate_block_rect(LayoutRect::new(x, 0.0, thickness_x, total_height)?, origin)?,
            style.border_color,
            OrnamentRole::Border,
        ));
        x += *column_width;
    }
    ornaments.push(OrnamentPrimitive::new(
        table_source,
        translate_block_rect(
            LayoutRect::new(
                (total_width - thickness_x).max(0.0),
                0.0,
                thickness_x,
                total_height,
            )?,
            origin,
        )?,
        style.border_color,
        OrnamentRole::Border,
    ));

    // 行高不是常数：一格里的内容换行之后那一行更高，横线要按每一行自己的
    // 上沿画。按 `行号 × 常数行高` 画的话，越往下越对不上格子。
    for row in table.rows() {
        ornaments.push(OrnamentPrimitive::new(
            table_source,
            translate_block_rect(
                LayoutRect::new(0.0, row.y(), total_width, thickness_y)?,
                origin,
            )?,
            style.border_color,
            OrnamentRole::Border,
        ));
    }
    ornaments.push(OrnamentPrimitive::new(
        table_source,
        translate_block_rect(
            LayoutRect::new(
                0.0,
                (total_height - thickness_y).max(0.0),
                total_width,
                thickness_y,
            )?,
            origin,
        )?,
        style.border_color,
        OrnamentRole::Border,
    ));
    Ok(())
}

fn ranges_intersect_or_caret(selection: TextRange, cell: TextRange) -> bool {
    if selection.is_empty() {
        if cell.is_empty() {
            return selection.start() == cell.start();
        }
        return cell.contains(selection.start());
    }
    selection.start() < cell.end() && cell.start() < selection.end()
}

/// 画一个任务项复选框。
///
/// 几何**全部**来自排版排出来的那个盒子（`placement.bounds()`）。此前是这里
/// 自己算：拿标记起点的 caret 当左上角、行高乘 0.68 当边长。那套算法在
/// `[x]` 是 `Decoration::Replace` 的时候是唯一能做的事——被藏掉的三个字节
/// 塌成一个点，点上没有宽度可用——代价是方框压在正文的第一个字上。复选框
/// 成为 widget 之后盒子在排版里占位，画的人不需要、也不许再算第二遍。
fn append_task_checkbox(
    ornaments: &mut Vec<OrnamentPrimitive>,
    origin: Point,
    placement: CheckboxPlacement,
) -> Result<(), ViewportSceneError> {
    let state = placement.state();
    let source = placement.source();
    let bounds = placement.bounds();
    let size = bounds.width().min(bounds.height());
    let x = bounds.x();
    let y = bounds.y();
    let border = match state {
        TaskState::Todo => Rgba8::new(118, 124, 134, 255),
        TaskState::Done => Rgba8::new(38, 111, 219, 255),
    };
    ornaments.push(OrnamentPrimitive::new(
        source,
        translate_block_rect(LayoutRect::new(x, y, size, size)?, origin)?,
        border,
        OrnamentRole::Border,
    ));

    match state {
        TaskState::Todo => {
            let inset = (size * 0.14).max(0.5).min(size * 0.3);
            ornaments.push(OrnamentPrimitive::new(
                source,
                translate_block_rect(
                    LayoutRect::new(x + inset, y + inset, size - inset * 2.0, size - inset * 2.0)?,
                    origin,
                )?,
                Rgba8::white(),
                OrnamentRole::Background,
            ));
        }
        TaskState::Done => {
            let unit = size / 5.0;
            for (column, row) in [(1.0, 2.4), (1.8, 3.1), (2.7, 2.4), (3.6, 1.5)] {
                ornaments.push(OrnamentPrimitive::new(
                    source,
                    translate_block_rect(
                        LayoutRect::new(
                            x + unit * column,
                            y + unit * row,
                            unit * 0.85,
                            unit * 0.85,
                        )?,
                        origin,
                    )?,
                    Rgba8::white(),
                    OrnamentRole::Mark,
                ));
            }
        }
    }
    Ok(())
}

/// Platform-selected colors and geometry for selection/caret scene layers.
///
/// This stays outside `yu-scene`: the retained scene records semantic roles,
/// while the workspace/platform boundary chooses product colors and whether
/// editor chrome belongs in a publication at all.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EditorDecorationStyle {
    selection: Rgba8,
    caret: Rgba8,
    composition_caret: Rgba8,
    caret_width: f32,
    search_match: Rgba8,
    search_current: Rgba8,
}

impl EditorDecorationStyle {
    #[must_use]
    pub const fn new(
        selection: Rgba8,
        caret: Rgba8,
        composition_caret: Rgba8,
        caret_width: f32,
    ) -> Self {
        Self {
            selection,
            caret,
            composition_caret,
            caret_width,
            // 不给搜索高亮编一个默认颜色：全透明等于「没配置就不画」，
            // 而随手编一个会让平台忘了配也看不出来。
            search_match: Rgba8::new(0, 0, 0, 0),
            search_current: Rgba8::new(0, 0, 0, 0),
        }
    }

    /// 搜索命中与「当前命中」两种底色。
    #[must_use]
    pub const fn with_search(mut self, search_match: Rgba8, search_current: Rgba8) -> Self {
        self.search_match = search_match;
        self.search_current = search_current;
        self
    }
}

/// caret 那一行有多高。
///
/// v1 拿的是 `config.line_height()`——标题把行高塞进了 config，所以那个值
/// 恰好对。v2 的行高住在行盒里（widget 会撑高行），所以要按行问。
fn caret_line_height(layout: &BlockView, line: usize) -> f32 {
    layout
        .lines()
        .get(line)
        .map_or_else(|| layout.config().line_height(), |line| line.height())
}

/// 一段视觉区间在一个块里覆盖的那几行矩形。
///
/// 选区与搜索命中是这件事的**两个消费者**，形状完全一样：一段源码 → 视觉
/// 区间 → 逐行按 cluster 收左右边界 → 一条矩形。抽出来是因为第二个消费者
/// 到了；在那之前它只是选区那一段循环体，抽了也没多出唯一性。
///
/// 高度取**行盒自己的**高度，不是 `config().line_height()`。
///
/// 那个基准值只在 v1 里恰好对：标题把行高塞进了 config。v2 的行高住在行盒
/// 里（`caret_line_height` 的文档写了这件事，caret 一直是对的），于是选区
/// 与搜索底色在标题、在带 widget 的行上都会画得又矮又靠上——**不报错，只是
/// 画在文字上方**。这是真实窗口截图抓出来的；在那之前没有任何断言压着它。
fn append_visual_span_rects(
    builder: &mut SceneBuilder,
    layout: &BlockView,
    block_y: f32,
    layer_source: TextRange,
    visual: VisualRange,
    color: Rgba8,
    role: EditorDecorationPrimitiveRole,
) -> Result<(), ViewportSceneError> {
    if visual.is_empty() {
        return Ok(());
    }
    for line in layout.lines() {
        let line_start = line.visual().start().max(visual.start());
        let line_end = line.visual().end().min(visual.end());
        if line_start >= line_end {
            continue;
        }
        let mut left = f32::INFINITY;
        let mut right = f32::NEG_INFINITY;
        for cluster_index in line.cluster_range() {
            let cluster = layout.clusters()[cluster_index];
            if cluster.is_line_break()
                || cluster.visual().end() <= line_start
                || cluster.visual().start() >= line_end
            {
                continue;
            }
            left = left.min(cluster.x());
            right = right.max(cluster.x() + cluster.width());
        }
        if left.is_finite() && right.is_finite() && right > left {
            builder.editor_decoration(EditorDecorationPrimitive::new(
                layer_source,
                Rect::new(left, block_y + line.y(), right - left, line.height())?,
                color,
                role,
            ))?;
        }
    }
    Ok(())
}

/// 搜索命中的底色。
///
/// **这不是装饰**（不变量 D1 管的是文字自己的视觉表现）：它与选区同形状，
/// 一段源码区间加一份 `BlockLayout` 直接产出场景图元。`DecorationCache` 与
/// `DecorationSet` 只在坐标映射上被用到，一个字节都不用清。
///
/// 搜索底色分成前后两层，中间夹着选区。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SearchHighlightLayer {
    /// 选区之下：其余命中。
    UnderSelection,
    /// 选区之上、caret 之下：当前命中。
    OverSelection,
}

impl SearchHighlightLayer {
    /// 这一层要不要画这个 role。`None` 表示这个 role 不归搜索管。
    const fn wants(self, role: EditorDecorationPrimitiveRole) -> Option<bool> {
        match (self, role) {
            (Self::UnderSelection, EditorDecorationPrimitiveRole::SearchMatch)
            | (Self::OverSelection, EditorDecorationPrimitiveRole::SearchCurrent) => Some(true),
            (Self::UnderSelection, EditorDecorationPrimitiveRole::SearchCurrent)
            | (Self::OverSelection, EditorDecorationPrimitiveRole::SearchMatch) => Some(false),
            _ => None,
        }
    }
}

/// **分两层发，因为「当前命中」按定义就是选区那一段。**
///
/// 普通命中排在选区**之前**：选区是半透明的蓝，压上去两者都还看得见。
/// 而当前命中排在选区**之后**——把它画在选区下面，半透明的蓝会把橙色调成
/// 一块说不清的灰（实测 `(255,184,85)` 叠 `(0,122,255,97)` 得到
/// `(158,160,150)`）。这是真实窗口截图抓出来的：所有断言都绿，画面只是脏。
/// 它仍然排在 caret **之前**，caret 是最后一层。
///
/// **IME 组字期间整个跳过**：那时块的视觉字节流带着 preedit 覆盖层，匹配是
/// 按 canonical 源码算的，两者对不上。少画几个框好过画在错的位置上。
/// 这一段是不是整个落在某一条**非空**选区里。
///
/// 选区按起点升序且互不重叠（不变量 B9），所以只用查「起点不晚于它的最后一条」
/// ——线性扫会让「N 处命中 × N 条选区」在全部选中之后变成 O(N²)。
fn covered_by_a_selection(selections: &Selections, source: TextRange) -> bool {
    let ranges = selections.as_slice();
    let upper =
        ranges.partition_point(|selection| selection.ordered_range().start() <= source.start());
    let Some(candidate) = upper.checked_sub(1).and_then(|index| ranges.get(index)) else {
        return false;
    };
    let range = candidate.ordered_range();
    !range.is_empty() && range.start() <= source.start() && source.end() <= range.end()
}

fn append_search_highlights(
    builder: &mut SceneBuilder,
    document: &EditorDocument,
    input: &ViewportSceneInput,
    layouts: &[BlockView],
    style: EditorDecorationStyle,
    layer: SearchHighlightLayer,
) -> Result<(), ViewportSceneError> {
    if document.composition().is_some() {
        return Ok(());
    }
    let Some(search) = document.search() else {
        return Ok(());
    };
    if search.is_empty() {
        return Ok(());
    }
    // **「当前命中」按 primary 判。**
    //
    // `SearchState::current` 仍然收一个区间，理由没变（见 `yu-editor/src/
    // search.rs` 的模块文档：不存下标，只留一份真相）。多光标之后要收紧到
    // primary：「选中全部匹配」之后每一条选区都恰好等于一处命中，按「有没有
    // 某条选区等于它」判会让**每一处**都变成当前命中——那正是那份文档里担心
    // 的「全选点亮每一个」换了个形状回来。
    //
    // primary 不是第二个可以对不上的下标：它是 `Selections` 自己的一部分，
    // 由导航必然更新的那条路带着走。
    let selections = document.selections();
    let current = search.current(document.selections().primary().ordered_range());
    for (geometry, layout) in input.blocks().iter().copied().zip(layouts.iter()) {
        let block = geometry.source();
        // 匹配按文档顺序，二分给出「起点 < 块尾」的上界；跨进这个块的那一条
        // 起点可能更靠前，所以下界给不出来，从头过滤。与
        // `DecorationSet::in_range` 同一个理由。
        let upper = search
            .matches()
            .partition_point(|hit| hit.start() < block.end());
        for (index, hit) in search.matches()[..upper].iter().copied().enumerate() {
            let start = hit.start().max(block.start());
            let end = hit.end().min(block.end());
            if start >= end {
                continue;
            }
            let source = TextRange::new(start, end).expect("ordered search intersection");
            let visual_start = layout
                .visual()
                .source_to_visual(start, Bias::Before)
                .map_err(EditorDocumentError::from)?;
            let visual_end = layout
                .visual()
                .source_to_visual(end, Bias::After)
                .map_err(EditorDocumentError::from)?;
            let Some(visual) = VisualRange::new(visual_start, visual_end) else {
                continue;
            };
            // **被选区完全盖住的普通命中不画。**
            //
            // 第三刀把「其余命中」排在选区之下，理由是「选区是半透明的蓝，
            // 压上去两者都还看得见」——那句话的前提是**选区与那些命中不是同一
            // 段**。多光标之后前提没了：「选中全部匹配」让每一处命中同时也是
            // 一段选区，黄底垫在半透明蓝之下合成出一块脏灰绿（与第三刀在
            // 当前命中上抓到的 `(158,160,150)` 是同一族颜色）。这是截图抓出来
            // 的，全部自动化断言都绿。
            //
            // 当前命中不受影响：它排在选区**之上**，而且按定义就等于选区那一段。
            if current != Some(index) && covered_by_a_selection(selections, source) {
                continue;
            }
            let (color, role) = if current == Some(index) {
                (
                    style.search_current,
                    EditorDecorationPrimitiveRole::SearchCurrent,
                )
            } else {
                (
                    style.search_match,
                    EditorDecorationPrimitiveRole::SearchMatch,
                )
            };
            if layer.wants(role) != Some(true) {
                continue;
            }
            append_visual_span_rects(builder, layout, geometry.y(), source, visual, color, role)?;
        }
    }
    Ok(())
}

/// 一根待发的 caret：位置算完了，但要等搜索的「当前命中」那一层发完才发。
struct PendingCaret {
    source: TextRange,
    caret: yu_editor::BlockCaret,
    role: EditorDecorationPrimitiveRole,
    block_y: f32,
    line_height: f32,
}

fn append_editor_decorations(
    builder: &mut SceneBuilder,
    document: &EditorDocument,
    input: &ViewportSceneInput,
    layouts: &[BlockView],
    style: EditorDecorationStyle,
) -> Result<(), ViewportSceneError> {
    if !style.caret_width.is_finite() || style.caret_width <= 0.0 {
        return Err(
            SceneError::InvalidGeometry("editor caret width must be finite and positive").into(),
        );
    }
    let selections = document.selections();
    let composition = document.composition();
    let composition_blocks = document.composition_block_range();

    // An empty canonical document intentionally has no Markdown block or text
    // layout. Publish its insertion point directly so the retained frame can
    // still own the visible editor instead of requiring a TextKit glyph pass.
    if input.blocks().is_empty() && document.snapshot().is_empty() && composition.is_none() {
        builder.editor_decoration(EditorDecorationPrimitive::new(
            TextRange::empty(selections.primary().focus()),
            Rect::new(
                0.0,
                0.0,
                style.caret_width,
                document.viewport_config().layout().line_height(),
            )?,
            style.caret,
            EditorDecorationPrimitiveRole::Caret,
        ))?;
        return Ok(());
    }

    // 其余命中在选区之下。
    append_search_highlights(
        builder,
        document,
        input,
        layouts,
        style,
        SearchHighlightLayer::UnderSelection,
    )?;

    let mut carets: Vec<PendingCaret> = Vec::new();
    for (geometry, layout) in input.blocks().iter().copied().zip(layouts.iter()) {
        if let Some(overlay) = composition {
            // 组字期间选区已经塌回一条（`EditorDocument::begin_composition`），
            // 所以这条路仍然是单数的。
            let focus_block = composition_blocks.as_ref().map(|span| span.start);
            let Some(visual) = layout.visual().composition_selection_visual() else {
                if focus_block == Some(geometry.index()) {
                    let visual = layout
                        .visual()
                        .source_to_visual(overlay.replacement_range().start(), Bias::Before)
                        .map_err(EditorDocumentError::from)?;
                    let layout_caret = layout
                        .caret_for_visual(visual, Bias::After)
                        .map_err(EditorDocumentError::from)?;
                    carets.push(PendingCaret {
                        source: TextRange::empty(overlay.replacement_range().start()),
                        caret: layout_caret,
                        role: EditorDecorationPrimitiveRole::CompositionCaret,
                        block_y: geometry.y(),
                        line_height: layout.config().line_height(),
                    });
                }
                continue;
            };
            if focus_block == Some(geometry.index()) {
                let layout_caret = layout
                    .caret_for_visual(visual.end(), Bias::After)
                    .map_err(EditorDocumentError::from)?;
                carets.push(PendingCaret {
                    source: TextRange::empty(overlay.replacement_range().start()),
                    caret: layout_caret,
                    role: EditorDecorationPrimitiveRole::CompositionCaret,
                    block_y: geometry.y(),
                    line_height: caret_line_height(layout, layout_caret.line()),
                });
            }
            if let Some(visual) = VisualRange::new(visual.start(), visual.end()) {
                append_visual_span_rects(
                    builder,
                    layout,
                    geometry.y(),
                    overlay.replacement_range(),
                    visual,
                    style.selection,
                    EditorDecorationPrimitiveRole::Selection,
                )?;
            }
            continue;
        }

        // **每一条选区各与这个块求一次交，各发各的矩形；每一根落在这个块里的
        // focus 各发一根 caret。** 只画 primary 的表现是「屏幕上只有一个光标」
        // ——多光标看上去完全没生效，而不报错。
        for selection in selections.as_slice() {
            if document.block_index_for_source(selection.focus()) == Some(geometry.index()) {
                let bias = match selection.affinity() {
                    CaretAffinity::Upstream => Bias::Before,
                    CaretAffinity::Downstream => Bias::After,
                };
                let layout_caret = layout
                    .caret_for_source(selection.focus(), bias)
                    .map_err(EditorDocumentError::from)?;
                carets.push(PendingCaret {
                    source: TextRange::empty(selection.focus()),
                    caret: layout_caret,
                    role: EditorDecorationPrimitiveRole::Caret,
                    block_y: geometry.y(),
                    line_height: caret_line_height(layout, layout_caret.line()),
                });
            }
            if selection.is_empty() {
                continue;
            }
            let range = selection.ordered_range();
            let start = range.start().max(geometry.source().start());
            let end = range.end().min(geometry.source().end());
            if start >= end {
                continue;
            }
            let source = TextRange::new(start, end).expect("ordered selection intersection");
            let visual_start = layout
                .visual()
                .source_to_visual(start, Bias::Before)
                .map_err(EditorDocumentError::from)?;
            let visual_end = layout
                .visual()
                .source_to_visual(end, Bias::After)
                .map_err(EditorDocumentError::from)?;
            // 起点在终点之后是「这个块里没有选区」，不是错误。
            if let Some(visual) = VisualRange::new(visual_start, visual_end) {
                append_visual_span_rects(
                    builder,
                    layout,
                    geometry.y(),
                    source,
                    visual,
                    style.selection,
                    EditorDecorationPrimitiveRole::Selection,
                )?;
            }
        }
    }

    // 当前命中在选区之上、caret 之下。**这个次序是这一层的唯一权威**——
    // 把它拆到调用方去排，caret 就会被盖住（它在这个函数的末尾才发出）。
    append_search_highlights(
        builder,
        document,
        input,
        layouts,
        style,
        SearchHighlightLayer::OverSelection,
    )?;

    for pending in carets {
        let color = if pending.role == EditorDecorationPrimitiveRole::CompositionCaret {
            style.composition_caret
        } else {
            style.caret
        };
        builder.editor_decoration(EditorDecorationPrimitive::new(
            pending.source,
            Rect::new(
                pending.caret.point().x(),
                pending.block_y + pending.caret.point().y(),
                style.caret_width,
                pending.line_height,
            )?,
            color,
            pending.role,
        ))?;
    }
    Ok(())
}

impl ViewportSceneFrame {
    #[must_use]
    pub fn revision(&self) -> Revision {
        self.scene.revision()
    }

    #[must_use]
    pub fn input(&self) -> &ViewportSceneInput {
        &self.input
    }

    #[must_use]
    pub fn scene(&self) -> &Scene {
        &self.scene
    }

    #[must_use]
    pub fn into_parts(self) -> (ViewportSceneInput, Scene) {
        (self.input, self.scene)
    }

    /// Resolves a document-space point against the task checkbox borders in
    /// this exact published frame. Interior/check painter layers are ignored,
    /// so one visible checkbox can produce at most one semantic hit.
    pub fn task_checkbox_hit_test(
        &self,
        expected_revision: Revision,
        point: Point,
    ) -> Result<Option<TaskCheckboxHit>, ViewportFrameError> {
        if self.revision() != expected_revision {
            return Err(ViewportFrameError::Stale {
                expected: expected_revision,
                actual: self.revision(),
            });
        }
        // 场景层不再有「任务框」这个 primitive，只有渲染中立的装饰
        // （不变量 E1）。命中判定因此改为：一块 `Border` 装饰，落点在它里面，
        // 而它的源码属于一个 task 块。块的语法类型由视口输入带着走，不用
        // 回头去解析文档（不变量 I1）。
        let Some((task, block)) = self.scene.primitives().iter().find_map(|primitive| {
            let Primitive::Ornament(ornament) = primitive else {
                return None;
            };
            if ornament.role() != OrnamentRole::Border || !ornament.bounds().contains(point) {
                return None;
            }
            let block = self.input.blocks().iter().copied().find(|block| {
                block.kind() == TASK_LIST_ITEM_TAG
                    && block.source().start() <= ornament.source().start()
                    && ornament.source().end() <= block.source().end()
            })?;
            Some((*ornament, block))
        }) else {
            return Ok(None);
        };
        Ok(Some(TaskCheckboxHit {
            revision: self.revision(),
            block_index: block.index(),
            source: task.source(),
            bounds: task.bounds(),
        }))
    }
}

/// A scene and its backend-neutral render plan published as one revision-bound
/// frame. Keeping them together prevents a new scene from being paired with an
/// older command list (or vice versa) at a platform boundary.
#[derive(Clone, Debug, PartialEq)]
pub struct ViewportRenderFrame {
    scene: ViewportSceneFrame,
    plan: RenderPlan,
}

impl ViewportRenderFrame {
    pub fn new(scene: ViewportSceneFrame, plan: RenderPlan) -> Result<Self, ViewportFrameError> {
        if scene.revision() != plan.revision() {
            return Err(ViewportFrameError::RevisionMismatch {
                scene: scene.revision(),
                plan: plan.revision(),
            });
        }
        Ok(Self { scene, plan })
    }

    #[must_use]
    pub fn revision(&self) -> Revision {
        self.scene.revision()
    }

    #[must_use]
    pub fn scene(&self) -> &ViewportSceneFrame {
        &self.scene
    }

    #[must_use]
    pub fn plan(&self) -> &RenderPlan {
        &self.plan
    }

    #[must_use]
    pub fn into_parts(self) -> (ViewportSceneFrame, RenderPlan) {
        (self.scene, self.plan)
    }
}

/// Immutable inputs shared by one scene/render frame build.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ViewportRenderConfig {
    viewport: ViewportSpan,
    font_size: f32,
    /// 编辑区背景色。
    ///
    /// 删除 TextKit fallback 后，Rust surface 是唯一渲染路径（不变量 I5），
    /// 背景必须由这一帧自己画出来：Metal layer 是透明的，未触及的像素会露出
    /// 下层视图，而下层视图已经不再绘制任何东西。
    background: Rgba8,
    /// glyph 栅格化相对逻辑尺寸的倍率，默认 `1.0`。
    ///
    /// Retina 上应设为 backing scale：字形按 `font_size × raster_scale`
    /// 取样，后端再把 atlas 矩形除回逻辑坐标，纹理才能与物理像素 1:1 对应。
    raster_scale: f32,
    scene_viewport: Rect,
    color: Rgba8,
    table_resize: Option<TableResizeCommit>,
    editor_decorations: Option<EditorDecorationStyle>,
}

impl ViewportRenderConfig {
    #[must_use]
    pub const fn new(
        viewport: ViewportSpan,
        font_size: f32,
        scene_viewport: Rect,
        color: Rgba8,
    ) -> Self {
        Self {
            viewport,
            font_size,
            background: Rgba8::white(),
            raster_scale: 1.0,
            scene_viewport,
            color,
            table_resize: None,
            editor_decorations: None,
        }
    }

    /// Returns a copy that carries one caller-owned, session-only table
    /// geometry override into the next scene/render-plan build.
    #[must_use]
    pub const fn with_table_resize(mut self, resize: TableResizeCommit) -> Self {
        self.table_resize = Some(resize);
        self
    }

    #[must_use]
    pub const fn with_editor_decorations(mut self, style: EditorDecorationStyle) -> Self {
        self.editor_decorations = Some(style);
        self
    }

    #[must_use]
    pub const fn viewport(self) -> ViewportSpan {
        self.viewport
    }

    #[must_use]
    pub const fn font_size(self) -> f32 {
        self.font_size
    }

    #[must_use]
    pub const fn with_background(mut self, background: Rgba8) -> Self {
        self.background = background;
        self
    }

    #[must_use]
    pub const fn background(self) -> Rgba8 {
        self.background
    }

    /// 设置 glyph 栅格化倍率。非有限或非正值会被忽略，保留原值。
    #[must_use]
    pub fn with_raster_scale(mut self, scale: f32) -> Self {
        if scale.is_finite() && scale > 0.0 {
            self.raster_scale = scale;
        }
        self
    }

    #[must_use]
    pub const fn raster_scale(self) -> f32 {
        self.raster_scale
    }

    #[must_use]
    pub const fn scene_viewport(self) -> Rect {
        self.scene_viewport
    }

    #[must_use]
    pub const fn color(self) -> Rgba8 {
        self.color
    }

    #[must_use]
    pub const fn table_resize(self) -> Option<TableResizeCommit> {
        self.table_resize
    }

    #[must_use]
    pub const fn editor_decorations(self) -> Option<EditorDecorationStyle> {
        self.editor_decorations
    }
}

/// Errors raised when a scene and render plan are combined or published.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ViewportFrameError {
    RevisionMismatch {
        scene: Revision,
        plan: Revision,
    },
    Stale {
        expected: Revision,
        actual: Revision,
    },
}

impl fmt::Display for ViewportFrameError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::RevisionMismatch { scene, plan } => {
                write!(
                    formatter,
                    "viewport scene revision {scene:?} does not match plan {plan:?}"
                )
            }
            Self::Stale { expected, actual } => {
                write!(
                    formatter,
                    "viewport frame {actual:?} is stale for {expected:?}"
                )
            }
        }
    }
}

impl Error for ViewportFrameError {}

/// Single-entry cache for the latest publishable viewport frame.
///
/// A lookup is revision-aware and never returns a frame for another source
/// revision. Hosts can call `invalidate_stale` after an edit to eagerly drop
/// the old scene and plan, while a stale publish is rejected even if the host
/// forgot to clear first.
#[derive(Clone, Debug, Default)]
pub struct ViewportFrameCache {
    current: Option<Arc<ViewportRenderFrame>>,
}

impl ViewportFrameCache {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    #[must_use]
    pub fn current_revision(&self) -> Option<Revision> {
        self.current.as_ref().map(|frame| frame.revision())
    }

    #[must_use]
    pub fn get(&self, revision: Revision) -> Option<&ViewportRenderFrame> {
        self.current
            .as_ref()
            .filter(|frame| frame.revision() == revision)
            .map(Arc::as_ref)
    }

    #[must_use]
    pub fn current_frame_handle(&self, revision: Revision) -> Option<Arc<ViewportRenderFrame>> {
        self.current
            .as_ref()
            .filter(|frame| frame.revision() == revision)
            .map(Arc::clone)
    }

    /// Drops the cached frame unless it belongs to `revision`.
    ///
    /// Returns `true` when an old frame was removed. An empty cache is already
    /// synchronized and returns `false`.
    pub fn invalidate_stale(&mut self, revision: Revision) -> bool {
        if self
            .current
            .as_ref()
            .is_some_and(|frame| frame.revision() != revision)
        {
            self.current = None;
            true
        } else {
            false
        }
    }

    pub fn clear(&mut self) {
        self.current = None;
    }

    /// Publishes a complete frame only if it still belongs to the caller's
    /// current document revision. Replacement is atomic at the cache level.
    pub fn publish_if_current(
        &mut self,
        current_revision: Revision,
        frame: ViewportRenderFrame,
    ) -> Result<(), ViewportFrameError> {
        self.publish_shared_if_current(current_revision, Arc::new(frame))
    }

    /// Publishes a shared immutable frame handle without cloning the scene or
    /// render plan. The caller may retain another handle for a platform
    /// handoff; all handles still refer to the same revision-bound frame.
    pub fn publish_shared_if_current(
        &mut self,
        current_revision: Revision,
        frame: Arc<ViewportRenderFrame>,
    ) -> Result<(), ViewportFrameError> {
        if frame.revision() != current_revision {
            return Err(ViewportFrameError::Stale {
                expected: current_revision,
                actual: frame.revision(),
            });
        }
        if let Some(existing) = self.current.as_ref()
            && existing.revision() > frame.revision()
        {
            return Err(ViewportFrameError::Stale {
                expected: existing.revision(),
                actual: frame.revision(),
            });
        }
        self.current = Some(frame);
        Ok(())
    }
}

/// Errors raised while assembling an editor viewport into a retained scene.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ViewportSceneError {
    Document(EditorDocumentError),
    Scene(SceneError),
    Render(RenderError),
    Frame(ViewportFrameError),
    InvalidTaskMarker { block: usize },
}

impl fmt::Display for ViewportSceneError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Document(error) => error.fmt(formatter),
            Self::Scene(error) => error.fmt(formatter),
            Self::Render(error) => error.fmt(formatter),
            Self::Frame(error) => error.fmt(formatter),
            Self::InvalidTaskMarker { block } => {
                write!(
                    formatter,
                    "task-list block {block} has no parser-owned marker"
                )
            }
        }
    }
}

impl Error for ViewportSceneError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Document(error) => Some(error),
            Self::Scene(error) => Some(error),
            Self::Render(error) => Some(error),
            Self::Frame(error) => Some(error),
            Self::InvalidTaskMarker { .. } => None,
        }
    }
}

impl From<EditorDocumentError> for ViewportSceneError {
    fn from(error: EditorDocumentError) -> Self {
        Self::Document(error)
    }
}

impl From<yu_core::GeometryError> for ViewportSceneError {
    fn from(error: yu_core::GeometryError) -> Self {
        Self::Scene(SceneError::from(error))
    }
}

impl From<SceneError> for ViewportSceneError {
    fn from(error: SceneError) -> Self {
        Self::Scene(error)
    }
}

impl From<RenderError> for ViewportSceneError {
    fn from(error: RenderError) -> Self {
        Self::Render(error)
    }
}

impl From<ViewportFrameError> for ViewportSceneError {
    fn from(error: ViewportFrameError) -> Self {
        Self::Frame(error)
    }
}

/// An owned viewport frame handed from the shared workspace publisher to a
/// platform host. The frame keeps the scene, render plan, source Revision and
/// publication serial together so a host cannot accidentally pair a frame
/// with metadata from another build.
#[derive(Clone, Debug, PartialEq)]
pub struct ViewportFramePublication {
    revision: Revision,
    serial: u64,
    frame: Arc<ViewportRenderFrame>,
}

impl ViewportFramePublication {
    #[must_use]
    pub const fn revision(&self) -> Revision {
        self.revision
    }

    #[must_use]
    pub const fn serial(&self) -> u64 {
        self.serial
    }

    #[must_use]
    pub fn frame(&self) -> &ViewportRenderFrame {
        self.frame.as_ref()
    }

    #[must_use]
    pub fn frame_handle(&self) -> Arc<ViewportRenderFrame> {
        Arc::clone(&self.frame)
    }

    #[must_use]
    pub fn into_parts(self) -> (Revision, u64, Arc<ViewportRenderFrame>) {
        (self.revision, self.serial, self.frame)
    }
}

/// Errors raised while assembling and publishing a workspace viewport frame.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ViewportPublishError {
    Scene(ViewportSceneError),
    SerialOverflow,
}

impl fmt::Display for ViewportPublishError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Scene(error) => error.fmt(formatter),
            Self::SerialOverflow => formatter.write_str("viewport publication serial overflowed"),
        }
    }
}

impl Error for ViewportPublishError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Scene(error) => Some(error),
            Self::SerialOverflow => None,
        }
    }
}

impl From<ViewportSceneError> for ViewportPublishError {
    fn from(error: ViewportSceneError) -> Self {
        Self::Scene(error)
    }
}

/// Shared, platform-free owner of the latest assembled viewport frame.
///
/// The publisher reads the current `EditorDocument` Revision, assembles the
/// revision-bound scene and render plan, and returns an owned publication with
/// a monotonic serial. It does not own source text, an editor document, a
/// native surface or a GPU object. A host may retain the returned publication
/// while the publisher keeps a revision-aware cache for the latest frame.
#[derive(Clone, Debug, Default)]
pub struct ViewportFramePublisher {
    cache: ViewportFrameCache,
    next_serial: u64,
    last_publication: Option<ViewportFramePublication>,
}

impl ViewportFramePublisher {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Creates a publisher whose next publication serial follows an existing
    /// publisher. This is used when a platform rebuilds only its shaping or
    /// layout state while retaining the same document/session lifecycle.
    #[must_use]
    pub fn with_next_serial(next_serial: u64) -> Self {
        Self {
            cache: ViewportFrameCache::new(),
            next_serial,
            last_publication: None,
        }
    }

    #[must_use]
    pub fn current_revision(&self) -> Option<Revision> {
        self.cache.current_revision()
    }

    #[must_use]
    pub fn current_frame(&self, revision: Revision) -> Option<&ViewportRenderFrame> {
        self.cache.get(revision)
    }

    #[must_use]
    pub fn current_frame_handle(&self, revision: Revision) -> Option<Arc<ViewportRenderFrame>> {
        self.cache.current_frame_handle(revision)
    }

    #[must_use]
    pub fn last_publication(&self) -> Option<&ViewportFramePublication> {
        self.last_publication.as_ref()
    }

    /// Drops the cached frame and publication unless they belong to `revision`.
    pub fn invalidate_stale(&mut self, revision: Revision) -> bool {
        let cache_dropped = self.cache.invalidate_stale(revision);
        let publication_dropped = self
            .last_publication
            .as_ref()
            .is_some_and(|publication| publication.revision() != revision);
        if publication_dropped {
            self.last_publication = None;
        }
        cache_dropped || publication_dropped
    }

    /// Assembles and publishes the editor's current viewport as one owned
    /// publication. All mutable state is updated only after assembly and the
    /// next serial have both been validated.
    pub fn publish<S: ShapingProvider>(
        &mut self,
        document: &mut EditorDocument,
        config: ViewportRenderConfig,
        shaper: &S,
        atlas: &GlyphAtlas,
        render_plans: &mut RenderPlanBuilder,
    ) -> Result<ViewportFramePublication, ViewportPublishError> {
        self.publish_with_images(document, config, shaper, atlas, render_plans, &[])
    }

    /// Publishes the viewport while applying dimensions from already-ready
    /// image publications. Missing images keep the normal placeholder layout.
    pub fn publish_with_images<S: ShapingProvider>(
        &mut self,
        document: &mut EditorDocument,
        config: ViewportRenderConfig,
        shaper: &S,
        atlas: &GlyphAtlas,
        render_plans: &mut RenderPlanBuilder,
        image_publications: &[ImagePublication],
    ) -> Result<ViewportFramePublication, ViewportPublishError> {
        self.publish_with_images_and_intrinsics(
            document,
            config,
            shaper,
            atlas,
            render_plans,
            image_publications,
            &[],
        )
    }

    /// Publishes the viewport while applying ready pixels and/or intrinsic
    /// dimensions retained after those pixels were evicted.
    #[allow(clippy::too_many_arguments)]
    pub fn publish_with_images_and_intrinsics<S: ShapingProvider>(
        &mut self,
        document: &mut EditorDocument,
        config: ViewportRenderConfig,
        shaper: &S,
        atlas: &GlyphAtlas,
        render_plans: &mut RenderPlanBuilder,
        image_publications: &[ImagePublication],
        image_intrinsics: &[ImageIntrinsicPublication],
    ) -> Result<ViewportFramePublication, ViewportPublishError> {
        // RenderPlanBuilder carries page-fingerprint state across frames. Build against a
        // staged copy so a later publication failure cannot advance caller-owned state.
        let mut staged_render_plans = render_plans.clone();
        let frame = assemble_viewport_render_frame_with_images_and_intrinsics(
            document,
            config.viewport(),
            config,
            shaper,
            atlas,
            &mut staged_render_plans,
            image_publications,
            image_intrinsics,
        )?;
        let revision = document.revision();
        if frame.revision() != revision {
            return Err(ViewportPublishError::Scene(ViewportSceneError::Frame(
                ViewportFrameError::Stale {
                    expected: revision,
                    actual: frame.revision(),
                },
            )));
        }
        let serial = self
            .next_serial
            .checked_add(1)
            .ok_or(ViewportPublishError::SerialOverflow)?;
        let frame = Arc::new(frame);
        self.cache
            .publish_shared_if_current(revision, Arc::clone(&frame))
            .map_err(|error| ViewportPublishError::Scene(ViewportSceneError::Frame(error)))?;

        *render_plans = staged_render_plans;
        let publication = ViewportFramePublication {
            revision,
            serial,
            frame,
        };
        self.next_serial = serial;
        self.last_publication = Some(publication.clone());
        Ok(publication)
    }
}

/// Measures the current editor viewport, converts its block metadata into the
/// scene boundary, materializes matching shaped block layouts, and appends
/// them atomically through `SceneBuilder::append_viewport`.
pub fn assemble_viewport_scene<S: ShapingProvider>(
    document: &mut EditorDocument,
    viewport: ViewportSpan,
    shaper: &S,
    font_size: f32,
    scene_viewport: Rect,
    atlas: &GlyphAtlas,
    color: Rgba8,
) -> Result<ViewportSceneFrame, ViewportSceneError> {
    assemble_viewport_scene_with_images(
        document,
        viewport,
        shaper,
        font_size,
        scene_viewport,
        atlas,
        color,
        &[],
    )
}

/// Builds a viewport scene with one caller-owned session-only table resize.
/// The override is applied only to the transient block layout used by this
/// scene; source, selection, history and the document layout cache remain
/// unchanged.
#[allow(clippy::too_many_arguments)]
pub fn assemble_viewport_scene_with_table_resize<S: ShapingProvider>(
    document: &mut EditorDocument,
    viewport: ViewportSpan,
    shaper: &S,
    font_size: f32,
    scene_viewport: Rect,
    atlas: &GlyphAtlas,
    color: Rgba8,
    table_resize: TableResizeCommit,
) -> Result<ViewportSceneFrame, ViewportSceneError> {
    assemble_viewport_scene_with_images_and_intrinsics_and_table_resize(
        document,
        viewport,
        shaper,
        font_size,
        scene_viewport,
        atlas,
        color,
        &[],
        &[],
        Some(table_resize),
    )
}

/// Builds a viewport scene with dimensions from ready image publications.
/// Publications are optional and are matched by the same source-backed
/// destination fingerprint used by the image primitive.
#[allow(clippy::too_many_arguments)]
pub fn assemble_viewport_scene_with_images<S: ShapingProvider>(
    document: &mut EditorDocument,
    viewport: ViewportSpan,
    shaper: &S,
    font_size: f32,
    scene_viewport: Rect,
    atlas: &GlyphAtlas,
    color: Rgba8,
    image_publications: &[ImagePublication],
) -> Result<ViewportSceneFrame, ViewportSceneError> {
    assemble_viewport_scene_with_images_and_intrinsics(
        document,
        viewport,
        shaper,
        font_size,
        scene_viewport,
        atlas,
        color,
        image_publications,
        &[],
    )
}

/// Builds a viewport scene with ready pixels and/or retained intrinsic image
/// dimensions. Intrinsic publications are deliberately separate from decoded
/// pixels so CPU/GPU cache eviction cannot make document geometry jump.
#[allow(clippy::too_many_arguments)]
pub fn assemble_viewport_scene_with_images_and_intrinsics<S: ShapingProvider>(
    document: &mut EditorDocument,
    viewport: ViewportSpan,
    shaper: &S,
    font_size: f32,
    scene_viewport: Rect,
    atlas: &GlyphAtlas,
    color: Rgba8,
    image_publications: &[ImagePublication],
    image_intrinsics: &[ImageIntrinsicPublication],
) -> Result<ViewportSceneFrame, ViewportSceneError> {
    assemble_viewport_scene_with_images_and_intrinsics_and_table_resize(
        document,
        viewport,
        shaper,
        font_size,
        scene_viewport,
        atlas,
        color,
        image_publications,
        image_intrinsics,
        None,
    )
}

/// Builds a viewport scene with ready image dimensions and an optional
/// caller-owned session-only table column override. The override is rejected
/// when stale or when it targets the deferred variable-row path.
#[allow(clippy::too_many_arguments)]
pub fn assemble_viewport_scene_with_images_and_intrinsics_and_table_resize<S: ShapingProvider>(
    document: &mut EditorDocument,
    viewport: ViewportSpan,
    shaper: &S,
    font_size: f32,
    scene_viewport: Rect,
    atlas: &GlyphAtlas,
    color: Rgba8,
    image_publications: &[ImagePublication],
    image_intrinsics: &[ImageIntrinsicPublication],
    table_resize: Option<TableResizeCommit>,
) -> Result<ViewportSceneFrame, ViewportSceneError> {
    assemble_viewport_scene_with_images_and_intrinsics_and_embedded_and_table_resize(
        document,
        viewport,
        shaper,
        font_size,
        scene_viewport,
        atlas,
        color,
        // 这些兼容入口不带背景配置，按白底渲染。
        Rgba8::white(),
        image_publications,
        image_intrinsics,
        &[],
        table_resize,
        None,
    )
}

/// Builds a viewport scene with ready image dimensions and revision-bound SVG
/// publications. Embedded primitives are appended only for matching visible
/// fenced blocks; source glyphs remain in painter order and the primitive uses
/// a transparent fallback until a native SVG consumer is available.
#[allow(clippy::too_many_arguments)]
pub fn assemble_viewport_scene_with_images_and_intrinsics_and_embedded_and_table_resize<
    S: ShapingProvider,
>(
    document: &mut EditorDocument,
    viewport: ViewportSpan,
    shaper: &S,
    font_size: f32,
    scene_viewport: Rect,
    atlas: &GlyphAtlas,
    color: Rgba8,
    background: Rgba8,
    image_publications: &[ImagePublication],
    image_intrinsics: &[ImageIntrinsicPublication],
    embedded_publications: &[EmbeddedRenderPublication],
    table_resize: Option<TableResizeCommit>,
    editor_decorations: Option<EditorDecorationStyle>,
) -> Result<ViewportSceneFrame, ViewportSceneError> {
    let source = document.snapshot();
    let definitions = document.markdown().reference_definitions().clone();
    let document_revision = document.revision();
    if let Some(resize) = table_resize {
        if resize.revision() != document_revision {
            return Err(EditorDocumentError::Layout(LayoutError::Upstream(
                "table resize and viewport document revisions differ".into(),
            ))
            .into());
        }
        if matches!(resize.target(), TableResizeTarget::Row { .. }) {
            return Err(EditorDocumentError::Layout(LayoutError::Upstream(
                "row resize requires variable-row table layout".into(),
            ))
            .into());
        }
    }
    let selection = Some(document.selection().ordered_range());
    let image_key = |image: ImageSpan| {
        let destination = image.destination().or_else(|| {
            image
                .reference()
                .and_then(|reference| definitions.lookup(&source, reference))
                .map(|definition| definition.destination())
        })?;
        let start = usize::try_from(destination.start().get()).ok()?;
        let end = usize::try_from(destination.end().get()).ok()?;
        let destination = source.as_str().get(start..end)?;
        ImageKey::new(destination.to_owned()).ok()
    };
    let intrinsic_size = |image: ImageSpan| {
        let key = image_key(image)?;
        if let Some(publication) = image_publications.iter().find(|publication| {
            publication.revision() == document_revision
                && publication.key().fingerprint() == key.fingerprint()
        }) {
            return ImageIntrinsicSize::new(
                publication.image().width(),
                publication.image().height(),
            )
            .ok();
        }
        let intrinsic = image_intrinsics.iter().find(|intrinsic| {
            intrinsic.revision() == document_revision
                && intrinsic.key().fingerprint() == key.fingerprint()
        })?;
        let dimensions = intrinsic.dimensions();
        ImageIntrinsicSize::new(dimensions.width(), dimensions.height()).ok()
    };
    let viewport_snapshot = document
        .visible_blocks_with_visual_state_and_shaper_and_image_resolver(
            viewport,
            shaper,
            intrinsic_size,
        )?;
    let revision = viewport_snapshot.revision();
    let config = document.viewport_config().layout();
    let geometries = viewport_snapshot
        .blocks()
        .iter()
        .copied()
        .map(|block| {
            ViewportBlockGeometry::new(
                revision,
                block.index(),
                block.source(),
                block.y(),
                block.height(),
                block.is_measured(),
                block.kind().viewport_tag(),
            )
        })
        .collect::<Result<Vec<_>, SceneError>>()?;
    let content_height = if viewport_snapshot.blocks().is_empty()
        && source.is_empty()
        && document.composition().is_none()
    {
        config.line_height()
    } else {
        viewport_snapshot.content_height()
    };
    let input = ViewportSceneInput::new(
        revision,
        viewport_snapshot.range().start()..viewport_snapshot.range().end(),
        content_height,
        geometries,
    )?;

    let mut layouts = Vec::with_capacity(viewport_snapshot.blocks().len());
    for block in viewport_snapshot.blocks() {
        let sizes = document.block_image_sizes(block.index(), &intrinsic_size)?;
        let mut layout = document.block_layout_for_visual_state_with_shaper_and_images(
            block.index(),
            config,
            shaper,
            &sizes,
        )?;
        if let Some(resize) = table_resize.filter(|resize| resize.block_index() == block.index()) {
            layout
                .apply_table_resize(resize)
                .map_err(EditorDocumentError::from)?;
        }
        layouts.push(layout);
    }
    let mut builder = SceneBuilder::new(revision, scene_viewport)?;
    // 背景必须是这一帧的第一个 primitive：Metal layer 透明，未触及的像素会
    // 露出下层视图，而 TextKit fallback 删除后下层已不再绘制任何东西
    // （不变量 I5）。放在最前面也保证它位于所有内容之下。
    builder.fill_rect(scene_viewport, background)?;
    // 每个可见块的装饰、字形与图片，全部搬到文档坐标之后交给场景层。
    // 「这些装饰是什么语法」到这里为止：场景层只看见矩形与角色。
    let mut ornaments = Vec::with_capacity(layouts.len());
    let mut overlays = Vec::with_capacity(layouts.len());
    let mut images = Vec::with_capacity(layouts.len());
    let mut glyphs = Vec::with_capacity(layouts.len());
    for (block, layout) in viewport_snapshot.blocks().iter().zip(layouts.iter()) {
        let origin = Point::new(0.0, block.y());
        let mut block_ornaments = Vec::new();
        if let Some(table) = layout.table() {
            append_table_ornaments(
                &mut block_ornaments,
                table,
                origin,
                viewport_table_style(),
                selection,
            )?;
        }
        if let Some(quote) = layout.ornaments().quote() {
            for bounds in quote
                .bars(layout.height())
                .map_err(EditorDocumentError::from)?
            {
                block_ornaments.push(OrnamentPrimitive::new(
                    quote.source(),
                    translate_block_rect(bounds, origin)?,
                    viewport_block_quote_color(),
                    OrnamentRole::Bar,
                ));
            }
        }
        // 任务框压在文字上面，不是衬在下面。
        //
        // 「这个块有没有复选框」不再问 `BlockKind`，问排出来的盒子——装饰
        // 产出了 widget，排版给了它位置，画的人照着画。少一次「块类型说有
        // 而标记问不出来」的错误路径。
        let mut block_overlays = Vec::new();
        for placement in layout.checkboxes() {
            append_task_checkbox(&mut block_overlays, origin, *placement)?;
        }
        overlays.push(block_overlays);

        let mut block_images = Vec::new();
        for placement in layout.images() {
            let Some(image) = layout
                .decorations()
                .widgets()
                .iter()
                .copied()
                .filter_map(|widget| match widget {
                    BlockWidget::Image(image) => Some(image),
                    BlockWidget::Checkbox(_) => None,
                })
                .find(|image| image.source() == placement.source())
            else {
                continue;
            };
            let Some(key) = image_key(image) else {
                // widget 在行里占了一个盒子，而这一层查不到目标。什么都不画
                // 的话那块就是**白的**——用户看不出那里有过一张图，而替代
                // 文字已经进 widget 了。画一个空框，让它看得见。
                //
                // **走到这里意味着装饰与这一层看的不是同一份引用表。**
                // 装饰阶段现在自己查表（不变量 C6），查不到的候选根本不产
                // widget；而 `DecorationCache` 在引用表的指纹变了的时候整个
                // 清掉。两道合起来，编辑器自己那条路走不到这一支。留着它是
                // 因为「什么都不画」比「画错一个框」更难发现——这一支没有
                // 用例，它守的是别的调用方拿一份对不上的表来渲染。
                block_ornaments.push(OrnamentPrimitive::new(
                    image.source(),
                    translate_block_rect(placement.bounds(), origin)?,
                    viewport_broken_image_color(),
                    OrnamentRole::Border,
                ));
                continue;
            };
            block_images.push(ImagePrimitive::new(
                key.fingerprint(),
                translate_block_rect(placement.bounds(), origin)?,
                Rgba8::new(232, 234, 238, 255),
            ));
        }
        ornaments.push(block_ornaments);
        images.push(block_images);

        glyphs.push(
            layout
                .glyphs()
                .iter()
                .copied()
                .map(|glyph| {
                    let scene_glyph = SceneGlyph::new(
                        glyph.face(),
                        glyph.glyph(),
                        glyph.origin(),
                        glyph.size_scale(),
                    );
                    // 「这个字什么颜色」到这里为止：场景层拿到的是 RGBA，
                    // 不是 `TextRole`。
                    match viewport_code_role_color(glyph.role()) {
                        Some(color) => scene_glyph.with_color(color),
                        None => scene_glyph,
                    }
                })
                .collect::<Vec<_>>(),
        );
    }
    let contents = viewport_snapshot
        .blocks()
        .iter()
        .enumerate()
        .map(|(offset, block)| {
            ViewportBlockContent::new(revision, block.source(), &glyphs[offset])
                .with_fill(viewport_block_background(block.kind()))
                .with_ornaments(&ornaments[offset])
                .with_images(&images[offset])
                .with_overlays(&overlays[offset])
        })
        .collect::<Vec<_>>();
    builder.append_viewport(&input, &contents, atlas, font_size, color)?;
    for (block, layout) in viewport_snapshot.blocks().iter().zip(layouts.iter()) {
        let Some(publication) = embedded_publications.iter().find(|publication| {
            publication.revision() == revision
                && publication.source_range() == layout.visual().source_range()
        }) else {
            continue;
        };
        let EmbeddedRenderPayload::Svg { dimensions, .. } = publication.payload() else {
            continue;
        };
        let width = (dimensions.width() as f32).min(scene_viewport.width());
        let height = (dimensions.height() as f32).min(block.height().max(1.0));
        if width <= 0.0 || height <= 0.0 {
            continue;
        }
        let bounds = Rect::new(0.0, block.y(), width, height)?;
        builder.embedded_svg(EmbeddedSvgPrimitive::new(
            publication.key().fingerprint(),
            publication.generation(),
            publication.kind().tag(),
            publication.source_range(),
            bounds,
            dimensions.width(),
            dimensions.height(),
            Rgba8::new(0, 0, 0, 0),
        ))?;
    }
    if let Some(style) = editor_decorations {
        // 搜索底色的两层夹着选区，次序归 `append_editor_decorations` 排——
        // caret 是它的末尾一层，拆出来排会把 caret 盖掉。
        append_editor_decorations(&mut builder, document, &input, &layouts, style)?;
    }
    Ok(ViewportSceneFrame {
        input,
        scene: builder.finish(),
    })
}

/// Builds a scene and its backend-neutral render plan from one editor viewport
/// measurement. The render-plan builder is updated only after scene assembly
/// succeeds; callers can then publish the returned pair through
/// `ViewportFrameCache::publish_if_current`.
pub fn assemble_viewport_render_frame<S: ShapingProvider>(
    document: &mut EditorDocument,
    config: ViewportRenderConfig,
    shaper: &S,
    atlas: &GlyphAtlas,
    render_plans: &mut RenderPlanBuilder,
) -> Result<ViewportRenderFrame, ViewportSceneError> {
    assemble_viewport_render_frame_with_images(
        document,
        config.viewport(),
        config,
        shaper,
        atlas,
        render_plans,
        &[],
    )
}

/// Builds a render frame while applying ready image intrinsic dimensions.
pub fn assemble_viewport_render_frame_with_images<S: ShapingProvider>(
    document: &mut EditorDocument,
    viewport: ViewportSpan,
    config: ViewportRenderConfig,
    shaper: &S,
    atlas: &GlyphAtlas,
    render_plans: &mut RenderPlanBuilder,
    image_publications: &[ImagePublication],
) -> Result<ViewportRenderFrame, ViewportSceneError> {
    assemble_viewport_render_frame_with_images_and_intrinsics(
        document,
        viewport,
        config,
        shaper,
        atlas,
        render_plans,
        image_publications,
        &[],
    )
}

/// Builds a render frame while applying ready image pixels and retained
/// intrinsic dimensions.
#[allow(clippy::too_many_arguments)]
pub fn assemble_viewport_render_frame_with_images_and_intrinsics<S: ShapingProvider>(
    document: &mut EditorDocument,
    viewport: ViewportSpan,
    config: ViewportRenderConfig,
    shaper: &S,
    atlas: &GlyphAtlas,
    render_plans: &mut RenderPlanBuilder,
    image_publications: &[ImagePublication],
    image_intrinsics: &[ImageIntrinsicPublication],
) -> Result<ViewportRenderFrame, ViewportSceneError> {
    assemble_viewport_render_frame_with_images_and_intrinsics_and_embedded(
        document,
        viewport,
        config,
        shaper,
        atlas,
        render_plans,
        image_publications,
        image_intrinsics,
        &[],
    )
}

/// Builds a render frame while applying ready image dimensions and explicit
/// revision-bound embedded SVG publications. The same publication list is
/// passed to scene assembly and RenderPlan construction so a command cannot
/// be emitted without its matching upload payload.
#[allow(clippy::too_many_arguments)]
pub fn assemble_viewport_render_frame_with_images_and_intrinsics_and_embedded<
    S: ShapingProvider,
>(
    document: &mut EditorDocument,
    viewport: ViewportSpan,
    config: ViewportRenderConfig,
    shaper: &S,
    atlas: &GlyphAtlas,
    render_plans: &mut RenderPlanBuilder,
    image_publications: &[ImagePublication],
    image_intrinsics: &[ImageIntrinsicPublication],
    embedded_publications: &[EmbeddedRenderPublication],
) -> Result<ViewportRenderFrame, ViewportSceneError> {
    let scene = assemble_viewport_scene_with_images_and_intrinsics_and_embedded_and_table_resize(
        document,
        viewport,
        shaper,
        config.font_size(),
        config.scene_viewport(),
        atlas,
        config.color(),
        config.background(),
        image_publications,
        image_intrinsics,
        embedded_publications,
        config.table_resize(),
        config.editor_decorations(),
    )?;
    if scene.revision() != document.revision() {
        return Err(ViewportFrameError::Stale {
            expected: document.revision(),
            actual: scene.revision(),
        }
        .into());
    }
    let plan = render_plans.build_with_embedded(scene.scene(), atlas, embedded_publications)?;
    ViewportRenderFrame::new(scene, plan).map_err(ViewportSceneError::from)
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use yu_assets::{EmbeddedRenderPayload, EmbeddedRenderRequest, EmbeddedResourceKind};
    use yu_core::{ByteOffset, TextRange, Utf16Offset, Utf16Range};
    use yu_editor::{
        CaretAffinity,
        EditorCommand,
        EditorSelection,
        LayoutConfig,
        LayoutPoint,
        TableResizeGesture,
        ViewportConfig,
        // 复选框的源码区间由 widget 带着走，用例拿 `block_sequence` 那一份
        // 当参照——**判据不来自被测的那条路**（两份判断由
        // `yu-markdown/tests/task_identity.rs` 锁在一起）。
        task_marker,
    };
    use yu_font::{
        FontDatabase, FontFaceSpec, FontRequest, FontShaper, GlyphAtlasConfig, GlyphBitmap,
        GlyphMetrics, GlyphRasterKey, RasterizedGlyph,
    };
    use yu_render::RenderPlanBuilder;
    use yu_scene::{Primitive, Rgba8};

    use super::*;

    fn shaper(font_size: f32) -> FontShaper {
        let mut database = FontDatabase::new();
        database
            .register(FontFaceSpec::new("Test", 0.5))
            .expect("font face");
        FontShaper::new(
            Arc::new(database),
            FontRequest::new("Test", font_size).expect("font request"),
        )
        .expect("shaper")
    }

    fn atlas_for_document(
        document: &mut EditorDocument,
        viewport: ViewportSpan,
        shaper: &FontShaper,
        font_size: f32,
    ) -> GlyphAtlas {
        let snapshot = document
            .visible_blocks_with_visual_state_and_shaper(viewport, shaper)
            .expect("viewport");
        let config = document.viewport_config().layout();
        let mut atlas = GlyphAtlas::new(GlyphAtlasConfig::new(64, 64, 1).expect("atlas config"));
        for block in snapshot.blocks() {
            let layout = document
                .block_layout_for_visual_state_with_shaper(block.index(), config, shaper)
                .expect("layout");
            for placement in layout.glyphs() {
                let key = GlyphRasterKey::new(
                    placement.face(),
                    placement.glyph(),
                    font_size * placement.size_scale(),
                )
                .expect("glyph key");
                if atlas.entry(key).is_none() {
                    let glyph = RasterizedGlyph::new(
                        key,
                        GlyphMetrics::new(0.0, 10.0, 7.0).expect("metrics"),
                        GlyphBitmap::new(2, 3, 2, vec![255; 6]).expect("bitmap"),
                    );
                    atlas.insert(glyph).expect("atlas insert");
                }
            }
        }
        atlas
    }

    #[test]
    fn editor_viewport_is_assembled_into_revision_bound_render_plan() {
        let font_size = 14.0;
        let shaper = shaper(font_size);
        let viewport = ViewportSpan::new(0.0, 200.0);
        let mut document = EditorDocument::new("# title\n\nhello **world**");
        document
            .set_viewport_config(ViewportConfig::new(
                LayoutConfig::new(240.0, 20.0),
                20.0,
                0.0,
            ))
            .expect("viewport config");
        let atlas = atlas_for_document(&mut document, viewport, &shaper, font_size);
        let frame = assemble_viewport_scene(
            &mut document,
            viewport,
            &shaper,
            font_size,
            Rect::new(0.0, 0.0, 240.0, 200.0).expect("scene viewport"),
            &atlas,
            Rgba8::black(),
        )
        .expect("scene frame");

        assert_eq!(frame.revision(), document.revision());
        assert_eq!(frame.input().revision(), document.revision());
        assert_eq!(frame.input().blocks().len(), 3);
        assert!(!frame.scene().primitives().is_empty());
        let mut plans = RenderPlanBuilder::new();
        let plan = plans.build(frame.scene(), &atlas).expect("render plan");
        assert_eq!(plan.revision(), document.revision());
        assert_eq!(plan.commands().len(), frame.scene().primitives().len());
        assert!(
            plan.commands()
                .iter()
                .any(|command| matches!(command, yu_render::RenderCommand::Glyph { .. }))
        );
        // 首个 primitive 是整帧背景，其余都应是字形。
        let primitives = frame.scene().primitives();
        assert!(matches!(primitives[0], Primitive::FillRect { .. }));
        assert!(
            primitives[1..]
                .iter()
                .all(|primitive| matches!(primitive, Primitive::Glyph(_)))
        );
    }

    #[test]
    fn blockquote_bar_precedes_glyphs_and_lowers_to_source_backed_fill() {
        let font_size = 14.0;
        let shaper = shaper(font_size);
        let viewport = ViewportSpan::new(0.0, 120.0);
        let source = "> quoted\n> second\n";
        let mut document = EditorDocument::new(source);
        document
            .set_viewport_config(ViewportConfig::new(
                LayoutConfig::new(240.0, 20.0),
                20.0,
                0.0,
            ))
            .expect("viewport config");
        let atlas = atlas_for_document(&mut document, viewport, &shaper, font_size);
        let config = ViewportRenderConfig::new(
            viewport,
            font_size,
            Rect::new(0.0, 0.0, 240.0, 120.0).expect("scene viewport"),
            Rgba8::black(),
        );
        let mut plans = RenderPlanBuilder::new();
        let frame =
            assemble_viewport_render_frame(&mut document, config, &shaper, &atlas, &mut plans)
                .expect("blockquote frame");
        let primitives = frame.scene().scene().primitives();
        let quote_index = primitives
            .iter()
            .position(|primitive| matches!(primitive, Primitive::Ornament(ornament) if ornament.role() == OrnamentRole::Bar))
            .expect("blockquote primitive");
        let glyph_index = primitives
            .iter()
            .position(|primitive| matches!(primitive, Primitive::Glyph(_)))
            .expect("glyph primitive");
        assert!(quote_index < glyph_index);

        let Primitive::Ornament(quote) = primitives[quote_index] else {
            unreachable!("located blockquote primitive");
        };
        assert_eq!(
            quote.source(),
            TextRange::new(ByteOffset::ZERO, ByteOffset::new(source.len() as u64))
                .expect("source range")
        );
        assert_eq!(quote.color(), viewport_block_quote_color());
        assert_eq!(
            quote.bounds().height(),
            frame.scene().input().blocks()[0].height()
        );
        assert!(quote.bounds().right() < 7.0);

        let yu_render::RenderCommand::FillRect { bounds, color } =
            frame.plan().commands()[quote_index]
        else {
            panic!("blockquote must lower to a fill command");
        };
        assert_eq!(bounds, quote.bounds());
        assert_eq!(color, quote.color());
    }

    #[test]
    fn heading_and_body_share_a_frame_with_distinct_raster_sizes() {
        let font_size = 14.0;
        let shaper = shaper(font_size);
        let viewport = ViewportSpan::new(0.0, 180.0);
        let mut document = EditorDocument::new("# title\n\nplain");
        document
            .set_viewport_config(ViewportConfig::new(
                LayoutConfig::new(240.0, 20.0),
                20.0,
                0.0,
            ))
            .expect("viewport config");
        let atlas = atlas_for_document(&mut document, viewport, &shaper, font_size);
        let config = ViewportRenderConfig::new(
            viewport,
            font_size,
            Rect::new(0.0, 0.0, 240.0, 180.0).expect("scene viewport"),
            Rgba8::black(),
        );
        let mut plans = RenderPlanBuilder::new();
        let frame =
            assemble_viewport_render_frame(&mut document, config, &shaper, &atlas, &mut plans)
                .expect("mixed typography frame");

        let glyph_sizes = frame
            .scene()
            .scene()
            .primitives()
            .iter()
            .filter_map(|primitive| match primitive {
                Primitive::Glyph(glyph) => Some(glyph.key().size()),
                _ => None,
            })
            .collect::<Vec<_>>();
        let heading = glyph_sizes
            .iter()
            .position(|size| *size == 28.0)
            .expect("H1 raster size");
        let body = glyph_sizes
            .iter()
            .position(|size| *size == 14.0)
            .expect("body raster size");
        assert!(heading < body);
        assert_eq!(
            frame.plan().commands().len(),
            frame.scene().scene().primitives().len()
        );
        assert!(frame.scene().input().blocks()[0].height() > 20.0);
    }

    #[test]
    fn empty_document_publishes_one_line_retained_caret_frame() {
        let font_size = 14.0;
        let shaper = shaper(font_size);
        let viewport = ViewportSpan::new(0.0, 120.0);
        let mut document = EditorDocument::new("");
        document
            .set_viewport_config(ViewportConfig::new(
                LayoutConfig::new(240.0, 20.0),
                20.0,
                0.0,
            ))
            .expect("viewport config");
        let atlas = atlas_for_document(&mut document, viewport, &shaper, font_size);
        let config = ViewportRenderConfig::new(
            viewport,
            font_size,
            Rect::new(0.0, 0.0, 240.0, 120.0).expect("scene viewport"),
            Rgba8::black(),
        )
        .with_editor_decorations(EditorDecorationStyle::new(
            Rgba8::new(0, 122, 255, 97),
            Rgba8::black(),
            Rgba8::new(0, 122, 255, 255),
            1.0,
        ));
        let mut plans = RenderPlanBuilder::new();
        let frame =
            assemble_viewport_render_frame(&mut document, config, &shaper, &atlas, &mut plans)
                .expect("empty retained frame");

        assert!(frame.scene().input().blocks().is_empty());
        assert_eq!(frame.scene().input().content_height(), 20.0);
        let primitives = frame.scene().scene().primitives();
        // 背景 + caret：空文档也必须画出背景，否则透出下层视图。
        assert_eq!(primitives.len(), 2);
        assert!(matches!(primitives[0], Primitive::FillRect { .. }));
        let Primitive::EditorDecoration(caret) = primitives[1] else {
            panic!("empty scene must contain its background and caret");
        };
        assert_eq!(caret.role(), EditorDecorationPrimitiveRole::Caret);
        assert_eq!(caret.source(), TextRange::empty(ByteOffset::new(0)));
        assert_eq!(
            caret.bounds(),
            Rect::new(0.0, 0.0, 1.0, 20.0).expect("empty caret bounds")
        );
        // 背景 + caret 两条 fill 指令。
        assert!(matches!(
            frame.plan().commands(),
            [
                yu_render::RenderCommand::FillRect { .. },
                yu_render::RenderCommand::FillRect { .. }
            ]
        ));
    }

    #[test]
    fn whitespace_document_keeps_a_submittable_retained_caret_frame() {
        let font_size = 14.0;
        let shaper = shaper(font_size);
        let viewport = ViewportSpan::new(0.0, 120.0);
        let mut document = EditorDocument::new("   \n");
        document
            .set_viewport_config(ViewportConfig::new(
                LayoutConfig::new(240.0, 20.0),
                20.0,
                0.0,
            ))
            .expect("viewport config");
        let atlas = atlas_for_document(&mut document, viewport, &shaper, font_size);
        let config = ViewportRenderConfig::new(
            viewport,
            font_size,
            Rect::new(0.0, 0.0, 240.0, 120.0).expect("scene viewport"),
            Rgba8::black(),
        )
        .with_editor_decorations(EditorDecorationStyle::new(
            Rgba8::new(0, 122, 255, 97),
            Rgba8::black(),
            Rgba8::new(0, 122, 255, 255),
            1.0,
        ));
        let mut plans = RenderPlanBuilder::new();
        let frame =
            assemble_viewport_render_frame(&mut document, config, &shaper, &atlas, &mut plans)
                .expect("whitespace retained frame");

        assert!(frame.scene().input().content_height() >= 20.0);
        assert!(!frame.plan().commands().is_empty());
        assert!(frame.scene().scene().primitives().iter().any(|primitive| {
            matches!(
                primitive,
                Primitive::EditorDecoration(decoration)
                    if decoration.role() == EditorDecorationPrimitiveRole::Caret
            )
        }));
    }

    #[test]
    fn empty_document_composition_does_not_hide_unprojected_preedit() {
        let font_size = 14.0;
        let shaper = shaper(font_size);
        let viewport = ViewportSpan::new(0.0, 120.0);
        let mut document = EditorDocument::new("");
        document
            .set_viewport_config(ViewportConfig::new(
                LayoutConfig::new(240.0, 20.0),
                20.0,
                0.0,
            ))
            .expect("viewport config");
        document
            .begin_composition(
                TextRange::empty(ByteOffset::new(0)),
                "日",
                Utf16Range::new(Utf16Offset::new(0), Utf16Offset::new(1))
                    .expect("preedit selection"),
            )
            .expect("begin composition");
        let atlas = atlas_for_document(&mut document, viewport, &shaper, font_size);
        let config = ViewportRenderConfig::new(
            viewport,
            font_size,
            Rect::new(0.0, 0.0, 240.0, 120.0).expect("scene viewport"),
            Rgba8::black(),
        )
        .with_editor_decorations(EditorDecorationStyle::new(
            Rgba8::new(0, 122, 255, 97),
            Rgba8::black(),
            Rgba8::new(0, 122, 255, 255),
            1.0,
        ));
        let mut plans = RenderPlanBuilder::new();
        let frame =
            assemble_viewport_render_frame(&mut document, config, &shaper, &atlas, &mut plans)
                .expect("composition fallback frame");

        // 只剩背景：preedit 未投影时不得凭空画出内容，但背景仍要覆盖，
        // 否则会透出下层视图。
        assert!(matches!(
            frame.plan().commands(),
            [yu_render::RenderCommand::FillRect { .. }]
        ));
        assert!(matches!(
            frame.scene().scene().primitives(),
            [Primitive::FillRect { .. }]
        ));
    }

    #[test]
    fn configured_editor_decorations_publish_selection_and_caret_layers() {
        let font_size = 14.0;
        let shaper = shaper(font_size);
        let viewport = ViewportSpan::new(0.0, 120.0);
        let mut document = EditorDocument::new("alpha beta");
        document
            .set_viewport_config(ViewportConfig::new(
                LayoutConfig::new(240.0, 20.0),
                20.0,
                0.0,
            ))
            .expect("viewport config");
        let snapshot = document.snapshot();
        document
            .set_selection(
                EditorSelection::range(
                    &snapshot,
                    ByteOffset::new(0),
                    ByteOffset::new(5),
                    CaretAffinity::Downstream,
                )
                .expect("selection"),
            )
            .expect("set selection");
        let atlas = atlas_for_document(&mut document, viewport, &shaper, font_size);
        let style = EditorDecorationStyle::new(
            Rgba8::new(0, 122, 255, 97),
            Rgba8::black(),
            Rgba8::new(0, 122, 255, 255),
            1.0,
        );
        let config = ViewportRenderConfig::new(
            viewport,
            font_size,
            Rect::new(0.0, 0.0, 240.0, 120.0).expect("scene viewport"),
            Rgba8::black(),
        )
        .with_editor_decorations(style);
        let mut plans = RenderPlanBuilder::new();
        let frame =
            assemble_viewport_render_frame(&mut document, config, &shaper, &atlas, &mut plans)
                .expect("decorated frame");
        let decorations = frame
            .scene()
            .scene()
            .primitives()
            .iter()
            .filter_map(|primitive| match primitive {
                Primitive::EditorDecoration(decoration) => Some(*decoration),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(
            decorations
                .iter()
                .filter(|decoration| {
                    decoration.role() == EditorDecorationPrimitiveRole::Selection
                })
                .count(),
            1
        );
        let caret = decorations
            .iter()
            .find(|decoration| decoration.role() == EditorDecorationPrimitiveRole::Caret)
            .expect("caret decoration");
        assert_eq!(caret.source(), TextRange::empty(ByteOffset::new(5)));
        assert_eq!(caret.bounds().width(), 1.0);
        assert_eq!(
            frame.plan().commands().len(),
            frame.scene().scene().primitives().len()
        );
    }

    #[test]
    fn retained_frame_reveals_active_markdown_syntax_without_source_edit() {
        let source = "# before **strong** after\n\nplain";
        let font_size = 14.0;
        let shaper = shaper(font_size);
        let viewport = ViewportSpan::new(0.0, 120.0);
        let mut document = EditorDocument::new(source);
        // 基线要求「没有任何语法被聚焦」。新文档的光标落在文首，正好会揭示
        // 第一个块的语法，因此这里把它显式移到最后一个普通段落里。
        {
            let snapshot = document.snapshot();
            document
                .set_selection(
                    EditorSelection::cursor(
                        &snapshot,
                        snapshot.len_bytes(),
                        CaretAffinity::Downstream,
                    )
                    .expect("baseline caret"),
                )
                .expect("baseline selection");
        }
        document
            .set_viewport_config(ViewportConfig::new(
                LayoutConfig::new(240.0, 20.0),
                20.0,
                0.0,
            ))
            .expect("viewport config");
        let style = EditorDecorationStyle::new(
            Rgba8::new(0, 122, 255, 97),
            Rgba8::black(),
            Rgba8::new(0, 122, 255, 255),
            1.0,
        );
        let config = ViewportRenderConfig::new(
            viewport,
            font_size,
            Rect::new(0.0, 0.0, 240.0, 120.0).expect("scene viewport"),
            Rgba8::black(),
        )
        .with_editor_decorations(style);
        let atlas = atlas_for_document(&mut document, viewport, &shaper, font_size);
        let mut plans = RenderPlanBuilder::new();
        let hidden =
            assemble_viewport_render_frame(&mut document, config, &shaper, &atlas, &mut plans)
                .expect("hidden frame");
        let hidden_glyphs = hidden
            .scene()
            .scene()
            .primitives()
            .iter()
            .filter(|primitive| matches!(primitive, Primitive::Glyph(_)))
            .count();
        let revision = document.revision();

        let strong = source.find("strong").expect("strong content");
        let snapshot = document.snapshot();
        document
            .set_selection(
                EditorSelection::cursor(
                    &snapshot,
                    ByteOffset::new((strong + 2) as u64),
                    CaretAffinity::Downstream,
                )
                .expect("selection"),
            )
            .expect("set selection");
        let atlas = atlas_for_document(&mut document, viewport, &shaper, font_size);
        let revealed =
            assemble_viewport_render_frame(&mut document, config, &shaper, &atlas, &mut plans)
                .expect("revealed frame");
        let revealed_glyphs = revealed
            .scene()
            .scene()
            .primitives()
            .iter()
            .filter(|primitive| matches!(primitive, Primitive::Glyph(_)))
            .count();

        assert_eq!(document.revision(), revision);
        assert_eq!(revealed.revision(), revision);
        assert_eq!(revealed_glyphs, hidden_glyphs + 6);
        let caret = revealed
            .scene()
            .scene()
            .primitives()
            .iter()
            .find_map(|primitive| match primitive {
                Primitive::EditorDecoration(decoration)
                    if decoration.role() == EditorDecorationPrimitiveRole::Caret =>
                {
                    Some(*decoration)
                }
                _ => None,
            })
            .expect("revealed caret");
        assert_eq!(
            caret.source(),
            TextRange::empty(ByteOffset::new((strong + 2) as u64))
        );
        assert!(caret.bounds().x() > 0.0);
    }

    #[test]
    fn composition_decorations_use_transient_projection_without_source_edit() {
        let font_size = 14.0;
        let shaper = shaper(font_size);
        let viewport = ViewportSpan::new(0.0, 120.0);
        let mut document = EditorDocument::new("日本 alpha");
        document
            .set_viewport_config(ViewportConfig::new(
                LayoutConfig::new(240.0, 20.0),
                20.0,
                0.0,
            ))
            .expect("viewport config");
        let revision = document.revision();
        document
            .begin_composition(
                TextRange::new(ByteOffset::new(7), ByteOffset::new(12)).expect("replacement"),
                "日本",
                Utf16Range::new(Utf16Offset::new(0), Utf16Offset::new(1))
                    .expect("preedit selection"),
            )
            .expect("begin composition");
        let atlas = atlas_for_document(&mut document, viewport, &shaper, font_size);
        let config = ViewportRenderConfig::new(
            viewport,
            font_size,
            Rect::new(0.0, 0.0, 240.0, 120.0).expect("scene viewport"),
            Rgba8::black(),
        )
        .with_editor_decorations(EditorDecorationStyle::new(
            Rgba8::new(0, 122, 255, 97),
            Rgba8::black(),
            Rgba8::new(0, 122, 255, 255),
            1.0,
        ));
        let mut plans = RenderPlanBuilder::new();
        let frame =
            assemble_viewport_render_frame(&mut document, config, &shaper, &atlas, &mut plans)
                .expect("composition frame");
        assert_eq!(document.revision(), revision);
        let roles = frame
            .scene()
            .scene()
            .primitives()
            .iter()
            .filter_map(|primitive| match primitive {
                Primitive::EditorDecoration(decoration) => Some(decoration.role()),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert!(roles.contains(&EditorDecorationPrimitiveRole::Selection));
        assert!(roles.contains(&EditorDecorationPrimitiveRole::CompositionCaret));
        assert!(!roles.contains(&EditorDecorationPrimitiveRole::Caret));
    }

    /// 复选框画在**排版给它的那个盒子**里，不是画在一个自己算出来的点上。
    ///
    /// 这一条压的是它成为 widget 的全部理由。此前场景层拿标记起点的 caret
    /// 当左上角、行高乘 0.68 当边长自己算一遍——那是「同一个几何两套实现」，
    /// 而被隐藏的 `[x]` 塌成一个点，点上没有宽度可用，于是方框压在正文的
    /// 第一个字上。截图一眼就看得见，当时所有断言都是绿的。
    ///
    /// 判据来自 `BlockView::checkboxes()`——排版那一条路，与场景层画的那一条
    /// 分开。
    #[test]
    fn a_checkbox_is_painted_where_layout_put_its_box() {
        let font_size = 14.0;
        let shaper = shaper(font_size);
        let viewport = ViewportSpan::new(0.0, 120.0);
        let mut document = EditorDocument::new("- [x] done\n");
        document
            .set_viewport_config(ViewportConfig::new(
                LayoutConfig::new(240.0, 20.0),
                20.0,
                0.0,
            ))
            .expect("viewport config");
        let config = document.viewport_config().layout();
        let placement = document
            .block_layout_for_visual_state_with_shaper(0, config, &shaper)
            .expect("block layout")
            .checkboxes()
            .first()
            .copied()
            .expect("这个块上有一个复选框");

        let atlas = atlas_for_document(&mut document, viewport, &shaper, font_size);
        let frame = assemble_viewport_scene(
            &mut document,
            viewport,
            &shaper,
            font_size,
            Rect::new(0.0, 0.0, 240.0, 120.0).expect("scene viewport"),
            &atlas,
            Rgba8::black(),
        )
        .expect("task scene");
        let border = frame
            .scene()
            .primitives()
            .iter()
            .find_map(|primitive| match primitive {
                Primitive::Ornament(ornament) if ornament.role() == OrnamentRole::Border => {
                    Some(*ornament)
                }
                _ => None,
            })
            .expect("复选框的边框");

        let bounds = placement.bounds();
        assert_eq!(border.bounds().width(), bounds.width());
        assert_eq!(border.bounds().height(), bounds.height());
        assert_eq!(border.bounds().x(), bounds.x(), "x 必须来自排好的盒子");
        assert!(bounds.width() > 0.0, "盒子没有宽度就等于没有占位");
        assert_eq!(border.source(), placement.source());
    }

    #[test]
    fn task_markers_become_source_backed_checkbox_layers() {
        let font_size = 14.0;
        let shaper = shaper(font_size);
        let viewport = ViewportSpan::new(0.0, 120.0);
        let mut document = EditorDocument::new("- [ ] todo\n- [x] done\n");
        document
            .set_viewport_config(ViewportConfig::new(
                LayoutConfig::new(240.0, 20.0),
                20.0,
                0.0,
            ))
            .expect("viewport config");
        let atlas = atlas_for_document(&mut document, viewport, &shaper, font_size);
        let frame = assemble_viewport_scene(
            &mut document,
            viewport,
            &shaper,
            font_size,
            Rect::new(0.0, 0.0, 240.0, 120.0).expect("scene viewport"),
            &atlas,
            Rgba8::black(),
        )
        .expect("task scene");
        let layers = frame
            .scene()
            .primitives()
            .iter()
            .filter_map(|primitive| match primitive {
                Primitive::Ornament(task) => Some(*task),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(layers.len(), 7);
        let snapshot = document.snapshot();
        let todo = task_marker(
            &snapshot,
            document.markdown().blocks().get(0).expect("todo block"),
        )
        .expect("todo marker")
        .range();
        let done = task_marker(
            &snapshot,
            document.markdown().blocks().get(1).expect("done block"),
        )
        .expect("done marker")
        .range();
        assert_eq!(
            layers.iter().filter(|layer| layer.source() == todo).count(),
            2
        );
        assert_eq!(
            layers.iter().filter(|layer| layer.source() == done).count(),
            5
        );
        assert!(
            layers.iter().any(|layer| {
                layer.source() == todo && layer.role() == OrnamentRole::Background
            })
        );
        assert!(
            layers
                .iter()
                .any(|layer| { layer.source() == done && layer.role() == OrnamentRole::Mark })
        );
        assert!(
            layers
                .iter()
                .all(|layer| { layer.bounds().width() > 0.0 && layer.bounds().height() > 0.0 })
        );

        let initial_revision = document.revision();
        document
            .execute(EditorCommand::toggle_task(0))
            .expect("toggle todo");
        assert!(document.revision() > initial_revision);
        let toggled_atlas = atlas_for_document(&mut document, viewport, &shaper, font_size);
        let toggled = assemble_viewport_scene(
            &mut document,
            viewport,
            &shaper,
            font_size,
            Rect::new(0.0, 0.0, 240.0, 120.0).expect("scene viewport"),
            &toggled_atlas,
            Rgba8::black(),
        )
        .expect("toggled task scene");
        assert_eq!(
            toggled
                .scene()
                .primitives()
                .iter()
                .filter(|primitive| {
                    matches!(
                        primitive,
                        Primitive::Ornament(layer) if layer.source() == todo
                    )
                })
                .count(),
            5
        );
    }

    #[test]
    fn task_checkbox_hit_test_uses_published_revision_and_border_geometry() {
        let font_size = 14.0;
        let shaper = shaper(font_size);
        let viewport = ViewportSpan::new(0.0, 120.0);
        let mut document = EditorDocument::new("- [ ] todo\nparagraph\n- [x] done\n");
        document
            .set_viewport_config(ViewportConfig::new(
                LayoutConfig::new(240.0, 20.0),
                20.0,
                0.0,
            ))
            .expect("viewport config");
        let atlas = atlas_for_document(&mut document, viewport, &shaper, font_size);
        let frame = assemble_viewport_scene(
            &mut document,
            viewport,
            &shaper,
            font_size,
            Rect::new(0.0, 0.0, 240.0, 120.0).expect("scene viewport"),
            &atlas,
            Rgba8::black(),
        )
        .expect("task scene");
        let border = frame
            .scene()
            .primitives()
            .iter()
            .find_map(|primitive| match primitive {
                Primitive::Ornament(task) if task.role() == OrnamentRole::Border => Some(*task),
                _ => None,
            })
            .expect("todo checkbox border");
        let bounds = border.bounds();
        let point = Point::new(
            bounds.x() + bounds.width() * 0.5,
            bounds.y() + bounds.height() * 0.5,
        );
        let hit = frame
            .task_checkbox_hit_test(frame.revision(), point)
            .expect("current revision")
            .expect("checkbox hit");
        assert_eq!(hit.revision(), document.revision());
        assert_eq!(hit.block_index(), 0);
        assert_eq!(hit.source(), border.source());
        assert_eq!(hit.bounds(), bounds);
        assert_eq!(
            frame
                .task_checkbox_hit_test(
                    frame.revision(),
                    Point::new(bounds.right() + 1.0, bounds.y())
                )
                .expect("outside query"),
            None
        );
        assert!(matches!(
            frame.task_checkbox_hit_test(Revision::new(frame.revision().get() + 1), point),
            Err(ViewportFrameError::Stale { .. })
        ));
        assert_eq!(
            frame
                .task_checkbox_hit_test(frame.revision(), Point::new(f32::NAN, bounds.y()))
                .expect("non-finite point is not a hit"),
            None
        );
    }

    #[test]
    fn published_math_is_consumed_by_viewport_scene_and_render_plan() {
        let font_size = 14.0;
        let shaper = shaper(font_size);
        let viewport = ViewportSpan::new(0.0, 240.0);
        let mut document = EditorDocument::new("```math\nx^2 + y^2\n```\n");
        document
            .set_viewport_config(ViewportConfig::new(
                LayoutConfig::new(240.0, 20.0),
                20.0,
                0.0,
            ))
            .expect("viewport config");
        let atlas = atlas_for_document(&mut document, viewport, &shaper, font_size);
        let source_range = document
            .block_decorations(0)
            .expect("math decorations")
            .range();
        let request = EmbeddedRenderRequest::new(
            document.revision(),
            source_range,
            EmbeddedResourceKind::Math,
            "x^2 + y^2",
        )
        .expect("math request");
        let mut cache = yu_assets::EmbeddedResourceCache::new();
        let publication = cache
            .publish(
                request,
                document.revision(),
                EmbeddedRenderPayload::svg(640, 320, "<svg viewBox=\"0 0 640 320\"/>")
                    .expect("math SVG"),
            )
            .expect("math publication");
        let mut plans = RenderPlanBuilder::new();
        let frame = assemble_viewport_render_frame_with_images_and_intrinsics_and_embedded(
            &mut document,
            viewport,
            ViewportRenderConfig::new(
                viewport,
                font_size,
                Rect::new(0.0, 0.0, 240.0, 240.0).expect("scene viewport"),
                Rgba8::black(),
            ),
            &shaper,
            &atlas,
            &mut plans,
            &[],
            &[],
            std::slice::from_ref(&publication),
        )
        .expect("embedded frame");
        assert!(frame.scene().scene().primitives().iter().any(|primitive| {
            matches!(primitive, Primitive::EmbeddedSvg(svg) if svg.source() == source_range)
        }));
        assert!(frame.plan().commands().iter().any(|command| {
            matches!(command, yu_render::RenderCommand::EmbeddedSvg {
                resource,
                generation,
                ..
            } if *resource == publication.key().fingerprint()
                && *generation == publication.generation())
        }));
        assert_eq!(frame.plan().embedded_uploads().len(), 1);
        assert_eq!(frame.plan().embedded_uploads()[0].source(), source_range);
    }

    /// 网格的横线画在**每一行自己的上沿**，不是按常数行高等距排。
    ///
    /// 行高不是常数：一格里的内容换行之后那一行更高。按 `行号 × 常数行高`
    /// 画的话，越往下横线与格子错得越开——不 panic、不报错，只是画歪。
    #[test]
    fn table_grid_lines_follow_each_row_top() {
        let font_size = 14.0;
        let shaper = shaper(font_size);
        let viewport = ViewportSpan::new(0.0, 400.0);
        // 中间那一行的一格长到必须换行，于是它比上下两行都高。**要三行**：
        // 只有两行时，第一行的高度恰好等于到第二行的间距，「按第一行的高度
        // 等距画」与「按每行自己的上沿画」画出来一样，压不住。
        let mut document = EditorDocument::new(
            "| a | b |\n| --- | --- |\n| 这一格的内容长到必须换行才放得下 | x |\n| c | d |\n",
        );
        // 宽度窄到列必须压缩：压缩之后长的那一格放不下，只能换行。
        let config = LayoutConfig::new(10.0, 20.0);
        document
            .set_viewport_config(ViewportConfig::new(config, 20.0, 0.0))
            .expect("viewport config");
        let atlas = atlas_for_document(&mut document, viewport, &shaper, font_size);

        // 参照必须来自**场景走的那一条路**：场景按 shaper 排，而
        // `block_layout` 按 `MonospaceMetrics` 排，两者断行不同，行高也就
        // 不同。拿后者当判据会得到一条假红。
        let laid_out = document
            .block_layout_for_visual_state_with_shaper(0, config, &shaper)
            .expect("block layout");
        let table = laid_out.table().expect("这个块是一张表");
        let rows: Vec<(f32, f32)> = table
            .rows()
            .iter()
            .map(|row| (row.y(), row.height()))
            .collect();
        let total_width = table.bounds().width();
        assert!(
            rows.iter()
                .any(|(_, height)| *height > config.line_height()),
            "语料要造出一行比 line_height 高的行，实际是 {rows:?}"
        );

        let frame = assemble_viewport_scene_with_images(
            &mut document,
            viewport,
            &shaper,
            font_size,
            Rect::new(0.0, 0.0, 160.0, 400.0).expect("scene viewport"),
            &atlas,
            Rgba8::black(),
            &[],
        )
        .expect("scene frame");
        // 横线是「宽度等于整张表、高度等于线宽」的那些 Border。
        let mut tops: Vec<f32> = frame
            .scene()
            .primitives()
            .iter()
            .filter_map(|primitive| match primitive {
                Primitive::Ornament(ornament)
                    if ornament.role() == OrnamentRole::Border
                        && (ornament.bounds().width() - total_width).abs() < 0.001 =>
                {
                    Some(ornament.bounds().y())
                }
                _ => None,
            })
            .collect();
        tops.sort_by(f32::total_cmp);
        tops.dedup_by(|a, b| (*a - *b).abs() < 0.001);

        for (y, _) in &rows {
            assert!(
                tops.iter().any(|top| (top - y).abs() < 0.001),
                "第 {y} 行的上沿没有横线，横线在 {tops:?}，行是 {rows:?}"
            );
        }
    }

    /// 查不到定义的引用**根本不是**图片，一个盒子都不占。
    ///
    /// 不变量 C6 说 parser 只产出候选引用，成立与否由装饰阶段判定。此前装饰
    /// 阶段不查表，于是 `![alt][undefined]` 也占一个 widget，而这一层查不到
    /// `ImageKey`——画面上是一个空框（S6 第七刀登记的那条行为变化）。现在它是
    /// 一段普通文字。
    #[test]
    fn an_unresolvable_reference_is_not_an_image_at_all() {
        let font_size = 14.0;
        let shaper = shaper(font_size);
        let viewport = ViewportSpan::new(0.0, 160.0);
        let mut document = EditorDocument::new("![alt][undefined]");
        document
            .set_viewport_config(ViewportConfig::new(
                LayoutConfig::new(240.0, 20.0),
                20.0,
                0.0,
            ))
            .expect("viewport config");
        let atlas = atlas_for_document(&mut document, viewport, &shaper, font_size);
        let frame = assemble_viewport_scene_with_images(
            &mut document,
            viewport,
            &shaper,
            font_size,
            Rect::new(0.0, 0.0, 240.0, 160.0).expect("scene viewport"),
            &atlas,
            Rgba8::black(),
            &[],
        )
        .expect("scene frame");
        let boxes = frame
            .scene()
            .primitives()
            .iter()
            .filter(|primitive| {
                matches!(primitive, Primitive::Ornament(ornament)
                    if ornament.role() == OrnamentRole::Border
                        && ornament.bounds().width() > 0.0)
            })
            .count();
        assert_eq!(boxes, 0, "不成立的引用不该留一个空框");
    }

    #[test]
    fn ready_image_publication_updates_scene_intrinsic_bounds() {
        let font_size = 14.0;
        let shaper = shaper(font_size);
        let viewport = ViewportSpan::new(0.0, 160.0);
        let mut document = EditorDocument::new("![alt](image.png)");
        document
            .set_viewport_config(ViewportConfig::new(
                LayoutConfig::new(240.0, 20.0),
                20.0,
                0.0,
            ))
            .expect("viewport config");
        let atlas = atlas_for_document(&mut document, viewport, &shaper, font_size);
        let source = document
            .block_decorations(0)
            .expect("image decorations")
            .widgets()
            .iter()
            .copied()
            .filter_map(|widget| match widget {
                BlockWidget::Image(image) => Some(image.source()),
                BlockWidget::Checkbox(_) => None,
            })
            .next()
            .expect("这个块上有一张图");
        let mut cache = yu_assets::ImageCache::new();
        let publication = cache
            .publish_decoded(
                yu_assets::ImageRequest::new(document.revision(), source, "image.png")
                    .expect("image request"),
                document.revision(),
                yu_assets::DecodedImage::new(200, 100, vec![255; 200 * 100 * 4])
                    .expect("decoded image"),
            )
            .expect("image publication");
        let intrinsic = publication.intrinsic_publication();
        let frame = assemble_viewport_scene_with_images(
            &mut document,
            viewport,
            &shaper,
            font_size,
            Rect::new(0.0, 0.0, 240.0, 160.0).expect("scene viewport"),
            &atlas,
            Rgba8::black(),
            &[publication],
        )
        .expect("scene frame");
        let image = frame
            .scene()
            .primitives()
            .iter()
            .find_map(|primitive| match primitive {
                Primitive::Image(image) => Some(*image),
                Primitive::FillRect { .. }
                | Primitive::Glyph(_)
                | Primitive::EmbeddedSvg(_)
                | Primitive::Ornament(_)
                | Primitive::EditorDecoration(_) => None,
            })
            .expect("image primitive");
        assert_eq!(image.bounds().width(), 200.0);
        assert_eq!(image.bounds().height(), 100.0);
        assert!(frame.input().content_height() >= 100.0);

        let metadata_only = assemble_viewport_scene_with_images_and_intrinsics(
            &mut document,
            viewport,
            &shaper,
            font_size,
            Rect::new(0.0, 0.0, 240.0, 160.0).expect("scene viewport"),
            &atlas,
            Rgba8::black(),
            &[],
            &[intrinsic],
        )
        .expect("metadata-only scene frame");
        let metadata_image = metadata_only
            .scene()
            .primitives()
            .iter()
            .find_map(|primitive| match primitive {
                Primitive::Image(image) => Some(*image),
                Primitive::FillRect { .. }
                | Primitive::Glyph(_)
                | Primitive::EmbeddedSvg(_)
                | Primitive::Ornament(_)
                | Primitive::EditorDecoration(_) => None,
            })
            .expect("metadata-only image primitive");
        assert_eq!(metadata_image.bounds().width(), 200.0);
        assert_eq!(metadata_image.bounds().height(), 100.0);
        assert!(metadata_only.input().content_height() >= 100.0);
    }

    /// 高亮真的画到了屏幕上——判据是**场景里的字形颜色**。
    ///
    /// 差分的两边只差一个语言名：`\u{60}\u{60}\u{60}rust` 与 `\u{60}\u{60}\u{60}`。文本、几何、
    /// 字形数量全部相同，所以「颜色变了」不可能来自别的原因。**判据不来自
    /// 被测的那条路**：这里一次都没问过 `TextRole`，只数场景图元的颜色。
    #[test]
    fn code_highlight_reaches_the_scene_as_glyph_colors() {
        let body = "fn main() {\n    let x: u32 = 1;\n}\n";
        let highlighted = scene_glyph_colors(&format!(
            "\u{60}\u{60}\u{60}rust\n{body}\u{60}\u{60}\u{60}\n"
        ));
        let plain = scene_glyph_colors(&format!("\u{60}\u{60}\u{60}\n{body}\u{60}\u{60}\u{60}\n"));

        assert_eq!(
            highlighted.len(),
            plain.len(),
            "两边的字形数量必须相同——不同就说明差的不只是颜色"
        );
        assert_eq!(distinct(&plain), 1, "没有语言名的代码块只该有一种字形颜色");
        // 数的是**正文色之外**还有几种。把正文色也算进去的话，「所有角色都
        // 用同一种颜色」这个变异就是 2 种（黑 + 那一种），照样大于 1——它活过
        // 一次，正是因为这里少减了正文色那一项。
        let extra = highlighted
            .iter()
            .copied()
            .filter(|color| *color != Rgba8::black())
            .collect::<std::collections::HashSet<_>>();
        assert!(
            extra.len() >= 2,
            "关键字、注释、字符串该是不同的颜色，实际只有 {} 种：{extra:?}",
            extra.len()
        );
        // 没被着色的那些字形仍然用这一帧的正文颜色。这半句压的是「颜色覆盖
        // 漏给了所有字形」——那样两边的颜色集合都会变，上面两条却都还过。
        assert!(
            highlighted.contains(&Rgba8::black()),
            "未着色的字形该保持正文颜色"
        );
    }

    fn distinct(colors: &[Rgba8]) -> usize {
        colors
            .iter()
            .copied()
            .collect::<std::collections::HashSet<_>>()
            .len()
    }

    /// 一份文档排出来的全部字形颜色，按场景顺序。
    fn scene_glyph_colors(source: &str) -> Vec<Rgba8> {
        let font_size = 14.0;
        let shaper = shaper(font_size);
        let viewport = ViewportSpan::new(0.0, 200.0);
        let mut document = EditorDocument::new(source);
        document
            .set_viewport_config(ViewportConfig::new(
                LayoutConfig::new(400.0, 20.0),
                20.0,
                0.0,
            ))
            .expect("viewport config");
        let atlas = atlas_for_document(&mut document, viewport, &shaper, font_size);
        let frame = assemble_viewport_render_frame(
            &mut document,
            ViewportRenderConfig::new(
                viewport,
                font_size,
                Rect::new(0.0, 0.0, 400.0, 200.0).expect("scene viewport"),
                Rgba8::black(),
            ),
            &shaper,
            &atlas,
            &mut RenderPlanBuilder::new(),
        )
        .expect("frame");
        frame
            .scene()
            .scene()
            .primitives()
            .iter()
            .filter_map(|primitive| match primitive {
                Primitive::Glyph(glyph) => Some(glyph.color()),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn fenced_code_viewport_emits_fill_before_glyphs() {
        let font_size = 14.0;
        let shaper = shaper(font_size);
        let viewport = ViewportSpan::new(0.0, 160.0);
        let mut document = EditorDocument::new("```rust\nlet x = 1;\n```\n");
        document
            .set_viewport_config(ViewportConfig::new(
                LayoutConfig::new(240.0, 20.0),
                20.0,
                0.0,
            ))
            .expect("viewport config");
        let atlas = atlas_for_document(&mut document, viewport, &shaper, font_size);
        let frame = assemble_viewport_render_frame(
            &mut document,
            ViewportRenderConfig::new(
                viewport,
                font_size,
                Rect::new(0.0, 0.0, 240.0, 160.0).expect("scene viewport"),
                Rgba8::black(),
            ),
            &shaper,
            &atlas,
            &mut RenderPlanBuilder::new(),
        )
        .expect("code block frame");

        let primitives = frame.scene().scene().primitives();
        // 跳过整帧背景，再检查代码块自己的底色先于字形。
        let Some((background, primitives)) = primitives.split_first() else {
            panic!("code block scene should not be empty");
        };
        assert!(matches!(background, Primitive::FillRect { .. }));
        let Some((first, rest)) = primitives.split_first() else {
            panic!("code block scene should carry its own fill");
        };
        match first {
            Primitive::FillRect { bounds, color } => {
                assert_eq!(*color, Rgba8::new(245, 246, 248, 255));
                assert_eq!(bounds.x(), 0.0);
                assert_eq!(bounds.y(), 0.0);
                assert_eq!(bounds.width(), 240.0);
                assert!(bounds.height() > 0.0);
            }
            Primitive::Glyph(_)
            | Primitive::Image(_)
            | Primitive::EmbeddedSvg(_)
            | Primitive::Ornament(_)
            | Primitive::EditorDecoration(_) => {
                panic!("code block background must precede glyphs")
            }
        }
        assert!(
            rest.iter()
                .any(|primitive| matches!(primitive, Primitive::Glyph(_)))
        );
        assert!(matches!(
            frame.plan().commands().first(),
            Some(yu_render::RenderCommand::FillRect { .. })
        ));
        assert!(
            frame
                .plan()
                .commands()
                .iter()
                .any(|command| matches!(command, yu_render::RenderCommand::Glyph { .. }))
        );
    }

    #[test]
    fn table_viewport_emits_decorations_before_source_backed_cell_glyphs() {
        let font_size = 14.0;
        let shaper = shaper(font_size);
        let source = "| A | B |\n| --- | :---: |\n| 1 | 2 |\n";
        let viewport = ViewportSpan::new(0.0, 160.0);
        let mut document = EditorDocument::new(source);
        document
            .set_viewport_config(ViewportConfig::new(
                LayoutConfig::new(240.0, 20.0),
                20.0,
                0.0,
            ))
            .expect("viewport config");
        let body_offset = ByteOffset::new(source.rfind('2').expect("body cell") as u64);
        let selection = EditorSelection::range(
            &document.snapshot(),
            body_offset,
            ByteOffset::new(body_offset.get() + 1),
            CaretAffinity::Downstream,
        )
        .expect("selection");
        document.set_selection(selection).expect("set selection");
        let atlas = atlas_for_document(&mut document, viewport, &shaper, font_size);
        let frame = assemble_viewport_scene(
            &mut document,
            viewport,
            &shaper,
            font_size,
            Rect::new(0.0, 0.0, 240.0, 160.0).expect("scene viewport"),
            &atlas,
            Rgba8::black(),
        )
        .expect("table scene frame");

        let primitives = frame.scene().primitives();
        assert!(matches!(primitives[0], Primitive::FillRect { .. }));
        // 背景之后、首个字形之前只应有表格装饰。
        let primitives = &primitives[1..];
        let first_glyph = primitives
            .iter()
            .position(|primitive| matches!(primitive, Primitive::Glyph(_)))
            .expect("cell glyph");
        assert!(
            primitives[..first_glyph]
                .iter()
                .all(|primitive| matches!(primitive, Primitive::Ornament(_)))
        );
        assert!(primitives[..first_glyph].iter().any(|primitive| {
            matches!(
                primitive,
                Primitive::Ornament(table) if table.role() == OrnamentRole::Background
            )
        }));
        assert!(primitives[..first_glyph].iter().any(|primitive| {
            matches!(
                primitive,
                Primitive::Ornament(table)
                    if table.role() == OrnamentRole::Fill
                        && table.source().contains(body_offset)
            )
        }));
        assert!(
            primitives[first_glyph..]
                .iter()
                .any(|primitive| matches!(primitive, Primitive::Glyph(_)))
        );
    }

    #[test]
    fn table_resize_override_reaches_scene_and_render_plan_without_mutating_document() {
        let font_size = 14.0;
        let shaper = shaper(font_size);
        let viewport = ViewportSpan::new(0.0, 160.0);
        let source = "| A | B |\n| --- | :---: |\n| 1 | 2 |\n";
        let mut document = EditorDocument::new(source);
        document
            .set_viewport_config(ViewportConfig::new(
                LayoutConfig::new(240.0, 20.0),
                20.0,
                0.0,
            ))
            .expect("viewport config");
        let layout_config = document.viewport_config().layout();
        let canonical = document
            .block_layout_with_shaper(0, layout_config, &shaper)
            .expect("canonical table layout")
            .clone();
        let table = canonical.table().expect("table metadata");
        let divider = table.bounds().x() + table.column_widths()[0];
        let hit = table
            .resize_hit_test(LayoutPoint::new(divider, 0.5), 0.0)
            .expect("divider hit-test")
            .expect("column divider");
        let mut gesture = TableResizeGesture::begin(canonical.revision(), 0, hit, divider)
            .expect("resize gesture");
        gesture
            .update(canonical.revision(), divider + 1.0)
            .expect("resize update");
        let commit = gesture.finish(canonical.revision()).expect("resize commit");
        let atlas = atlas_for_document(&mut document, viewport, &shaper, font_size);
        let frame = assemble_viewport_render_frame(
            &mut document,
            ViewportRenderConfig::new(
                viewport,
                font_size,
                Rect::new(0.0, 0.0, 240.0, 160.0).expect("scene viewport"),
                Rgba8::black(),
            )
            .with_table_resize(commit),
            &shaper,
            &atlas,
            &mut RenderPlanBuilder::new(),
        )
        .expect("transient table render frame");

        let expected_divider = divider + 1.0;
        assert!(frame.scene().scene().primitives().iter().any(|primitive| {
            matches!(
                primitive,
                Primitive::Ornament(table)
                    if table.role() == OrnamentRole::Border
                        && (table.bounds().x() - expected_divider).abs() < 0.001
            )
        }));
        assert!(frame.plan().commands().iter().any(|command| {
            matches!(
                command,
                yu_render::RenderCommand::FillRect { bounds, .. }
                    if (bounds.x() - expected_divider).abs() < 0.001
            )
        }));
        assert_eq!(document.snapshot().as_str(), source);
        let canonical_after = document
            .block_layout_with_shaper(0, layout_config, &shaper)
            .expect("canonical layout after frame");
        assert!(
            (canonical_after.table().expect("table").column_widths()[0] - table.column_widths()[0])
                .abs()
                < 0.001
        );
        assert_eq!(document.layout_cache_stats().entries(), 1);

        document
            .apply_transaction(&yu_text::Transaction::new(
                canonical.revision(),
                [yu_text::Edit::new(
                    yu_core::TextRange::empty(ByteOffset::ZERO),
                    "前",
                )],
            ))
            .expect("source edit");
        let stale_error = assemble_viewport_render_frame(
            &mut document,
            ViewportRenderConfig::new(
                viewport,
                font_size,
                Rect::new(0.0, 0.0, 240.0, 160.0).expect("scene viewport"),
                Rgba8::black(),
            )
            .with_table_resize(commit),
            &shaper,
            &atlas,
            &mut RenderPlanBuilder::new(),
        )
        .expect_err("stale table override");
        let ViewportSceneError::Document(EditorDocumentError::Layout(
            yu_editor::LayoutError::Upstream(message),
        )) = stale_error
        else {
            panic!("expected a stale table override error, got {stale_error:?}");
        };
        assert_eq!(
            message,
            "table resize and viewport document revisions differ"
        );
    }

    #[test]
    fn missing_atlas_is_rejected_before_frame_publication() {
        let font_size = 14.0;
        let shaper = shaper(font_size);
        let viewport = ViewportSpan::new(0.0, 80.0);
        let mut document = EditorDocument::new("paragraph");
        document
            .set_viewport_config(ViewportConfig::new(
                LayoutConfig::new(240.0, 20.0),
                20.0,
                0.0,
            ))
            .expect("viewport config");
        let empty_atlas = GlyphAtlas::new(GlyphAtlasConfig::new(64, 64, 1).expect("atlas config"));
        let error = assemble_viewport_scene(
            &mut document,
            viewport,
            &shaper,
            font_size,
            Rect::new(0.0, 0.0, 240.0, 80.0).expect("scene viewport"),
            &empty_atlas,
            Rgba8::black(),
        )
        .expect_err("missing glyph must fail");
        assert!(matches!(
            error,
            ViewportSceneError::Scene(SceneError::MissingGlyphAtlas(_))
        ));
    }

    #[test]
    fn frame_cache_rejects_stale_publish_and_replaces_after_edit() {
        let font_size = 14.0;
        let shaper = shaper(font_size);
        let viewport = ViewportSpan::new(0.0, 80.0);
        let mut document = EditorDocument::new("paragraph");
        document
            .set_viewport_config(ViewportConfig::new(
                LayoutConfig::new(240.0, 20.0),
                20.0,
                0.0,
            ))
            .expect("viewport config");
        let atlas = atlas_for_document(&mut document, viewport, &shaper, font_size);
        let mut plans = RenderPlanBuilder::new();
        let old_frame = assemble_viewport_render_frame(
            &mut document,
            ViewportRenderConfig::new(
                viewport,
                font_size,
                Rect::new(0.0, 0.0, 240.0, 80.0).expect("scene viewport"),
                Rgba8::black(),
            ),
            &shaper,
            &atlas,
            &mut plans,
        )
        .expect("old frame");
        let old_revision = document.revision();
        let mut cache = ViewportFrameCache::new();
        cache
            .publish_if_current(old_revision, old_frame.clone())
            .expect("initial publish");
        assert_eq!(cache.current_revision(), Some(old_revision));
        assert!(cache.get(old_revision).is_some());

        document
            .execute(EditorCommand::insert_text("!"))
            .expect("edit");
        let new_revision = document.revision();
        assert_ne!(new_revision, old_revision);
        assert_eq!(
            cache.publish_if_current(new_revision, old_frame.clone()),
            Err(ViewportFrameError::Stale {
                expected: new_revision,
                actual: old_revision,
            })
        );
        assert_eq!(cache.current_revision(), Some(old_revision));
        assert!(cache.invalidate_stale(new_revision));
        assert!(cache.get(old_revision).is_none());
        assert!(!cache.invalidate_stale(new_revision));

        let new_atlas = atlas_for_document(&mut document, viewport, &shaper, font_size);
        let new_frame = assemble_viewport_render_frame(
            &mut document,
            ViewportRenderConfig::new(
                viewport,
                font_size,
                Rect::new(0.0, 0.0, 240.0, 80.0).expect("scene viewport"),
                Rgba8::black(),
            ),
            &shaper,
            &new_atlas,
            &mut plans,
        )
        .expect("new frame");
        cache
            .publish_if_current(new_revision, new_frame)
            .expect("new publish");
        assert_eq!(cache.current_revision(), Some(new_revision));
        assert_eq!(
            cache.get(new_revision).expect("new frame").revision(),
            new_revision
        );
        assert_eq!(
            cache.publish_if_current(old_revision, old_frame),
            Err(ViewportFrameError::Stale {
                expected: new_revision,
                actual: old_revision,
            })
        );
    }

    #[test]
    fn frame_publisher_returns_owned_revision_bound_publications() {
        use std::sync::Arc;

        let font_size = 14.0;
        let shaper = shaper(font_size);
        let viewport = ViewportSpan::new(0.0, 80.0);
        let config = ViewportRenderConfig::new(
            viewport,
            font_size,
            Rect::new(0.0, 0.0, 240.0, 80.0).expect("scene viewport"),
            Rgba8::black(),
        );
        let mut document = EditorDocument::new("paragraph");
        document
            .set_viewport_config(ViewportConfig::new(
                LayoutConfig::new(240.0, 20.0),
                20.0,
                0.0,
            ))
            .expect("viewport config");
        let atlas = atlas_for_document(&mut document, viewport, &shaper, font_size);
        let mut plans = RenderPlanBuilder::new();
        let mut publisher = ViewportFramePublisher::new();

        let first = publisher
            .publish(&mut document, config, &shaper, &atlas, &mut plans)
            .expect("first publication");
        let first_revision = document.revision();
        assert_eq!(first.revision(), first_revision);
        assert_eq!(first.frame().revision(), first_revision);
        assert_eq!(first.serial(), 1);
        assert_eq!(publisher.current_revision(), Some(first_revision));
        assert_eq!(publisher.last_publication(), Some(&first));
        assert_eq!(publisher.current_frame(first_revision), Some(first.frame()));
        let cached_handle = publisher
            .current_frame_handle(first_revision)
            .expect("cached frame handle");
        assert!(Arc::ptr_eq(&cached_handle, &first.frame_handle()));

        document
            .execute(EditorCommand::insert_text("!"))
            .expect("edit");
        let second_revision = document.revision();
        assert!(publisher.invalidate_stale(second_revision));
        assert_eq!(publisher.current_revision(), None);
        assert_eq!(publisher.last_publication(), None);
        assert_eq!(first.frame().revision(), first_revision);

        let new_atlas = atlas_for_document(&mut document, viewport, &shaper, font_size);
        let second = publisher
            .publish(&mut document, config, &shaper, &new_atlas, &mut plans)
            .expect("second publication");
        assert_eq!(second.revision(), second_revision);
        assert_eq!(second.serial(), 2);
        assert_eq!(publisher.last_publication(), Some(&second));
    }

    #[test]
    fn frame_publisher_failure_does_not_commit_builder_or_cache() {
        let font_size = 14.0;
        let shaper = shaper(font_size);
        let viewport = ViewportSpan::new(0.0, 80.0);
        let config = ViewportRenderConfig::new(
            viewport,
            font_size,
            Rect::new(0.0, 0.0, 240.0, 80.0).expect("scene viewport"),
            Rgba8::black(),
        );
        let mut document = EditorDocument::new("paragraph");
        document
            .set_viewport_config(ViewportConfig::new(
                LayoutConfig::new(240.0, 20.0),
                20.0,
                0.0,
            ))
            .expect("viewport config");
        let atlas = atlas_for_document(&mut document, viewport, &shaper, font_size);
        let mut initial_plans = RenderPlanBuilder::new();
        let mut publisher = ViewportFramePublisher::new();
        let first = publisher
            .publish(&mut document, config, &shaper, &atlas, &mut initial_plans)
            .expect("initial publication");
        let first_revision = first.revision();

        document
            .execute(EditorCommand::insert_text("!"))
            .expect("edit");
        let new_atlas = atlas_for_document(&mut document, viewport, &shaper, font_size);
        let mut retry_plans = RenderPlanBuilder::new();
        publisher.next_serial = u64::MAX;

        let error = publisher
            .publish(&mut document, config, &shaper, &new_atlas, &mut retry_plans)
            .expect_err("serial overflow must reject publication");
        assert_eq!(error, ViewportPublishError::SerialOverflow);
        assert_eq!(retry_plans.uploaded_page_count(), 0);
        assert_eq!(publisher.current_revision(), Some(first_revision));
        assert_eq!(publisher.last_publication(), Some(&first));
        assert_eq!(publisher.next_serial, u64::MAX);

        publisher.next_serial = first.serial();
        let retry = publisher
            .publish(&mut document, config, &shaper, &new_atlas, &mut retry_plans)
            .expect("retry after overflow");
        assert_eq!(retry.serial(), 2);
        assert_eq!(retry.revision(), document.revision());
        assert!(retry_plans.uploaded_page_count() > 0);
    }

    /// 守护不变量 I5：Rust 渲染器是唯一渲染路径，不存在第二条兜底路径。
    ///
    /// parser 能产出的每一种 block kind 都必须能被投影、布局并产出 glyph。
    /// 一旦某个 kind 画不出来，删掉 TextKit fallback 后它就会变成空白区域，
    /// 因此这条断言是「允许删除 fallback」的前提证明，不能降级为警告。
    #[test]
    fn every_parser_block_kind_produces_renderable_glyphs() {
        let font_size = 14.0;
        let shaper = shaper(font_size);
        let viewport = ViewportSpan::new(0.0, 900.0);
        let source = concat!(
            "# heading\n",
            "\n",
            "paragraph with **bold**\n",
            "\n",
            "> quoted line\n",
            "\n",
            "- list item\n",
            "- [ ] task item\n",
            "\n",
            "```rust\n",
            "code line\n",
            "```\n",
            "\n",
            "| a | b |\n",
            "| --- | --- |\n",
            "| 1 | 2 |\n",
            "\n",
            "[ref]: /url\n",
        );
        let mut document = EditorDocument::new(source);
        document
            .set_viewport_config(ViewportConfig::new(
                LayoutConfig::new(240.0, 20.0),
                20.0,
                0.0,
            ))
            .expect("viewport config");
        let atlas = atlas_for_document(&mut document, viewport, &shaper, font_size);
        let frame = assemble_viewport_scene(
            &mut document,
            viewport,
            &shaper,
            font_size,
            Rect::new(0.0, 0.0, 240.0, 900.0).expect("scene viewport"),
            &atlas,
            Rgba8::black(),
        )
        .expect("scene frame");

        // 全部 8 种 parser block kind 都必须出现在这一个 viewport 里，
        // 否则这条测试就没有真正覆盖到它声称覆盖的范围。
        let kinds: std::collections::BTreeSet<u8> = frame
            .input()
            .blocks()
            .iter()
            .map(|block| block.kind())
            .collect();
        let expected: std::collections::BTreeSet<u8> = (0..=7).collect();
        assert_eq!(
            kinds, expected,
            "fixture 未覆盖全部 block kind，实际 {kinds:?}"
        );

        // 每个非空行 block 都必须产出 glyph。
        let config = document.viewport_config().layout();
        for block in frame.input().blocks() {
            if block.kind() == 0 {
                continue; // BlankLine 没有可见字形
            }
            let layout = document
                .block_layout_for_visual_state_with_shaper(block.index(), config, &shaper)
                .expect("block layout");
            assert!(
                !layout.glyphs().is_empty(),
                "block kind {} (source {:?}) 没有产出任何 glyph，\
                 删除 TextKit fallback 后它会变成空白",
                block.kind(),
                block.source()
            );
        }

        // 这些 glyph 必须真的进入 scene，而不是只存在于 layout 里。
        let glyph_count = frame
            .scene()
            .primitives()
            .iter()
            .filter(|primitive| matches!(primitive, Primitive::Glyph(_)))
            .count();
        assert!(glyph_count > 0, "scene 中没有任何 glyph primitive");
    }

    /// 搜索高亮真的进了场景，「当前命中」与其余分得开，跨块的命中两边都画。
    ///
    /// 判据来自**场景**，不来自 `SearchState`：后者是被测的那条路的上游，
    /// 拿它当参照只能证明「我把它读出来了」。
    /// **N 个光标与 N 块选区底色真的进了场景。**
    ///
    /// 判据来自**场景里的图元**，不是 `document.selections()`——后者是被测的
    /// 那条路。只画 primary 的表现是「屏幕上只有一个光标」，多光标看上去完全
    /// 没生效，而所有编辑器层的断言仍然全绿。
    #[test]
    fn every_selection_and_caret_reaches_the_scene() {
        let font_size = 14.0;
        let shaper = shaper(font_size);
        let viewport = ViewportSpan::new(0.0, 400.0);
        let source = "alpha beta gamma\n\nsecond line here\n";
        let mut document = EditorDocument::new(source);
        document
            .set_viewport_config(ViewportConfig::new(
                LayoutConfig::new(240.0, 20.0),
                20.0,
                0.0,
            ))
            .expect("viewport config");

        let style = EditorDecorationStyle::new(
            Rgba8::new(0, 122, 255, 97),
            Rgba8::black(),
            Rgba8::new(0, 122, 255, 255),
            1.0,
        );
        let config = ViewportRenderConfig::new(
            viewport,
            font_size,
            Rect::new(0.0, 0.0, 240.0, 400.0).expect("scene viewport"),
            Rgba8::black(),
        )
        .with_editor_decorations(style);

        let roles = |document: &mut EditorDocument| -> Vec<EditorDecorationPrimitiveRole> {
            let atlas = atlas_for_document(document, viewport, &shaper, font_size);
            let mut plans = RenderPlanBuilder::new();
            let frame =
                assemble_viewport_render_frame(document, config, &shaper, &atlas, &mut plans)
                    .expect("frame");
            frame
                .scene()
                .scene()
                .primitives()
                .iter()
                .filter_map(|primitive| match primitive {
                    Primitive::EditorDecoration(decoration) => Some(decoration.role()),
                    _ => None,
                })
                .collect()
        };
        let count = |roles: &[EditorDecorationPrimitiveRole],
                     role: EditorDecorationPrimitiveRole| {
            roles.iter().filter(|found| **found == role).count()
        };

        let caret_at = |document: &mut EditorDocument, offsets: &[u64]| {
            let snapshot = document.snapshot();
            let carets: Vec<_> = offsets
                .iter()
                .map(|offset| {
                    EditorSelection::cursor(
                        &snapshot,
                        ByteOffset::new(*offset),
                        CaretAffinity::Downstream,
                    )
                    .expect("caret")
                })
                .collect();
            document.set_selections(carets, 0).expect("selections");
        };

        // 一根光标：一根 caret，没有选区底色。
        caret_at(&mut document, &[1]);
        let single = roles(&mut document);
        assert_eq!(count(&single, EditorDecorationPrimitiveRole::Caret), 1);
        assert_eq!(count(&single, EditorDecorationPrimitiveRole::Selection), 0);

        // 三根光标，两根在第一个块里、一根在第二个块里——**跨块也要各画各的**。
        let second_block = source.find("second").expect("第二个块") as u64;
        caret_at(&mut document, &[1, 7, second_block + 2]);
        let three = roles(&mut document);
        assert_eq!(
            count(&three, EditorDecorationPrimitiveRole::Caret),
            3,
            "三根光标必须画出三根 caret"
        );

        // 两段非空选区：两块底色，外加两根 caret（focus 各在自己那一段的末尾）。
        let snapshot = document.snapshot();
        let spans: Vec<_> = [(0_u64, 5_u64), (6, 10)]
            .iter()
            .map(|(anchor, focus)| {
                EditorSelection::range(
                    &snapshot,
                    ByteOffset::new(*anchor),
                    ByteOffset::new(*focus),
                    CaretAffinity::Downstream,
                )
                .expect("selection")
            })
            .collect();
        document.set_selections(spans, 0).expect("selections");
        let selected = roles(&mut document);
        assert_eq!(
            count(&selected, EditorDecorationPrimitiveRole::Selection),
            2,
            "两段选区必须画出两块底色"
        );
        assert_eq!(
            count(&selected, EditorDecorationPrimitiveRole::Caret),
            2,
            "两段选区各有一根 caret"
        );

        // **caret 仍然是最后一层。** 多光标之后 caret 从「一个 Option」变成
        // 「一个 Vec」，最容易丢的就是这条次序——丢了不报错，只是光标被底色盖住。
        let first_selection = selected
            .iter()
            .position(|role| *role == EditorDecorationPrimitiveRole::Selection)
            .expect("选区图元");
        let first_caret = selected
            .iter()
            .position(|role| *role == EditorDecorationPrimitiveRole::Caret)
            .expect("caret 图元");
        assert!(
            first_caret > first_selection,
            "全部 caret 必须排在全部选区底色之后"
        );
    }

    #[test]
    fn search_highlights_reach_the_scene_and_mark_the_current_match() {
        let font_size = 14.0;
        let shaper = shaper(font_size);
        let viewport = ViewportSpan::new(0.0, 400.0);
        let source = "# alpha\n\nalpha and alpha\n";
        let mut document = EditorDocument::new(source);
        document
            .set_viewport_config(ViewportConfig::new(
                LayoutConfig::new(240.0, 20.0),
                20.0,
                0.0,
            ))
            .expect("viewport config");

        let style = EditorDecorationStyle::new(
            Rgba8::new(0, 122, 255, 97),
            Rgba8::black(),
            Rgba8::new(0, 122, 255, 255),
            1.0,
        )
        .with_search(Rgba8::new(255, 214, 10, 120), Rgba8::new(255, 149, 0, 170));
        let config = ViewportRenderConfig::new(
            viewport,
            font_size,
            Rect::new(0.0, 0.0, 240.0, 400.0).expect("scene viewport"),
            Rgba8::black(),
        )
        .with_editor_decorations(style);

        let roles = |document: &mut EditorDocument| -> Vec<EditorDecorationPrimitiveRole> {
            let atlas = atlas_for_document(document, viewport, &shaper, font_size);
            let mut plans = RenderPlanBuilder::new();
            let frame =
                assemble_viewport_render_frame(document, config, &shaper, &atlas, &mut plans)
                    .expect("frame");
            frame
                .scene()
                .scene()
                .primitives()
                .iter()
                .filter_map(|primitive| match primitive {
                    Primitive::EditorDecoration(decoration) => Some(decoration.role()),
                    _ => None,
                })
                .collect()
        };
        let count = |roles: &[EditorDecorationPrimitiveRole],
                     role: EditorDecorationPrimitiveRole| {
            roles.iter().filter(|found| **found == role).count()
        };

        // 没有查询：一个搜索矩形都不该有。
        let baseline = roles(&mut document);
        assert_eq!(
            count(&baseline, EditorDecorationPrimitiveRole::SearchMatch),
            0
        );

        document.set_search_query("alpha");
        let searched = roles(&mut document);
        assert_eq!(
            count(&searched, EditorDecorationPrimitiveRole::SearchMatch),
            3,
            "标题一处、段落两处"
        );
        assert_eq!(
            count(&searched, EditorDecorationPrimitiveRole::SearchCurrent),
            0,
            "光标没有落在任何一处命中上"
        );

        // 把选区放到段落里第二个 `alpha` 上——那一处变成「当前」，其余不变。
        let second = source.rfind("alpha").expect("最后一处 alpha");
        let snapshot = document.snapshot();
        document
            .set_selection(
                EditorSelection::range(
                    &snapshot,
                    ByteOffset::new(second as u64),
                    ByteOffset::new((second + "alpha".len()) as u64),
                    CaretAffinity::Downstream,
                )
                .expect("selection"),
            )
            .expect("set selection");
        let current = roles(&mut document);
        assert_eq!(
            count(&current, EditorDecorationPrimitiveRole::SearchCurrent),
            1,
            "选区恰好落在一处命中上，它必须与其余分得开"
        );
        assert_eq!(
            count(&current, EditorDecorationPrimitiveRole::SearchMatch),
            2,
            "另外两处仍然是普通命中"
        );

        // **选中全部匹配之后，「当前命中」仍然只有一处。**
        //
        // 这条压住的是一个只在多光标下才存在的形状：每一条选区都恰好等于一处
        // 命中，如果按「有没有**某条**选区等于它」判定，每一处都会变成当前命中
        // ——正是 `yu-editor/src/search.rs` 模块文档里担心的「全选点亮每一个」
        // 换了个形状回来。判据必须收紧到 primary。
        //
        // 单选区的用例压不住它：那时「任意一条」与「primary」是同一条路。
        {
            let snapshot = document.snapshot();
            let all: Vec<_> = document
                .search()
                .expect("查询还在")
                .matches()
                .iter()
                .map(|hit| {
                    EditorSelection::range(
                        &snapshot,
                        hit.start(),
                        hit.end(),
                        CaretAffinity::Downstream,
                    )
                    .expect("selection")
                })
                .collect();
            let total = all.len();
            assert!(total >= 2, "语料里至少要有两处命中，否则这条压不住任何东西");
            let expected = all[1].ordered_range();
            document.set_selections(all, 1).expect("selections");

            let atlas = atlas_for_document(&mut document, viewport, &shaper, font_size);
            let mut plans = RenderPlanBuilder::new();
            let frame =
                assemble_viewport_render_frame(&mut document, config, &shaper, &atlas, &mut plans)
                    .expect("frame");
            let currents: Vec<_> = frame
                .scene()
                .scene()
                .primitives()
                .iter()
                .filter_map(|primitive| match primitive {
                    Primitive::EditorDecoration(decoration)
                        if decoration.role() == EditorDecorationPrimitiveRole::SearchCurrent =>
                    {
                        Some(decoration.source())
                    }
                    _ => None,
                })
                .collect();
            assert_eq!(
                currents.len(),
                1,
                "全部选中之后「当前命中」必须仍然只有一处"
            );
            // **判据是它指着哪一处，不只是有几处。** 只数条数的话，「按第一条
            // 选区判」与「按 primary 判」画出来都是一处——只是橙色套在了错的那
            // 一段上，不报错。
            assert_eq!(
                currents[0], expected,
                "「当前命中」必须是 primary 那一条选区所在的那一处"
            );

            // **被选区盖住的普通命中不再画黄底。**
            //
            // 第三刀让「其余命中」垫在半透明选区之下，前提是两者不是同一段；
            // 「选中全部匹配」之后前提没了，黄底垫在蓝下面合成出一块脏灰绿。
            // 这一条是截图抓出来的，在它之前全部断言都绿。
            let plain = count(
                &roles(&mut document),
                EditorDecorationPrimitiveRole::SearchMatch,
            );
            assert_eq!(
                plain, 0,
                "全部选中之后不该再有垫在选区下面的普通命中底色，实际 {plain} 块"
            );
        }

        // IME 组字期间整个跳过：那时块的视觉字节流带着 preedit 覆盖层，而匹配
        // 是按 canonical 源码算的。少画几个框好过画在错的位置上。
        document
            .begin_composition(
                TextRange::empty(ByteOffset::new(9)),
                "ぁ",
                yu_core::Utf16Range::new(
                    yu_core::Utf16Offset::new(1),
                    yu_core::Utf16Offset::new(1),
                )
                .expect("composition selection"),
            )
            .expect("begin composition");
        let composing = roles(&mut document);
        assert_eq!(
            count(&composing, EditorDecorationPrimitiveRole::SearchMatch),
            0,
            "组字期间不该画搜索高亮"
        );
        assert!(document.cancel_composition(), "取消组字");

        // **次序**：其余命中在选区之下，当前命中在选区之上、caret 之下。
        // 画反了不报错，只是当前那一处被半透明的选区调成一块脏灰——所以这条
        // 断言落在图元的先后上，那正是被改的那件事。
        let order = |roles: &[EditorDecorationPrimitiveRole],
                     role: EditorDecorationPrimitiveRole| {
            roles.iter().position(|found| *found == role)
        };
        let selection_at = order(&current, EditorDecorationPrimitiveRole::Selection)
            .expect("当前命中那一段是选中的，必须有选区图元");
        let match_at =
            order(&current, EditorDecorationPrimitiveRole::SearchMatch).expect("其余命中");
        let current_at =
            order(&current, EditorDecorationPrimitiveRole::SearchCurrent).expect("当前命中");
        let caret_at = order(&current, EditorDecorationPrimitiveRole::Caret).expect("caret");
        assert!(match_at < selection_at, "其余命中必须画在选区之下");
        assert!(current_at > selection_at, "当前命中必须画在选区之上");
        assert!(
            current_at < caret_at,
            "caret 必须是最后一层，不能被底色盖住"
        );

        // 矩形的**高度**必须是行盒自己的高度，不是基准行高。标题那一行更高，
        // 拿基准值会画出一条又矮又靠上的底色——不报错，只是没盖在字上。
        // 这一条是真实窗口截图抓出来的；在那之前选区也一直这么画，没有断言。
        document
            .set_selection(
                EditorSelection::cursor(
                    &document.snapshot(),
                    ByteOffset::new(0),
                    CaretAffinity::Downstream,
                )
                .expect("caret"),
            )
            .expect("set selection");
        let atlas = atlas_for_document(&mut document, viewport, &shaper, font_size);
        let mut plans = RenderPlanBuilder::new();
        let frame =
            assemble_viewport_render_frame(&mut document, config, &shaper, &atlas, &mut plans)
                .expect("frame");
        let heading_line_height = document
            .block_layout_for_visual_state_with_shaper(
                0,
                document.viewport_config().layout(),
                &shaper,
            )
            .expect("heading layout")
            .lines()
            .first()
            .expect("heading has a line")
            .height();
        let base_line_height = document.viewport_config().layout().line_height();
        assert!(
            heading_line_height > base_line_height,
            "标题那一行必须比基准行高更高，否则这一条压不住任何东西：\
             {heading_line_height} vs {base_line_height}"
        );
        let heading_highlight = frame
            .scene()
            .scene()
            .primitives()
            .iter()
            .filter_map(|primitive| match primitive {
                Primitive::EditorDecoration(decoration)
                    if decoration.role() == EditorDecorationPrimitiveRole::SearchMatch
                        || decoration.role() == EditorDecorationPrimitiveRole::SearchCurrent =>
                {
                    Some(*decoration)
                }
                _ => None,
            })
            .min_by(|a, b| {
                a.bounds()
                    .y()
                    .partial_cmp(&b.bounds().y())
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
            .expect("标题那一处高亮");
        assert!(
            (heading_highlight.bounds().height() - heading_line_height).abs() < 0.01,
            "标题那一处底色高 {}，而那一行是 {heading_line_height}",
            heading_highlight.bounds().height()
        );

        // 收掉搜索，矩形必须一起消失。
        document.clear_search();
        let cleared = roles(&mut document);
        assert_eq!(
            count(&cleared, EditorDecorationPrimitiveRole::SearchMatch),
            0
        );
        assert_eq!(
            count(&cleared, EditorDecorationPrimitiveRole::SearchCurrent),
            0
        );
    }
}
