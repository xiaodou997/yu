//! Markdown 图片排出来的盒子。
//!
//! # 它现在只是一次查表
//!
//! 图片是 widget（第 3 节的对照表）：整段 `![替代](目标)` 由
//! `Decoration::Widget` 覆盖，盒子由 `BlockLayout` 连同文字一起排，尺寸由
//! [`crate::widget::BlockWidgets`] 给。这里做的是把排好的 [`WidgetBox`] 与
//! 它的语义（哪一段 source、替代文字在哪）接回来，好让绘制方与命中测试
//! 拿到一样东西。
//!
//! 此前这里自己划盒子：按替代文字排出来的那几个簇取包围盒，再在资源解码
//! 之后就地改尺寸。那套「排完再改」是 widget 还没到位时的替身——盒子撑高
//! 一行的后果得由调用方自己补（`BlockView::height` 曾经要把图片的下沿并进
//! 块高），而 widget 在排的时候就把行撑高了。
//!
//! [`WidgetBox`]: yu_layout::WidgetBox

use yu_core::{TextRange, VisualRange};
use yu_decoration::Bias;
use yu_layout::{LayoutError, LayoutPoint, LayoutRect};
use yu_markdown::BlockWidget;

use crate::blockview::{BlockHit, BlockView, shift_range};
use crate::geometry::source_range_contains;
use crate::table::TableLayout;

/// 一张图片占的位置。
///
/// 图片指向的资源由 workspace 那一层解析；这里只有几何。图片本身不进
/// source（不变量 A2），`source` 指的是那段 Markdown。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ImagePlacement {
    source: TextRange,
    label: TextRange,
    visual: VisualRange,
    line: usize,
    bounds: LayoutRect,
}

impl ImagePlacement {
    #[must_use]
    pub const fn source(self) -> TextRange {
        self.source
    }

    #[must_use]
    pub const fn label(self) -> TextRange {
        self.label
    }

    /// 盒子锚在视觉字节流的哪一点。
    ///
    /// 它是**空**区间：widget 覆盖的那段 source 在视觉文本里不占位，
    /// 整段 `![替代](目标)` 塌到同一个视觉偏移上。
    #[must_use]
    pub const fn visual(self) -> VisualRange {
        self.visual
    }

    #[must_use]
    pub const fn line(self) -> usize {
        self.line
    }

    #[must_use]
    pub const fn bounds(self) -> LayoutRect {
        self.bounds
    }

    /// 点在图片上时给出边界：左半边归标签起点，右半边归终点。
    #[must_use]
    pub(crate) fn hit(self, point: LayoutPoint) -> BlockHit {
        let midpoint = self.bounds.x() + self.bounds.width() * 0.5;
        let before = point.x() < midpoint;
        BlockHit::image_hit(
            if before {
                self.source.start()
            } else {
                self.source.end()
            },
            if before {
                self.visual.start()
            } else {
                self.visual.end()
            },
            self.line,
            LayoutPoint::new(
                if before {
                    self.bounds.x()
                } else {
                    self.bounds.x() + self.bounds.width()
                },
                self.bounds.y(),
            ),
            if before { Bias::Before } else { Bias::After },
            self.source,
        )
    }

    pub(crate) fn shifted(self, delta: i64) -> Result<Self, LayoutError> {
        Ok(Self {
            source: shift_range(self.source, delta)?,
            label: shift_range(self.label, delta)?,
            ..self
        })
    }
}

/// 把排好的 widget 盒接回它那段 Markdown。
///
/// 顺序与 [`yu_layout::BlockLayout::widgets`] 一致，也就是与装饰集合里的
/// widget 一一对应、同序。查不到的 id 是错误：`BlockLayout` 已经因为
/// 同一个 id 报过一次 `UnknownWidget`，这里再查不到说明两张表分叉了。
pub(crate) fn build_image_placements(view: &BlockView) -> Result<Vec<ImagePlacement>, LayoutError> {
    let decorations = view.decorations();
    let boxes = view.layout().widgets();
    let mut placements = Vec::with_capacity(boxes.len());
    for placed in boxes {
        let Some(BlockWidget::Image(image)) = decorations.widget(placed.widget()) else {
            return Err(LayoutError::UnknownWidget(placed.widget()));
        };
        placements.push(ImagePlacement {
            source: image.source(),
            label: image.label(),
            visual: VisualRange::empty(placed.visual()),
            line: placed.line(),
            bounds: placed.bounds(),
        });
    }
    Ok(placements)
}

/// 表格块里的图片跟着单元格走。
pub(crate) fn place_images_in_table(
    images: &mut [ImagePlacement],
    table: &TableLayout,
) -> Result<(), LayoutError> {
    for image in images {
        let Some(cell) = table
            .cells()
            .iter()
            .copied()
            .find(|cell| source_range_contains(cell.source(), image.source))
        else {
            continue;
        };
        let available = (cell.bounds().x() + cell.bounds().width() - cell.content_x()).max(1.0);
        let width = image.bounds.width().min(available).max(1.0);
        image.line = cell.row();
        image.bounds = LayoutRect::new(
            cell.content_x(),
            cell.bounds().y(),
            width,
            image.bounds.height().min(cell.bounds().height()),
        )?;
    }
    Ok(())
}
