//! 块布局：输入只有样式化的视觉文本。
//!
//! # 这一层看得见什么
//!
//! [`BlockLayout`] 只看见三样东西：
//!
//! - **视觉文本**：源码里没被隐藏的那些字节，已经拼成一段连续的 `&str`；
//! - **[`StyledRun`]**：视觉文本上的样式区间，样式是不透明的 [`StyleId`]；
//! - **样式表**：[`StyleTable`]，把 `StyleId` 翻译成排版属性 [`TextAttrs`]。
//!
//! 表由产出装饰的那一层填（它知道 `StyleId(3)` 是什么），这一层只会读到
//! 「斜体、1.0 倍字号」。这就是不变量 E1 在布局层的落法：不是「不写某个
//! 词」，是**拿不到**判断语法语义所需的信息。overview-v2 §2.1 点名的那条
//! 泄漏——布局层为了排版必须先认识文档语法——到此为止。
//!
//! # 坐标
//!
//! 输出里没有源码坐标，只有 [`VisualOffset`]。视觉↔源码是
//! `DecorationSet` 的双向映射（不变量 D4「这是投影映射链的唯一实现」），
//! 让布局也做一遍就会有第二套映射。调用方在拿到布局结果之后自己换算。
//!
//! # 断行与重排
//!
//! 断行是 UAX #14（`unicode-linebreak`），在**逻辑**顺序上做；重排是 UAX #9
//! （`unicode-bidi`）的 L1/L2，在断行**之后**逐行做。顺序不能反：L1 要重置的
//! 是「行尾」的空白，而哪里是行尾要断完行才知道。
//!
//! CJK 禁则不需要另加 tailoring，UAX #14 的默认对表已经覆盖。
//!
//! # 行级样式
//!
//! [`LineSpan`] 给出缩进与行高倍率（[`LineAttrs`]）。缩进**吃掉可用宽度**，
//! 不只是把内容右移。背景与前缀装饰的样子不在这一层：[`LineBox`] 原样带出
//! 它的 [`LineStyleId`]，由上层拿同一张表去画。
//!
//! # widget
//!
//! widget 在视觉字节流里不占位（不变量 D7 的字节层面语义），但在行里占宽度
//! 与高度。尺寸由 [`WidgetMeasure`] 给；资源没就绪时给 placeholder 尺寸，
//! 布局照常完成，[`BlockLayout::pending_widgets`] 报出还欠着谁。发通知、
//! 退避重试、源码回退不在这一层。
//!
//! # 这一版还没做的两件事
//!
//! - **RTL 段落不右对齐。** 重排给出的是行内的相对顺序，把整行推到
//!   `max_width` 那一侧是对齐，属于 `LineStyle` 的事。
//! - **方向变化处只给一个 caret 位置。** 见 [`BlockLayout::caret`]。

use std::ops::Range;

use unicode_bidi::{BidiInfo, Level};
use unicode_segmentation::UnicodeSegmentation;
use yu_core::{
    ByteOffset, CaretAffinity, ClusterMetrics, FontFaceId, GlyphId, LineStyleId, ShapedText,
    ShapingProvider, Size, StyleId, TextAttrs, TextRange, VisualOffset, VisualRange, WidgetId,
    WidgetSide,
};

use crate::{BaseDirection, LayoutConfig, LayoutError, LayoutPoint, LayoutRect};

/// 把 [`StyleId`] 翻译成排版属性。
///
/// 实现住在产出装饰的那一层——只有它知道自己给 `StyleId(3)` 赋了什么含义。
/// 未知 id 返回 `None`，布局会报 [`LayoutError::UnknownStyle`] 而不是
/// 悄悄按默认字型排：一个「样式表没跟上装饰产出」的 bug 应该响，
/// 不应该只是画得不对。
pub trait StyleTable {
    fn attrs(&self, style: StyleId) -> Option<TextAttrs>;
}

/// 一张不区分 id 的样式表，全部返回同一组属性。
///
/// 给「还没有样式产出」的调用方与测试用。
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct UniformStyleTable {
    attrs: TextAttrs,
}

impl UniformStyleTable {
    #[must_use]
    pub const fn new(attrs: TextAttrs) -> Self {
        Self { attrs }
    }
}

impl StyleTable for UniformStyleTable {
    fn attrs(&self, _style: StyleId) -> Option<TextAttrs> {
        Some(self.attrs)
    }
}

/// 视觉文本上的一段样式。
///
/// `visual` 是**视觉**字节区间，不是源码区间。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct StyledRun {
    visual: VisualRange,
    style: StyleId,
}

impl StyledRun {
    #[must_use]
    pub const fn new(visual: VisualRange, style: StyleId) -> Self {
        Self { visual, style }
    }

    #[must_use]
    pub const fn visual(self) -> VisualRange {
        self.visual
    }

    #[must_use]
    pub const fn style(self) -> StyleId {
        self.style
    }
}

/// 一个 widget 在视觉文本里的锚点。
///
/// widget 在视觉字节流里**不占位**（不变量 D7 的字节层面语义），所以它只有
/// 一个偏移，没有区间。宽高是这一层的事，由 [`WidgetMeasure`] 给。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct WidgetSpan {
    visual: VisualOffset,
    widget: WidgetId,
    side: WidgetSide,
}

impl WidgetSpan {
    #[must_use]
    pub const fn new(visual: VisualOffset, widget: WidgetId, side: WidgetSide) -> Self {
        Self {
            visual,
            widget,
            side,
        }
    }

    #[must_use]
    pub const fn visual(self) -> VisualOffset {
        self.visual
    }

    #[must_use]
    pub const fn widget(self) -> WidgetId {
        self.widget
    }

    #[must_use]
    pub const fn side(self) -> WidgetSide {
        self.side
    }
}

/// 量一个 widget 时给它的约束。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WidgetConstraints {
    available_width: f32,
    line_height: f32,
}

impl WidgetConstraints {
    /// 量一个 widget 的约束。两个值都来自 [`LayoutConfig`]，所以布局之外
    /// 的调用方（比如判断「资源到位之后要不要重排」的缓存）也能造出与
    /// 布局里那一份**同样**的约束，不必自己猜。
    #[must_use]
    pub const fn new(available_width: f32, line_height: f32) -> Self {
        Self {
            available_width,
            line_height,
        }
    }

    /// 一整行的可用宽度。widget 拿到的是**整块**的宽度，不是当前行的剩余
    /// 宽度：剩余宽度取决于它排在哪，而「排在哪」要等它的宽度定了才知道。
    #[must_use]
    pub const fn available_width(self) -> f32 {
        self.available_width
    }

    /// 纯文本行的行高，给「跟文字一样高」这类 widget 用。
    #[must_use]
    pub const fn line_height(self) -> f32 {
        self.line_height
    }
}

/// 一个 widget 的尺寸与基线。
///
/// `baseline` 从盒子顶端往下量。它让 widget 与同一行的文字对齐——
/// 把它当成「从底往上」会让图片和文字错开一整行高，而且不报错。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WidgetMetrics {
    size: Size<yu_core::Block>,
    baseline: f32,
}

impl WidgetMetrics {
    /// 非有限或负的基线被拒绝；越过盒子底端也被拒绝。
    pub fn new(size: Size<yu_core::Block>, baseline: f32) -> Result<Self, LayoutError> {
        if !baseline.is_finite() || baseline < 0.0 || baseline > size.height() {
            return Err(LayoutError::InvalidWidgetBaseline);
        }
        Ok(Self { size, baseline })
    }

    /// 基线落在盒子底端，也就是「坐在文字基线上」。
    pub fn sitting_on_baseline(size: Size<yu_core::Block>) -> Result<Self, LayoutError> {
        let baseline = size.height();
        Self::new(size, baseline)
    }

    #[must_use]
    pub const fn size(self) -> Size<yu_core::Block> {
        self.size
    }

    #[must_use]
    pub const fn baseline(self) -> f32 {
        self.baseline
    }
}

/// 量一个 widget 的结果。
///
/// 不变量 D7：资源没就绪时返回 [`WidgetMeasurement::Placeholder`]，布局照常
/// 完成，不阻塞、不整帧失败。就绪之后由资源层发一次 Revision-bound 通知，
/// 触发受影响范围重排——那一步不在这一层，这一层只负责**能不能在没就绪的
/// 时候把画面排出来**，以及**告诉调用方哪些还没就绪**。
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum WidgetMeasurement {
    Ready(WidgetMetrics),
    Placeholder(WidgetMetrics),
}

impl WidgetMeasurement {
    #[must_use]
    pub const fn metrics(self) -> WidgetMetrics {
        match self {
            Self::Ready(metrics) | Self::Placeholder(metrics) => metrics,
        }
    }

    #[must_use]
    pub const fn is_ready(self) -> bool {
        matches!(self, Self::Ready(_))
    }
}

/// 把 [`WidgetId`] 翻译成尺寸。
///
/// 与 [`StyleTable`] 一样，实现住在产出装饰的那一层。未知 id 返回 `None`，
/// 布局报 [`LayoutError::UnknownWidget`]——「装饰产出了一个没人认识的
/// widget」应该响，不该悄悄画成一个零宽的空洞。
pub trait WidgetMeasure {
    fn measure(
        &self,
        widget: WidgetId,
        constraints: WidgetConstraints,
    ) -> Option<WidgetMeasurement>;
}

/// 一张空的 widget 表。给还没有 widget 产出的调用方与测试用。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct NoWidgets;

impl WidgetMeasure for NoWidgets {
    fn measure(
        &self,
        _widget: WidgetId,
        _constraints: WidgetConstraints,
    ) -> Option<WidgetMeasurement> {
        None
    }
}

/// 排好的一个 widget 盒。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WidgetBox {
    widget: WidgetId,
    visual: VisualOffset,
    side: WidgetSide,
    line: usize,
    bounds: LayoutRect,
    baseline: f32,
    ready: bool,
}

impl WidgetBox {
    #[must_use]
    pub const fn widget(self) -> WidgetId {
        self.widget
    }

    #[must_use]
    pub const fn visual(self) -> VisualOffset {
        self.visual
    }

    #[must_use]
    pub const fn side(self) -> WidgetSide {
        self.side
    }

    #[must_use]
    pub const fn line(self) -> usize {
        self.line
    }

    #[must_use]
    pub const fn bounds(self) -> LayoutRect {
        self.bounds
    }

    /// 盒子顶端到文字基线的距离。
    #[must_use]
    pub const fn baseline(self) -> f32 {
        self.baseline
    }

    /// 资源是否已就绪。`false` 表示这一格画的是 placeholder，资源到位之后
    /// 这个块要重排。
    #[must_use]
    pub const fn is_ready(self) -> bool {
        self.ready
    }
}

/// 视觉文本上的一段行级样式。
///
/// 「行级」指缩进、行高、背景、前缀装饰这类作用于整行的东西
/// （overview-v2 §5.1 的 `Decoration::Line`）。一条视觉行的样式由**行首**
/// 那个视觉偏移落在哪一段决定。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct LineSpan {
    visual: VisualRange,
    style: LineStyleId,
}

impl LineSpan {
    #[must_use]
    pub const fn new(visual: VisualRange, style: LineStyleId) -> Self {
        Self { visual, style }
    }

    #[must_use]
    pub const fn visual(self) -> VisualRange {
        self.visual
    }

    #[must_use]
    pub const fn style(self) -> LineStyleId {
        self.style
    }
}

/// 一个 [`LineStyleId`] 解释之后的行级属性。
///
/// 只有影响**几何**的两项在这里。背景色、前缀装饰的样子这些是画的事：
/// [`LineBox`] 会把它的 [`LineStyleId`] 原样带出去，由上层拿同一张表去画。
/// 让布局层认识「引用条是什么颜色」既没必要，也正是 E1 要挡的那种泄漏。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LineAttrs {
    indent: f32,
    line_height_scale: f32,
}

impl LineAttrs {
    /// 非有限或负的缩进、非正的行高倍率都被拒绝——一个 NaN 缩进会一路传播
    /// 成不 panic 的错画面。
    pub fn new(indent: f32, line_height_scale: f32) -> Result<Self, LayoutError> {
        if !indent.is_finite() || indent < 0.0 {
            return Err(LayoutError::InvalidLineStyle);
        }
        if !line_height_scale.is_finite() || line_height_scale <= 0.0 {
            return Err(LayoutError::InvalidLineStyle);
        }
        Ok(Self {
            indent,
            line_height_scale,
        })
    }

    /// 内容从行左边缘往右让开多少。
    #[must_use]
    pub const fn indent(self) -> f32 {
        self.indent
    }

    /// 相对 `LayoutConfig::line_height` 的倍率。
    #[must_use]
    pub const fn line_height_scale(self) -> f32 {
        self.line_height_scale
    }
}

impl Default for LineAttrs {
    fn default() -> Self {
        Self {
            indent: 0.0,
            line_height_scale: 1.0,
        }
    }
}

/// 把 [`LineStyleId`] 翻译成行级属性。与 [`StyleTable`] 同样住在产出装饰的
/// 那一层。未知 id 是错误，不是默认值。
pub trait LineStyleTable {
    fn attrs(&self, style: LineStyleId) -> Option<LineAttrs>;
}

/// 一张空的行样式表。没有行级装饰的调用方与测试用。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct NoLineStyles;

impl LineStyleTable for NoLineStyles {
    fn attrs(&self, _style: LineStyleId) -> Option<LineAttrs> {
        None
    }
}

/// 一个块的布局输入。
///
/// `runs` 必须**无缝铺满** `text`：从 0 开始、首尾相接、终点等于
/// `text.len()`。空文本对应空 run 列表。校验在 [`BlockLayout::build`] 里，
/// 不满足直接报错——一个漏掉半段文本的 run 列表会画出少了几个字的一行，
/// 而那既不 panic 也不报错。
#[derive(Clone, Copy, Debug)]
pub struct LayoutInput<'a> {
    text: &'a str,
    runs: &'a [StyledRun],
    widgets: &'a [WidgetSpan],
    lines: &'a [LineSpan],
}

impl<'a> LayoutInput<'a> {
    #[must_use]
    pub const fn new(text: &'a str, runs: &'a [StyledRun]) -> Self {
        Self {
            text,
            runs,
            widgets: &[],
            lines: &[],
        }
    }

    /// 挂上 widget。必须按视觉偏移升序，同一偏移上 `Before` 在 `After` 之前
    /// ——也就是 `DecorationRange::order_key` 的顺序（不变量 D6）。乱序会让
    /// 同一位置的两个 widget 画反，不报错。
    #[must_use]
    pub const fn with_widgets(mut self, widgets: &'a [WidgetSpan]) -> Self {
        self.widgets = widgets;
        self
    }

    #[must_use]
    pub const fn widgets(&self) -> &'a [WidgetSpan] {
        self.widgets
    }

    /// 挂上行级样式。必须按视觉偏移升序且互不重叠。
    #[must_use]
    pub const fn with_line_styles(mut self, lines: &'a [LineSpan]) -> Self {
        self.lines = lines;
        self
    }

    #[must_use]
    pub const fn line_styles(&self) -> &'a [LineSpan] {
        self.lines
    }

    #[must_use]
    pub const fn text(&self) -> &'a str {
        self.text
    }

    #[must_use]
    pub const fn runs(&self) -> &'a [StyledRun] {
        self.runs
    }
}

/// 视觉文本里的一个 grapheme 盒。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ClusterBox {
    visual: VisualRange,
    line: usize,
    x: f32,
    width: f32,
    style: StyleId,
    line_break: bool,
    /// UAX #9 的嵌入层级。偶数从左往右，奇数从右往左。
    level: u8,
}

impl ClusterBox {
    #[must_use]
    pub const fn visual(self) -> VisualRange {
        self.visual
    }

    #[must_use]
    pub const fn line(self) -> usize {
        self.line
    }

    #[must_use]
    pub const fn x(self) -> f32 {
        self.x
    }

    #[must_use]
    pub const fn width(self) -> f32 {
        self.width
    }

    #[must_use]
    pub const fn style(self) -> StyleId {
        self.style
    }

    /// 这个盒子是不是一个强制换行符。强制换行占视觉字节但不占宽度。
    #[must_use]
    pub const fn is_line_break(self) -> bool {
        self.line_break
    }

    /// UAX #9 的嵌入层级。偶数从左往右，奇数从右往左。
    #[must_use]
    pub const fn level(self) -> u8 {
        self.level
    }

    /// 这个 grapheme 是不是从右往左排的。
    ///
    /// 它决定「视觉左边缘」对应的是这段文字的开头还是结尾——搞反了不会
    /// panic，只会让光标停在字的另一头。
    #[must_use]
    pub const fn is_rtl(self) -> bool {
        self.level % 2 == 1
    }

    /// 文字前进方向上的起点（LTR 是左边缘，RTL 是右边缘）。
    const fn leading_x(self) -> f32 {
        if self.is_rtl() {
            self.x + self.width
        } else {
            self.x
        }
    }

    /// 文字前进方向上的终点。
    const fn trailing_x(self) -> f32 {
        if self.is_rtl() {
            self.x
        } else {
            self.x + self.width
        }
    }
}

/// 一个排好位置的字形。
///
/// 只在 shaping 那条路上产生（[`BlockLayout::build_shaped`]）；按度量排的
/// 布局没有字形，`glyphs()` 是空的。
///
/// `origin` 是字形的**基线左端**在 block 空间里的位置，字形偏移已经加进去
/// 了。它不摊开成 `x/y: f32`——不变量 E6。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GlyphBox {
    face: FontFaceId,
    glyph: GlyphId,
    visual: VisualRange,
    line: usize,
    origin: LayoutPoint,
    style: StyleId,
    /// 相对 shaper 基准字号的倍率，来自这个字形所在 [`StyleId`] 的
    /// [`TextAttrs::size_scale`]。栅格化要按它选字号。
    size_scale: f32,
}

impl GlyphBox {
    #[must_use]
    pub const fn face(self) -> FontFaceId {
        self.face
    }

    #[must_use]
    pub const fn glyph(self) -> GlyphId {
        self.glyph
    }

    /// 这个字形覆盖的视觉字节。连字覆盖多个 grapheme，组合记号可能为空。
    #[must_use]
    pub const fn visual(self) -> VisualRange {
        self.visual
    }

    #[must_use]
    pub const fn line(self) -> usize {
        self.line
    }

    #[must_use]
    pub const fn origin(self) -> LayoutPoint {
        self.origin
    }

    #[must_use]
    pub const fn style(self) -> StyleId {
        self.style
    }

    #[must_use]
    pub const fn size_scale(self) -> f32 {
        self.size_scale
    }
}

/// 一条视觉行。
#[derive(Clone, Debug, PartialEq)]
pub struct LineBox {
    index: usize,
    /// 内容起点 x，也就是这一行的行级缩进。空行上 caret 与 hit-test 只能
    /// 问它——问不到就把光标停在块的左边缘。
    indent: f32,
    visual: VisualRange,
    /// 行盒在 block 局部坐标里的矩形。`x` 现在恒为 0（左对齐），
    /// 它是一个 [`LayoutRect`] 而不是散装的三个 `f32`，因为不变量 E6 要求
    /// 视觉坐标只有一套实现、空间进类型。
    bounds: LayoutRect,
    baseline: f32,
    style: Option<LineStyleId>,
    clusters: Range<usize>,
    widgets: Range<usize>,
}

impl LineBox {
    #[must_use]
    pub const fn index(&self) -> usize {
        self.index
    }

    #[must_use]
    pub const fn visual(&self) -> VisualRange {
        self.visual
    }

    #[must_use]
    pub const fn bounds(&self) -> LayoutRect {
        self.bounds
    }

    /// 内容起点 x（行级缩进）。行盒本身仍然从 0 开始——缩进吃掉的是可用
    /// 宽度，不是把整行推走。
    #[must_use]
    pub const fn indent(&self) -> f32 {
        self.indent
    }

    #[must_use]
    pub const fn y(&self) -> f32 {
        self.bounds.y()
    }

    #[must_use]
    pub const fn width(&self) -> f32 {
        self.bounds.width()
    }

    /// 行高。纯文本行等于 `LayoutConfig::line_height`；有 widget 时按基线
    /// 对齐撑高。
    #[must_use]
    pub const fn height(&self) -> f32 {
        self.bounds.height()
    }

    /// 行顶到文字基线的距离。
    #[must_use]
    pub const fn baseline(&self) -> f32 {
        self.baseline
    }

    /// 这条行用的行级样式。`None` 表示没有行级装饰盖到行首。
    ///
    /// 背景与前缀装饰的**几何**由 [`LineBox::bounds`] 给，**长什么样**由上层
    /// 拿这个 id 去查同一张表。布局层不解释它。
    #[must_use]
    pub const fn style(&self) -> Option<LineStyleId> {
        self.style
    }

    #[must_use]
    pub fn cluster_range(&self) -> Range<usize> {
        self.clusters.clone()
    }

    #[must_use]
    pub fn widget_range(&self) -> Range<usize> {
        self.widgets.clone()
    }
}

/// 一个视觉偏移落在布局里的位置。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CaretBox {
    visual: VisualOffset,
    line: usize,
    point: LayoutPoint,
    widget_affinity: Option<CaretAffinity>,
    line_affinity: CaretAffinity,
}

impl CaretBox {
    #[must_use]
    pub const fn visual(self) -> VisualOffset {
        self.visual
    }

    #[must_use]
    pub const fn line(self) -> usize {
        self.line
    }

    #[must_use]
    pub const fn point(self) -> LayoutPoint {
        self.point
    }

    /// 这个位置落在 widget 的哪一侧，没有 widget 参与就是 `None`。
    ///
    /// widget **有宽度**，所以同一个视觉偏移在它两侧是两个不同的 x：
    /// 「图片前面」与「图片后面」。行内其余位置的两侧是同一个 x——被隐藏的
    /// 语法在字节流里塌成一个点——落在哪一侧由行的规则（不变量 H5）决定，
    /// 与这里无关。
    ///
    /// [`BlockLayout::hit`] 因此要把它带出来：点在图片右边时选的是右沿，
    /// 而调用方光看视觉偏移分不出是哪一沿，会把光标画回图片左边。
    #[must_use]
    pub const fn widget_affinity(self) -> Option<CaretAffinity> {
        self.widget_affinity
    }

    /// 让 [`BlockLayout::caret`] 落回**这一行**的那个 affinity。
    ///
    /// 软换行边界上的一个视觉偏移在两行各有一个位置，`caret` 靠 affinity
    /// 选。[`BlockLayout::hit`] 已经决定了是哪一行，把这个值带出来，调用方
    /// 才不用再猜一遍——猜错就是「点第二行，光标画在第一行末尾」。
    #[must_use]
    pub const fn line_affinity(self) -> CaretAffinity {
        self.line_affinity
    }
}

/// 一个块布局好的样子。只有视觉坐标，没有源码坐标。
#[derive(Clone, Debug, PartialEq)]
pub struct BlockLayout {
    config: LayoutConfig,
    visual_len: VisualOffset,
    lines: Vec<LineBox>,
    clusters: Vec<ClusterBox>,
    widgets: Vec<WidgetBox>,
    /// 只有 shaping 那条路会填。按度量排的布局没有字形。
    glyphs: Vec<GlyphBox>,
    /// 行级样式按视觉区间排好的解释结果，行首落在哪一段就用哪一段。
    line_attrs: Vec<(VisualRange, LineStyleId, LineAttrs)>,
    /// 后端排不出来、换成了替代字形的簇数。见 [`BlockLayout::substituted_clusters`]。
    substituted: u32,
}

/// 重排时行内待摆放的一样东西。
#[derive(Clone, Copy, Debug)]
enum Item {
    Cluster(usize),
    Widget(usize),
}

struct LineCursor {
    index: usize,
    visual_start: VisualOffset,
    /// 当前行内容右边缘的绝对 x（含缩进）。
    width: f32,
    /// 内容起始 x。行级样式的缩进。
    indent: f32,
    /// 本行的文字行高（`config.line_height` × 行级倍率）。
    line_height: f32,
    style: Option<LineStyleId>,
    cluster_start: usize,
    widget_start: usize,
}

impl LineCursor {
    /// 这一行有没有已经放下的内容。判「排不下要断行」时必须用它而不是
    /// `width > 0`：缩进不是内容，拿它当内容会让每一行开头就断一次。
    const fn has_content(&self) -> bool {
        self.width > self.indent
    }
}

/// 一个字形在它自己的簇里的样子。位置要等断行与重排定了才知道。
#[derive(Clone, Copy, Debug)]
struct GlyphRecord {
    face: FontFaceId,
    glyph: GlyphId,
    x_offset: f32,
    y_offset: f32,
    size_scale: f32,
}

/// 第一遍度量出来的一个 grapheme，还没有被分配到行。
///
/// 度量与断行分成两遍，因为 UAX #14 的断行机会要在整段视觉文本上算，
/// 而它不认识 [`StyledRun`] 的边界。第一遍走 run 拿宽度，第二遍走断行机会
/// 分行。
struct Measured {
    visual: VisualRange,
    style: StyleId,
    advance: f32,
    /// shaping 那条路上这个簇画出来是哪个字形。按度量排时是 `None`。
    glyph: Option<GlyphRecord>,
    /// 这个 grapheme 本身就是一个强制换行（UAX #14 的 BK / CR / LF / NL）。
    mandatory_break: bool,
    /// 全是空白。行尾的空白不参与「排不下」的判断，它们悬在行外。
    space: bool,
    /// UAX #9 的嵌入层级，取这个 grapheme 第一个字节的层级。
    level: u8,
}

/// UAX #14 里构成强制换行的字符。
///
/// 注意这与 `docs/specs/coordinates.md` 的「源码逻辑行只按 LF 分隔」不冲突：
/// 那条说的是 `LineIndex`（源码行），这里说的是**视觉行**。软换行早就让两者
/// 不是一回事了；LS / PS / FF / NEL 只是又多了几个视觉行会断而源码行不断的
/// 位置。
fn is_mandatory_break_char(value: char) -> bool {
    matches!(
        value,
        '\n' | '\r' | '\u{b}' | '\u{c}' | '\u{85}' | '\u{2028}' | '\u{2029}'
    )
}

impl BlockLayout {
    /// 按 [`LayoutConfig`] 与调用方的度量把输入排成视觉行。
    pub fn build<T: StyleTable, M: ClusterMetrics>(
        input: LayoutInput<'_>,
        config: LayoutConfig,
        styles: &T,
        metrics: &M,
    ) -> Result<Self, LayoutError> {
        Self::build_with_widgets(input, config, styles, &NoWidgets, metrics)
    }

    /// 带 widget 的完整版本。
    pub fn build_with_widgets<T: StyleTable, W: WidgetMeasure, M: ClusterMetrics>(
        input: LayoutInput<'_>,
        config: LayoutConfig,
        styles: &T,
        widgets: &W,
        metrics: &M,
    ) -> Result<Self, LayoutError> {
        Self::build_all(input, config, styles, widgets, &NoLineStyles, metrics)
    }

    /// 带 widget 与行级样式的完整版本。
    pub fn build_all<T, W, L, M>(
        input: LayoutInput<'_>,
        config: LayoutConfig,
        styles: &T,
        widgets: &W,
        line_styles: &L,
        metrics: &M,
    ) -> Result<Self, LayoutError>
    where
        T: StyleTable,
        W: WidgetMeasure,
        L: LineStyleTable,
        M: ClusterMetrics,
    {
        let (visual_len, bidi) = Self::prepare(input, config)?;
        let measured = measure(input, styles, metrics, &bidi)?;
        Self::assemble(
            input,
            config,
            widgets,
            line_styles,
            &bidi,
            visual_len,
            measured,
        )
    }

    /// 走 shaping 后端的完整版本。断行、重排、widget、行级样式与
    /// [`BlockLayout::build_all`] 完全一致，只有「一个簇有多宽」换成了
    /// 「shaper 给出的字形推进量」，并且多出 [`BlockLayout::glyphs`]。
    ///
    /// # shaping 坐标空间
    ///
    /// 每个 [`StyledRun`] 单独交给 shaper。`ShapingProvider` 的 range 参数是
    /// 一个**零基的局部空间**（`0..text.len()`），不是源码坐标——布局层
    /// 手里根本没有源码坐标（见本模块开头「坐标」一节）。返回的字形区间按
    /// 这个局部空间读，再加上 run 的视觉起点变成视觉区间。
    pub fn build_shaped<T, W, L, S>(
        input: LayoutInput<'_>,
        config: LayoutConfig,
        styles: &T,
        widgets: &W,
        line_styles: &L,
        shaper: &S,
    ) -> Result<Self, LayoutError>
    where
        T: StyleTable,
        W: WidgetMeasure,
        L: LineStyleTable,
        S: ShapingProvider,
    {
        let (visual_len, bidi) = Self::prepare(input, config)?;
        let (measured, substituted) = measure_shaped(input, styles, shaper, &bidi)?;
        let mut layout = Self::assemble(
            input,
            config,
            widgets,
            line_styles,
            &bidi,
            visual_len,
            measured,
        )?;
        layout.substituted = substituted;
        Ok(layout)
    }

    fn prepare<'a>(
        input: LayoutInput<'a>,
        config: LayoutConfig,
    ) -> Result<(VisualOffset, BidiInfo<'a>), LayoutError> {
        config.validate()?;
        let visual_len =
            VisualOffset::try_from(input.text.len()).map_err(|_| LayoutError::OffsetOverflow)?;
        validate_runs(input, visual_len)?;
        validate_widgets(input, visual_len)?;
        Ok((
            visual_len,
            BidiInfo::new(input.text(), base_level(config.base_direction())),
        ))
    }

    /// 断行、摆 widget、重排、放字形。两条度量路径共用这一段——共用代码
    /// 路径的差分是自证的，所以它只写一遍。
    fn assemble<W, L>(
        input: LayoutInput<'_>,
        config: LayoutConfig,
        widgets: &W,
        line_styles: &L,
        bidi: &BidiInfo<'_>,
        visual_len: VisualOffset,
        measured: Vec<Measured>,
    ) -> Result<Self, LayoutError>
    where
        W: WidgetMeasure,
        L: LineStyleTable,
    {
        let segment_starts = segment_starts(input.text, &measured)?;
        let sizes = measure_widgets(input, config, widgets)?;
        let line_attrs = resolve_line_styles(input, line_styles)?;

        let mut layout = Self {
            config,
            visual_len,
            lines: Vec::new(),
            clusters: Vec::new(),
            widgets: Vec::new(),
            glyphs: Vec::new(),
            line_attrs,
            substituted: 0,
        };
        let mut cursor = layout.start_line(0, VisualOffset::ZERO, 0, 0);
        let mut last_was_break = false;
        let mut next_widget = 0_usize;

        for segment in segment_starts.windows(2) {
            let (from, to) = (segment[0], segment[1]);
            // 这一段里到最后一个非空白 grapheme 为止的宽度。行尾空白不参与
            // 判断——UAX #14 的断行机会在空白**之后**，让空白把整个词挤到
            // 下一行是错的。
            let fit = measured[from..to]
                .iter()
                .rev()
                .skip_while(|cluster| cluster.space || cluster.mandatory_break)
                .map(|cluster| cluster.advance)
                .sum::<f32>();
            if cursor.has_content() && cursor.width + fit > config.max_width() {
                layout.push_line(&cursor, measured[from].visual.start())?;
                cursor = layout.next_line(&cursor, measured[from].visual.start());
            }

            for cluster in &measured[from..to] {
                layout.place_widgets_at(
                    input.widgets(),
                    &sizes,
                    &mut next_widget,
                    cluster.visual.start(),
                    &mut cursor,
                )?;
                if cluster.mandatory_break {
                    layout.clusters.push(ClusterBox {
                        visual: cluster.visual,
                        line: cursor.index,
                        x: cursor.width,
                        width: 0.0,
                        style: cluster.style,
                        line_break: true,
                        level: cluster.level,
                    });
                    layout.push_line(&cursor, cluster.visual.end())?;
                    cursor = layout.next_line(&cursor, cluster.visual.end());
                    last_was_break = true;
                    continue;
                }
                // 一个自身就超过整行宽度的「词」必须还能排出来：段内退回
                // 按 grapheme 断（UAX #14 允许的应急断行）。
                if !cluster.space
                    && cursor.has_content()
                    && cursor.width + cluster.advance > config.max_width()
                {
                    layout.push_line(&cursor, cluster.visual.start())?;
                    cursor = layout.next_line(&cursor, cluster.visual.start());
                }
                layout.clusters.push(ClusterBox {
                    visual: cluster.visual,
                    line: cursor.index,
                    x: cursor.width,
                    width: cluster.advance,
                    style: cluster.style,
                    line_break: false,
                    level: cluster.level,
                });
                cursor.width += cluster.advance;
                last_was_break = false;
            }
        }

        layout.place_widgets_at(
            input.widgets(),
            &sizes,
            &mut next_widget,
            visual_len,
            &mut cursor,
        )?;
        // 每一个 widget 都必须落地。锚点只在**簇的起点**与视觉末尾被查看，
        // 锚在别处（比如一个 grapheme 内部）的那个会被 `place_widgets_at`
        // 的 `while` 静静跳过，连同排在它后面的每一个 widget 一起——症状是
        // 画面上少了一张图，不 panic、不报错。
        if next_widget != input.widgets().len() {
            return Err(LayoutError::WidgetNotAnchored);
        }

        if layout.lines.is_empty() || !last_was_break {
            layout.push_line(&cursor, visual_len)?;
        } else {
            let empty = cursor.visual_start;
            layout.push_line(
                &LineCursor {
                    width: cursor.indent,
                    ..cursor
                },
                empty,
            )?;
        }
        layout.reorder_for_bidi(input.text(), bidi)?;
        layout.place_glyphs(&measured)?;
        Ok(layout)
    }

    /// 字形跟着它的簇走。位置在重排**之后**才算——重排改的就是簇的 x，
    /// 在那之前算等于把 RTL 的字形画在逻辑位置上，不报错，只是画反。
    fn place_glyphs(&mut self, measured: &[Measured]) -> Result<(), LayoutError> {
        if measured.iter().all(|cluster| cluster.glyph.is_none()) {
            return Ok(());
        }
        // 一个 Measured 恰好推出一个 ClusterBox，同序。
        if measured.len() != self.clusters.len() {
            return Err(LayoutError::Shaping(
                "shaped cluster count does not match the laid out clusters".into(),
            ));
        }
        let mut glyphs = Vec::new();
        for (cluster, record) in self.clusters.iter().zip(measured) {
            let Some(record) = record.glyph else {
                continue;
            };
            let line = self
                .lines
                .get(cluster.line)
                .ok_or(LayoutError::OffsetOverflow)?;
            // 字形的绘制原点是推进盒的**左**边缘，与文字方向无关：RTL 的
            // 字形也从左边缘往右画，方向只决定盒子按什么顺序摆。
            let origin = LayoutPoint::new(
                cluster.x + record.x_offset,
                line.bounds.y() + line.baseline + record.y_offset,
            );
            if !origin.is_finite() {
                return Err(LayoutError::InvalidPoint);
            }
            glyphs.push(GlyphBox {
                face: record.face,
                glyph: record.glyph,
                visual: cluster.visual,
                line: cluster.line,
                origin,
                style: cluster.style,
                size_scale: record.size_scale,
            });
        }
        self.glyphs = glyphs;
        Ok(())
    }

    #[must_use]
    pub const fn config(&self) -> LayoutConfig {
        self.config
    }

    #[must_use]
    pub const fn visual_len(&self) -> VisualOffset {
        self.visual_len
    }

    #[must_use]
    pub fn lines(&self) -> &[LineBox] {
        &self.lines
    }

    #[must_use]
    pub fn clusters(&self) -> &[ClusterBox] {
        &self.clusters
    }

    /// 排好的 widget 盒，与输入的 [`WidgetSpan`] 一一对应、同序。
    #[must_use]
    pub fn widgets(&self) -> &[WidgetBox] {
        &self.widgets
    }

    /// 排好位置的字形，按逻辑顺序。按度量排的布局这里是空的。
    #[must_use]
    pub fn glyphs(&self) -> &[GlyphBox] {
        &self.glyphs
    }

    /// 这个块里有多少个簇是后端排不出来、换成了替代字形的。
    ///
    /// **降级必须看得见。** 「排不出来就画替代字形」如果不留痕迹，一整屏
    /// 豆腐块与一整屏正常文字在任何断言下都一样——那正是这个项目最危险的
    /// 失败模式。判据落在这个数上：正常语料必须是 0，降级语料必须正好等于
    /// 排不出来的簇数。
    #[must_use]
    pub const fn substituted_clusters(&self) -> u32 {
        self.substituted
    }

    /// 还画着 placeholder 的 widget。
    ///
    /// 不变量 D7 要求资源就绪后触发受影响范围重排。发通知不是这一层的事，
    /// 但**哪些没就绪**只有排完才知道，所以由这里报出来。空表示这一块的
    /// 几何已经是最终的。
    #[must_use]
    pub fn pending_widgets(&self) -> Vec<WidgetId> {
        self.widgets
            .iter()
            .filter(|widget| !widget.ready)
            .map(|widget| widget.widget)
            .collect()
    }

    /// 块的高度。逐行累加——有 widget 的行比 `line_height` 高。
    #[must_use]
    pub fn height(&self) -> f32 {
        self.lines
            .last()
            .map_or(0.0, |line| line.bounds.y() + line.bounds.height())
    }

    /// 视觉偏移落在哪里。
    ///
    /// `affinity` 只在偏移正好落在软换行边界上时起作用：
    /// [`CaretAffinity::Upstream`] 给上一行的行末，
    /// [`CaretAffinity::Downstream`] 给下一行的行首（见
    /// `docs/specs/coordinates.md`）。
    ///
    /// 落在一个 grapheme **内部**的偏移不是合法的 caret 位置，返回该 grapheme
    /// 前进方向上的起点。
    ///
    /// # 方向变化处只给一个位置
    ///
    /// 方向变化处的一个视觉偏移在几何上对应**两个**位置：前一段的后沿与
    /// 后一段的前沿。这里按 UAX #9 §3.4 的做法取**层级更低**（更接近段落
    /// 基准方向）的那一侧。于是边界两侧的两个几何位置分别归属两个不同的
    /// 偏移，都够得着；代价是同一个偏移的另一个位置画不出来。要两个都给，
    /// caret 得再带一个方向参数，那是独立的一件事，见 overview-v2 §8 S5 的
    /// 登记。
    pub fn caret(
        &self,
        visual: VisualOffset,
        affinity: CaretAffinity,
    ) -> Result<CaretBox, LayoutError> {
        if visual > self.visual_len {
            return Err(LayoutError::VisualOutOfBounds(visual));
        }
        let line_index = self.line_for_visual(visual, affinity);
        let line = &self.lines[line_index];
        // 行内没有簇时循环不跑，x 落在行首。
        let mut before = None;
        let mut after = None;
        let mut inside = None;
        for index in line.clusters.clone() {
            let cluster = self.clusters[index];
            if cluster.visual.end() == visual {
                before = Some(cluster);
            }
            // 换行符不提供「它之前」那个位置：那个位置由行内最后一个文字
            // grapheme 的后沿给出，与它自己的方向无关。
            if cluster.visual.start() == visual && !cluster.line_break {
                after = Some(cluster);
            }
            if cluster.visual.start() < visual && visual < cluster.visual.end() {
                inside = Some(cluster);
            }
        }
        // widget 夹在 `before` 与 `after` 之间，两侧的 x 差着它的宽度。
        // 落在哪一侧只有 affinity 说得出来。
        if let Some((left, right)) = self.widget_edges(line, visual) {
            let x = match affinity {
                CaretAffinity::Upstream => left,
                CaretAffinity::Downstream => right,
            };
            return Ok(CaretBox {
                visual,
                line: line_index,
                point: LayoutPoint::new(x, line.bounds.y()),
                widget_affinity: Some(affinity),
                line_affinity: affinity,
            });
        }
        let x = match (before, after, inside) {
            (None, None, None) => self.content_start(line),
            _ => caret_x(before, after, inside),
        };
        Ok(CaretBox {
            visual,
            line: line_index,
            point: LayoutPoint::new(x, line.bounds.y()),
            widget_affinity: None,
            line_affinity: affinity,
        })
    }

    /// 锚在 `visual` 上的那些 widget 的左右外沿。一个都没有就是 `None`。
    ///
    /// 同一个偏移上可以摞着好几个 widget（`![a](x)![b](y)` 两段都塌在这
    /// 一点上）。取最外面那两条边：中间的缝隙不对应任何一个源码位置。
    fn widget_edges(&self, line: &LineBox, visual: VisualOffset) -> Option<(f32, f32)> {
        let mut edges: Option<(f32, f32)> = None;
        for index in line.widgets.clone() {
            let placed = self.widgets[index];
            if placed.visual != visual {
                continue;
            }
            let (left, right) = (placed.bounds.x(), placed.bounds.x() + placed.bounds.width());
            edges = Some(match edges {
                Some((known_left, known_right)) => (known_left.min(left), known_right.max(right)),
                None => (left, right),
            });
        }
        edges
    }

    /// 一行内容的起点 x，也就是它的行级缩进。
    ///
    /// 空行上没有簇可问，caret 与 hit-test 只能问它。少了这一条，缩进块里
    /// 的空行会把光标停在块的左边缘。
    const fn content_start(&self, line: &LineBox) -> f32 {
        line.indent
    }

    /// 一个 block 局部坐标点落在哪个视觉偏移上。
    ///
    /// 取离该点最近的一个**caret 位置**，而不是最近的一条 grapheme 边缘。
    /// 两者在 LTR 下一样，在重排过的行里不一样：方向变化处两条边缘落在同一个
    /// x 上，而其中只有一条是 [`BlockLayout::caret`] 画得出来的。取边缘会让
    /// 「点一下，光标跳到别处」——不 panic、不报错，只是不听话。
    pub fn hit(&self, point: LayoutPoint) -> Result<CaretBox, LayoutError> {
        let line_index = self.line_for_y(point.y());
        let line = &self.lines[line_index];
        let target = point.x().clamp(0.0, line.bounds.width());
        let mut best: Option<(f32, CaretPosition)> = None;
        for position in self.caret_positions(line) {
            let distance = (position.x - target).abs();
            // 打平时取更靠右那个，与 v1 的「过了中点算下一个」一致。
            let better = match best {
                None => true,
                Some((best_distance, best_position)) => {
                    distance < best_distance
                        || (distance == best_distance && position.x > best_position.x)
                }
            };
            if better {
                best = Some((distance, position));
            }
        }
        let position = best.map_or_else(
            || CaretPosition {
                x: self.content_start(line),
                visual: line.visual.start(),
                widget_affinity: None,
            },
            |(_, position)| position,
        );
        // 让 `caret` 落回**这一行**的那个 affinity。软换行边界上的偏移在
        // 两行各有一个位置，而 `hit` 已经按 y 决定了是哪一行。
        let line_affinity =
            if self.line_for_visual(position.visual, CaretAffinity::Upstream) == line_index {
                CaretAffinity::Upstream
            } else {
                CaretAffinity::Downstream
            };
        Ok(CaretBox {
            visual: position.visual,
            line: line_index,
            point: LayoutPoint::new(position.x, line.bounds.y()),
            widget_affinity: position.widget_affinity,
            line_affinity,
        })
    }

    /// 一行里所有 caret 位置，按逻辑顺序。
    ///
    /// 与 [`BlockLayout::caret`] 用同一条规则（[`caret_x`]），所以
    /// `caret(hit(p).visual())` 一定回到 `hit(p)` 的位置。两处各写一遍规则
    /// 就会在方向变化处对不上。
    fn caret_positions(&self, line: &LineBox) -> Vec<CaretPosition> {
        let mut positions = Vec::new();
        let mut before: Option<ClusterBox> = None;
        for index in line.clusters.clone() {
            let cluster = self.clusters[index];
            if !cluster.line_break {
                let matching = before.filter(|prev| prev.visual.end() == cluster.visual.start());
                positions.push(CaretPosition {
                    x: caret_x(matching, Some(cluster), None),
                    visual: cluster.visual.start(),
                    widget_affinity: None,
                });
            }
            before = Some(cluster);
        }
        // 行末那个位置由最后一个**文字** grapheme 的后沿给出：换行符的
        // 后沿属于下一行的行首。
        if let Some(last) = line
            .clusters
            .clone()
            .map(|index| self.clusters[index])
            .rfind(|cluster| !cluster.line_break)
        {
            positions.push(CaretPosition {
                x: caret_x(Some(last), None, None),
                visual: last.visual.end(),
                widget_affinity: None,
            });
        }
        // widget 的两沿。一行上只有一个 widget、周围一个簇都没有时（整段就
        // 是一张图），上面两个循环一个位置都产不出来——点在图片右边会一路
        // 退到行首，光标跳到图片左边去。
        let mut anchors: Vec<VisualOffset> = line
            .widgets
            .clone()
            .map(|index| self.widgets[index].visual)
            .collect();
        anchors.dedup();
        for anchor in anchors {
            let Some((left, right)) = self.widget_edges(line, anchor) else {
                continue;
            };
            positions.push(CaretPosition {
                x: left,
                visual: anchor,
                widget_affinity: Some(CaretAffinity::Upstream),
            });
            positions.push(CaretPosition {
                x: right,
                visual: anchor,
                widget_affinity: Some(CaretAffinity::Downstream),
            });
        }
        positions
    }

    /// UAX #9 的 L1/L2：按行把 level run 重排成视觉顺序，重新分配 x。
    ///
    /// 断行本身在**逻辑**顺序上做（UAX #14 就是这么定义的），重排在断行之后
    /// 逐行做。簇仍然按逻辑顺序存放，只有 `x` 变了——视觉区间保持升序，
    /// 上层按偏移查找的代码因此不需要知道有没有发生重排。
    fn reorder_for_bidi(&mut self, text: &str, bidi: &BidiInfo<'_>) -> Result<(), LayoutError> {
        if !bidi.has_rtl() {
            return Ok(());
        }
        for line_index in 0..self.lines.len() {
            let line = self.lines[line_index].clone();
            if line.clusters.is_empty() && line.widgets.is_empty() {
                continue;
            }
            let from = usize::try_from(line.visual.start().get())
                .map_err(|_| LayoutError::OffsetOverflow)?;
            let to = usize::try_from(line.visual.end().get())
                .map_err(|_| LayoutError::OffsetOverflow)?;
            if from >= to || from >= text.len() {
                continue;
            }
            let paragraph = bidi
                .paragraphs
                .iter()
                .find(|paragraph| paragraph.range.contains(&from))
                .ok_or(LayoutError::OffsetOverflow)?;
            let end = to.min(paragraph.range.end);
            let (levels, runs) = bidi.visual_runs(paragraph, from..end);

            let mut x = 0.0_f32;
            for run in runs {
                let rtl = levels[run.start].is_rtl();
                let mut items = self.items_in(&line, |offset| run.contains(&offset));
                if rtl {
                    items.reverse();
                }
                x = self.lay_out(&items, x)?;
            }
            // 段落之外的东西（行尾的强制换行符、锚在行末的 widget）留在行末。
            let tail = self.items_in(&line, |offset| offset >= end);
            self.lay_out(&tail, x)?;
        }
        Ok(())
    }

    /// 一行里锚点满足 `keep` 的簇与 widget，按**逻辑**顺序。
    ///
    /// 同一个锚点上 widget 排在簇前面——widget 的视觉区间是空的，它插在
    /// 前一个 grapheme 与后一个之间。
    fn items_in(&self, line: &LineBox, keep: impl Fn(usize) -> bool) -> Vec<Item> {
        let mut items: Vec<(u64, u8, Item)> = Vec::new();
        for index in line.widgets.clone() {
            let offset = self.widgets[index].visual.get();
            if keep(offset as usize) {
                items.push((offset, 0, Item::Widget(index)));
            }
        }
        for index in line.clusters.clone() {
            let offset = self.clusters[index].visual.start().get();
            if keep(offset as usize) {
                items.push((offset, 1, Item::Cluster(index)));
            }
        }
        items.sort_by_key(|(offset, rank, _)| (*offset, *rank));
        items.into_iter().map(|(_, _, item)| item).collect()
    }

    /// 从 `x` 起依次摆下这些东西，返回摆完之后的 x。
    fn lay_out(&mut self, items: &[Item], mut x: f32) -> Result<f32, LayoutError> {
        for item in items {
            match *item {
                Item::Cluster(index) => {
                    self.clusters[index].x = x;
                    x += self.clusters[index].width;
                }
                Item::Widget(index) => {
                    let bounds = self.widgets[index].bounds;
                    self.widgets[index].bounds =
                        LayoutRect::new(x, bounds.y(), bounds.width(), bounds.height())?;
                    x += bounds.width();
                }
            }
        }
        Ok(x)
    }

    /// 放下锚在 `visual` 上的所有 widget。
    ///
    /// widget 排不进当前行时先断行——它像 UAX #14 的 CB（contingent break）：
    /// 前后都允许断，但自身不可分割。
    fn place_widgets_at(
        &mut self,
        spans: &[WidgetSpan],
        sizes: &[WidgetMeasurement],
        next: &mut usize,
        visual: VisualOffset,
        cursor: &mut LineCursor,
    ) -> Result<(), LayoutError> {
        while *next < spans.len() && spans[*next].visual == visual {
            let span = spans[*next];
            let measurement = sizes[*next];
            let metrics = measurement.metrics();
            let width = metrics.size().width();
            if cursor.has_content() && cursor.width + width > self.config.max_width() {
                self.push_line(cursor, visual)?;
                *cursor = self.next_line(cursor, visual);
            }
            // y 要等整行的基线定下来才知道，先记 0，在 push_line 里修正。
            let bounds = LayoutRect::new(cursor.width, 0.0, width, metrics.size().height())?;
            self.widgets.push(WidgetBox {
                widget: span.widget,
                visual,
                side: span.side,
                line: cursor.index,
                bounds,
                baseline: metrics.baseline(),
                ready: measurement.is_ready(),
            });
            cursor.width += width;
            *next += 1;
        }
        Ok(())
    }

    fn next_line(&self, cursor: &LineCursor, visual_start: VisualOffset) -> LineCursor {
        self.start_line(
            cursor.index.saturating_add(1),
            visual_start,
            self.clusters.len(),
            self.widgets.len(),
        )
    }

    /// 开一条新行，按**行首**的视觉偏移定它的行级样式。
    ///
    /// 文末那个空行（视觉文本以强制换行结尾时会有一条）的行首正好落在最后
    /// 一段的**右端点**上，半开区间接不住它。它属于最后那一段：接不住会让
    /// 一个缩进块的最后一行退回零缩进，光标停在块的左边缘——不报错，只是
    /// 停错地方。
    fn start_line(
        &self,
        index: usize,
        visual_start: VisualOffset,
        cluster_start: usize,
        widget_start: usize,
    ) -> LineCursor {
        let (style, attrs) = self
            .line_attrs
            .iter()
            .find(|(range, _, _)| {
                range.start() <= visual_start
                    && (visual_start < range.end() || range.start() == range.end())
            })
            .or_else(|| {
                // 只有文末那个空行走这一条。块中间的一条行首落在某段右端点
                // 上的行归下一段管，`find` 已经把它接走了。
                (visual_start == self.visual_len)
                    .then(|| {
                        self.line_attrs
                            .last()
                            .filter(|(range, _, _)| range.end() == visual_start)
                    })
                    .flatten()
            })
            .map_or((None, LineAttrs::default()), |(_, style, attrs)| {
                (Some(*style), *attrs)
            });
        LineCursor {
            index,
            visual_start,
            width: attrs.indent(),
            indent: attrs.indent(),
            line_height: self.config.line_height() * attrs.line_height_scale(),
            style,
            cluster_start,
            widget_start,
        }
    }

    fn push_line(
        &mut self,
        cursor: &LineCursor,
        visual_end: VisualOffset,
    ) -> Result<(), LayoutError> {
        let visual =
            VisualRange::new(cursor.visual_start, visual_end).ok_or(LayoutError::OffsetOverflow)?;
        // y 逐行累加而不是 `index * line_height`：有 widget 的行会更高。
        let y = self
            .lines
            .last()
            .map_or(0.0, |line| line.bounds.y() + line.bounds.height());
        // 基线对齐：文字的基线在行底（ascent = line_height、descent = 0），
        // widget 按自己的 baseline 往下挂。谁要求的基线更深，行就往下长。
        let mut baseline = cursor.line_height;
        let mut descent = 0.0_f32;
        for widget in &self.widgets[cursor.widget_start..] {
            baseline = baseline.max(widget.baseline);
            descent = descent.max(widget.bounds.height() - widget.baseline);
        }
        let height = baseline + descent;
        if !y.is_finite() || !height.is_finite() {
            return Err(LayoutError::InvalidPoint);
        }
        let widget_start = cursor.widget_start;
        for index in widget_start..self.widgets.len() {
            let widget = self.widgets[index];
            self.widgets[index].bounds = LayoutRect::new(
                widget.bounds.x(),
                y + baseline - widget.baseline,
                widget.bounds.width(),
                widget.bounds.height(),
            )?;
        }
        self.lines.push(LineBox {
            index: cursor.index,
            indent: cursor.indent,
            visual,
            bounds: LayoutRect::new(0.0, y, cursor.width, height)?,
            baseline,
            style: cursor.style,
            clusters: cursor.cluster_start..self.clusters.len(),
            widgets: widget_start..self.widgets.len(),
        });
        Ok(())
    }

    fn line_for_y(&self, y: f32) -> usize {
        // 行高不再一定相同（widget 会撑高），所以按行的 y 区间找而不是除。
        for (index, line) in self.lines.iter().enumerate() {
            if y < line.bounds.y() + line.bounds.height() {
                return index;
            }
        }
        self.lines.len().saturating_sub(1)
    }

    fn line_for_visual(&self, visual: VisualOffset, affinity: CaretAffinity) -> usize {
        for (index, line) in self.lines.iter().enumerate() {
            if visual < line.visual.end()
                || (visual == line.visual.end()
                    && (affinity == CaretAffinity::Upstream || index + 1 == self.lines.len()))
            {
                return index;
            }
        }
        self.lines.len().saturating_sub(1)
    }
}

/// 第一遍：把每个 run 切成 grapheme 并量出宽度。
fn measure<T: StyleTable, M: ClusterMetrics>(
    input: LayoutInput<'_>,
    styles: &T,
    metrics: &M,
    bidi: &BidiInfo<'_>,
) -> Result<Vec<Measured>, LayoutError> {
    let mut measured = Vec::new();
    for run in input.runs() {
        let attrs = styles
            .attrs(run.style())
            .ok_or(LayoutError::UnknownStyle(run.style()))?;
        let start =
            usize::try_from(run.visual().start().get()).map_err(|_| LayoutError::OffsetOverflow)?;
        let end =
            usize::try_from(run.visual().end().get()).map_err(|_| LayoutError::OffsetOverflow)?;
        let text = input
            .text()
            .get(start..end)
            .ok_or(LayoutError::RunNotOnCharBoundary)?;
        for (local, cluster_text) in text.grapheme_indices(true) {
            let visual_start =
                VisualOffset::try_from(start + local).map_err(|_| LayoutError::OffsetOverflow)?;
            let visual_end = VisualOffset::try_from(start + local + cluster_text.len())
                .map_err(|_| LayoutError::OffsetOverflow)?;
            let visual =
                VisualRange::new(visual_start, visual_end).ok_or(LayoutError::OffsetOverflow)?;
            let mandatory_break = cluster_text.chars().all(is_mandatory_break_char);
            let advance = if mandatory_break {
                0.0
            } else {
                metrics.advance(cluster_text, attrs.style()) * attrs.size_scale()
            };
            if !advance.is_finite() || advance < 0.0 {
                return Err(LayoutError::InvalidMetrics(advance.to_bits()));
            }
            measured.push(Measured {
                visual,
                style: run.style(),
                advance,
                glyph: None,
                mandatory_break,
                space: !mandatory_break && cluster_text.chars().all(char::is_whitespace),
                level: bidi.levels[start + local].number(),
            });
        }
    }
    Ok(measured)
}

/// 第一遍的 shaping 版本：把每个 run 交给 shaper，一个字形一个簇。
///
/// 与按度量走的 [`measure`] 有一处**有意**的不同：那一版按 grapheme 切，
/// 这一版按 shaper 给出的字形切。连字因此不会被劈开——劈开画出来是两个不
/// 相干的字形，不 panic 也不报错。
///
/// run 的文本交给 shaper 时带的是一个**零基局部空间**的 range，返回的字形
/// range 按这个空间读。布局层没有源码坐标，也不该有（不变量 D4）。
/// 排不出来的簇画什么。
///
/// **不是「重试」**：S7 第七刀 b 之后那一刀实测过，单独排一个希伯来字符、
/// 一个阿拉伯字符、一个天城文元音符号，CoreText 照样拒——逐簇重试救不回
/// 这些脚本本身。但它**救得回同一个 run 里排得出来的那些簇**：`aאb` 三个
/// 字符里 `a` 与 `b` 单独都排得出来。所以降级是两级的：整段失败 → 逐簇
/// 重试 → 仍然失败的那些簇换成替代字形。
const REPLACEMENT_CLUSTER: &str = "\u{fffd}";

/// 替代字形的身份与推进量。
#[derive(Clone, Copy)]
struct Replacement {
    face: FontFaceId,
    glyph: GlyphId,
    advance: f32,
}

fn replacement_glyph<S: ShapingProvider>(
    shaper: &S,
    attrs: TextAttrs,
) -> Result<Replacement, LayoutError> {
    let local = local_range(REPLACEMENT_CLUSTER.len())?;
    let shaped = shaper
        .shape_scaled(
            REPLACEMENT_CLUSTER,
            local,
            attrs.style(),
            attrs.size_scale(),
        )
        .map_err(|error| LayoutError::Shaping(error.to_string()))?;
    let glyph = shaped
        .runs()
        .iter()
        .flat_map(|run| run.glyphs().iter().map(|glyph| (run.face(), glyph)))
        .next()
        .ok_or_else(|| LayoutError::Shaping("replacement glyph shaped to nothing".into()))?;
    let advance = shaped.advance();
    if !advance.is_finite() || advance < 0.0 {
        return Err(LayoutError::InvalidMetrics(advance.to_bits()));
    }
    Ok(Replacement {
        face: glyph.0,
        glyph: glyph.1.id(),
        advance,
    })
}

/// 把一段排好的文本变成 `Measured`。
///
/// `piece` 是 run 里的一段（整段，或降级时的一个簇），`piece_start` 是它在
/// run 里的字节起点，`run_start` 是 run 在视觉文本里的起点。三者相加才是
/// 视觉偏移与 bidi 等级的下标。
///
/// 契约的校验都在这里，两条路共用一份——共用代码路径的差分是自证的。
#[allow(clippy::too_many_arguments)]
fn measured_from_shaped(
    shaped: &ShapedText,
    piece: &str,
    piece_start: usize,
    run_start: usize,
    style: StyleId,
    attrs: TextAttrs,
    bidi: &BidiInfo<'_>,
    out: &mut Vec<Measured>,
) -> Result<(), LayoutError> {
    let local = local_range(piece.len())?;
    if shaped.source() != local {
        return Err(LayoutError::Shaping(
            "shaper returned a range different from the requested run".into(),
        ));
    }
    let base = run_start + piece_start;
    let mut cursor = 0_usize;
    for glyph_run in shaped.runs() {
        for glyph in glyph_run.glyphs() {
            let from = usize::try_from(glyph.source().start().get())
                .map_err(|_| LayoutError::OffsetOverflow)?;
            let to = usize::try_from(glyph.source().end().get())
                .map_err(|_| LayoutError::OffsetOverflow)?;
            // 字形必须按逻辑顺序、无缝、不越界地铺满这个 run。缺一段
            // 就是丢字，重一段就是重画——两样都不 panic。
            if from != cursor || to > piece.len() {
                return Err(LayoutError::Shaping(
                    "glyph ranges must tile the run in logical order".into(),
                ));
            }
            // 空区间**过得了**上面那道门（`from != cursor` 对它恒不成立），
            // 而它是「一簇多形」被反转成一形一区间时最自然的填法：多出来
            // 的字形贴在簇尾拿一个零长度区间。放进来的后果不是画错，是
            // 下面 `bidi.levels[base + from]` 在 run 末尾**越界 panic**
            // （S7 第七刀 spike 实测），落在中间则凭空多算一段 advance。
            // 契约里这是 C4，见 `yu_core::ShapingProvider`。
            if to == from {
                return Err(LayoutError::Shaping(
                    "glyph source ranges must not be empty".into(),
                ));
            }
            if !piece.is_char_boundary(from) || !piece.is_char_boundary(to) {
                return Err(LayoutError::RunNotOnCharBoundary);
            }
            cursor = to;
            let cluster_text = &piece[from..to];
            let mandatory_break =
                !cluster_text.is_empty() && cluster_text.chars().all(is_mandatory_break_char);
            let advance = if mandatory_break {
                0.0
            } else {
                glyph.advance()
            };
            if !advance.is_finite() || advance < 0.0 {
                return Err(LayoutError::InvalidMetrics(advance.to_bits()));
            }
            if !glyph.x_offset().is_finite() || !glyph.y_offset().is_finite() {
                return Err(LayoutError::Shaping("glyph offsets must be finite".into()));
            }
            out.push(Measured {
                visual: visual_range(base + from, base + to)?,
                style,
                advance,
                // 强制换行符不进字形流：它没有可画的形状，画出来是一个
                // 豆腐块。它仍然是一个簇，仍然占视觉字节。
                glyph: (!mandatory_break).then_some(GlyphRecord {
                    face: glyph_run.face(),
                    glyph: glyph.id(),
                    x_offset: glyph.x_offset(),
                    y_offset: glyph.y_offset(),
                    size_scale: attrs.size_scale(),
                }),
                mandatory_break,
                space: !mandatory_break && cluster_text.chars().all(char::is_whitespace),
                level: bidi.levels[base + from].number(),
            });
        }
    }
    if cursor != piece.len() {
        return Err(LayoutError::Shaping(
            "glyph ranges must tile the run in logical order".into(),
        ));
    }
    Ok(())
}

fn visual_range(from: usize, to: usize) -> Result<VisualRange, LayoutError> {
    let start = VisualOffset::try_from(from).map_err(|_| LayoutError::OffsetOverflow)?;
    let end = VisualOffset::try_from(to).map_err(|_| LayoutError::OffsetOverflow)?;
    VisualRange::new(start, end).ok_or(LayoutError::OffsetOverflow)
}

/// 整段排不出来时逐簇重来，返回换成替代字形的簇数。
///
/// **单独排一个簇的结果与整段排的结果可以不同**（连字与上下文成形都没了）。
/// 这只在整段已经失败之后才发生，所以正常路径上量不出差别；但它是这条
/// 降级路的一笔代价，写在这里免得以后被当成 bug。
fn substitute_run<S: ShapingProvider>(
    shaper: &S,
    text: &str,
    run_start: usize,
    style: StyleId,
    attrs: TextAttrs,
    bidi: &BidiInfo<'_>,
    out: &mut Vec<Measured>,
) -> Result<u32, LayoutError> {
    let replacement = replacement_glyph(shaper, attrs)?;
    let mut substituted = 0_u32;
    for (local, cluster_text) in text.grapheme_indices(true) {
        let mut scratch = Vec::new();
        let shaped = local_range(cluster_text.len()).ok().and_then(|range| {
            shaper
                .shape_scaled(cluster_text, range, attrs.style(), attrs.size_scale())
                .ok()
        });
        let recovered = shaped.is_some_and(|shaped| {
            measured_from_shaped(
                &shaped,
                cluster_text,
                local,
                run_start,
                style,
                attrs,
                bidi,
                &mut scratch,
            )
            .is_ok()
        });
        if recovered {
            out.append(&mut scratch);
            continue;
        }
        let base = run_start + local;
        let mandatory_break = cluster_text.chars().all(is_mandatory_break_char);
        // 强制换行本来就不进字形流（`glyph: None`），它没有被「换成替代字形」
        // ——把它算进去会让这个判据虚高，而这个数正是「降级看得见」那条断言
        // 依据的东西。
        if !mandatory_break {
            substituted = substituted.saturating_add(1);
        }
        out.push(Measured {
            visual: visual_range(base, base + cluster_text.len())?,
            style,
            advance: if mandatory_break {
                0.0
            } else {
                replacement.advance
            },
            glyph: (!mandatory_break).then_some(GlyphRecord {
                face: replacement.face,
                glyph: replacement.glyph,
                x_offset: 0.0,
                y_offset: 0.0,
                size_scale: attrs.size_scale(),
            }),
            mandatory_break,
            space: !mandatory_break && cluster_text.chars().all(char::is_whitespace),
            level: bidi.levels[base].number(),
        });
    }
    Ok(substituted)
}

/// 返回度量结果与「换成了替代字形的簇数」。
///
/// **shaper 返回 `Err` 不再往上传。** 契约（`yu_core::ShapingProvider`）允许
/// 后端说「我排不了」，而不变量 I5 要求「永不白屏、永远可编辑」——两条合起来
/// 的意思就是：**决定排不出来的东西画成什么，是布局层的事，不是后端的事。**
/// 在这之前 `Err` 一路传成 `?`，一份含希伯来文的文档整屏发不出来。
///
/// **契约违约仍然是错误。** 后端说「排好了」却给出不铺满、有空隙、越界的
/// 字形，那是 bug 不是「排不了」——那条继续返回 `Err`，由 conformance 套件
/// 与这里一起压着。两者混在一起会让一个坏后端悄悄退化成一屏替代字形。
fn measure_shaped<T: StyleTable, S: ShapingProvider>(
    input: LayoutInput<'_>,
    styles: &T,
    shaper: &S,
    bidi: &BidiInfo<'_>,
) -> Result<(Vec<Measured>, u32), LayoutError> {
    let mut measured = Vec::new();
    let mut substituted = 0_u32;
    for run in input.runs() {
        let attrs = styles
            .attrs(run.style())
            .ok_or(LayoutError::UnknownStyle(run.style()))?;
        let start =
            usize::try_from(run.visual().start().get()).map_err(|_| LayoutError::OffsetOverflow)?;
        let end =
            usize::try_from(run.visual().end().get()).map_err(|_| LayoutError::OffsetOverflow)?;
        let text = input
            .text()
            .get(start..end)
            .ok_or(LayoutError::RunNotOnCharBoundary)?;
        if text.is_empty() {
            continue;
        }
        let local = local_range(text.len())?;
        match shaper.shape_scaled(text, local, attrs.style(), attrs.size_scale()) {
            Ok(shaped) => measured_from_shaped(
                &shaped,
                text,
                0,
                start,
                run.style(),
                attrs,
                bidi,
                &mut measured,
            )?,
            Err(_) => {
                substituted = substituted.saturating_add(substitute_run(
                    shaper,
                    text,
                    start,
                    run.style(),
                    attrs,
                    bidi,
                    &mut measured,
                )?);
            }
        }
    }
    Ok((measured, substituted))
}

/// shaping 的零基局部空间。见 [`BlockLayout::build_shaped`]。
fn local_range(len: usize) -> Result<TextRange, LayoutError> {
    let end = u64::try_from(len).map_err(|_| LayoutError::OffsetOverflow)?;
    TextRange::new(ByteOffset::ZERO, ByteOffset::new(end)).ok_or(LayoutError::OffsetOverflow)
}

/// 第二遍的输入：每一段的第一个 grapheme 下标，首尾各一个哨兵。
///
/// 段界来自 UAX #14 的断行机会。两侧都是升序的，所以是一次归并走位。
///
/// 落不到 grapheme 边界上的机会会被丢掉——断行不能把一个 grapheme 劈开，
/// 劈开画出来是两个不相干的字形，不 panic 也不报错。这一条是防御性的：
/// 在采样过的 ZWJ 序列、区域指示符、组合记号、泰文与天城文样本上，
/// UAX #14 没有给出过落在 grapheme 内部的机会。
fn segment_starts(text: &str, measured: &[Measured]) -> Result<Vec<usize>, LayoutError> {
    if measured.is_empty() {
        return Ok(Vec::new());
    }
    let mut starts = vec![0_usize];
    let mut cluster = 0_usize;
    for (offset, _) in unicode_linebreak::linebreaks(text) {
        let offset = u64::try_from(offset).map_err(|_| LayoutError::OffsetOverflow)?;
        while cluster < measured.len() && measured[cluster].visual.start().get() < offset {
            cluster += 1;
        }
        if cluster >= measured.len() {
            break;
        }
        if measured[cluster].visual.start().get() == offset
            && cluster > *starts.last().expect("至少有哨兵 0")
        {
            starts.push(cluster);
        }
    }
    starts.push(measured.len());
    Ok(starts)
}

/// caret 的 x：给定「终点落在这里的簇」与「起点落在这里的簇」，选哪一侧。
///
/// 两侧方向相同时两个位置重合，选谁都一样。方向不同时按 UAX #9 §3.4 取
/// **层级更低**的那一侧，也就是更接近段落基准方向的那一段。这样边界两侧的
/// 两个几何位置分别归属两个不同的偏移，点得到也画得出。
/// 一行上一个可以停光标的位置。
#[derive(Clone, Copy)]
struct CaretPosition {
    x: f32,
    visual: VisualOffset,
    widget_affinity: Option<CaretAffinity>,
}

fn caret_x(
    before: Option<ClusterBox>,
    after: Option<ClusterBox>,
    inside: Option<ClusterBox>,
) -> f32 {
    match (before, after) {
        (Some(before), Some(after)) => {
            if before.level <= after.level {
                before.trailing_x()
            } else {
                after.leading_x()
            }
        }
        (Some(before), None) => before.trailing_x(),
        (None, Some(after)) => after.leading_x(),
        (None, None) => inside.map_or(0.0, ClusterBox::leading_x),
    }
}

/// 把配置里的基准方向翻译成 UAX #9 的段落层级。`None` 表示按 P2/P3 推断。
fn base_level(direction: BaseDirection) -> Option<Level> {
    match direction {
        BaseDirection::Auto => None,
        BaseDirection::Ltr => Some(Level::ltr()),
        BaseDirection::Rtl => Some(Level::rtl()),
    }
}

/// 量所有 widget。约束是整块的宽度，不是当前行的剩余宽度——剩余宽度取决于
/// 它排在哪，而「排在哪」要等它的宽度定了才知道。
fn measure_widgets<W: WidgetMeasure>(
    input: LayoutInput<'_>,
    config: LayoutConfig,
    widgets: &W,
) -> Result<Vec<WidgetMeasurement>, LayoutError> {
    let constraints = WidgetConstraints {
        available_width: config.max_width(),
        line_height: config.line_height(),
    };
    input
        .widgets()
        .iter()
        .map(|span| {
            let measurement = widgets
                .measure(span.widget, constraints)
                .ok_or(LayoutError::UnknownWidget(span.widget))?;
            let size = measurement.metrics().size();
            if !size.width().is_finite() || !size.height().is_finite() {
                return Err(LayoutError::InvalidWidgetSize);
            }
            Ok(measurement)
        })
        .collect()
}

/// widget 必须按视觉偏移升序、同偏移上 `Before` 在前，且都落在文本范围内。
///
/// 这就是 `DecorationRange::order_key` 的顺序（不变量 D6）。乱序不会 panic，
/// 只会让同一处的两个 widget 画反。
fn validate_widgets(input: LayoutInput<'_>, visual_len: VisualOffset) -> Result<(), LayoutError> {
    let mut previous: Option<WidgetSpan> = None;
    for span in input.widgets() {
        if span.visual > visual_len {
            return Err(LayoutError::VisualOutOfBounds(span.visual));
        }
        let index = usize::try_from(span.visual.get()).map_err(|_| LayoutError::OffsetOverflow)?;
        if !input.text().is_char_boundary(index) {
            return Err(LayoutError::RunNotOnCharBoundary);
        }
        if let Some(previous) = previous
            && (previous.visual, side_rank(previous.side)) > (span.visual, side_rank(span.side))
        {
            return Err(LayoutError::WidgetsOutOfOrder);
        }
        previous = Some(*span);
    }
    Ok(())
}

const fn side_rank(side: WidgetSide) -> u8 {
    match side {
        WidgetSide::Before => 0,
        WidgetSide::After => 1,
    }
}

/// 解释所有行级样式段，顺带校验它们升序、不重叠、落在文本范围内。
fn resolve_line_styles<L: LineStyleTable>(
    input: LayoutInput<'_>,
    table: &L,
) -> Result<Vec<(VisualRange, LineStyleId, LineAttrs)>, LayoutError> {
    let mut resolved = Vec::with_capacity(input.line_styles().len());
    let mut previous_end = VisualOffset::ZERO;
    for span in input.line_styles() {
        if span.visual.start() < previous_end {
            return Err(LayoutError::LineStylesOutOfOrder);
        }
        let attrs = table
            .attrs(span.style)
            .ok_or(LayoutError::UnknownLineStyle(span.style))?;
        resolved.push((span.visual, span.style, attrs));
        previous_end = span.visual.end();
    }
    Ok(resolved)
}

/// runs 必须无缝铺满视觉文本，且每个边界都在 UTF-8 字符边界上。
fn validate_runs(input: LayoutInput<'_>, visual_len: VisualOffset) -> Result<(), LayoutError> {
    let mut expected = VisualOffset::ZERO;
    for run in input.runs {
        if run.visual.start() != expected {
            return Err(LayoutError::RunsNotContiguous {
                expected,
                found: run.visual.start(),
            });
        }
        for offset in [run.visual.start().get(), run.visual.end().get()] {
            let index = usize::try_from(offset).map_err(|_| LayoutError::OffsetOverflow)?;
            if !input.text.is_char_boundary(index) {
                return Err(LayoutError::RunNotOnCharBoundary);
            }
        }
        expected = run.visual.end();
    }
    if expected != visual_len {
        return Err(LayoutError::RunsNotContiguous {
            expected: visual_len,
            found: expected,
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{
        BlockLayout, LayoutInput, LineAttrs, LineSpan, LineStyleTable, NoWidgets, StyleTable,
        StyledRun, UniformStyleTable, WidgetConstraints, WidgetMeasure, WidgetMeasurement,
        WidgetMetrics, WidgetSpan,
    };
    use super::{NoLineStyles, local_range};
    use crate::{BaseDirection, LayoutConfig, LayoutError, LayoutPoint, MonospaceMetrics};
    use unicode_segmentation::UnicodeSegmentation;
    use yu_core::{
        ByteOffset, CaretAffinity, FontFaceId, Glyph, GlyphId, GlyphRun, LineStyleId, Script,
        ShapedText, ShapingProvider, Size, StyleId, TextAttrs, TextDirection, TextRange, TextStyle,
        VisualOffset, VisualRange, WidgetId, WidgetSide,
    };

    fn visual(start: u64, end: u64) -> VisualRange {
        VisualRange::new(VisualOffset::new(start), VisualOffset::new(end)).expect("有序")
    }

    fn plain(text: &str) -> Vec<StyledRun> {
        vec![StyledRun::new(visual(0, text.len() as u64), StyleId(0))]
    }

    fn build(text: &str, runs: &[StyledRun], width: f32) -> Result<BlockLayout, LayoutError> {
        BlockLayout::build(
            LayoutInput::new(text, runs),
            LayoutConfig::new(width, 1.0),
            &UniformStyleTable::default(),
            &MonospaceMetrics::default(),
        )
    }

    /// 一个 grapheme 一个字形，推进量固定；`ligate` 里的两个字节连成一个。
    struct TestShaper {
        advance: f32,
        ligate: Option<&'static str>,
        /// 每个字形往上抬多少。用来验证 y 偏移进了 origin。
        rise: f32,
        /// 每个字形往右挪多少。用来验证 x 偏移进了 origin。
        shift: f32,
    }

    impl TestShaper {
        const fn new(advance: f32) -> Self {
            Self {
                advance,
                ligate: None,
                rise: 0.0,
                shift: 0.0,
            }
        }
    }

    impl ShapingProvider for TestShaper {
        type Error = String;

        fn shape(
            &self,
            text: &str,
            source: TextRange,
            style: TextStyle,
        ) -> Result<ShapedText, Self::Error> {
            let mut glyphs = Vec::new();
            let mut cursor = 0_usize;
            while cursor < text.len() {
                let len = self
                    .ligate
                    .filter(|pair| text[cursor..].starts_with(*pair))
                    .map_or_else(
                        || {
                            text[cursor..]
                                .graphemes(true)
                                .next()
                                .map_or(0, |cluster| cluster.len())
                        },
                        str::len,
                    );
                // 契约 C8：字形区间是 `source.start()` **加上**局部偏移。
                // 布局层今天总是传零基 range（`local_range`），所以少加这一下
                // 在产品链路上看不出来——压它的是 conformance 套件。
                let range = TextRange::new(
                    ByteOffset::new(source.start().get() + cursor as u64),
                    ByteOffset::new(source.start().get() + (cursor + len) as u64),
                )
                .ok_or("有序")?;
                glyphs.push(Glyph::new(
                    GlyphId::from_raw(cursor as u32 + 1),
                    range,
                    self.advance,
                    self.shift,
                    self.rise,
                ));
                cursor += len;
            }
            Ok(ShapedText::new(
                source,
                vec![GlyphRun::new(
                    FontFaceId::from_raw(7),
                    source,
                    style,
                    TextDirection::Ltr,
                    Script::Latin,
                    glyphs,
                )],
            ))
        }
    }

    fn build_shaped(
        text: &str,
        runs: &[StyledRun],
        width: f32,
        shaper: &TestShaper,
    ) -> Result<BlockLayout, LayoutError> {
        BlockLayout::build_shaped(
            LayoutInput::new(text, runs),
            LayoutConfig::new(width, 4.0),
            &UniformStyleTable::default(),
            &NoWidgets,
            &NoLineStyles,
            shaper,
        )
    }

    /// shaping 那条路与度量那条路共用断行、重排与行盒。共用的部分不重复
    /// 验证（共用代码路径的差分是自证的），这里只钉住「字形跟着簇走」。
    #[test]
    fn shaped_glyphs_sit_on_their_cluster_and_baseline() {
        let text = "ab cd";
        let layout = build_shaped(text, &plain(text), 4.0, &TestShaper::new(1.0)).expect("布局");

        // 宽度 4，"ab " 的可见部分是 2，加上 "cd" 的 2 正好 4——UAX #14 的
        // 断点在空白之后，空白悬在行外。
        assert_eq!(layout.lines().len(), 2);
        assert_eq!(layout.glyphs().len(), 5);
        for (glyph, cluster) in layout.glyphs().iter().zip(layout.clusters()) {
            assert_eq!(glyph.visual(), cluster.visual());
            assert_eq!(glyph.line(), cluster.line());
            assert_eq!(glyph.origin().x(), cluster.x());
            let line = &layout.lines()[cluster.line()];
            assert_eq!(glyph.origin().y(), line.bounds().y() + line.baseline());
            assert_eq!(glyph.face(), FontFaceId::from_raw(7));
            assert_eq!(glyph.size_scale(), 1.0);
        }
    }

    /// 字形偏移必须进 origin。丢掉它不会 panic，只会让重音记号落在错的地方。
    #[test]
    fn glyph_offsets_reach_the_origin() {
        let text = "ab";
        let shaper = TestShaper {
            rise: -1.5,
            shift: 0.25,
            ..TestShaper::new(1.0)
        };
        let layout = build_shaped(text, &plain(text), 80.0, &shaper).expect("布局");
        let line = &layout.lines()[0];
        for (glyph, cluster) in layout.glyphs().iter().zip(layout.clusters()) {
            assert_eq!(glyph.origin().x(), cluster.x() + 0.25);
            assert_eq!(
                glyph.origin().y(),
                line.bounds().y() + line.baseline() - 1.5
            );
        }
    }

    /// 连字是一个簇，不能被断行劈开——劈开画出来是两个不相干的字形。
    #[test]
    fn a_ligature_stays_one_cluster() {
        let text = "fi";
        let shaper = TestShaper {
            ligate: Some("fi"),
            ..TestShaper::new(3.0)
        };
        let layout = build_shaped(text, &plain(text), 2.0, &shaper).expect("布局");
        assert_eq!(layout.clusters().len(), 1);
        assert_eq!(layout.clusters()[0].visual(), visual(0, 2));
        assert_eq!(layout.glyphs().len(), 1);
        assert_eq!(layout.lines().len(), 1);
    }

    /// 换行符不进字形流，但仍然是一个簇。少了这一条会画出一个豆腐块。
    #[test]
    fn a_mandatory_break_is_a_cluster_without_a_glyph() {
        let text = "a\nb";
        let layout = build_shaped(text, &plain(text), 80.0, &TestShaper::new(1.0)).expect("布局");
        assert_eq!(layout.clusters().len(), 3);
        assert!(layout.clusters()[1].is_line_break());
        assert_eq!(layout.glyphs().len(), 2);
        assert_eq!(layout.glyphs()[1].visual(), visual(2, 3));
        assert_eq!(layout.glyphs()[1].line(), 1);
    }

    /// 字号倍率既要进推进量（断行随之改变），也要进 `size_scale`（栅格化
    /// 按它选字号）。只做前者会得到几何对、糊掉的字。
    #[test]
    fn size_scale_reaches_both_the_advance_and_the_glyph() {
        struct Doubling;
        impl StyleTable for Doubling {
            fn attrs(&self, style: StyleId) -> Option<TextAttrs> {
                match style {
                    StyleId(1) => TextAttrs::default().with_size_scale(2.0),
                    _ => Some(TextAttrs::default()),
                }
            }
        }
        let text = "ab";
        let runs = vec![
            StyledRun::new(visual(0, 1), StyleId(0)),
            StyledRun::new(visual(1, 2), StyleId(1)),
        ];
        let layout = BlockLayout::build_shaped(
            LayoutInput::new(text, &runs),
            LayoutConfig::new(80.0, 4.0),
            &Doubling,
            &NoWidgets,
            &NoLineStyles,
            &TestShaper::new(1.0),
        )
        .expect("布局");
        assert_eq!(layout.clusters()[0].width(), 1.0);
        assert_eq!(layout.clusters()[1].width(), 2.0);
        assert_eq!(layout.glyphs()[0].size_scale(), 1.0);
        assert_eq!(layout.glyphs()[1].size_scale(), 2.0);
    }

    /// 按 `ranges` 逐个产字形，不管文本实际是什么。**故意违约的 mock**，
    /// 用来压那些真实后端造不出来的输入。
    struct Ranges(&'static [(u64, u64)]);

    impl ShapingProvider for Ranges {
        type Error = String;

        fn shape(
            &self,
            _text: &str,
            source: TextRange,
            style: TextStyle,
        ) -> Result<ShapedText, Self::Error> {
            let glyphs = self
                .0
                .iter()
                .map(|(from, to)| {
                    let range = TextRange::new(ByteOffset::new(*from), ByteOffset::new(*to))
                        .ok_or("有序")?;
                    Ok(Glyph::new(GlyphId::from_raw(1), range, 1.0, 0.0, 0.0))
                })
                .collect::<Result<Vec<_>, Self::Error>>()?;
            Ok(ShapedText::new(
                source,
                vec![GlyphRun::new(
                    FontFaceId::from_raw(1),
                    source,
                    style,
                    TextDirection::Ltr,
                    Script::Latin,
                    glyphs,
                )],
            ))
        }
    }

    fn shaped_with(text: &str, ranges: &'static [(u64, u64)]) -> Result<BlockLayout, LayoutError> {
        BlockLayout::build_shaped(
            LayoutInput::new(text, &plain(text)),
            LayoutConfig::new(80.0, 4.0),
            &UniformStyleTable::default(),
            &NoWidgets,
            &NoLineStyles,
            &Ranges(ranges),
        )
    }

    /// 布局层自己那个 mock 也得守契约——否则整个 `block.rs` 的用例都建在
    /// 一个不合规的后端上，而「布局能不能消费一个合规后端」正是这些用例
    /// 要证明的事。
    #[test]
    fn the_test_shaper_conforms_to_the_shaping_contract() {
        let violations = yu_core::shaping_conformance::audit(&TestShaper::new(1.0));
        assert!(violations.is_empty(), "{violations:#?}");
    }

    /// 一个「凡是含某个字符的文本就报错」的后端，用来压降级那条路。
    /// 其余文本一 `char` 一形，字形 id 就是那个 `char` 的码位。
    struct Refusing(char);

    impl ShapingProvider for Refusing {
        type Error = String;

        fn shape(
            &self,
            text: &str,
            source: TextRange,
            style: TextStyle,
        ) -> Result<ShapedText, Self::Error> {
            if text.contains(self.0) {
                return Err(format!("排不了 {:?}", self.0));
            }
            let glyphs = text
                .char_indices()
                .map(|(offset, character)| {
                    let from = source.start().get() + offset as u64;
                    let to = from + character.len_utf8() as u64;
                    Ok(Glyph::new(
                        GlyphId::from_raw(character as u32),
                        TextRange::new(ByteOffset::new(from), ByteOffset::new(to)).ok_or("有序")?,
                        1.0,
                        0.0,
                        0.0,
                    ))
                })
                .collect::<Result<Vec<_>, Self::Error>>()?;
            Ok(ShapedText::new(
                source,
                vec![GlyphRun::new(
                    FontFaceId::from_raw(1),
                    source,
                    style,
                    TextDirection::Ltr,
                    Script::Latin,
                    glyphs,
                )],
            ))
        }
    }

    /// 排不出来的文本必须画成替代字形，而不是让整个块失败。
    ///
    /// 这一条压的是不变量 I5 的「永不白屏、永远可编辑」。在它之前，
    /// `ShapingProvider` 的 `Err` 一路传成 `?`，一份含希伯来文的文档
    /// **整屏发不出来**——不是画错，是不出现。
    ///
    /// **判据落在三样上，缺一不可**：布局成功、字形数等于簇数（画得出来）、
    /// `substituted_clusters` 正好等于排不出来的那几簇（降级看得见）。
    /// 只断前两样的话，一个「把整段都换成豆腐块」的实现也会绿。
    #[test]
    fn unshapable_clusters_become_replacement_glyphs_instead_of_failing() {
        let text = "a\u{05d0}b";
        let layout = BlockLayout::build_shaped(
            LayoutInput::new(text, &plain(text)),
            LayoutConfig::new(80.0, 4.0),
            &UniformStyleTable::default(),
            &NoWidgets,
            &NoLineStyles,
            &Refusing('\u{05d0}'),
        )
        .expect("排不出来的簇不该让整个块失败");

        assert_eq!(layout.clusters().len(), 3);
        assert_eq!(layout.glyphs().len(), 3, "三个簇都要画得出来");
        assert_eq!(layout.substituted_clusters(), 1, "只有那一个簇该被替换");
        // 排得出来的两个簇保留**自己的**字形，不跟着一起变豆腐。
        let ids: Vec<u32> = layout
            .glyphs()
            .iter()
            .map(|glyph| glyph.glyph().get())
            .collect();
        assert_eq!(ids, vec!['a' as u32, 0xfffd, 'b' as u32]);
    }

    /// 正常语料一个都不该被替换——这条是上一条的反面，防止「无脑全替换」。
    #[test]
    fn a_shapable_run_substitutes_nothing() {
        let text = "abc";
        let layout = BlockLayout::build_shaped(
            LayoutInput::new(text, &plain(text)),
            LayoutConfig::new(80.0, 4.0),
            &UniformStyleTable::default(),
            &NoWidgets,
            &NoLineStyles,
            &Refusing('\u{05d0}'),
        )
        .expect("布局");
        assert_eq!(layout.substituted_clusters(), 0);
        assert_eq!(layout.glyphs().len(), 3);
    }

    /// 连替代字形都排不出来时没有第二条退路，只能报错——**不许伪造一个
    /// 字形 id**，那画出来是别的字。
    #[test]
    fn a_backend_that_refuses_everything_is_still_an_error() {
        struct RefusesEverything;
        impl ShapingProvider for RefusesEverything {
            type Error = String;
            fn shape(
                &self,
                _text: &str,
                _source: TextRange,
                _style: TextStyle,
            ) -> Result<ShapedText, Self::Error> {
                Err("什么都排不了".to_owned())
            }
        }
        let text = "abc";
        assert!(matches!(
            BlockLayout::build_shaped(
                LayoutInput::new(text, &plain(text)),
                LayoutConfig::new(80.0, 4.0),
                &UniformStyleTable::default(),
                &NoWidgets,
                &NoLineStyles,
                &RefusesEverything,
            ),
            Err(LayoutError::Shaping(_))
        ));
    }

    /// 空区间**过得了**下面那道 tiling 门，所以要单独钉一条。
    ///
    /// 它不是想出来的坏输入：DirectWrite 的 `clusterMap` 反转在「一簇多形」
    /// 上最自然的填法就是给多出来的字形一个贴在簇尾的零长度区间
    /// （S7 第七刀的 spike）。修之前的表现分两种，**都不是「画错一点」**：
    ///
    /// - 落在 run 末尾：`bidi.levels[start + from]` 越界 **panic**
    ///   （实测 `index out of bounds: the len is 3 but the index is 3`）；
    /// - 落在中间：多出一个零长度的 cluster，advance 照算，行宽凭空变宽。
    ///
    /// 两个位置都要钉——只钉末尾那种会让中间那种继续溜过去，而中间那种不
    /// panic，最难发现。
    #[test]
    fn empty_glyph_ranges_are_rejected() {
        let text = "abc";
        for ranges in [
            &[(0, 1), (1, 1), (1, 3)][..],
            &[(0, 0), (0, 3)][..],
            &[(0, 1), (1, 3), (3, 3)][..],
        ] {
            assert!(
                matches!(shaped_with(text, ranges), Err(LayoutError::Shaping(_))),
                "{ranges:?} 里有空区间，应该被拒绝"
            );
        }
    }

    /// 字形没铺满 run 就是丢字或重画，两样都不 panic。
    ///
    /// 三种坏法各钉一条：中间缺一段（丢字）、区间回退（重画）、末尾短一截。
    /// 只钉最后一种会让「总长对得上但中间有缝」溜过去。
    #[test]
    fn glyph_ranges_must_tile_the_run() {
        let text = "abc";
        for ranges in [
            &[(0, 1), (2, 3)][..],
            &[(0, 2), (1, 3)][..],
            &[(0, 1)][..],
            &[(0, 1), (1, 2), (2, 4)][..],
        ] {
            assert!(
                matches!(shaped_with(text, ranges), Err(LayoutError::Shaping(_))),
                "{ranges:?} 应该被拒绝"
            );
        }
        assert!(
            BlockLayout::build_shaped(
                LayoutInput::new(text, &plain(text)),
                LayoutConfig::new(80.0, 4.0),
                &UniformStyleTable::default(),
                &NoWidgets,
                &NoLineStyles,
                &Ranges(&[(0, 1), (1, 2), (2, 3)]),
            )
            .is_ok()
        );
    }

    /// RTL 行里字形跟着重排走。在重排之前算 origin 会把它们画在逻辑位置上。
    #[test]
    fn shaped_glyphs_follow_the_bidi_reorder() {
        let text = "a\u{5d0}\u{5d1}";
        let layout = BlockLayout::build_shaped(
            LayoutInput::new(text, &plain(text)),
            LayoutConfig::new(80.0, 4.0),
            &UniformStyleTable::default(),
            &NoWidgets,
            &NoLineStyles,
            &TestShaper::new(1.0),
        )
        .expect("布局");
        assert_eq!(layout.clusters().len(), 3);
        // 逻辑顺序 a ℵ ℶ，视觉顺序 a ℶ ℵ：后两个换了位置。
        assert_eq!(layout.clusters()[1].x(), 2.0);
        assert_eq!(layout.clusters()[2].x(), 1.0);
        for (glyph, cluster) in layout.glyphs().iter().zip(layout.clusters()) {
            assert_eq!(glyph.origin().x(), cluster.x());
        }
    }

    /// 零基局部空间就是 `0..len`。shaper 拿到的不是源码坐标。
    #[test]
    fn the_shaping_space_is_local_and_zero_based() {
        assert_eq!(
            local_range(5).expect("局部空间"),
            TextRange::new(ByteOffset::ZERO, ByteOffset::new(5)).expect("有序")
        );
    }

    /// 文末的空行属于最后一段行级样式。半开区间接不住它。
    #[test]
    fn the_trailing_empty_line_keeps_the_last_line_style() {
        struct Indent;
        impl LineStyleTable for Indent {
            fn attrs(&self, _style: LineStyleId) -> Option<LineAttrs> {
                LineAttrs::new(4.0, 1.0).ok()
            }
        }
        let text = "ab\n";
        let spans = vec![LineSpan::new(visual(0, 3), LineStyleId(0))];
        let layout = BlockLayout::build_all(
            LayoutInput::new(text, &plain(text)).with_line_styles(&spans),
            LayoutConfig::new(80.0, 1.0),
            &UniformStyleTable::default(),
            &NoWidgets,
            &Indent,
            &MonospaceMetrics::default(),
        )
        .expect("布局");
        assert_eq!(layout.lines().len(), 2);
        assert_eq!(layout.lines()[1].style(), Some(LineStyleId(0)));
        // caret 与 hit-test 在空行上都只能问行级缩进：问不到就把光标停在
        // 块的左边缘，点一下也跳回那里。
        assert_eq!(
            layout
                .caret(VisualOffset::new(3), CaretAffinity::Downstream)
                .expect("行末 caret")
                .point()
                .x(),
            4.0
        );
        let hit = layout
            .hit(LayoutPoint::new(0.0, 1.5))
            .expect("空行 hit-test");
        assert_eq!(hit.line(), 1);
        assert_eq!(hit.point().x(), 4.0);
        assert_eq!(hit.visual(), VisualOffset::new(3));
    }

    /// 一份漏掉半段文本的 run 列表会画出少了几个字的一行，既不 panic 也不
    /// 报错——这个项目最危险的失败模式。所以铺不满就是错误。
    #[test]
    fn runs_must_tile_the_visual_text() {
        let text = "abcdef";
        let short = vec![StyledRun::new(visual(0, 3), StyleId(0))];
        assert_eq!(
            build(text, &short, 80.0),
            Err(LayoutError::RunsNotContiguous {
                expected: VisualOffset::new(6),
                found: VisualOffset::new(3),
            })
        );

        let gapped = vec![
            StyledRun::new(visual(0, 2), StyleId(0)),
            StyledRun::new(visual(3, 6), StyleId(0)),
        ];
        assert_eq!(
            build(text, &gapped, 80.0),
            Err(LayoutError::RunsNotContiguous {
                expected: VisualOffset::new(2),
                found: VisualOffset::new(3),
            })
        );

        let overlapping = vec![
            StyledRun::new(visual(0, 4), StyleId(0)),
            StyledRun::new(visual(3, 6), StyleId(0)),
        ];
        assert!(matches!(
            build(text, &overlapping, 80.0),
            Err(LayoutError::RunsNotContiguous { .. })
        ));

        assert!(build(text, &plain(text), 80.0).is_ok());
        assert!(build("", &[], 80.0).is_ok());
    }

    #[test]
    fn run_boundaries_must_land_on_char_boundaries() {
        let text = "中文";
        let split = vec![
            StyledRun::new(visual(0, 1), StyleId(0)),
            StyledRun::new(visual(1, 6), StyleId(0)),
        ];
        assert_eq!(
            build(text, &split, 80.0),
            Err(LayoutError::RunNotOnCharBoundary)
        );
    }

    /// 装饰产出与样式表脱节必须响。按默认字型排会画出一份「看起来正常但
    /// 强调没生效」的画面，没有任何东西会拦住它。
    #[test]
    fn unknown_style_is_an_error_not_a_default() {
        struct OnlyZero;
        impl StyleTable for OnlyZero {
            fn attrs(&self, style: StyleId) -> Option<TextAttrs> {
                (style == StyleId(0)).then(TextAttrs::default)
            }
        }
        let text = "ab";
        let runs = vec![StyledRun::new(visual(0, 2), StyleId(7))];
        let built = BlockLayout::build(
            LayoutInput::new(text, &runs),
            LayoutConfig::new(80.0, 1.0),
            &OnlyZero,
            &MonospaceMetrics::default(),
        );
        assert_eq!(built, Err(LayoutError::UnknownStyle(StyleId(7))));
    }

    /// 字号倍率是布局层唯一知道的「这段比别处大」——标题靠它变大，而布局层
    /// 看不见「标题」。几何差分压不到它（那条差分的语料全是 1.0 倍），
    /// 所以在这里单独钉住。
    #[test]
    fn size_scale_multiplies_advance_and_therefore_wrapping() {
        struct Doubling;
        impl StyleTable for Doubling {
            fn attrs(&self, style: StyleId) -> Option<TextAttrs> {
                let attrs = TextAttrs::new(TextStyle::Plain);
                match style {
                    StyleId(0) => Some(attrs),
                    StyleId(1) => attrs.with_size_scale(2.0),
                    _ => None,
                }
            }
        }
        let text = "abcd";
        let config = LayoutConfig::new(4.0, 1.0);
        let metrics = MonospaceMetrics::default();

        let normal = BlockLayout::build(
            LayoutInput::new(text, &[StyledRun::new(visual(0, 4), StyleId(0))]),
            config,
            &Doubling,
            &metrics,
        )
        .expect("布局");
        assert_eq!(normal.lines().len(), 1);
        assert_eq!(normal.lines()[0].width(), 4.0);

        let doubled = BlockLayout::build(
            LayoutInput::new(text, &[StyledRun::new(visual(0, 4), StyleId(1))]),
            config,
            &Doubling,
            &metrics,
        )
        .expect("布局");
        assert_eq!(doubled.clusters()[0].width(), 2.0);
        assert_eq!(doubled.lines().len(), 2, "倍率翻倍后 4 个字排不进宽度 4");
    }

    #[test]
    fn caret_rejects_offsets_past_the_block() {
        let text = "abc";
        let layout = build(text, &plain(text), 80.0).expect("布局");
        assert_eq!(
            layout.caret(VisualOffset::new(4), CaretAffinity::Downstream),
            Err(LayoutError::VisualOutOfBounds(VisualOffset::new(4)))
        );
        assert!(
            layout
                .caret(VisualOffset::new(3), CaretAffinity::Downstream)
                .is_ok()
        );
    }

    /// 软换行边界上 affinity 决定 caret 画在上一行末还是下一行首。
    /// 这两个位置几何上差一整行高，混淆了不 panic，只是光标跳到别处。
    #[test]
    fn affinity_picks_the_visual_line_at_a_soft_wrap() {
        let text = "abcdef";
        let layout = build(text, &plain(text), 3.0).expect("布局");
        assert_eq!(layout.lines().len(), 2);
        let boundary = VisualOffset::new(3);
        let upstream = layout
            .caret(boundary, CaretAffinity::Upstream)
            .expect("caret");
        let downstream = layout
            .caret(boundary, CaretAffinity::Downstream)
            .expect("caret");
        assert_eq!(upstream.line(), 0);
        assert_eq!(upstream.point(), LayoutPoint::new(3.0, 0.0));
        assert_eq!(downstream.line(), 1);
        assert_eq!(downstream.point(), LayoutPoint::new(0.0, 1.0));
    }

    /// 把每一行的视觉区间还原成文本，便于把断行结果写成看得懂的期望。
    fn line_texts(text: &str, width: f32) -> Vec<String> {
        let layout = build(text, &plain(text), width).expect("布局");
        layout
            .lines()
            .iter()
            .map(|line| {
                let from = line.visual().start().get() as usize;
                let to = line.visual().end().get() as usize;
                text[from..to].to_owned()
            })
            .collect()
    }

    /// 一张按 id 给行级属性的表。
    struct LineStyles {
        entries: Vec<(LineStyleId, f32, f32)>,
    }

    impl LineStyleTable for LineStyles {
        fn attrs(&self, style: LineStyleId) -> Option<LineAttrs> {
            let (_, indent, scale) = *self.entries.iter().find(|entry| entry.0 == style)?;
            Some(LineAttrs::new(indent, scale).expect("测试给的值合法"))
        }
    }

    fn build_lines(
        text: &str,
        width: f32,
        spans: &[LineSpan],
        table: &LineStyles,
    ) -> Result<BlockLayout, LayoutError> {
        BlockLayout::build_all(
            LayoutInput::new(text, &plain(text)).with_line_styles(spans),
            LayoutConfig::new(width, 1.0),
            &UniformStyleTable::default(),
            &NoWidgets,
            table,
            &MonospaceMetrics::default(),
        )
    }

    /// 缩进把内容整体右移，而且**吃掉可用宽度**：同一段文字在缩进之后
    /// 会更早换行。只移 x 不减可用宽度，缩进的段落会溢出到画面外。
    #[test]
    fn indent_shifts_content_and_shrinks_the_usable_width() {
        let text = "abcd";
        let table = LineStyles {
            entries: vec![(LineStyleId(1), 2.0, 1.0)],
        };
        let spans = [LineSpan::new(visual(0, 4), LineStyleId(1))];

        let plain_layout = build(text, &plain(text), 4.0).expect("布局");
        assert_eq!(plain_layout.lines().len(), 1);

        let layout = build_lines(text, 4.0, &spans, &table).expect("布局");
        assert_eq!(layout.lines().len(), 2, "缩进 2.0 之后 4 个字排不进宽度 4");
        assert_eq!(layout.clusters()[0].x(), 2.0, "内容从缩进处开始");
        assert_eq!(layout.lines()[0].width(), 4.0, "行宽含缩进");
        assert_eq!(layout.lines()[1].bounds().y(), 1.0);
    }

    /// 缩进不是内容：它不能让每一行开头就断一次。
    #[test]
    fn indent_alone_never_triggers_a_break() {
        let text = "ab";
        let table = LineStyles {
            entries: vec![(LineStyleId(1), 3.0, 1.0)],
        };
        let spans = [LineSpan::new(visual(0, 2), LineStyleId(1))];
        // 缩进 3.0 已经超过整行宽度 2.0；内容还是得排出来，不能无限断行。
        let layout = build_lines(text, 2.0, &spans, &table).expect("布局");
        assert_eq!(layout.lines().len(), 2);
        assert_eq!(layout.clusters()[0].x(), 3.0);
    }

    /// 行高倍率撑高整行，后面的行跟着往下移。
    #[test]
    fn line_height_scale_grows_the_line_box() {
        let text = "a\nb";
        let table = LineStyles {
            entries: vec![(LineStyleId(1), 0.0, 2.5)],
        };
        // 只盖住第一行。
        let spans = [LineSpan::new(visual(0, 2), LineStyleId(1))];
        let layout = build_lines(text, 80.0, &spans, &table).expect("布局");
        assert_eq!(layout.lines()[0].height(), 2.5);
        assert_eq!(layout.lines()[0].baseline(), 2.5);
        assert_eq!(layout.lines()[1].bounds().y(), 2.5);
        assert_eq!(
            layout.lines()[1].height(),
            1.0,
            "第二行没被盖到，用默认行高"
        );
        assert_eq!(layout.height(), 3.5);
    }

    /// 行样式的 id 原样带出去，布局层不解释它。背景与前缀装饰长什么样是
    /// 上层拿这个 id 查同一张表的事——让布局认识「引用条什么颜色」正是
    /// 不变量 E1 要挡的泄漏。
    #[test]
    fn the_line_style_id_is_carried_through_untouched() {
        let text = "a\nb";
        let table = LineStyles {
            entries: vec![(LineStyleId(9), 1.0, 1.0)],
        };
        let spans = [LineSpan::new(visual(0, 2), LineStyleId(9))];
        let layout = build_lines(text, 80.0, &spans, &table).expect("布局");
        assert_eq!(layout.lines()[0].style(), Some(LineStyleId(9)));
        assert_eq!(layout.lines()[1].style(), None);
    }

    /// 表里没有的 id 是错误，不是默认属性；乱序或重叠的段也是错误。
    #[test]
    fn line_styles_are_validated() {
        let text = "abcd";
        let table = LineStyles {
            entries: vec![(LineStyleId(1), 0.0, 1.0)],
        };
        assert_eq!(
            build_lines(
                text,
                80.0,
                &[LineSpan::new(visual(0, 2), LineStyleId(5))],
                &table
            ),
            Err(LayoutError::UnknownLineStyle(LineStyleId(5)))
        );
        let overlapping = [
            LineSpan::new(visual(0, 3), LineStyleId(1)),
            LineSpan::new(visual(2, 4), LineStyleId(1)),
        ];
        assert_eq!(
            build_lines(text, 80.0, &overlapping, &table),
            Err(LayoutError::LineStylesOutOfOrder)
        );
        assert_eq!(
            LineAttrs::new(f32::NAN, 1.0),
            Err(LayoutError::InvalidLineStyle)
        );
        assert_eq!(
            LineAttrs::new(-1.0, 1.0),
            Err(LayoutError::InvalidLineStyle)
        );
        assert_eq!(LineAttrs::new(0.0, 0.0), Err(LayoutError::InvalidLineStyle));
    }

    /// 一张按 id 给尺寸的 widget 表。`ready` 为假模拟资源没就绪。
    struct Widgets {
        entries: Vec<(WidgetId, f32, f32, f32, bool)>,
    }

    impl WidgetMeasure for Widgets {
        fn measure(
            &self,
            widget: WidgetId,
            constraints: WidgetConstraints,
        ) -> Option<WidgetMeasurement> {
            let (_, width, height, baseline, ready) =
                *self.entries.iter().find(|entry| entry.0 == widget)?;
            assert!(constraints.available_width() > 0.0);
            assert!(constraints.line_height() > 0.0);
            let metrics = WidgetMetrics::new(Size::new(width, height).expect("有限"), baseline)
                .expect("基线合法");
            Some(if ready {
                WidgetMeasurement::Ready(metrics)
            } else {
                WidgetMeasurement::Placeholder(metrics)
            })
        }
    }

    fn build_widgets(
        text: &str,
        width: f32,
        spans: &[WidgetSpan],
        table: &Widgets,
    ) -> Result<BlockLayout, LayoutError> {
        BlockLayout::build_with_widgets(
            LayoutInput::new(text, &plain(text)).with_widgets(spans),
            LayoutConfig::new(width, 1.0),
            &UniformStyleTable::default(),
            table,
            &MonospaceMetrics::default(),
        )
    }

    /// widget 在视觉字节流里不占位，但在行里占宽度：它后面的字要让开。
    #[test]
    fn a_widget_takes_horizontal_space_without_taking_visual_bytes() {
        let text = "ab";
        let table = Widgets {
            entries: vec![(WidgetId(1), 3.0, 1.0, 1.0, true)],
        };
        let spans = [WidgetSpan::new(
            VisualOffset::new(1),
            WidgetId(1),
            WidgetSide::Before,
        )];
        let layout = build_widgets(text, 80.0, &spans, &table).expect("布局");
        assert_eq!(
            layout.visual_len(),
            VisualOffset::new(2),
            "视觉长度不含 widget"
        );
        assert_eq!(layout.clusters()[0].x(), 0.0);
        assert_eq!(
            layout.clusters()[1].x(),
            4.0,
            "'b' 要给 3.0 宽的 widget 让开"
        );
        let placed = layout.widgets()[0];
        assert_eq!(placed.bounds().x(), 1.0);
        assert_eq!(placed.bounds().width(), 3.0);
        assert_eq!(layout.lines()[0].width(), 5.0);
    }

    /// widget 参与断行，而且自身不可分割：排不下就整个挪到下一行。
    /// UAX #14 的 CB 就是这个语义——前后都能断，中间不能。
    #[test]
    fn a_widget_moves_to_the_next_line_when_it_does_not_fit() {
        let text = "aab";
        let table = Widgets {
            entries: vec![(WidgetId(1), 3.0, 1.0, 1.0, true)],
        };
        let spans = [WidgetSpan::new(
            VisualOffset::new(2),
            WidgetId(1),
            WidgetSide::Before,
        )];
        // 宽度 4 的行放得下 "aa"，再塞 3.0 宽的 widget 就超了。
        let layout = build_widgets(text, 4.0, &spans, &table).expect("布局");
        assert_eq!(layout.lines().len(), 2);
        assert_eq!(
            layout.lines()[0].width(),
            2.0,
            "widget 整个挪走，不切一半留下"
        );
        assert_eq!(layout.widgets()[0].line(), 1);
        assert_eq!(layout.widgets()[0].bounds().x(), 0.0);
        assert_eq!(layout.clusters()[2].x(), 3.0, "'b' 跟在 widget 后面");
    }

    /// 基线对齐：widget 按自己的 baseline 往下挂，谁要求得更深行就往下长，
    /// 后面的行跟着往下移。行高一律按 `line_height` 算会让高图片压住下一行。
    #[test]
    fn a_tall_widget_grows_its_line_and_pushes_later_lines_down() {
        let text = "a\nb";
        let table = Widgets {
            entries: vec![(WidgetId(1), 2.0, 4.0, 3.0, true)],
        };
        let spans = [WidgetSpan::new(
            VisualOffset::ZERO,
            WidgetId(1),
            WidgetSide::Before,
        )];
        let layout = build_widgets(text, 80.0, &spans, &table).expect("布局");
        assert_eq!(layout.lines().len(), 2);
        // 文字的 ascent 是 1.0，widget 要求基线在 3.0 处，行基线取更深的那个。
        assert_eq!(layout.lines()[0].baseline(), 3.0);
        // 基线下面还剩 4.0 - 3.0 = 1.0 的 descent。
        assert_eq!(layout.lines()[0].height(), 4.0);
        assert_eq!(layout.lines()[0].y(), 0.0);
        assert_eq!(layout.lines()[1].y(), 4.0, "第二行被顶下去");
        assert_eq!(layout.lines()[1].height(), 1.0);
        assert_eq!(layout.height(), 5.0);
        // widget 盒顶 = 行顶 + 行基线 - widget 基线。
        assert_eq!(layout.widgets()[0].bounds().y(), 0.0);
    }

    /// 不变量 D7：资源没就绪时用 placeholder 尺寸，布局照常完成、不报错，
    /// 并且把「哪些还没就绪」报出来——不然没人知道该在资源到位后重排。
    #[test]
    fn a_pending_widget_lays_out_with_its_placeholder_size() {
        let text = "ab";
        let table = Widgets {
            entries: vec![(WidgetId(7), 2.0, 1.0, 1.0, false)],
        };
        let spans = [WidgetSpan::new(
            VisualOffset::new(1),
            WidgetId(7),
            WidgetSide::Before,
        )];
        let layout = build_widgets(text, 80.0, &spans, &table).expect("没就绪也要排出来");
        assert_eq!(layout.lines().len(), 1);
        assert_eq!(layout.widgets()[0].bounds().width(), 2.0);
        assert!(!layout.widgets()[0].is_ready());
        assert_eq!(layout.pending_widgets(), vec![WidgetId(7)]);

        let ready = Widgets {
            entries: vec![(WidgetId(7), 2.0, 1.0, 1.0, true)],
        };
        let layout = build_widgets(text, 80.0, &spans, &ready).expect("布局");
        assert!(layout.pending_widgets().is_empty(), "就绪之后没有待办");
    }

    /// widget 表查不到的 id 是错误，不是零宽的空洞。
    #[test]
    fn unknown_widget_is_an_error() {
        let text = "ab";
        let table = Widgets { entries: vec![] };
        let spans = [WidgetSpan::new(
            VisualOffset::new(1),
            WidgetId(3),
            WidgetSide::Before,
        )];
        assert_eq!(
            build_widgets(text, 80.0, &spans, &table),
            Err(LayoutError::UnknownWidget(WidgetId(3)))
        );
    }

    /// widget 必须按 `(offset, side)` 升序给出（不变量 D6 的定序）。
    /// 乱序不会 panic，只会让同一处的两个 widget 画反。
    #[test]
    fn widgets_must_arrive_in_decoration_order() {
        let text = "abc";
        let table = Widgets {
            entries: vec![
                (WidgetId(1), 1.0, 1.0, 1.0, true),
                (WidgetId(2), 1.0, 1.0, 1.0, true),
            ],
        };
        let backwards = [
            WidgetSpan::new(VisualOffset::new(2), WidgetId(1), WidgetSide::Before),
            WidgetSpan::new(VisualOffset::new(1), WidgetId(2), WidgetSide::Before),
        ];
        assert_eq!(
            build_widgets(text, 80.0, &backwards, &table),
            Err(LayoutError::WidgetsOutOfOrder)
        );
        let side_backwards = [
            WidgetSpan::new(VisualOffset::new(1), WidgetId(1), WidgetSide::After),
            WidgetSpan::new(VisualOffset::new(1), WidgetId(2), WidgetSide::Before),
        ];
        assert_eq!(
            build_widgets(text, 80.0, &side_backwards, &table),
            Err(LayoutError::WidgetsOutOfOrder)
        );
        let ordered = [
            WidgetSpan::new(VisualOffset::new(1), WidgetId(1), WidgetSide::Before),
            WidgetSpan::new(VisualOffset::new(1), WidgetId(2), WidgetSide::After),
        ];
        let layout = build_widgets(text, 80.0, &ordered, &table).expect("布局");
        assert_eq!(layout.widgets()[0].bounds().x(), 1.0);
        assert_eq!(layout.widgets()[1].bounds().x(), 2.0);
    }

    /// widget 也要跟着 bidi 重排走。只搬文字不搬 widget，图片会压在字上。
    #[test]
    fn widgets_follow_the_bidi_reordering() {
        let text = "\u{5e9}\u{5dc}\u{5d5}\u{5dd}";
        let table = Widgets {
            entries: vec![(WidgetId(1), 2.0, 1.0, 1.0, true)],
        };
        // 锚在第一个与第二个希伯来字母之间。逻辑顺序里它排第二，
        // 视觉顺序里就该排倒数第二。
        let spans = [WidgetSpan::new(
            VisualOffset::new(2),
            WidgetId(1),
            WidgetSide::Before,
        )];
        let layout = build_widgets(text, 80.0, &spans, &table).expect("布局");
        assert_eq!(layout.lines()[0].width(), 6.0);
        // 视觉从左到右：\u{5dd}(1) \u{5d5}(1) \u{5dc}(1) widget(2) \u{5e9}(1)
        let xs: Vec<f32> = layout.clusters().iter().map(|c| c.x()).collect();
        assert_eq!(xs, vec![5.0, 2.0, 1.0, 0.0]);
        assert_eq!(
            layout.widgets()[0].bounds().x(),
            3.0,
            "重排前它在 x=1，跟着走才会到 x=3"
        );
    }

    /// 锚点必须落在某个簇的起点或视觉末尾上。
    ///
    /// 落在别处的那个 widget 会被 `place_widgets_at` 的 `while` 静静跳过，
    /// 连同排在它后面的每一个 widget 一起——画面上少了一张图，不 panic、
    /// 不报错。这里用一个组合字符簇的中间那个字节：那是既能过
    /// `validate_widgets`（它只查 char 边界）又落不到任何簇起点上的位置。
    #[test]
    fn a_widget_anchored_off_every_cluster_start_is_rejected() {
        // 组合重音跟着 'a' 合成一个簇，簇的起点只有 0 与 3。
        let text = "a\u{301}b";
        let table = Widgets {
            entries: vec![(WidgetId(1), 1.0, 1.0, 1.0, true)],
        };
        let inside = [WidgetSpan::new(
            VisualOffset::new(1),
            WidgetId(1),
            WidgetSide::Before,
        )];
        assert_eq!(
            build_widgets(text, 80.0, &inside, &table),
            Err(LayoutError::WidgetNotAnchored)
        );
        let on_a_cluster = [WidgetSpan::new(
            VisualOffset::new(3),
            WidgetId(1),
            WidgetSide::Before,
        )];
        assert!(build_widgets(text, 80.0, &on_a_cluster, &table).is_ok());
    }

    /// widget 有宽度，所以它两侧是两个不同的 x，而视觉偏移只有一个。
    ///
    /// 「图片前面」与「图片后面」由 affinity 分。分不出来的话，点在图片右边
    /// 光标会画回它左边去——不报错，只是不听话。
    #[test]
    fn a_widget_has_a_caret_position_on_each_side() {
        let text = "ab";
        let table = Widgets {
            entries: vec![(WidgetId(1), 3.0, 1.0, 1.0, true)],
        };
        let spans = [WidgetSpan::new(
            VisualOffset::new(1),
            WidgetId(1),
            WidgetSide::Before,
        )];
        let layout = build_widgets(text, 80.0, &spans, &table).expect("布局");
        let visual = VisualOffset::new(1);
        let before = layout
            .caret(visual, CaretAffinity::Upstream)
            .expect("caret");
        let after = layout
            .caret(visual, CaretAffinity::Downstream)
            .expect("caret");
        assert_eq!(before.point().x(), 1.0, "'a' 的后沿，也是盒子的左沿");
        assert_eq!(after.point().x(), 4.0, "盒子的右沿，也是 'b' 的前沿");
        assert_eq!(before.widget_affinity(), Some(CaretAffinity::Upstream));
        assert_eq!(after.widget_affinity(), Some(CaretAffinity::Downstream));

        // 点在盒子右半边要落到右沿，而且要说得出「落在右侧」——调用方光看
        // 视觉偏移分不出是哪一沿。
        let hit = layout.hit(LayoutPoint::new(3.5, 0.0)).expect("hit");
        assert_eq!(hit.visual(), visual);
        assert_eq!(hit.point().x(), 4.0);
        assert_eq!(hit.widget_affinity(), Some(CaretAffinity::Downstream));
    }

    /// 整块就是一个 widget 时，两侧的位置仍然点得到。
    ///
    /// 一张图自成一段就是这个形状：视觉文本是空的，一个簇都没有。少了
    /// widget 那两个候选位置，`hit` 会一路退到行首——点图片右边，光标跳到
    /// 图片左边。
    #[test]
    fn a_line_holding_only_a_widget_is_still_hittable_on_both_sides() {
        let table = Widgets {
            entries: vec![(WidgetId(1), 6.0, 1.0, 1.0, true)],
        };
        let spans = [WidgetSpan::new(
            VisualOffset::ZERO,
            WidgetId(1),
            WidgetSide::Before,
        )];
        let layout = build_widgets("", 80.0, &spans, &table).expect("布局");
        assert_eq!(layout.clusters().len(), 0);
        let right = layout.hit(LayoutPoint::new(5.0, 0.0)).expect("hit");
        assert_eq!(right.point().x(), 6.0);
        assert_eq!(right.widget_affinity(), Some(CaretAffinity::Downstream));
        let left = layout.hit(LayoutPoint::new(1.0, 0.0)).expect("hit");
        assert_eq!(left.point().x(), 0.0);
        assert_eq!(left.widget_affinity(), Some(CaretAffinity::Upstream));
    }

    /// widget 的基线必须落在盒子里。越界会让它挂到别的行上去。
    #[test]
    fn widget_baseline_must_lie_inside_the_box() {
        let size = Size::new(2.0, 3.0).expect("有限");
        assert_eq!(
            WidgetMetrics::new(size, 4.0),
            Err(LayoutError::InvalidWidgetBaseline)
        );
        assert_eq!(
            WidgetMetrics::new(size, -1.0),
            Err(LayoutError::InvalidWidgetBaseline)
        );
        assert_eq!(
            WidgetMetrics::new(size, f32::NAN),
            Err(LayoutError::InvalidWidgetBaseline)
        );
        assert_eq!(
            WidgetMetrics::sitting_on_baseline(size)
                .expect("合法")
                .baseline(),
            3.0
        );
    }

    /// UAX #14：断在词之间，不再断在词中间。
    #[test]
    fn wraps_at_break_opportunities_not_inside_words() {
        assert_eq!(line_texts("one two", 5.0), ["one ", "two"]);
        assert_eq!(
            line_texts("re-usable words", 5.0),
            ["re-", "usabl", "e ", "words"]
        );
    }

    /// 行尾空白悬在行外：断行机会在空白**之后**，让空白把整个词挤到下一行
    /// 是错的。代价是这样的行宽度会超过 `max_width`，这是有意的。
    #[test]
    fn trailing_spaces_hang_past_the_line_width() {
        assert_eq!(line_texts("abc   def", 3.0), ["abc   ", "def"]);
        let layout = build("abc   def", &plain("abc   def"), 3.0).expect("布局");
        assert_eq!(layout.lines()[0].width(), 6.0, "三个空白悬在 3.0 宽的行外");

        // 这一条压的是「一段的空白算不算进排不排得下」——上面两条压不到，
        // 因为它们的那一段都是行首第一段，行宽为 0 时怎么算都放得下。
        // "bb " 的空白若参与判断，5 宽的行就会提前断成 ["aa ", "bb cc"]。
        assert_eq!(line_texts("aa bb cc", 5.0), ["aa bb ", "cc"]);
    }

    /// 一个比整行还宽的词仍然必须排得出来：段内退回按 grapheme 应急断行。
    /// 没有这一条，一串长 URL 会把整行撑出画面。
    #[test]
    fn an_over_wide_word_falls_back_to_grapheme_breaks() {
        assert_eq!(line_texts("abcdefgh", 3.0), ["abc", "def", "gh"]);
        assert_eq!(line_texts("don't break", 4.0), ["don'", "t ", "brea", "k"]);
    }

    /// CJK 禁则由 UAX #14 的默认规则覆盖，不另加 tailoring：
    /// 行首不出现 `、`「`」`，行末不出现 `「`。
    #[test]
    fn cjk_line_break_prohibitions_hold() {
        assert_eq!(
            line_texts("「あ」、「い」です。", 4.0),
            ["「あ」、", "「い」で", "す。"]
        );
        assert_eq!(
            line_texts("Hello, 世界！ ok", 4.0),
            ["Hell", "o, 世", "界！ ", "ok"]
        );
        // 表意文字之间可以自由断行。
        assert_eq!(
            line_texts("中文换行测试文字", 3.0),
            ["中文换", "行测试", "文字"]
        );
    }

    /// 窄到一行放不下「开括号 + 一个字」时只能应急断行，`、` 会落到行首。
    /// 记下来是因为它看起来像禁则失效，其实是宽度不够——UAX #14 允许在
    /// 没有任何合法断点时断在 grapheme 边界。
    #[test]
    fn prohibitions_yield_to_emergency_breaks_when_nothing_fits() {
        assert_eq!(
            line_texts("「あ」、「い」です。", 3.0),
            ["「あ」", "、", "「い」", "です。"]
        );
    }

    /// UAX #14 的强制换行不只有 LF。这与 `docs/specs/coordinates.md` 的
    /// 「源码逻辑行只按 LF 分隔」不冲突：那条说的是 `LineIndex`（源码行），
    /// 这里是**视觉行**，软换行早就让两者不是一回事了。
    #[test]
    fn mandatory_breaks_cover_the_whole_uax14_set() {
        for text in [
            "a\nb",
            "a\r\nb",
            "a\rb",
            "a\u{b}b",
            "a\u{c}b",
            "a\u{85}b",
            "a\u{2028}b",
            "a\u{2029}b",
        ] {
            let layout = build(text, &plain(text), 80.0).expect("布局");
            assert_eq!(layout.lines().len(), 2, "{text:?} 应当强制换成两行");
            let breaks: Vec<_> = layout
                .clusters()
                .iter()
                .filter(|cluster| cluster.is_line_break())
                .collect();
            assert_eq!(breaks.len(), 1, "{text:?} 的换行簇");
            assert_eq!(breaks[0].width(), 0.0, "{text:?} 的换行簇不占宽度");
        }
    }

    /// 断行不得把一个 grapheme 劈开。ZWJ 序列是最容易被劈的一类：
    /// 劈开之后画出来是两个不相干的 emoji，不 panic、不报错。
    #[test]
    fn breaking_never_splits_a_grapheme() {
        let text = "\u{1f469}\u{200d}\u{1f4bb}\u{1f469}\u{200d}\u{1f4bb}";
        let layout = build(text, &plain(text), 1.0).expect("布局");
        assert_eq!(layout.clusters().len(), 2);
        assert_eq!(layout.lines().len(), 2);
        for line in layout.lines() {
            let from = line.visual().start().get() as usize;
            let to = line.visual().end().get() as usize;
            assert!(
                text[from..to].chars().count() == 3,
                "每一行应当正好是一个完整的 ZWJ 序列"
            );
        }
    }

    fn build_with(text: &str, width: f32, direction: BaseDirection) -> BlockLayout {
        BlockLayout::build(
            LayoutInput::new(text, &plain(text)),
            LayoutConfig::new(width, 1.0).with_base_direction(direction),
            &UniformStyleTable::default(),
            &MonospaceMetrics::default(),
        )
        .expect("布局")
    }

    /// 一行里每个 grapheme 占的 x 区间，按视觉从左到右排。
    fn visual_order(layout: &BlockLayout, text: &str, line: usize) -> Vec<String> {
        let mut boxes: Vec<_> = layout.lines()[line]
            .cluster_range()
            .map(|index| layout.clusters()[index])
            .filter(|cluster| !cluster.is_line_break())
            .collect();
        boxes.sort_by(|a, b| a.x().partial_cmp(&b.x()).expect("有限"));
        boxes
            .iter()
            .map(|cluster| {
                let from = cluster.visual().start().get() as usize;
                let to = cluster.visual().end().get() as usize;
                text[from..to].to_owned()
            })
            .collect()
    }

    /// UAX #9 的 L2：RTL 段落里的字从右往左排。
    #[test]
    fn rtl_text_is_reordered_right_to_left() {
        let text = "\u{5e9}\u{5dc}\u{5d5}\u{5dd}";
        let layout = build_with(text, 80.0, BaseDirection::Auto);
        assert!(layout.clusters().iter().all(|cluster| cluster.is_rtl()));
        assert_eq!(
            visual_order(&layout, text, 0),
            ["\u{5dd}", "\u{5d5}", "\u{5dc}", "\u{5e9}"],
            "视觉从左到右读到的应当是逻辑顺序的倒序"
        );
    }

    /// LTR 段落里嵌一段 RTL：只有那一段就地翻转，两侧的拉丁字母不动。
    #[test]
    fn an_rtl_run_inside_ltr_flips_in_place() {
        let text = "abc \u{5e9}\u{5dc}\u{5d5}\u{5dd} def";
        let layout = build_with(text, 80.0, BaseDirection::Auto);
        assert_eq!(
            visual_order(&layout, text, 0),
            [
                "a", "b", "c", " ", "\u{5dd}", "\u{5d5}", "\u{5dc}", "\u{5e9}", " ", "d", "e", "f"
            ]
        );
    }

    /// RTL 段落里嵌一段 LTR：整行翻转，而那一段拉丁字母内部仍然从左往右。
    /// 层级 2 是 UAX #9 的 I1——搞成 1 会把 "abc" 画成 "cba"。
    #[test]
    fn an_ltr_run_inside_rtl_keeps_its_own_order() {
        let text = "\u{5e9}\u{5dc} abc \u{5d5}\u{5dd}";
        let layout = build_with(text, 80.0, BaseDirection::Auto);
        assert_eq!(
            visual_order(&layout, text, 0),
            [
                "\u{5dd}", "\u{5d5}", " ", "a", "b", "c", " ", "\u{5dc}", "\u{5e9}"
            ]
        );
        let latin: Vec<u8> = layout
            .clusters()
            .iter()
            .filter(|cluster| {
                let from = cluster.visual().start().get() as usize;
                text[from..].starts_with(['a', 'b', 'c'])
            })
            .map(|cluster| cluster.level())
            .collect();
        assert_eq!(latin, [2, 2, 2]);
    }

    /// 基准方向可以被显式指定，不必等内容里出现强方向字符。
    /// 纯拉丁文本在 RTL 段落里层级是 2，不是 0。
    #[test]
    fn base_direction_overrides_the_p2_p3_guess() {
        let auto = build_with("abc", 80.0, BaseDirection::Auto);
        let forced = build_with("abc", 80.0, BaseDirection::Rtl);
        assert!(auto.clusters().iter().all(|cluster| cluster.level() == 0));
        assert!(forced.clusters().iter().all(|cluster| cluster.level() == 2));
    }

    /// RTL 上下文里的数字仍然从左往右（UAX #9 的 W2/I1 把它们放到层级 2）。
    /// 反过来会把电话号码和年份画反，而且不报错。
    #[test]
    fn european_numbers_stay_ltr_inside_rtl() {
        let text = "\u{5e9}\u{5dc} 12";
        let layout = build_with(text, 80.0, BaseDirection::Auto);
        assert_eq!(
            visual_order(&layout, text, 0),
            ["1", "2", " ", "\u{5dc}", "\u{5e9}"]
        );
    }

    /// 重排只换位置，不改宽度：每一行的 grapheme 必须严丝合缝地铺满
    /// `[0, line.width)`，既不重叠也不留缝。一个错位的重排会让两个字画在
    /// 同一处，而它不 panic、不报错。
    #[test]
    fn reordering_tiles_every_line_exactly_once() {
        let cases = [
            ("\u{5e9}\u{5dc}\u{5d5}\u{5dd} abc \u{5d0}\u{5d1} 12 xy", 5.0),
            (
                "\u{5e9}\u{5dc}\u{5d5}\u{5dd} abc \u{5d0}\u{5d1} 12 xy",
                80.0,
            ),
            ("abc \u{5e9}\u{5dc}\n\u{5d5}\u{5dd} def", 4.0),
            (
                "\u{5e9}\u{5dc}\u{5d5}\u{5dd}\u{5e9}\u{5dc}\u{5d5}\u{5dd}",
                3.0,
            ),
        ];
        for (text, width) in cases {
            let layout = build_with(text, width, BaseDirection::Auto);
            for line in layout.lines() {
                let mut spans: Vec<(f32, f32)> = line
                    .cluster_range()
                    .map(|index| layout.clusters()[index])
                    .filter(|cluster| !cluster.is_line_break())
                    .map(|cluster| (cluster.x(), cluster.x() + cluster.width()))
                    .collect();
                spans.sort_by(|a, b| a.0.partial_cmp(&b.0).expect("有限"));
                let mut expected = 0.0_f32;
                for (from, to) in &spans {
                    assert_eq!(
                        *from,
                        expected,
                        "{text:?} @ {width} 第 {} 行有重叠或空隙",
                        line.index()
                    );
                    expected = *to;
                }
                assert_eq!(
                    expected,
                    line.width(),
                    "{text:?} @ {width} 第 {} 行铺出来的宽度与行宽不符",
                    line.index()
                );
            }
        }
    }

    /// UAX #9 的 L1 是**逐行**的：行尾的空白要重置成段落层级。
    ///
    /// LTR 段落里 `"abc \u{5e9}\u{5dc} \u{5d3}\u{5d1} def"` 的两个希伯来词
    /// 之间那个空格层级是 1，跟着 RTL 一起翻转；可一旦它落在**行尾**，
    /// L1 把它重置回层级 0，于是它排到行的最右边而不是留在翻转的那一段里。
    /// 把整段一次性重排（而不是逐行）就会漏掉这一步，空格会画在词的左边。
    #[test]
    fn line_trailing_whitespace_is_reset_to_the_paragraph_level() {
        let text = "abc \u{5e9}\u{5dc} \u{5d3}\u{5d1} def";
        let layout = build_with(text, 6.0, BaseDirection::Auto);
        assert_eq!(layout.lines().len(), 2);
        assert_eq!(
            visual_order(&layout, text, 0),
            ["a", "b", "c", " ", "\u{5dc}", "\u{5e9}", " "],
            "行尾空格排在最右，不在翻转的那一段里"
        );
        assert_eq!(
            visual_order(&layout, text, 1),
            ["\u{5d1}", "\u{5d3}", " ", "d", "e", "f"]
        );
    }

    /// 重排是**逐行**做的（UAX #9 的 L1/L2 在断行之后）。整块一起翻转会让
    /// 第二行的内容跑到第一行的位置上。
    #[test]
    fn reordering_happens_per_visual_line() {
        let text = "\u{5e9}\u{5dc}\u{5d5}\u{5dd}\u{5d0}\u{5d1}";
        let layout = build_with(text, 3.0, BaseDirection::Auto);
        assert_eq!(layout.lines().len(), 2);
        // 第一行是逻辑上前三个字，倒着排；第二行是后三个，也倒着排。
        assert_eq!(
            visual_order(&layout, text, 0),
            ["\u{5d5}", "\u{5dc}", "\u{5e9}"]
        );
        assert_eq!(
            visual_order(&layout, text, 1),
            ["\u{5d1}", "\u{5d0}", "\u{5dd}"]
        );
    }

    /// hit-test 取最近的一条 grapheme 边缘。重排之后簇的 x 不再随逻辑顺序
    /// 递增，「从左往右扫到第一个越过中点的」会给出错的答案。
    ///
    /// 方向变化处两条边缘**落在同一个 x 上**（前一段的后沿与后一段的前沿）。
    /// 这时取逻辑上在前的那一个，与 [`BlockLayout::caret`] 的选择一致——
    /// 两边不一致的话，点一下光标就会跳到另一处。这条用例压的正是这个一致性。
    #[test]
    fn hit_test_finds_the_nearest_edge_after_reordering() {
        let text = "abc \u{5e9}\u{5dc}\u{5d5}\u{5dd} def";
        let layout = build_with(text, 80.0, BaseDirection::Auto);
        let mut checked = 0_usize;
        for cluster in layout.clusters() {
            for x in [cluster.x(), cluster.x() + cluster.width()] {
                let hit = layout.hit(LayoutPoint::new(x, 0.0)).expect("hit");
                assert_eq!(hit.point().x(), x, "命中点应当落在那条边上");
                let caret = layout
                    .caret(hit.visual(), CaretAffinity::Downstream)
                    .expect("caret");
                assert_eq!(
                    caret.point(),
                    hit.point(),
                    "x={x} 命中到 {:?} 之后，caret 应当回到同一处",
                    hit.visual()
                );
                checked += 1;
            }
        }
        assert_eq!(checked, 24);

        // 不含歧义的一条：RTL 段落中间某个字的左边缘是它逻辑上的**终点**。
        let hit = layout.hit(LayoutPoint::new(6.0, 0.0)).expect("hit");
        assert_eq!(hit.visual(), VisualOffset::new(8));
    }

    /// 方向变化处两侧各有一个几何位置。取层级更低（更接近段落基准方向）
    /// 的那一侧之后，两个位置分别归属两个不同的偏移，都够得着。
    ///
    /// 换成「一律取逻辑上在前那个的后沿」会让两个偏移都落在同一处，
    /// 于是 RTL 段落右端那个位置画不出来也点不到——不 panic，只是光标不听话。
    #[test]
    fn caret_at_a_direction_boundary_follows_the_base_direction_side() {
        let text = "abc \u{5e9}\u{5dc}\u{5d5}\u{5dd} def";
        let layout = build_with(text, 80.0, BaseDirection::Auto);
        // 偏移 4：左边是层级 0 的空格，右边是层级 1 的 \u{5e9}。取层级低的
        // 那侧，也就是空格的后沿。
        assert_eq!(
            layout
                .caret(VisualOffset::new(4), CaretAffinity::Downstream)
                .expect("caret")
                .point(),
            LayoutPoint::new(4.0, 0.0)
        );
        // 偏移 12：左边是层级 1 的 \u{5dd}，右边是层级 0 的空格。同样取层级低
        // 的那侧——空格的前沿，也就是希伯来文那一段的右端。
        assert_eq!(
            layout
                .caret(VisualOffset::new(12), CaretAffinity::Downstream)
                .expect("caret")
                .point(),
            LayoutPoint::new(8.0, 0.0)
        );
    }

    /// RTL 行的行首 caret 在**右**边缘。这是「前进方向上的起点」与
    /// 「左边缘」不是一回事的最小例子——两者搞混不 panic，只是光标画在
    /// 整段文字的另一头。
    #[test]
    fn caret_at_the_start_of_an_rtl_line_sits_on_the_right() {
        let text = "\u{5e9}\u{5dc}\u{5d5}\u{5dd}";
        let layout = build_with(text, 80.0, BaseDirection::Auto);
        assert_eq!(layout.lines()[0].width(), 4.0);
        let start = layout
            .caret(VisualOffset::ZERO, CaretAffinity::Downstream)
            .expect("caret");
        assert_eq!(start.point(), LayoutPoint::new(4.0, 0.0), "逻辑起点在最右");
        let end = layout
            .caret(VisualOffset::new(8), CaretAffinity::Downstream)
            .expect("caret");
        assert_eq!(end.point(), LayoutPoint::new(0.0, 0.0), "逻辑终点在最左");
    }

    /// caret 与 hit-test 必须用同一条规则。分别实现一遍，在方向变化处就会
    /// 对不上：点一下，光标跳到别处。
    ///
    /// 软换行边界上的偏移在两行各有一个位置，由 affinity 选。所以这里要求的
    /// 是**存在**一个 affinity，让 caret 回到 hit 给出的那一行的同一个 x——
    /// 调用方本来就该用 hit 返回的行去定 affinity。
    #[test]
    fn every_caret_position_is_reachable_by_hit_test() {
        for (text, width) in [
            ("abc \u{5e9}\u{5dc}\u{5d5}\u{5dd} def", 80.0),
            ("\u{5e9}\u{5dc} abc \u{5d5}\u{5dd}", 80.0),
            ("\u{5e9}\u{5dc}\u{5d5}\u{5dd} 12 abc", 5.0),
            ("abc \u{5e9}\u{5dc}\n\u{5d5}\u{5dd} def", 4.0),
        ] {
            let layout = build_with(text, width, BaseDirection::Auto);
            for line in layout.lines() {
                for position in layout.caret_positions(line) {
                    let (x, offset) = (position.x, position.visual);
                    let hit = layout.hit(LayoutPoint::new(x, line.y())).expect("hit");
                    assert_eq!(
                        hit.point(),
                        LayoutPoint::new(x, line.y()),
                        "{text:?} @ {width} 的 caret 位置 x={x}（偏移 {offset:?}）点不回原处"
                    );
                    let reachable = [CaretAffinity::Upstream, CaretAffinity::Downstream]
                        .into_iter()
                        .any(|affinity| {
                            let caret = layout.caret(hit.visual(), affinity).expect("caret");
                            caret.line() == hit.line() && caret.point() == hit.point()
                        });
                    assert!(
                        reachable,
                        "{text:?} @ {width} 命中到 {:?} 之后，两种 affinity 都回不到 {:?}",
                        hit.visual(),
                        hit.point()
                    );
                }
            }
        }
    }

    /// grapheme 内部的视觉偏移不是合法 caret，取所在 grapheme 的左边缘，
    /// 不得落在字中间。
    #[test]
    fn offsets_inside_a_grapheme_snap_to_its_left_edge() {
        let text = "中文";
        let layout = build(text, &plain(text), 80.0).expect("布局");
        let inside = layout
            .caret(VisualOffset::new(1), CaretAffinity::Downstream)
            .expect("caret");
        assert_eq!(inside.point(), LayoutPoint::new(0.0, 0.0));
        let boundary = layout
            .caret(VisualOffset::new(3), CaretAffinity::Downstream)
            .expect("caret");
        assert_eq!(boundary.point(), LayoutPoint::new(1.0, 0.0));
    }
}
