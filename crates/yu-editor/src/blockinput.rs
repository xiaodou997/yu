//! 从 `DecorationSet` 派生 `yu-layout` 的输入。
//!
//! # 为什么这个模块住在 `yu-editor`
//!
//! v1 的 `LayoutSnapshot` 直接吃 `Projection`——一个认识标题、引用、列表
//! 标记、表格的类型。布局层为了排版必须先认识 Markdown，那正是
//! overview-v2 §2.1 点名的泄漏，也是不变量 E1 禁止的事。
//!
//! v2 的 [`BlockLayout`] 只吃「视觉文本 + [`StyledRun`] + [`WidgetSpan`] +
//! [`LineSpan`]」，加上三张把不透明 id 翻译成排版属性的表。**翻译的活儿
//! 得有人干**，干活的必须是一个允许认识 Markdown 的层。`yu-editor` 就是
//! ——E1 的禁止清单里没有它，`tools/check-deps.py` 也已登记
//! `yu-editor → yu-markdown`。
//!
//! # 什么进布局，什么不进
//!
//! 进布局的只有**几何**：字号倍率、行高倍率、缩进、widget 的盒子。
//!
//! 不进布局的是**长什么样**：引用竖条的宽度与颜色、列表标记画的是哪个
//! 字符。它们留在 [`BlockOrnaments`] 里，由绘制方拿去画。布局层拿到的是
//! 「缩进 8.0」，不是「这是二级引用」。

use crate::layout_tokens::{
    ContainerMetrics, box_content_inset_x, box_layout_config_with_quote, is_code_block,
};
use crate::marks::{Mark, flatten_composed};
use yu_core::{
    ByteOffset, ClusterMetrics, LineStyleId, ShapedText, ShapingProvider, StyleId, TextAttrs,
    TextRange, TextStyle,
};
use yu_core::{VisualOffset, VisualRange};
use yu_decoration::Decoration;
use yu_layout::{
    LayoutConfig, LayoutError, LayoutInput, LayoutRect, LineAttrs, LineSpan, LineStyleTable,
    StyleTable, StyledRun, WidgetSpan,
};
use yu_markdown::{BlockDecorations, BlockKind, BlockOrnament};

use crate::visual::VisualText;

/// 整块只有一段行级样式，id 固定。
const BLOCK_LINE_STYLE: LineStyleId = LineStyleId(0);

/// `StyleId` → [`TextAttrs`]。
///
/// 表由产出装饰的这一层填，长度由 extension 登记了多少种字型决定。查不到的
/// id 是错误而不是默认字型：一个「装饰产出与样式表脱节」的 bug 应该响，
/// 不应该只是画得不对。
#[derive(Clone, Debug, PartialEq)]
pub struct BlockStyleTable {
    attrs: Vec<TextAttrs>,
}

impl StyleTable for BlockStyleTable {
    fn attrs(&self, style: StyleId) -> Option<TextAttrs> {
        usize::try_from(style.0)
            .ok()
            .and_then(|index| self.attrs.get(index))
            .copied()
    }
}

/// `LineStyleId` → [`LineAttrs`]。整块共用一段。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BlockLineStyleTable {
    attrs: LineAttrs,
}

impl LineStyleTable for BlockLineStyleTable {
    fn attrs(&self, style: LineStyleId) -> Option<LineAttrs> {
        (style == BLOCK_LINE_STYLE).then_some(self.attrs)
    }
}

/// 标题的排版参数。
///
/// `font_scale` 已经进了 [`BlockStyleTable`]，`line_height_scale` 已经进了
/// [`BlockLineStyleTable`]；这里留一份是给 Accessibility 与平台样式用的——
/// 它们要知道的是「几级标题」，那不是几何。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct HeadingOrnament {
    source: TextRange,
    level: u8,
    font_scale: f32,
    line_height_scale: f32,
}

impl HeadingOrnament {
    #[must_use]
    pub const fn source(self) -> TextRange {
        self.source
    }

    #[must_use]
    pub const fn level(self) -> u8 {
        self.level
    }

    #[must_use]
    pub const fn font_scale(self) -> f32 {
        self.font_scale
    }

    #[must_use]
    pub const fn line_height_scale(self) -> f32 {
        self.line_height_scale
    }
}

/// 列表/任务的行首标记。
///
/// 它画在行级缩进让出来的那条 gutter 里，只画在这个块的**第一行**：后续
/// 软换行出来的行同样缩进（悬挂缩进），但不再重复画标记。
///
/// 标记文本不在 source 里——`•` 是 `-` 的替代呈现。它的 `source` 指着被
/// 替代掉的那段源码，选中与编辑仍然走那一段（不变量 A2）。
#[derive(Clone, Debug, PartialEq)]
pub struct MarkerOrnament {
    quoted: bool,
    source: TextRange,
    text: String,
    x: f32,
    advance: f32,
    shaped: Option<ShapedText>,
    shape: Option<MarkerShape>,
}

/// Geometric list marker, expressed relative to the first text baseline.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MarkerShapeKind {
    Disc,
    Circle,
    Square,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MarkerShape {
    pub kind: MarkerShapeKind,
    pub size: f32,
    pub baseline_offset: f32,
    pub before_text: f32,
    pub stroke: f32,
}

impl MarkerOrnament {
    pub const fn is_quoted(&self) -> bool {
        self.quoted
    }
    #[must_use]
    pub const fn shape(&self) -> Option<MarkerShape> {
        self.shape
    }

    #[must_use]
    pub const fn source(&self) -> TextRange {
        self.source
    }

    #[must_use]
    pub fn text(&self) -> &str {
        &self.text
    }

    /// 标记左边缘在 block 坐标里的 x。
    #[must_use]
    pub const fn x(&self) -> f32 {
        self.x
    }

    #[must_use]
    pub const fn advance(&self) -> f32 {
        self.advance
    }

    /// 文字型标记排好的字形。纯度量或几何型标记为 `None`。
    #[must_use]
    pub const fn shaped(&self) -> Option<&ShapedText> {
        self.shaped.as_ref()
    }
}

/// Paragraph quote semantics. Continuous border geometry belongs to the
/// containing node in LayoutSnapshot, not to this individual paragraph.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BlockQuoteOrnament {
    source: TextRange,
    depth: u8,
}

impl BlockQuoteOrnament {
    #[must_use]
    pub const fn source(self) -> TextRange {
        self.source
    }

    #[must_use]
    pub const fn depth(self) -> u8 {
        self.depth
    }
}

/// 分隔线画的那条横线。
///
/// 与 [`BlockQuoteOrnament`] 同一个形状，理由也同一条：块高要等布局排完才
/// 知道，所以这里只留参数，矩形由 [`ThematicBreakOrnament::bounds`] 现算。
///
/// **它在不在这里，等于线画不画。** `yu-markdown` 只在块没有焦点时才产出
/// `BlockOrnament::ThematicBreak`（光标进来时 `---` 要露出来给人改，线再画
/// 上去就成了删除线），所以焦点块的这个字段是 `None`。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ThematicBreakOrnament {
    source: TextRange,
    x: f32,
    width: f32,
    thickness: f32,
}

impl ThematicBreakOrnament {
    #[must_use]
    pub const fn source(self) -> TextRange {
        self.source
    }

    #[must_use]
    pub const fn thickness(self) -> f32 {
        self.thickness
    }

    /// 横线的矩形，竖直方向居中于块高。
    ///
    /// 居中而不是贴着上沿或下沿：这一块的视觉文本只剩一个换行符，画出来就是
    /// 一个空行，线贴边的话会紧挨着上一块或下一块的文字，看上去像那一块的
    /// 下划线。
    ///
    /// # Errors
    ///
    /// 几何参数不合法（宽或高非有限）。
    pub fn bounds(self, height: f32) -> Result<LayoutRect, LayoutError> {
        let y = ((height - self.thickness) * 0.5).max(0.0);
        Ok(LayoutRect::new(self.x, y, self.width, self.thickness)?)
    }
}

/// 一个块上「长什么样」的那部分装饰。布局层看不见它们。
#[derive(Clone, Debug, Default, PartialEq)]
pub struct BlockOrnaments {
    heading: Option<HeadingOrnament>,
    marker: Option<MarkerOrnament>,
    ancestors: Vec<MarkerOrnament>,
    quote: Option<BlockQuoteOrnament>,
    rule: Option<ThematicBreakOrnament>,
}

impl BlockOrnaments {
    #[must_use]
    pub const fn heading(&self) -> Option<HeadingOrnament> {
        self.heading
    }

    #[must_use]
    pub const fn marker(&self) -> Option<&MarkerOrnament> {
        self.marker.as_ref()
    }
    pub fn markers(&self) -> impl Iterator<Item = &MarkerOrnament> {
        self.ancestors.iter().chain(self.marker.iter())
    }

    #[must_use]
    pub const fn quote(&self) -> Option<BlockQuoteOrnament> {
        self.quote
    }

    /// 分隔线那条横线。不是分隔线、或者光标正落在这一块里，都是 `None`。
    #[must_use]
    pub const fn rule(&self) -> Option<ThematicBreakOrnament> {
        self.rule
    }
}

/// 一个块排版所需的全部输入。
///
/// 它拥有视觉文本，所以 [`BlockLayoutInput::layout_input`] 借出去的
/// [`LayoutInput`] 与它同生命周期。
#[derive(Clone, Debug, PartialEq)]
pub struct BlockLayoutInput {
    text: String,
    runs: Vec<StyledRun>,
    widgets: Vec<WidgetSpan>,
    /// 每个 widget 覆盖的那段 source，与 `widgets` 同序、等长。
    ///
    /// 布局层不要它（不变量 E1：那一层不认识源码坐标），**分派 widget 的
    /// 人要**：widget 的视觉锚点是它覆盖的 source 塌下去的那**一个点**，而
    /// 表格里相邻两格的视觉区间首尾相接——同一个点同时是上一格的末尾、
    /// 下一格的开头，还可能是中间某个空格子的起止。按视觉偏移分派会让一张
    /// 图被好几个格子同时认领，画好几遍。归属只有 source 说得清。
    widget_sources: Vec<TextRange>,
    lines: Vec<LineSpan>,
    styles: BlockStyleTable,
    line_styles: BlockLineStyleTable,
    ornaments: BlockOrnaments,
    /// 这一块**生效的**布局配置：代码块/引用块已在 `box_layout_config` 里
    /// 按水平内边距收窄过断行宽度。排版必须用它而不是调用方那份原始 config
    /// ——两份对不上，断行就按旧宽度算，长行会溢出背景盒。
    config: LayoutConfig,
    content_left: f32,
    container_left: f32,
}

impl BlockLayoutInput {
    /// 从 `yu-markdown` 的 extension 产出派生。
    ///
    /// `visual` 必须是同一份装饰投影出来的（[`VisualText`] 与
    /// `decorations` 同 range 同 Revision）：视觉文本从那边来，样式段在这边
    /// 算，两者对不上就是「画面少了几个字」。
    ///
    /// `kind` 是这一块在块序里的身份（段前段后间距、内边距、行高倍率都按
    /// 块类取，见 `layout_tokens`）；布局层不认识 Markdown，这份翻译只能发生
    /// 在这一层。
    ///
    /// # Errors
    ///
    /// 装饰指向的 id 查不到、几何参数不合法、视觉偏移溢出。
    pub fn from_decorations<M: ClusterMetrics>(
        kind: BlockKind,
        decorations: &BlockDecorations,
        visual: &VisualText,
        config: LayoutConfig,
        metrics: &M,
    ) -> Result<Self, LayoutError> {
        let draft = DecorationDraft::read(kind, decorations, visual, config)?;
        let marker = draft
            .marker
            .as_ref()
            .map(|marker| measure_marker_text(marker, metrics))
            .transpose()?;
        let config = box_layout_config_with_quote(kind, config, draft.quote.is_some());
        let ancestors = draft
            .ancestors
            .iter()
            .map(|marker| measure_marker_text(marker, metrics))
            .collect::<Result<Vec<_>, _>>()?;
        draft.assemble(config, marker, ancestors)
    }

    /// 按 shaping 后端派生。列表标记的字形一并留下。
    ///
    /// # Errors
    ///
    /// 同 [`BlockLayoutInput::from_decorations`]，外加 shaping 失败。
    pub fn from_decorations_shaped<S: ShapingProvider>(
        kind: BlockKind,
        decorations: &BlockDecorations,
        visual: &VisualText,
        config: LayoutConfig,
        shaper: &S,
    ) -> Result<Self, LayoutError> {
        let draft = DecorationDraft::read(kind, decorations, visual, config)?;
        let marker = draft
            .marker
            .as_ref()
            .map(|marker| shape_marker_text(marker, config, shaper))
            .transpose()?;
        let config = box_layout_config_with_quote(kind, config, draft.quote.is_some());
        let ancestors = draft
            .ancestors
            .iter()
            .map(|marker| shape_marker_text(marker, config, shaper))
            .collect::<Result<Vec<_>, _>>()?;
        draft.assemble(config, marker, ancestors)
    }

    #[must_use]
    pub fn layout_input(&self) -> LayoutInput<'_> {
        LayoutInput::new(&self.text, &self.runs)
            .with_widgets(&self.widgets)
            .with_line_styles(&self.lines)
    }

    /// 这一块生效的布局配置（断行宽度已按块类内边距收窄，见字段文档）。
    #[must_use]
    pub const fn layout_config(&self) -> LayoutConfig {
        self.config
    }

    /// Horizontal content bounds shared by text, tables and embedded resources.
    #[must_use]
    pub const fn content_left(&self) -> f32 {
        self.content_left
    }

    #[must_use]
    pub fn available_width(&self) -> f32 {
        (self.config.max_width() - self.content_left).max(1.0)
    }

    /// Outer box origin, before the block's own padding.
    #[must_use]
    pub const fn container_left(&self) -> f32 {
        self.container_left
    }

    /// 这个块上的 widget 锚点，按 `(visual, side)` 升序。
    #[must_use]
    pub fn widgets(&self) -> &[WidgetSpan] {
        &self.widgets
    }

    /// 覆盖的 source 落在 `source` 里的那些 widget。
    ///
    /// 按 source 分派而不是按视觉锚点，理由见 [`BlockLayoutInput`] 的
    /// `widget_sources` 字段。
    pub(crate) fn widgets_in(&self, source: TextRange) -> impl Iterator<Item = WidgetSpan> + '_ {
        self.widgets
            .iter()
            .zip(self.widget_sources.iter())
            .filter(move |(_, covered)| {
                covered.start() >= source.start() && covered.end() <= source.end()
            })
            .map(|(widget, _)| *widget)
    }

    /// 切出一段视觉区间，做成一份**零基**的排版输入。
    ///
    /// 表格的单元格用它：每一格按自己那一列的宽度排一次，而 [`BlockLayout`]
    /// 排的是一条从零开始的视觉字节流。此前表格是把整块排成一条线性流、再
    /// 把簇搬进格子——那条流按整块宽度断行，格子里放不下的内容不会重排，
    /// 于是后一列的内容压在前一列上。
    ///
    /// # Errors
    ///
    /// 区间越界或落在字符中间。
    pub(crate) fn slice(
        &self,
        visual: VisualRange,
        source: TextRange,
    ) -> Result<BlockLayoutSlice, LayoutError> {
        let (from, to) = (
            usize::try_from(visual.start().get()).map_err(|_| LayoutError::OffsetOverflow)?,
            usize::try_from(visual.end().get()).map_err(|_| LayoutError::OffsetOverflow)?,
        );
        let text = self
            .text
            .get(from..to)
            .ok_or(LayoutError::RunNotOnCharBoundary)?
            .to_owned();
        let rebase = |offset: VisualOffset| -> VisualOffset {
            VisualOffset::new(offset.get().saturating_sub(visual.start().get()))
        };
        let mut runs = Vec::new();
        for run in &self.runs {
            let start = run.visual().start().max(visual.start());
            let end = run.visual().end().min(visual.end());
            if start >= end {
                continue;
            }
            push_run(&mut runs, rebase(start), rebase(end), run.style())?;
        }
        let widgets = self
            .widgets_in(source)
            .map(|widget| WidgetSpan::new(rebase(widget.visual()), widget.widget(), widget.side()))
            .collect();
        Ok(BlockLayoutSlice {
            text,
            runs,
            widgets,
        })
    }

    #[must_use]
    pub fn text(&self) -> &str {
        &self.text
    }

    #[must_use]
    pub const fn styles(&self) -> &BlockStyleTable {
        &self.styles
    }

    #[must_use]
    pub const fn line_styles(&self) -> &BlockLineStyleTable {
        &self.line_styles
    }

    #[must_use]
    pub const fn ornaments(&self) -> &BlockOrnaments {
        &self.ornaments
    }
}

/// [`BlockLayoutInput`] 的一段，视觉偏移从零开始。
///
/// 它拥有自己的文本，所以借出去的 [`LayoutInput`] 与它同生命周期。
pub(crate) struct BlockLayoutSlice {
    text: String,
    runs: Vec<StyledRun>,
    widgets: Vec<WidgetSpan>,
}

impl BlockLayoutSlice {
    pub(crate) fn layout_input(&self) -> LayoutInput<'_> {
        LayoutInput::new(&self.text, &self.runs).with_widgets(&self.widgets)
    }
}

#[derive(Clone)]
pub(crate) struct MarkerDraft {
    pub(crate) gap: f32,
    source: TextRange,
    text: String,
    pub(crate) advance: f32,
    pub(crate) shaped: Option<ShapedText>,
    shape: Option<MarkerShape>,
}

fn measure_marker_text<M: ClusterMetrics>(
    marker: &MarkerOrnamentSource,
    metrics: &M,
) -> Result<MarkerDraft, LayoutError> {
    measure_marker_parts(marker.source, &marker.text, metrics)
}

pub(crate) fn measure_marker_parts<M: ClusterMetrics>(
    source: TextRange,
    text: &str,
    metrics: &M,
) -> Result<MarkerDraft, LayoutError> {
    use unicode_segmentation::UnicodeSegmentation;

    let mut advance = 0.0_f32;
    for cluster in text.graphemes(true) {
        let width = metrics.advance(cluster, TextStyle::Plain);
        if !width.is_finite() || width < 0.0 {
            return Err(LayoutError::InvalidMetrics(width.to_bits()));
        }
        advance += width;
    }
    let gap = metrics.advance(" ", TextStyle::Plain);
    if !gap.is_finite() || gap < 0.0 {
        return Err(LayoutError::InvalidMetrics(gap.to_bits()));
    }
    Ok(MarkerDraft {
        gap,
        source,
        text: text.to_owned(),
        advance,
        shaped: None,
        shape: None,
    })
}

fn shape_marker_text<S: ShapingProvider>(
    marker: &MarkerOrnamentSource,
    config: LayoutConfig,
    shaper: &S,
) -> Result<MarkerDraft, LayoutError> {
    let kind = match marker.text.as_str() {
        "•" => Some(MarkerShapeKind::Disc),
        "◦" => Some(MarkerShapeKind::Circle),
        "▪" => Some(MarkerShapeKind::Square),
        _ => None,
    };
    if let (Some(kind), Some(provider)) = (kind, shaper.paragraph_provider()) {
        let paragraph = provider
            .layout(&yu_core::ParagraphInput {
                justify: false,
                font_strut_mode: yu_core::FontStrutMode::RunMetrics,
                text: "M",
                base_direction: config.base_direction(),
                width: 10000.0,
                indent: 0.0,
                line_height: crate::layout_tokens::resolved_line_height(
                    config,
                    config.theme().spec().body_line_ratio,
                ),
                runs: vec![yu_core::ParagraphRun {
                    range: yu_core::VisualRange::new(
                        yu_core::VisualOffset::ZERO,
                        yu_core::VisualOffset::new(1),
                    )
                    .ok_or(LayoutError::OffsetOverflow)?,
                    style: StyleId(0),
                    attrs: TextAttrs::default().with_font(config.theme().spec().body_font),
                }],
                objects: vec![],
            })
            .map_err(LayoutError::Shaping)?;
        let ascent = paragraph
            .lines
            .first()
            .ok_or_else(|| LayoutError::Shaping("marker metrics have no line".into()))?
            .font_ascent;
        let zoom = config.line_height() / config.theme().spec().body_size;
        let ascent = (ascent / zoom).round();
        let two_thirds = (ascent * 2.0 / 3.0).floor();
        let size = ((two_thirds + 1.0) / 2.0).floor().max(1.0) * zoom;
        return Ok(MarkerDraft {
            gap: 0.0,
            source: marker.source,
            text: marker.text.clone(),
            advance: size,
            shaped: None,
            shape: Some(MarkerShape {
                kind,
                size,
                baseline_offset: ((3.0 * (ascent - two_thirds) / 2.0).floor() - ascent) * zoom,
                before_text: (ascent + 1.0) * zoom,
                stroke: zoom,
            }),
        });
    }
    shape_marker_parts(marker.source, &marker.text, shaper)
}

pub(crate) fn shape_marker_parts<S: ShapingProvider>(
    source: TextRange,
    text: &str,
    shaper: &S,
) -> Result<MarkerDraft, LayoutError> {
    shape_marker_parts_at_scale(source, text, shaper, 1.0)
}

pub(crate) fn shape_marker_parts_at_scale<S: ShapingProvider>(
    source: TextRange,
    text: &str,
    shaper: &S,
    scale: f32,
) -> Result<MarkerDraft, LayoutError> {
    let len = u64::try_from(text.len()).map_err(|_| LayoutError::OffsetOverflow)?;
    // 标记文本是合成的，不在 source 里。它的 shaping 空间是零基的局部空间，
    // 与 `BlockLayout::build_shaped` 给普通 run 的那一个同一种东西。
    let local = TextRange::new(ByteOffset::ZERO, ByteOffset::new(len))
        .ok_or(LayoutError::OffsetOverflow)?;
    let shaped = shaper
        .shape_scaled(text, local, TextStyle::Plain, scale)
        .map_err(|error| LayoutError::Shaping(error.to_string()))?;
    if shaped.source() != local {
        return Err(LayoutError::Shaping(
            "shaper returned a range different from the requested marker".into(),
        ));
    }
    let advance = shaped.advance();
    if !advance.is_finite() || advance < 0.0 {
        return Err(LayoutError::InvalidMetrics(advance.to_bits()));
    }
    let gap = shaper
        .shape_scaled(
            " ",
            TextRange::new(ByteOffset::ZERO, ByteOffset::new(1))
                .ok_or(LayoutError::OffsetOverflow)?,
            TextStyle::Plain,
            scale,
        )
        .map_err(|error| LayoutError::Shaping(error.to_string()))?
        .advance();
    if !gap.is_finite() || gap < 0.0 {
        return Err(LayoutError::InvalidMetrics(gap.to_bits()));
    }
    Ok(MarkerDraft {
        gap,
        source,
        text: text.to_owned(),
        advance,
        shaped: Some(shaped),
        shape: None,
    })
}

#[derive(Clone, Copy)]
struct HeadingMetrics {
    level: u8,
    font_scale: f32,
    line_height_scale: f32,
}

fn heading_metrics(level: u8, theme: yu_core::ThemeSpec) -> Result<HeadingMetrics, LayoutError> {
    if !(1..=6).contains(&level) {
        return Err(LayoutError::InvalidConfig(
            "heading level must be between one and six",
        ));
    }
    let font_scale = theme.heading_sizes[usize::from(level - 1)];
    let line_height_scale = theme.heading_lines[usize::from(level - 1)];
    Ok(HeadingMetrics {
        level,
        font_scale,
        line_height_scale,
    })
}

#[derive(Clone, Copy)]
struct BlockQuoteMetrics {
    depth: u8,
    gutter: f32,
}

fn block_quote_metrics(
    depth: u8,
    outside_list: u8,
    config: LayoutConfig,
) -> Result<BlockQuoteMetrics, LayoutError> {
    if depth == 0 {
        return Err(LayoutError::InvalidConfig(
            "blockquote depth must be positive",
        ));
    }
    let unit = ContainerMetrics::new(config).quote_indent;
    let gutter = unit * f32::from(depth)
        - ContainerMetrics::new(config).quote_margin_left
            * f32::from(depth.saturating_sub(outside_list));
    if !unit.is_finite() || !gutter.is_finite() {
        return Err(LayoutError::InvalidMetrics(gutter.to_bits()));
    }
    Ok(BlockQuoteMetrics { depth, gutter })
}

/// 分隔线那条横线有多粗：一个逻辑像素。
///
/// **不跟着行高缩放**，与引用竖条（`line_height * 0.12`）走的是两条路。竖条
/// 标示一整段引文的范围，是那段文字的一部分，跟着它一起大；横线是一道界线，
/// 与表格网格线（`yu_workspace::viewport_table_style` 的 `border_width`）同类
/// ——界线的粗细是一个常数，跟字号一起长会变成一条黑杠。
///
/// **它是一个整数，这一点有用**：后端画的是「这个矩形覆盖到的物理像素」，
/// 非整数的粗细会让同一条线在不同的纵坐标零头上占不同的像素行数。对齐那一半
/// 在 `yu-workspace`（那里才有绝对坐标），这一半在这里。
const THEMATIC_BREAK_THICKNESS: f32 = 1.0;

/// 分隔线那条横线的几何。
///
/// 宽度是**正文那一栏**减掉这一块自己的缩进，不是视口宽度——后者会让线比正文
/// 长出一截，右边悬空。今天分隔线不会出现在容器里（`indent` 恒为 0），减这一
/// 下是为了「块的边界由树定」之后它跟着正文走，而不是那时才发现算错了。
fn thematic_break_metrics(
    source: TextRange,
    indent: f32,
    config: LayoutConfig,
) -> Result<ThematicBreakOrnament, LayoutError> {
    let width = config.max_width() - indent;
    if !width.is_finite() || width <= 0.0 {
        return Err(LayoutError::InvalidMetrics(width.to_bits()));
    }
    Ok(ThematicBreakOrnament {
        source,
        x: indent,
        width,
        thickness: THEMATIC_BREAK_THICKNESS,
    })
}

/// 从 [`BlockDecorations`] 读出来的中间件。
///
/// 它只负责「装饰说了什么」：视觉文本、样式段、三种装饰的**语义值**。
/// 翻成几何（字号倍率、gutter、缩进）由 [`DecorationDraft::assemble`] 做，
/// 走的是与 v1 那条路**同一批**函数——差分要比的是「派生出了什么」，不是
/// 「同一段算术抄了两遍」。
struct DecorationDraft {
    /// 这一块在块序里的身份。决定盒模型待遇：代码块/引用块的内边距、代码块
    /// 的行高倍率（见 `layout_tokens`）。布局层不认识 Markdown，这份翻译只
    /// 能发生在这一层。
    kind: BlockKind,
    text: String,
    runs: Vec<StyledRun>,
    widgets: Vec<WidgetSpan>,
    widget_sources: Vec<TextRange>,
    styles: Vec<TextAttrs>,
    style_headings: Vec<Option<u8>>,
    source_range: TextRange,
    heading: Option<u8>,
    quote: Option<(u8, u8)>,
    /// 这一块的正文往右让多少列。列表项与任务项都有，标记只有列表项有。
    indent_columns: u8,
    marker: Option<MarkerOrnamentSource>,
    ancestors: Vec<MarkerOrnamentSource>,
    /// 这一块要画一条分隔线。**没有负载**：`BlockOrnament::ThematicBreak`
    /// 不带拼法也不带几何，画在哪由 [`DecorationDraft::assemble`] 现算。
    rule: bool,
    justify: bool,
}

/// 列表标记的语义值，还没量过宽度。
struct MarkerOrnamentSource {
    list_depth: u8,
    quote_depth: u8,
    source: TextRange,
    text: String,
}

impl DecorationDraft {
    fn read(
        kind: BlockKind,
        decorations: &BlockDecorations,
        visual: &VisualText,
        config: LayoutConfig,
    ) -> Result<Self, LayoutError> {
        let bounds = decorations.range();
        if bounds != visual.source_range() || decorations.revision() != visual.revision() {
            return Err(LayoutError::Upstream("视觉文本与装饰不是同一份产出".into()));
        }

        let marks: Vec<Mark> = decorations
            .set()
            .all()
            .iter()
            .filter_map(|entry| match entry.decoration {
                Decoration::Mark { style } => Some(Mark {
                    range: entry.range,
                    style,
                    priority: entry.priority,
                }),
                _ => None,
            })
            .collect();

        // 没有 Mark 盖着的那些段落到这个 id 上。它排在 extension 的 id 之后，
        // 所以不会与任何一个撞号。preedit 也用它——那段文字不在 source 里，
        // 没有任何 extension 会给它字型。
        let mut styles = decorations.styles().to_vec();
        let plain = StyleId(u32::try_from(styles.len()).map_err(|_| LayoutError::OffsetOverflow)?);
        styles.push(TextAttrs::new(TextStyle::Plain));

        // 样式段先在 **canonical** 视觉空间里排好：段落是 canonical 源码上的
        // 区间，而 preedit 是叠在最后的一层平移。混着算会让 preedit 旁边那
        // 一段文字排错字型——不报错，只是画得不对。
        //
        // 一段的两端各问一次映射，中间被隐藏的字节自然就没了：隐藏区间的
        // 视觉宽度是零。自己再切一遍「可见片段」是把 D4 那条映射重写一遍。
        let mut runs: Vec<StyledRun> = Vec::new();
        for (segment, style) in flatten_composed(bounds, &marks, &mut styles) {
            let style = style.unwrap_or(plain);
            let start = visual.canonical_source_to_visual(segment.start());
            let end = visual.canonical_source_to_visual(segment.end());
            push_run(&mut runs, start, end, style)?;
        }
        // widget 在视觉字节流里不占位，所以它只有一个锚点：被覆盖的那段
        // source 塌下去的那个视觉偏移。两端问同一个映射会得到同一个数，
        // 取起点。
        let mut widgets = Vec::new();
        let mut widget_sources = Vec::new();
        for entry in decorations.set().all() {
            let Decoration::Widget { widget, side } = entry.decoration else {
                continue;
            };
            widgets.push(WidgetSpan::new(
                visual.canonical_source_to_visual(entry.range.start()),
                widget,
                side,
            ));
            widget_sources.push(entry.range);
        }

        let (runs, widgets) = match visual.composition_visual() {
            Some(span) => (
                splice_composition(runs, visual, span, plain)?,
                shift_widgets(widgets, visual, span)?,
            ),
            None => (runs, widgets),
        };

        // Cell headings share this style table with glyph painting. Split runs
        // at paragraph boundaries instead of scaling an independently shaped copy.
        let mut style_headings = vec![None; styles.len()];
        let mut scoped = Vec::new();
        for (_, ornament) in decorations.line_ornaments() {
            if let BlockOrnament::Table(table) = ornament {
                for row in 0..table.visible_row_count() {
                    for column in 0..table.column_count() {
                        for part in
                            table.cell_paragraphs(yu_markdown::TableCellAddress::new(row, column))
                        {
                            if let Some(level) = part.heading {
                                let source = TextRange::new(
                                    ByteOffset::new(part.source.start() as u64),
                                    ByteOffset::new(part.source.end() as u64),
                                )
                                .ok_or(LayoutError::OffsetOverflow)?;
                                let range = visual
                                    .content_visual_range(source)
                                    .map_err(|e| LayoutError::Upstream(e.to_string()))?;
                                scoped.push((range.start(), range.end(), level));
                            }
                        }
                    }
                }
            }
        }
        scoped.sort_unstable_by_key(|(start, _, _)| *start);
        let runs = if scoped.is_empty() {
            runs
        } else {
            let mut contextual_runs = Vec::new();
            let mut contextual_styles = std::collections::BTreeMap::new();
            for run in runs {
                let mut boundaries = vec![run.visual().start(), run.visual().end()];
                let first = scoped.partition_point(|(_, end, _)| *end <= run.visual().start());
                for (start, end, _) in scoped[first..]
                    .iter()
                    .take_while(|(start, _, _)| *start < run.visual().end())
                {
                    if *start > run.visual().start() && *start < run.visual().end() {
                        boundaries.push(*start);
                    }
                    if *end > run.visual().start() && *end < run.visual().end() {
                        boundaries.push(*end);
                    }
                }
                boundaries.sort_unstable();
                boundaries.dedup();
                for pair in boundaries.windows(2) {
                    let candidate = scoped.partition_point(|(_, end, _)| *end <= pair[0]);
                    let level = scoped
                        .get(candidate)
                        .filter(|(start, end, _)| *start <= pair[0] && pair[1] <= *end)
                        .map(|(_, _, level)| *level);
                    let style = if let Some(level) = level {
                        let key = (run.style().0, level);
                        if let Some(style) = contextual_styles.get(&key) {
                            *style
                        } else {
                            let base = styles[run.style().0 as usize];
                            let style = StyleId(
                                u32::try_from(styles.len())
                                    .map_err(|_| LayoutError::OffsetOverflow)?,
                            );
                            styles.push(base);
                            style_headings.push(Some(level));
                            contextual_styles.insert(key, style);
                            style
                        }
                    } else {
                        run.style()
                    };
                    push_run(&mut contextual_runs, pair[0], pair[1], style)?;
                }
            }
            contextual_runs
        };

        let mut heading = None;
        let mut quote = None;
        let mut indent_columns = 0_u8;
        let mut marker = None;
        let mut ancestors = Vec::new();
        let mut rule = false;
        let mut justify = false;
        for (_, ornament) in decorations.line_ornaments() {
            match ornament {
                BlockOrnament::Heading { level } => heading = Some(*level),
                BlockOrnament::QuoteBar {
                    depth,
                    outside_list,
                } => quote = Some((*depth, *outside_list)),
                BlockOrnament::Indent { columns } => indent_columns = *columns,
                BlockOrnament::Marker(found) => {
                    if let Some(previous) = marker.take() {
                        ancestors.push(previous);
                    }
                    marker = Some(MarkerOrnamentSource {
                        list_depth: found.list_depth(),
                        quote_depth: found.quote_depth(),
                        source: found.source(),
                        text: if found.text() == "•" {
                            config
                                .theme()
                                .unordered_marker(found.list_depth())
                                .to_owned()
                        } else {
                            found.text().to_owned()
                        },
                    });
                }
                // 分隔线**不改变排版**：它那一块的视觉文本只剩一个换行符，
                // 排出来是一个空行，线画在那一行里。所以它进的是 ornaments，
                // 一个字节都不进 `LayoutInput`。
                BlockOrnament::ThematicBreak => rule = true,
                // 表格的网格不进文字流的排版输入：`TableLayout` 另算一遍
                // 几何，再把排好的簇搬进单元格。围栏代码块的语言名与正文
                // 也不进：它们是给嵌入渲染（KaTeX / Mermaid）看的语义，
                // 排版上代码块就是一段等宽文字。
                BlockOrnament::Alignment { alignment } => {
                    justify = *alignment == yu_markdown::html::HtmlAlignment::Justify;
                }
                BlockOrnament::Table(_) | BlockOrnament::FencedCode { .. } => {}
            }
        }

        // A checkbox owns the innermost list gutter. All emitted text/shape
        // markers belong to ancestors and must not add another body gutter.
        if matches!(kind, BlockKind::TaskListItem { .. })
            && let Some(previous) = marker.take()
        {
            ancestors.push(previous);
        }

        Ok(Self {
            kind,
            text: visual.text().to_owned(),
            runs,
            widgets,
            widget_sources,
            styles,
            style_headings,
            source_range: bounds,
            heading,
            quote,
            indent_columns,
            marker,
            ancestors,
            rule,
            justify,
        })
    }

    fn assemble(
        self,
        config: LayoutConfig,
        marker: Option<MarkerDraft>,
        ancestors: Vec<MarkerDraft>,
    ) -> Result<BlockLayoutInput, LayoutError> {
        let kind = self.kind;
        let theme = config.theme().spec();
        let heading = self
            .heading
            .map(|level| heading_metrics(level, theme))
            .transpose()?;
        let quote = self
            .quote
            .map(|(depth, outside_list)| block_quote_metrics(depth, outside_list, config))
            .transpose()?;

        let quote_gutter = quote.map_or(0.0, |quote| quote.gutter);
        let marker_quote_gutter = self.marker.as_ref().map_or(0.0, |marker| {
            let metrics = ContainerMetrics::new(config);
            let outside = self
                .quote
                .map_or(0, |(_, outside)| outside)
                .min(marker.quote_depth);
            f32::from(marker.quote_depth) * metrics.quote_indent
                - f32::from(marker.quote_depth - outside) * metrics.quote_margin_left
        });
        // 四段相加，各说一件事：块级盒模型的水平内边距（代码块/引用块，见
        // `layout_tokens::box_content_inset_x`——断行宽度已在
        // `box_layout_config` 里同步收窄）、引用的竖条让出多少、源码里缩进
        // 了几列、行首标记本身占多宽（外加它与正文之间那一列）。
        //
        // 缩进此前挂在标记上，于是**没有标记的块一列都让不出来**——嵌套的
        // 任务项贴着左边缘，而同一层的普通列表项缩进了。
        let box_inset = box_content_inset_x(kind, config);
        let list_unit = ContainerMetrics::new(config).list_indent;
        let column_gutter = list_unit * f32::from(self.indent_columns) / 2.0;
        let marker_gutter = marker
            .as_ref()
            .map_or(0.0, |marker| list_unit.max(marker.advance + marker.gap));
        let task_gutter = if matches!(kind, BlockKind::TaskListItem { .. }) {
            (list_unit - config.line_height() * yu_core::ThemeSpec::TASK_MARKER_ADVANCE_EM).max(0.0)
        } else {
            0.0
        };
        let indent = box_inset + quote_gutter + column_gutter + marker_gutter + task_gutter;
        let rule = self
            .rule
            .then(|| thematic_break_metrics(self.source_range, indent, config))
            .transpose()?;

        let visual_len =
            VisualOffset::try_from(self.text.len()).map_err(|_| LayoutError::OffsetOverflow)?;
        let visual = VisualRange::new(VisualOffset::ZERO, visual_len).ok_or(
            LayoutError::InvalidConfig("visual text length must be a valid range"),
        )?;

        // 标题的字号倍率盖在**整张表**上，而不是让 heading extension 产一条
        // 覆盖全块的 `Strong` Mark。理由是分层：「几级标题」是语义，归
        // `yu-markdown`；「1.7 倍字号、排粗体」是呈现，只有这一层有
        // `LayoutConfig` 说得出来。v1 的 `HeadingClusterMetrics` 也是在这一
        // 层把字型整个丢掉，一律按 `Strong` 量。
        //
        // **这里是重建，不是修改**：`TextAttrs::new` 从头造一份，所以装饰那边
        // 每加一样属性都必须在这里显式带过来。配色角色（S7 第五刀）就是这么
        // 掉过一次的——`assemble` 把它归零，代码块里一个字都不着色，而
        // `yu-markdown` 那一侧的断言全绿：装饰产出是对的，丢在下一层。
        let literal_monospace = self
            .styles
            .iter()
            .any(|style| style.font() == yu_core::ThemeFont::SystemMono);
        let mut attrs = Vec::with_capacity(self.styles.len());
        for (base, cell_heading) in self.styles.into_iter().zip(self.style_headings) {
            let heading = cell_heading
                .map(|level| heading_metrics(level, theme))
                .transpose()?
                .or(heading);
            let font_scale = heading.map_or(1.0, |heading| heading.font_scale);
            let style = if let Some(heading) = heading {
                if theme.heading_bold[usize::from(heading.level - 1)] != 0 {
                    base.style().union(TextStyle::Strong)
                } else {
                    base.style()
                }
            } else {
                base.style()
            };
            let font = if base.font() != yu_core::ThemeFont::Inherit {
                base.font()
            } else if style.is_code() {
                theme.code_font
            } else if heading.is_some() {
                theme.heading_font
            } else {
                theme.body_font
            };
            let (script_scale, script_rise) = match base.script() {
                yu_core::TextScript::Normal => (1.0, 0.0),
                yu_core::TextScript::Superscript => (
                    yu_core::ThemeSpec::SCRIPT_SIZE_RATIO,
                    yu_core::ThemeSpec::SUPERSCRIPT_RISE_EM,
                ),
                yu_core::TextScript::Subscript => (
                    yu_core::ThemeSpec::SCRIPT_SIZE_RATIO,
                    yu_core::ThemeSpec::SUBSCRIPT_RISE_EM,
                ),
            };
            attrs.push(
                TextAttrs::new(style)
                    .with_highlighted(base.highlighted())
                    .with_underlined(base.underlined())
                    .with_struck(base.struck())
                    .with_script(base.script())
                    .with_baseline_offset(script_rise * config.line_height() * font_scale)
                    .ok_or(LayoutError::InvalidMetrics(config.line_height().to_bits()))?
                    .with_inline_box_id(base.inline_box_id())
                    .with_inline_inset(if base.inline_box_id().is_some() && !is_code_block(kind) {
                        (config.theme().inline_padding().0 + config.theme().inline_border())
                            * config.line_height()
                            / theme.body_size
                    } else {
                        0.0
                    })
                    .ok_or(LayoutError::InvalidMetrics(config.line_height().to_bits()))?
                    .with_inline_inset_y(
                        if base.inline_box_id().is_some() && !is_code_block(kind) {
                            (config.theme().inline_padding().1 + config.theme().inline_border())
                                * config.line_height()
                                / theme.body_size
                        } else {
                            0.0
                        },
                    )
                    .ok_or(LayoutError::InvalidMetrics(config.line_height().to_bits()))?
                    .with_font(font)
                    .with_letter_spacing(heading.map_or(0.0, |h| {
                        theme.heading_letter_spacing[usize::from(h.level - 1)]
                            * config.line_height()
                            / theme.body_size
                    }))
                    .ok_or(LayoutError::InvalidMetrics(config.line_height().to_bits()))?
                    .with_size_scale(
                        font_scale
                            * script_scale
                            * if is_code_block(kind) && base.style().is_code() {
                                theme.code_block_size_ratio
                            } else if base.style().is_code() {
                                config.theme().inline_code_size_ratio(heading.is_some())
                            } else {
                                1.0
                            },
                    )
                    .ok_or(LayoutError::InvalidMetrics(font_scale.to_bits()))?
                    .with_role(base.role()),
            );
        }

        // 行高倍率按块类给：标题按级别（h1/h2 1.3，h3–h6 1.25），代码块
        // 1.65，其余一律正文 1.6（段落、引用、列表同待遇，见
        // `layout_tokens`——这是 Typora 标杆值，不再默认 1.0）。
        let line_height_scale = if let Some(heading) = heading {
            heading.line_height_scale * heading.font_scale
        } else if is_code_block(kind) {
            theme.code_line_ratio * theme.code_block_size_ratio
        } else {
            theme.body_line_ratio
        };

        let line_height_scale =
            crate::layout_tokens::resolved_line_height(config, line_height_scale)
                / config.line_height();

        let ancestor_ornaments = ancestors
            .into_iter()
            .zip(&self.ancestors)
            .map(|(marker, source)| {
                let metrics = ContainerMetrics::new(config);
                let outside = self
                    .quote
                    .map_or(0, |(_, outside)| outside)
                    .min(source.quote_depth);
                let quotes = f32::from(source.quote_depth) * metrics.quote_indent
                    - f32::from(source.quote_depth - outside) * metrics.quote_margin_left;
                let gutter = list_unit.max(marker.advance + marker.gap);
                MarkerOrnament {
                    quoted: source.quote_depth > 0,
                    source: marker.source,
                    text: marker.text,
                    x: box_inset
                        + quotes
                        + list_unit * f32::from(source.list_depth.saturating_sub(1))
                        + gutter
                        - marker
                            .shape
                            .map_or(marker.advance + marker.gap, |shape| shape.before_text),
                    advance: marker.advance,
                    shaped: marker.shaped,
                    shape: marker.shape,
                }
            })
            .collect();
        let primary_quoted = self
            .marker
            .as_ref()
            .is_some_and(|marker| marker.quote_depth > 0);

        Ok(BlockLayoutInput {
            content_left: indent,
            container_left: (indent - box_inset).max(0.0),
            text: self.text,
            runs: self.runs,
            widgets: self.widgets,
            widget_sources: self.widget_sources,
            lines: vec![LineSpan::new(visual, BLOCK_LINE_STYLE)],
            styles: BlockStyleTable { attrs },
            line_styles: BlockLineStyleTable {
                attrs: LineAttrs::new(indent, line_height_scale)?
                    .with_justification(self.justify)
                    .with_font_struts(if literal_monospace {
                        yu_core::FontStrutMode::BaseFont
                    } else if heading.is_some() {
                        yu_core::FontStrutMode::DeclaredFonts
                    } else if !is_code_block(kind) {
                        // Body text shares a baseline across independently sized
                        // inline font struts; their union may exceed line-height.
                        config.theme().body_font_struts()
                    } else {
                        config.theme().code_font_struts()
                    }),
            },
            ornaments: BlockOrnaments {
                heading: heading.map(|heading| HeadingOrnament {
                    source: self.source_range,
                    level: heading.level,
                    font_scale: heading.font_scale,
                    line_height_scale: heading.line_height_scale,
                }),
                ancestors: ancestor_ornaments,
                marker: marker.map(|marker| MarkerOrnament {
                    quoted: primary_quoted,
                    source: marker.source,
                    text: marker.text,
                    x: box_inset + marker_quote_gutter + column_gutter + marker_gutter
                        - marker
                            .shape
                            .map_or(marker.advance + marker.gap, |shape| shape.before_text),
                    advance: marker.advance,
                    shaped: marker.shaped,
                    shape: marker.shape,
                }),
                quote: quote.map(|quote| BlockQuoteOrnament {
                    source: self.source_range,
                    depth: quote.depth,
                }),
                rule,
            },
            config,
        })
    }
}

/// 追加一段样式区间，与前一段同样式且相接时合成一段。
///
/// 合成不是为了省事：布局那边少一次换字型，差分也不会因为「隐藏区间把一段
/// 样式切成几截」而假红。空段直接丢——整段都被隐藏时两端映射到同一个视觉
/// 偏移，那不是一段文字。
fn push_run(
    runs: &mut Vec<StyledRun>,
    start: VisualOffset,
    end: VisualOffset,
    style: StyleId,
) -> Result<(), LayoutError> {
    if start >= end {
        return Ok(());
    }
    match runs.last_mut() {
        Some(last) if last.style() == style && last.visual().end() == start => {
            *last = StyledRun::new(
                VisualRange::new(last.visual().start(), end).ok_or(LayoutError::OffsetOverflow)?,
                style,
            );
        }
        _ => runs.push(StyledRun::new(
            VisualRange::new(start, end).ok_or(LayoutError::OffsetOverflow)?,
            style,
        )),
    }
    Ok(())
}

/// 把 preedit 叠进 canonical 视觉空间里排好的样式段。
///
/// preedit 替换掉的那一段 canonical 文字整个让位：跨在边界上的样式段被裁到
/// 边界处，之后的整体后移，中间空出来的那一段是 preedit 自己的 run。
///
/// preedit **不排 Markdown 字型**：它还没进 source，把它按周围的样式排会让
/// 用户以为已经生效了。v1 也是给它一个 `Plain` 的 run。
fn splice_composition(
    runs: Vec<StyledRun>,
    visual: &VisualText,
    span: VisualRange,
    plain: StyleId,
) -> Result<Vec<StyledRun>, LayoutError> {
    let composition = CompositionShift::read(visual, span)?;
    let (old_start, old_end) = (composition.old_start, composition.old_end);
    let shift = |offset: VisualOffset| composition.shift(offset);

    let mut spliced: Vec<StyledRun> = Vec::with_capacity(runs.len().saturating_add(1));
    for run in runs {
        let (from, to) = (run.visual().start(), run.visual().end());
        if to <= old_start {
            push_run(&mut spliced, from, to, run.style())?;
            continue;
        }
        if from >= old_end {
            push_run(&mut spliced, shift(from)?, shift(to)?, run.style())?;
            continue;
        }
        if from < old_start {
            push_run(&mut spliced, from, old_start, run.style())?;
        }
        if to > old_end {
            push_run(&mut spliced, shift(old_end)?, shift(to)?, run.style())?;
        }
    }
    if !span.is_empty() {
        let at = spliced
            .iter()
            .position(|run| run.visual().start() >= span.end())
            .unwrap_or(spliced.len());
        spliced.insert(
            at,
            StyledRun::new(
                VisualRange::new(span.start(), span.end()).ok_or(LayoutError::OffsetOverflow)?,
                plain,
            ),
        );
    }
    Ok(spliced)
}

/// preedit 把 canonical 视觉空间里的一段换成了另一段。
///
/// 样式段与 widget 锚点都排在 canonical 空间里，然后由这一层整体让位——
/// 两处各写一遍这个平移，就会在「preedit 前面还是后面」的边界上分叉，而
/// 分叉的表现是 preedit 旁边的字型或图片盒子偏几个字节，不报错。
#[derive(Clone, Copy)]
struct CompositionShift {
    old_start: VisualOffset,
    old_end: VisualOffset,
    span: VisualRange,
}

impl CompositionShift {
    fn read(visual: &VisualText, span: VisualRange) -> Result<Self, LayoutError> {
        let replacement = visual
            .composition_range()
            .ok_or(LayoutError::Upstream("preedit 区间缺失".into()))?;
        Ok(Self {
            old_start: visual.canonical_source_to_visual(replacement.start()),
            old_end: visual.canonical_source_to_visual(replacement.end()),
            span,
        })
    }

    /// preedit **之后**的一个偏移挪到它的新位置。
    ///
    /// 平移量可以是负数（三个字的 preedit 换成一个字符），所以中间那一步
    /// 走 `i128`——用无符号数算会在那种情况下饱和到 0，之后每个位置都差
    /// 几个字节，不 panic、不报错。
    fn shift(self, offset: VisualOffset) -> Result<VisualOffset, LayoutError> {
        let moved = i128::from(offset.get()) + i128::from(self.span.end().get())
            - i128::from(self.old_end.get());
        u64::try_from(moved)
            .map(VisualOffset::new)
            .map_err(|_| LayoutError::OffsetOverflow)
    }
}

/// 把 widget 锚点叠进 preedit。
///
/// preedit 盖住锚点时锚点落到 preedit 的起点：widget 覆盖的那段 source 正在
/// 被替换，它没有别的地方可去。丢掉它也是一种选择，但那会让「装饰集合里的
/// widget 与排出来的盒子一一对应」不再成立。
fn shift_widgets(
    widgets: Vec<WidgetSpan>,
    visual: &VisualText,
    span: VisualRange,
) -> Result<Vec<WidgetSpan>, LayoutError> {
    let composition = CompositionShift::read(visual, span)?;
    widgets
        .into_iter()
        .map(|widget| {
            let anchor = if widget.visual() <= composition.old_start {
                widget.visual()
            } else if widget.visual() >= composition.old_end {
                composition.shift(widget.visual())?
            } else {
                composition.span.start()
            };
            Ok(WidgetSpan::new(anchor, widget.widget(), widget.side()))
        })
        .collect()
}
