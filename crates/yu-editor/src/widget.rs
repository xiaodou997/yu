//! 把 `BlockWidget` 翻译成一个盒子。
//!
//! # 为什么这个模块住在 `yu-editor`
//!
//! 与 [`crate::blockinput`] 同一个理由：`yu-layout` 只认识
//! [`WidgetId`] 与「这个盒子多宽多高」，翻译的活儿得有人干，干活的必须是一
//! 个允许认识 Markdown 的层。图片的固有尺寸从哪来（解码、缓存、失败重试）
//! 又在这一层之上——那是 workspace 的事，这里只收一张已经算好的表。
//!
//! # 没就绪时给 placeholder，不阻塞
//!
//! 不变量 D7。资源没到位时 [`WidgetMeasurement::Placeholder`] 给一个固定
//! 大小的盒子，布局照常完成，[`yu_layout::BlockLayout::pending_widgets`]
//! 报出还欠着谁；资源到位之后这个块重排一次（见 [`crate::LayoutCache`]）。
//! 盒子大小**不看替代文字**：替代文字已经被 widget 藏起来了，让盒子随它
//! 变宽会让同一张图在加载前后跳两次。

use yu_core::{Size, TextRange, WidgetId};
use yu_layout::{
    ImageIntrinsicSize, LayoutConfig, WidgetConstraints, WidgetMeasure, WidgetMeasurement,
    WidgetMetrics,
};
use yu_markdown::{BlockWidget, ImageSpan};

/// 资源没就绪时盒子有几个行高宽。
const PLACEHOLDER_WIDTH_IN_LINES: f32 = 4.0;

/// 一张已经解码到位的图片：它那段 Markdown，以及解码出来的像素尺寸。
///
/// 按**源码区间**索引而不是按目标 URL：同一个 URL 在一篇文档里可以出现
/// 多次，而 widget 是按位置排的。解析 URL 是 workspace 的事，这里不重复。
pub type ImageSize = (TextRange, ImageIntrinsicSize);

/// 一个块上的 widget 尺寸来源。
///
/// `sizes` 空表示「一张都还没就绪」——那是所有不关心图片的调用方（命中
/// 测试、Accessibility、纯度量排版）走的那条路。
#[derive(Clone, Copy, Debug)]
pub(crate) struct BlockWidgets<'a> {
    widgets: &'a [BlockWidget],
    sizes: &'a [ImageSize],
}

impl<'a> BlockWidgets<'a> {
    pub(crate) const fn new(widgets: &'a [BlockWidget], sizes: &'a [ImageSize]) -> Self {
        Self { widgets, sizes }
    }

    fn image(&self, widget: WidgetId) -> Option<ImageSpan> {
        let index = usize::try_from(widget.0).ok()?;
        let BlockWidget::Image(image) = self.widgets.get(index).copied()?;
        Some(image)
    }

    fn size_of(&self, image: ImageSpan) -> Option<ImageIntrinsicSize> {
        self.sizes
            .iter()
            .find(|(source, _)| *source == image.source())
            .map(|(_, size)| *size)
    }
}

impl WidgetMeasure for BlockWidgets<'_> {
    /// 查不到的 id 返回 `None`，布局报 `LayoutError::UnknownWidget`。
    /// 「装饰产出了一个没人认识的 widget」应该响，不该悄悄画成一个零宽的洞。
    fn measure(
        &self,
        widget: WidgetId,
        constraints: WidgetConstraints,
    ) -> Option<WidgetMeasurement> {
        let image = self.image(widget)?;
        match self.size_of(image) {
            Some(size) => intrinsic_metrics(size, constraints).map(WidgetMeasurement::Ready),
            None => placeholder_metrics(constraints).map(WidgetMeasurement::Placeholder),
        }
    }
}

/// 保持长宽比缩到可用宽度以内。
///
/// 可用宽度是**整块**的宽度，不是这一行剩下的宽度：剩余宽度取决于盒子排在
/// 哪，而排在哪要等宽度定了才知道。排不下时 `BlockLayout` 会先断行。
fn intrinsic_metrics(
    size: ImageIntrinsicSize,
    constraints: WidgetConstraints,
) -> Option<WidgetMetrics> {
    let intrinsic_width = size.width() as f32;
    let intrinsic_height = size.height() as f32;
    let available = constraints.available_width().max(1.0);
    let scale = (available / intrinsic_width).min(1.0);
    let width = (intrinsic_width * scale).max(1.0);
    let height = (intrinsic_height * scale).max(constraints.line_height());
    WidgetMetrics::sitting_on_baseline(Size::new(width, height).ok()?).ok()
}

fn placeholder_metrics(constraints: WidgetConstraints) -> Option<WidgetMetrics> {
    let width = (constraints.line_height() * PLACEHOLDER_WIDTH_IN_LINES)
        .min(constraints.available_width())
        .max(1.0);
    let size = Size::new(width, constraints.line_height()).ok()?;
    WidgetMetrics::sitting_on_baseline(size).ok()
}

/// 布局里量 widget 用的那一份约束。
///
/// 缓存要判断「资源到位了没有」，判断的依据必须与布局当时用的**同一份**
/// 约束——各算各的会让「量出来一样大」在边界上不成立，而不成立的表现是
/// 图片永远重排或者永远不重排。
pub(crate) fn constraints_of(config: LayoutConfig) -> WidgetConstraints {
    WidgetConstraints::new(config.max_width(), config.line_height())
}
