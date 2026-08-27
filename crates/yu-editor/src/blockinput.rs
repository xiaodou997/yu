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

use crate::marks::{Mark, flatten};
use yu_core::{
    ByteOffset, ClusterMetrics, LineStyleId, ShapedText, ShapingProvider, StyleId, TextAttrs,
    TextRange, TextStyle,
};
use yu_core::{VisualOffset, VisualRange};
use yu_decoration::Decoration;
use yu_layout::{
    LayoutConfig, LayoutError, LayoutInput, LayoutRect, LineAttrs, LineSpan, LineStyleTable,
    StyleTable, StyledRun,
};
use yu_markdown::{BlockDecorations, BlockOrnament};

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
    source: TextRange,
    text: String,
    x: f32,
    advance: f32,
    shaped: Option<ShapedText>,
}

impl MarkerOrnament {
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

    /// shaping 那条路上排好的字形。按度量派生时是 `None`。
    #[must_use]
    pub const fn shaped(&self) -> Option<&ShapedText> {
        self.shaped.as_ref()
    }
}

/// 引用的竖条。
///
/// 竖条贯穿整块，而块高要等布局排完才知道，所以这里只留参数，矩形由
/// [`BlockQuoteOrnament::bars`] 现算。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BlockQuoteOrnament {
    source: TextRange,
    depth: u8,
    unit: f32,
    bar_width: f32,
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

    /// 每一层引用竖条的矩形，从外到内。
    pub fn bars(self, height: f32) -> Result<Vec<LayoutRect>, LayoutError> {
        let mut bars = Vec::with_capacity(usize::from(self.depth));
        for level in 0..self.depth {
            let unit_start = f32::from(level) * self.unit;
            let x = unit_start + (self.unit - self.bar_width) * 0.25;
            bars.push(LayoutRect::new(x, 0.0, self.bar_width, height)?);
        }
        Ok(bars)
    }
}

/// 一个块上「长什么样」的那部分装饰。布局层看不见它们。
#[derive(Clone, Debug, Default, PartialEq)]
pub struct BlockOrnaments {
    heading: Option<HeadingOrnament>,
    marker: Option<MarkerOrnament>,
    quote: Option<BlockQuoteOrnament>,
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

    #[must_use]
    pub const fn quote(&self) -> Option<BlockQuoteOrnament> {
        self.quote
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
    lines: Vec<LineSpan>,
    styles: BlockStyleTable,
    line_styles: BlockLineStyleTable,
    ornaments: BlockOrnaments,
}

impl BlockLayoutInput {
    /// 从 `yu-markdown` 的 extension 产出派生。
    ///
    /// `visual` 必须是同一份装饰投影出来的（[`VisualText`] 与
    /// `decorations` 同 range 同 Revision）：视觉文本从那边来，样式段在这边
    /// 算，两者对不上就是「画面少了几个字」。
    ///
    /// # Errors
    ///
    /// 装饰指向的 id 查不到、几何参数不合法、视觉偏移溢出。
    pub fn from_decorations<M: ClusterMetrics>(
        decorations: &BlockDecorations,
        visual: &VisualText,
        config: LayoutConfig,
        metrics: &M,
    ) -> Result<Self, LayoutError> {
        let draft = DecorationDraft::read(decorations, visual)?;
        let marker = draft
            .marker
            .as_ref()
            .map(|marker| measure_marker_text(marker, metrics))
            .transpose()?;
        draft.assemble(config, marker)
    }

    /// 按 shaping 后端派生。列表标记的字形一并留下。
    ///
    /// # Errors
    ///
    /// 同 [`BlockLayoutInput::from_decorations`]，外加 shaping 失败。
    pub fn from_decorations_shaped<S: ShapingProvider>(
        decorations: &BlockDecorations,
        visual: &VisualText,
        config: LayoutConfig,
        shaper: &S,
    ) -> Result<Self, LayoutError> {
        let draft = DecorationDraft::read(decorations, visual)?;
        let marker = draft
            .marker
            .as_ref()
            .map(|marker| shape_marker_text(marker, shaper))
            .transpose()?;
        draft.assemble(config, marker)
    }

    #[must_use]
    pub fn layout_input(&self) -> LayoutInput<'_> {
        LayoutInput::new(&self.text, &self.runs).with_line_styles(&self.lines)
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

struct MarkerDraft {
    source: TextRange,
    text: String,
    indent: u8,
    advance: f32,
    shaped: Option<ShapedText>,
}

fn measure_marker_text<M: ClusterMetrics>(
    marker: &MarkerOrnamentSource,
    metrics: &M,
) -> Result<MarkerDraft, LayoutError> {
    measure_marker_parts(marker.source, &marker.text, marker.indent, metrics)
}

fn measure_marker_parts<M: ClusterMetrics>(
    source: TextRange,
    text: &str,
    indent: u8,
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
    Ok(MarkerDraft {
        source,
        text: text.to_owned(),
        indent,
        advance,
        shaped: None,
    })
}

fn shape_marker_text<S: ShapingProvider>(
    marker: &MarkerOrnamentSource,
    shaper: &S,
) -> Result<MarkerDraft, LayoutError> {
    shape_marker_parts(marker.source, &marker.text, marker.indent, shaper)
}

fn shape_marker_parts<S: ShapingProvider>(
    source: TextRange,
    text: &str,
    indent: u8,
    shaper: &S,
) -> Result<MarkerDraft, LayoutError> {
    let len = u64::try_from(text.len()).map_err(|_| LayoutError::OffsetOverflow)?;
    // 标记文本是合成的，不在 source 里。它的 shaping 空间是零基的局部空间，
    // 与 `BlockLayout::build_shaped` 给普通 run 的那一个同一种东西。
    let local = TextRange::new(ByteOffset::ZERO, ByteOffset::new(len))
        .ok_or(LayoutError::OffsetOverflow)?;
    let shaped = shaper
        .shape(text, local, TextStyle::Plain)
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
    Ok(MarkerDraft {
        source,
        text: text.to_owned(),
        indent,
        advance,
        shaped: Some(shaped),
    })
}

#[derive(Clone, Copy)]
struct HeadingMetrics {
    level: u8,
    font_scale: f32,
    line_height_scale: f32,
}

fn heading_metrics(level: u8) -> Result<HeadingMetrics, LayoutError> {
    let (font_scale, line_height_scale) = match level {
        1 => (2.0, 2.2),
        2 => (1.7, 1.9),
        3 => (1.45, 1.65),
        4 => (1.25, 1.4),
        5 => (1.1, 1.2),
        6 => (1.0, 1.1),
        _ => {
            return Err(LayoutError::InvalidConfig(
                "heading level must be between one and six",
            ));
        }
    };
    Ok(HeadingMetrics {
        level,
        font_scale,
        line_height_scale,
    })
}

#[derive(Clone, Copy)]
struct BlockQuoteMetrics {
    depth: u8,
    unit: f32,
    bar_width: f32,
    gutter: f32,
}

fn block_quote_metrics(depth: u8, config: LayoutConfig) -> Result<BlockQuoteMetrics, LayoutError> {
    if depth == 0 {
        return Err(LayoutError::InvalidConfig(
            "blockquote depth must be positive",
        ));
    }
    let bar_width = (config.line_height() * 0.12).clamp(1.0, 3.0);
    let unit = (config.default_advance() * 2.0).max(bar_width + config.default_advance());
    let gutter = unit * f32::from(depth);
    if !unit.is_finite() || !gutter.is_finite() {
        return Err(LayoutError::InvalidMetrics(gutter.to_bits()));
    }
    Ok(BlockQuoteMetrics {
        depth,
        unit,
        bar_width,
        gutter,
    })
}

/// 从 [`BlockDecorations`] 读出来的中间件。
///
/// 它只负责「装饰说了什么」：视觉文本、样式段、三种装饰的**语义值**。
/// 翻成几何（字号倍率、gutter、缩进）由 [`DecorationDraft::assemble`] 做，
/// 走的是与 v1 那条路**同一批**函数——差分要比的是「派生出了什么」，不是
/// 「同一段算术抄了两遍」。
struct DecorationDraft {
    text: String,
    runs: Vec<StyledRun>,
    styles: Vec<TextAttrs>,
    source_range: TextRange,
    heading: Option<u8>,
    quote: Option<u8>,
    marker: Option<MarkerOrnamentSource>,
}

/// 列表标记的语义值，还没量过宽度。
struct MarkerOrnamentSource {
    source: TextRange,
    text: String,
    indent: u8,
}

impl DecorationDraft {
    fn read(decorations: &BlockDecorations, visual: &VisualText) -> Result<Self, LayoutError> {
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
        for (segment, style) in flatten(bounds, &marks) {
            let style = style.unwrap_or(plain);
            let start = visual.canonical_source_to_visual(segment.start());
            let end = visual.canonical_source_to_visual(segment.end());
            push_run(&mut runs, start, end, style)?;
        }
        let runs = match visual.composition_visual() {
            Some(span) => splice_composition(runs, visual, span, plain)?,
            None => runs,
        };

        let mut heading = None;
        let mut quote = None;
        let mut marker = None;
        for (_, ornament) in decorations.line_ornaments() {
            match ornament {
                BlockOrnament::Heading { level } => heading = Some(*level),
                BlockOrnament::QuoteBar { depth } => quote = Some(*depth),
                BlockOrnament::Marker(found) => {
                    marker = Some(MarkerOrnamentSource {
                        source: found.source(),
                        text: found.text().to_owned(),
                        indent: found.indent(),
                    });
                }
                // 表格的网格不进文字流的排版输入：`TableLayout` 另算一遍
                // 几何，再把排好的簇搬进单元格。围栏代码块的语言名与正文
                // 也不进：它们是给嵌入渲染（KaTeX / Mermaid）看的语义，
                // 排版上代码块就是一段等宽文字。
                BlockOrnament::Table(_) | BlockOrnament::FencedCode { .. } => {}
            }
        }

        Ok(Self {
            text: visual.text().to_owned(),
            runs,
            styles,
            source_range: bounds,
            heading,
            quote,
            marker,
        })
    }

    fn assemble(
        self,
        config: LayoutConfig,
        marker: Option<MarkerDraft>,
    ) -> Result<BlockLayoutInput, LayoutError> {
        let heading = self.heading.map(heading_metrics).transpose()?;
        let quote = self
            .quote
            .map(|depth| block_quote_metrics(depth, config))
            .transpose()?;

        let quote_gutter = quote.map_or(0.0, |quote| quote.gutter);
        let marker_gutter = marker.as_ref().map_or(0.0, |marker| {
            marker.advance + config.default_advance() * (f32::from(marker.indent) + 1.0)
        });
        let indent = quote_gutter + marker_gutter;

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
        let font_scale = heading.map_or(1.0, |heading| heading.font_scale);
        let mut attrs = Vec::with_capacity(self.styles.len());
        for base in self.styles {
            let style = if heading.is_some() {
                TextStyle::Strong
            } else {
                base.style()
            };
            attrs.push(
                TextAttrs::new(style)
                    .with_size_scale(font_scale)
                    .ok_or(LayoutError::InvalidMetrics(font_scale.to_bits()))?,
            );
        }

        Ok(BlockLayoutInput {
            text: self.text,
            runs: self.runs,
            lines: vec![LineSpan::new(visual, BLOCK_LINE_STYLE)],
            styles: BlockStyleTable { attrs },
            line_styles: BlockLineStyleTable {
                attrs: LineAttrs::new(
                    indent,
                    heading.map_or(1.0, |heading| heading.line_height_scale),
                )?,
            },
            ornaments: BlockOrnaments {
                heading: heading.map(|heading| HeadingOrnament {
                    source: self.source_range,
                    level: heading.level,
                    font_scale: heading.font_scale,
                    line_height_scale: heading.line_height_scale,
                }),
                marker: marker.map(|marker| MarkerOrnament {
                    source: marker.source,
                    text: marker.text,
                    x: quote_gutter + f32::from(marker.indent) * config.default_advance(),
                    advance: marker.advance,
                    shaped: marker.shaped,
                }),
                quote: quote.map(|quote| BlockQuoteOrnament {
                    source: self.source_range,
                    depth: quote.depth,
                    unit: quote.unit,
                    bar_width: quote.bar_width,
                }),
            },
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
    let replacement = visual
        .composition_range()
        .ok_or(LayoutError::Upstream("preedit 区间缺失".into()))?;
    let old_start = visual.canonical_source_to_visual(replacement.start());
    let old_end = visual.canonical_source_to_visual(replacement.end());
    let shift = |offset: VisualOffset| -> Result<VisualOffset, LayoutError> {
        let moved =
            i128::from(offset.get()) + i128::from(span.end().get()) - i128::from(old_end.get());
        u64::try_from(moved)
            .map(VisualOffset::new)
            .map_err(|_| LayoutError::OffsetOverflow)
    };

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
