//! Markdown 图片的几何。
//!
//! # 为什么它住在 `yu-editor`
//!
//! 它此前住在 `yu-layout`：布局层为了给图片划一个盒子，必须先知道
//! 「哪一段是 alt 文本」——那是 Markdown 的语法知识，不变量 E1 禁止的。
//!
//! # 为什么它还不是 widget
//!
//! overview-v2 §5.3 说图片在 v2 里是 widget。真做成 widget 要求 alt 标签
//! 那几个字节从视觉文本里**消失**（`Decoration::Replace`），否则图片盒子
//! 会把标签挤到一边，然后两样都画出来。而 v1 的 `Projection` 表达不了
//! 「这一段隐藏但仍可被光标穿越」——它的隐藏 run 是给语法标记用的。
//!
//! 给一个 S6 就要删掉的 v1 类型加这个能力不划算。图片在 S6 随
//! `DecorationSet` 一起变成 widget，那时 `apply_intrinsic_size` 这套
//! 「解码后改尺寸再重排」正好就是不变量 D7 的 placeholder → ready → 重排。

use yu_core::TextRange;
use yu_layout::{ImageIntrinsicSize, LayoutConfig, LayoutError, LayoutPoint, LayoutRect};
use yu_projection::{ProjectionBias, VisualRange};

use crate::blockview::{BlockHit, BlockView};
use crate::geometry::{map_source_range, source_range_contains, upstream};
use crate::table::TableLayout;
use yu_text::ChangeSet;

/// 一张图片占的位置。
///
/// 图片指向的资源由 workspace 那一层解析；这里只决定它盖住 alt 标签的哪一段
/// 几何。图片本身不进 source（不变量 A2），`source` 指的是那段 Markdown。
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
            if before {
                ProjectionBias::Before
            } else {
                ProjectionBias::After
            },
            self.source,
        )
    }

    /// 解码后的尺寸到位：宽度受剩余行宽限制，高度保持解码时的长宽比。
    pub(crate) fn apply_intrinsic_size(
        &mut self,
        size: ImageIntrinsicSize,
        config: LayoutConfig,
    ) -> Result<(), LayoutError> {
        let intrinsic_width = size.width() as f32;
        let intrinsic_height = size.height() as f32;
        let available_width = (config.max_width() - self.bounds.x()).max(1.0);
        let scale = (available_width / intrinsic_width).min(1.0);
        let width = (intrinsic_width * scale).max(1.0);
        let height = (intrinsic_height * scale).max(config.line_height());
        self.bounds = LayoutRect::new(self.bounds.x(), self.bounds.y(), width, height)?;
        Ok(())
    }

    pub(crate) fn map_through(self, changes: &ChangeSet) -> Result<Self, LayoutError> {
        Ok(Self {
            source: map_source_range(self.source, changes)?,
            label: map_source_range(self.label, changes)?,
            ..self
        })
    }
}

/// 按 alt 标签排出来的位置划图片盒子。
///
/// 算法照搬 v1：盒子横跨标签那几个簇，宽度至少 4 个行高，右边不超出可用
/// 宽度。标签本身仍然在视觉文本里可编辑（不变量 I5：不支持的语法按源码
/// 画出来，永不白屏）。
pub(crate) fn build_image_placements(view: &BlockView) -> Result<Vec<ImagePlacement>, LayoutError> {
    let projection = view.projection();
    let config = view.config();
    let mut placements = Vec::with_capacity(projection.images().len());
    for image in projection.images().iter().copied() {
        let visual_start = projection
            .source_to_visual(image.label().start(), ProjectionBias::Before)
            .map_err(upstream)?;
        let visual_end = projection
            .source_to_visual(image.label().end(), ProjectionBias::After)
            .map_err(upstream)?;
        let visual =
            VisualRange::new(visual_start, visual_end).ok_or(LayoutError::OffsetOverflow)?;
        let caret = view.caret_for_visual(visual.start(), ProjectionBias::Before)?;
        let line = view
            .lines()
            .get(caret.line())
            .ok_or(LayoutError::OffsetOverflow)?;
        let mut left = f32::INFINITY;
        let mut right = 0.0_f32;
        let mut found = false;
        for index in line.cluster_range() {
            let cluster = view.clusters()[index];
            if cluster.is_line_break() {
                continue;
            }
            let overlaps = if visual.is_empty() {
                cluster.visual().start() == visual.start()
            } else {
                cluster.visual().start() < visual.end() && visual.start() < cluster.visual().end()
            };
            if overlaps {
                found = true;
                left = left.min(cluster.x());
                right = right.max(cluster.x() + cluster.width());
            }
        }
        if !found {
            left = caret.point().x();
            right = caret.point().x();
        }
        let minimum_width = config.line_height() * 4.0;
        let remaining = (config.max_width() - left).max(config.line_height());
        let width = (right - left).max(minimum_width).min(remaining);
        placements.push(ImagePlacement {
            source: image.source(),
            label: image.label(),
            visual,
            line: caret.line(),
            bounds: LayoutRect::new(left.max(0.0), line.y(), width, config.line_height())?,
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
