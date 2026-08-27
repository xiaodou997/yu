#![forbid(unsafe_code)]

//! Source-backed block layout contracts.
//!
//! This crate deliberately stops before font shaping and GPU painting. It
//! turns a projection into visual lines and grapheme clusters using a
//! replaceable advance provider, then exposes deterministic caret and hit-test
//! mappings for the future platform/layout layers.

use std::error::Error;
use std::fmt;

use yu_core::{
    ClusterMetrics, GeometryError, LineStyleId, StyleId, TextStyle, VisualOffset, WidgetId,
};

mod block;

pub use block::{
    BlockLayout, CaretBox, ClusterBox, GlyphBox, LayoutInput, LineAttrs, LineBox, LineSpan,
    LineStyleTable, NoLineStyles, NoWidgets, StyleTable, StyledRun, UniformStyleTable, WidgetBox,
    WidgetConstraints, WidgetMeasure, WidgetMeasurement, WidgetMetrics, WidgetSpan,
};

/// Layout dimensions and wrapping policy independent of any font backend.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LayoutConfig {
    max_width: f32,
    line_height: f32,
    default_advance: f32,
    base_direction: BaseDirection,
}

/// 段落的基准方向（UAX #9 的 P2/P3）。
///
/// 只有 [`BlockLayout`] 读它。`LayoutSnapshot`（v1）没有 bidi，忽略这个字段。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum BaseDirection {
    /// 按 UAX #9 的 P2/P3 从内容推断：取第一个强方向字符。
    #[default]
    Auto,
    Ltr,
    Rtl,
}

impl LayoutConfig {
    #[must_use]
    pub const fn new(max_width: f32, line_height: f32) -> Self {
        Self {
            max_width,
            line_height,
            default_advance: 1.0,
            base_direction: BaseDirection::Auto,
        }
    }

    /// 覆盖段落基准方向。默认按内容推断。
    #[must_use]
    pub const fn with_base_direction(mut self, base_direction: BaseDirection) -> Self {
        self.base_direction = base_direction;
        self
    }

    #[must_use]
    pub const fn base_direction(self) -> BaseDirection {
        self.base_direction
    }

    /// Returns a copy using the fallback advance for metrics-only layout.
    ///
    /// Shaped layout providers can replace this value per glyph run; the
    /// fallback is used by the deterministic metrics backend and by the FFI
    /// viewport contract before a native shaper is attached.
    #[must_use]
    pub const fn with_default_advance(mut self, default_advance: f32) -> Self {
        self.default_advance = default_advance;
        self
    }

    #[must_use]
    pub const fn max_width(self) -> f32 {
        self.max_width
    }

    #[must_use]
    pub const fn line_height(self) -> f32 {
        self.line_height
    }

    #[must_use]
    pub const fn default_advance(self) -> f32 {
        self.default_advance
    }

    /// 拒绝非有限或非正的尺寸。
    ///
    /// 一个 NaN 宽度在布局里会一路传播成不 panic 的错画面，那是本项目最
    /// 危险的失败模式。配置从调用方来，所以入口先验一遍。
    pub fn validate(self) -> Result<(), LayoutError> {
        if !self.max_width.is_finite() || self.max_width <= 0.0 {
            return Err(LayoutError::InvalidConfig(
                "max_width must be finite and positive",
            ));
        }
        if !self.line_height.is_finite() || self.line_height <= 0.0 {
            return Err(LayoutError::InvalidConfig(
                "line_height must be finite and positive",
            ));
        }
        if !self.default_advance.is_finite() || self.default_advance <= 0.0 {
            return Err(LayoutError::InvalidConfig(
                "default_advance must be finite and positive",
            ));
        }
        Ok(())
    }
}

impl Default for LayoutConfig {
    fn default() -> Self {
        Self::new(80.0, 1.0)
    }
}

/// Deterministic metrics used before a font/shaping backend exists.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MonospaceMetrics {
    advance: f32,
}

impl MonospaceMetrics {
    #[must_use]
    pub const fn new(advance: f32) -> Self {
        Self { advance }
    }

    #[must_use]
    pub const fn advance(self) -> f32 {
        self.advance
    }
}

impl Default for MonospaceMetrics {
    fn default() -> Self {
        Self::new(1.0)
    }
}

impl ClusterMetrics for MonospaceMetrics {
    fn advance(&self, _cluster: &str, _style: TextStyle) -> f32 {
        self.advance
    }
}

/// Errors raised while constructing or querying a layout snapshot.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LayoutError {
    /// 装配布局输入的那一层报了错。
    ///
    /// 布局层不认识它的错误类型——那一层在依赖图的上面（不变量 E2），
    /// 这里只带走说明文字。与 [`LayoutError::Shaping`] 同一个理由：跨层
    /// 的错误只能按 `Display` 搬运，够定位，不够反解。
    Upstream(String),
    Geometry(GeometryError),
    InvalidConfig(&'static str),
    InvalidMetrics(u32),
    Shaping(String),
    InvalidPoint,
    InvalidImageBounds,
    OffsetOverflow,
    /// 样式表里没有这个 id。装饰产出与样式表脱节时必须响，不能按默认字型排。
    UnknownStyle(StyleId),
    /// [`StyledRun`] 没有无缝铺满视觉文本。
    RunsNotContiguous {
        expected: VisualOffset,
        found: VisualOffset,
    },
    /// run 的边界不在 UTF-8 字符边界上。
    RunNotOnCharBoundary,
    /// 查询用的视觉偏移超出了这个块。
    VisualOutOfBounds(VisualOffset),
    /// widget 表里没有这个 id。
    UnknownWidget(WidgetId),
    /// widget 的基线不在 `[0, height]` 里。
    InvalidWidgetBaseline,
    /// widget 的尺寸不是有限值。
    InvalidWidgetSize,
    /// widget 没有按 `(from, side)` 升序给出（不变量 D6 的定序）。
    WidgetsOutOfOrder,
    /// widget 的锚点不在任何一个簇的起点上，也不在视觉末尾。
    WidgetNotAnchored,
    /// 行样式表里没有这个 id。
    UnknownLineStyle(LineStyleId),
    /// 行级属性里有非有限或非正的值。
    InvalidLineStyle,
    /// 行级样式段没有升序、互不重叠地给出。
    LineStylesOutOfOrder,
}

/// Errors raised by the viewport height index.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HeightIndexError {
    InvalidHeight(u32),
    OutOfBounds { index: usize, len: usize },
}

impl fmt::Display for HeightIndexError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidHeight(height) => {
                write!(formatter, "invalid line height {}", f32::from_bits(*height))
            }
            Self::OutOfBounds { index, len } => {
                write!(formatter, "height index {index} is outside {len} entries")
            }
        }
    }
}

impl Error for HeightIndexError {}

/// A Fenwick-tree index over variable visual line heights.
///
/// Prefix queries and point updates are O(log n); `find_line` locates the line
/// containing a viewport y coordinate without laying out off-screen blocks.
#[derive(Clone, Debug, PartialEq)]
pub struct HeightIndex {
    values: Vec<f32>,
    tree: Vec<f32>,
}

impl Default for HeightIndex {
    fn default() -> Self {
        Self {
            values: Vec::new(),
            tree: vec![0.0],
        }
    }
}

impl HeightIndex {
    pub fn new(heights: impl IntoIterator<Item = f32>) -> Result<Self, HeightIndexError> {
        let values = heights.into_iter().collect::<Vec<_>>();
        for height in &values {
            validate_height(*height)?;
        }
        let mut index = Self {
            tree: vec![0.0; values.len().saturating_add(1)],
            values,
        };
        let values = index.values.clone();
        for (position, height) in values.into_iter().enumerate() {
            index.add(position, height);
        }
        Ok(index)
    }

    pub fn uniform(count: usize, height: f32) -> Result<Self, HeightIndexError> {
        Self::new(std::iter::repeat_n(height, count))
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.values.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.values.is_empty()
    }

    #[must_use]
    pub fn height(&self, index: usize) -> Option<f32> {
        self.values.get(index).copied()
    }

    #[must_use]
    pub fn total_height(&self) -> f32 {
        self.prefix_height(self.values.len())
    }

    #[must_use]
    pub fn prefix_height(&self, end: usize) -> f32 {
        let mut position = end.min(self.values.len());
        let mut total = 0.0;
        while position > 0 {
            total += self.tree[position];
            position &= position - 1;
        }
        total
    }

    pub fn set(&mut self, index: usize, height: f32) -> Result<(), HeightIndexError> {
        let Some(previous) = self.values.get_mut(index) else {
            return Err(HeightIndexError::OutOfBounds {
                index,
                len: self.values.len(),
            });
        };
        validate_height(height)?;
        let delta = height - *previous;
        *previous = height;
        self.add(index, delta);
        Ok(())
    }

    pub fn push(&mut self, height: f32) -> Result<(), HeightIndexError> {
        validate_height(height)?;
        let index = self.values.len();
        let position = index.saturating_add(1);
        let low_bit = position & position.wrapping_neg();
        let existing =
            self.prefix_height(index) - self.prefix_height(position.saturating_sub(low_bit));
        self.values.push(height);
        self.tree.push(existing + height);
        Ok(())
    }

    #[must_use]
    pub fn find_line(&self, y: f32) -> Option<usize> {
        if self.values.is_empty() || !y.is_finite() {
            return None;
        }
        if y <= 0.0 {
            return Some(0);
        }
        let total = self.total_height();
        if y >= total {
            return Some(self.values.len().saturating_sub(1));
        }

        let mut position = 0_usize;
        let mut accumulated = 0.0_f32;
        let mut step = 1_usize;
        while step < self.values.len() {
            step <<= 1;
        }
        while step > 0 {
            let candidate = position.saturating_add(step);
            if candidate <= self.values.len() && accumulated + self.tree[candidate] <= y {
                accumulated += self.tree[candidate];
                position = candidate;
            }
            step >>= 1;
        }
        Some(position.min(self.values.len().saturating_sub(1)))
    }

    fn add(&mut self, index: usize, delta: f32) {
        let mut position = index.saturating_add(1);
        while position < self.tree.len() {
            self.tree[position] += delta;
            position = position.saturating_add(position & position.wrapping_neg());
        }
    }
}

fn validate_height(height: f32) -> Result<(), HeightIndexError> {
    if height.is_finite() && height >= 0.0 {
        Ok(())
    } else {
        Err(HeightIndexError::InvalidHeight(height.to_bits()))
    }
}

impl fmt::Display for LayoutError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Upstream(message) => formatter.write_str(message),
            Self::Geometry(error) => error.fmt(formatter),
            Self::InvalidConfig(message) => formatter.write_str(message),
            Self::InvalidMetrics(advance) => write!(
                formatter,
                "invalid cluster advance {}",
                f32::from_bits(*advance)
            ),
            Self::Shaping(message) => write!(formatter, "shaping failed: {message}"),
            Self::InvalidPoint => {
                formatter.write_str("layout point must contain finite coordinates")
            }
            Self::InvalidImageBounds => {
                formatter.write_str("image bounds must be finite and have positive height")
            }
            Self::OffsetOverflow => formatter.write_str("layout offset overflow"),
            Self::UnknownStyle(style) => {
                write!(formatter, "style table has no entry for {style:?}")
            }
            Self::RunsNotContiguous { expected, found } => write!(
                formatter,
                "styled runs must tile the visual text: expected {expected:?}, found {found:?}"
            ),
            Self::RunNotOnCharBoundary => {
                formatter.write_str("styled run boundary is not on a UTF-8 char boundary")
            }
            Self::VisualOutOfBounds(visual) => {
                write!(formatter, "visual offset {visual:?} is outside this block")
            }
            Self::UnknownWidget(widget) => {
                write!(formatter, "widget table has no entry for {widget:?}")
            }
            Self::InvalidWidgetBaseline => {
                formatter.write_str("widget baseline must lie between zero and its height")
            }
            Self::InvalidWidgetSize => formatter.write_str("widget size must be finite"),
            Self::WidgetsOutOfOrder => {
                formatter.write_str("widgets must be ordered by (offset, side)")
            }
            Self::WidgetNotAnchored => {
                formatter.write_str("every widget must be anchored on a cluster start")
            }
            Self::UnknownLineStyle(style) => {
                write!(formatter, "line style table has no entry for {style:?}")
            }
            Self::InvalidLineStyle => formatter.write_str(
                "line indent must be finite and non-negative, and the line height scale positive",
            ),
            Self::LineStylesOutOfOrder => {
                formatter.write_str("line styles must be ordered and non-overlapping")
            }
        }
    }
}

impl Error for LayoutError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Geometry(error) => Some(error),
            Self::Upstream(_)
            | Self::InvalidConfig(_)
            | Self::InvalidMetrics(_)
            | Self::Shaping(_)
            | Self::InvalidPoint
            | Self::InvalidImageBounds
            | Self::OffsetOverflow
            | Self::UnknownStyle(_)
            | Self::RunsNotContiguous { .. }
            | Self::RunNotOnCharBoundary
            | Self::VisualOutOfBounds(_)
            | Self::UnknownWidget(_)
            | Self::InvalidWidgetBaseline
            | Self::InvalidWidgetSize
            | Self::WidgetsOutOfOrder
            | Self::WidgetNotAnchored
            | Self::UnknownLineStyle(_)
            | Self::InvalidLineStyle
            | Self::LineStylesOutOfOrder => None,
        }
    }
}

impl From<GeometryError> for LayoutError {
    fn from(error: GeometryError) -> Self {
        Self::Geometry(error)
    }
}

/// Block 局部坐标系里的点与矩形。
///
/// 实现在 `yu-core`，空间是 [`yu_core::Block`]。用别名而不是自己再写一份：
/// 算术只写一遍，而「这是 block 局部坐标不是文档坐标」由类型参数带着走——
/// 把它传给要 [`yu_core::Document`] 坐标的函数是编译错误，不是画错。
pub type LayoutPoint = yu_core::Point<yu_core::Block>;

/// Block 局部坐标系里的矩形。见 [`LayoutPoint`]。
///
/// `yu_core::Block` 自带 `x >= 0 && y >= 0 && height > 0` 的约束，与这里
/// 原来手写的校验一致。
pub type LayoutRect = yu_core::Rect<yu_core::Block>;

/// 一张图片解码之后自身的像素尺寸。
///
/// 布局层拿它只做一件事：算出一个保持长宽比的有限矩形。像素与资源身份都
/// 在这个 crate 之外。它是整数，也不落在任何视觉坐标空间里——
/// `tools/check-geometry.py` 里登记的例外说的就是这一条。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ImageIntrinsicSize {
    width: u32,
    height: u32,
}

impl ImageIntrinsicSize {
    pub fn new(width: u32, height: u32) -> Result<Self, LayoutError> {
        if width == 0 || height == 0 {
            return Err(LayoutError::InvalidImageBounds);
        }
        Ok(Self { width, height })
    }

    #[must_use]
    pub const fn width(self) -> u32 {
        self.width
    }

    #[must_use]
    pub const fn height(self) -> u32 {
        self.height
    }
}
