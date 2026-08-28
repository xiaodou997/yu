//! 排出来的 widget 盒子：图片与任务项复选框。
//!
//! # 两种 widget 共用一套几何
//!
//! [`PlacedWidget`] 是「一个盒子排在哪」，[`ImagePlacement`] 与
//! [`CheckboxPlacement`] 各自在它上面加自己的语义（替代文字 / 勾没勾上）。
//! 分成两个结构而不是两套字段，是因为几何那一半的规则对两种 widget 是同一
//! 句话，抄第二遍就会分叉。
//!
//! 共用**不等于**两种都要走同一条命中快路：见
//! [`WidgetPlacements::placed`]。
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

use yu_core::{TextRange, VisualOffset, VisualRange};
use yu_decoration::Bias;
use yu_layout::{LayoutError, LayoutPoint, LayoutRect};
use yu_markdown::{BlockWidget, TaskState};

use crate::blockview::{BlockHit, BlockView, shift_range};
use crate::table::TableLayout;

/// 一个 widget 盒子排在哪。
///
/// 两种 widget 的几何完全一样，区别只在语义那一半。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PlacedWidget {
    source: TextRange,
    visual: VisualRange,
    line: usize,
    bounds: LayoutRect,
}

impl PlacedWidget {
    /// widget 替代掉的那段 Markdown。widget 本身不进 source（不变量 A2）。
    #[must_use]
    pub const fn source(self) -> TextRange {
        self.source
    }

    /// 盒子锚在视觉字节流的哪一点。
    ///
    /// 它是**空**区间：widget 覆盖的那段 source 在视觉文本里不占位，
    /// 整段塌到同一个视觉偏移上。
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

    /// 点在盒子上时给出边界：左半边归起点，右半边归终点。
    ///
    /// 被隐藏的语法塌成一个点、两侧 x 相同，所以「落在哪一侧」一直可以交给
    /// 行的规则（不变量 H5）。widget 不是：它两沿差着整个盒子的宽度，只有
    /// 排它的人知道点落在哪一沿，所以要显式带出来。
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

    fn shifted(self, delta: i64) -> Result<Self, LayoutError> {
        Ok(Self {
            source: shift_range(self.source, delta)?,
            ..self
        })
    }
}

/// 一张图片占的位置。
///
/// 图片指向的资源由 workspace 那一层解析；这里只有几何。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ImagePlacement {
    placed: PlacedWidget,
    label: TextRange,
}

impl ImagePlacement {
    #[must_use]
    pub const fn placed(self) -> PlacedWidget {
        self.placed
    }

    #[must_use]
    pub const fn source(self) -> TextRange {
        self.placed.source()
    }

    #[must_use]
    pub const fn label(self) -> TextRange {
        self.label
    }

    #[must_use]
    pub const fn visual(self) -> VisualRange {
        self.placed.visual()
    }

    #[must_use]
    pub const fn line(self) -> usize {
        self.placed.line()
    }

    #[must_use]
    pub const fn bounds(self) -> LayoutRect {
        self.placed.bounds()
    }

    pub(crate) fn shifted(self, delta: i64) -> Result<Self, LayoutError> {
        Ok(Self {
            placed: self.placed.shifted(delta)?,
            label: shift_range(self.label, delta)?,
        })
    }
}

/// 一个任务项复选框占的位置。
///
/// 它此前不占位：`[x]` 是 `Decoration::Replace`，塌成一个点，方框由
/// `yu-workspace` 事后贴在那个点上——于是压住正文的第一个字。现在它在排版
/// 里占一个盒子，画的人按 [`PlacedWidget::bounds`] 画，不再自己算几何。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CheckboxPlacement {
    placed: PlacedWidget,
    state: TaskState,
}

impl CheckboxPlacement {
    #[must_use]
    pub const fn placed(self) -> PlacedWidget {
        self.placed
    }

    /// `[x]` / `[ ]` 那三个字节。
    #[must_use]
    pub const fn source(self) -> TextRange {
        self.placed.source()
    }

    #[must_use]
    pub const fn state(self) -> TaskState {
        self.state
    }

    #[must_use]
    pub const fn line(self) -> usize {
        self.placed.line()
    }

    #[must_use]
    pub const fn bounds(self) -> LayoutRect {
        self.placed.bounds()
    }

    pub(crate) fn shifted(self, delta: i64) -> Result<Self, LayoutError> {
        Ok(Self {
            placed: self.placed.shifted(delta)?,
            ..self
        })
    }
}

/// 一个块上排好的全部 widget，按种类分开。
#[derive(Clone, Debug, Default)]
pub(crate) struct WidgetPlacements {
    pub images: Vec<ImagePlacement>,
    pub checkboxes: Vec<CheckboxPlacement>,
}

impl WidgetPlacements {
    /// 命中测试要先拦一道的那些盒子。
    ///
    /// **只有图片。** 第一版把复选框也串了进来，变异验证说那是死代码：去掉
    /// 之后全部用例照绿——因为 `BlockLayout::hit` 本来就带着
    /// `widget_affinity`（第七刀），落在盒子哪一沿的答案两条路一样。
    ///
    /// 图片需要这一道，是因为它的盒子**可以比行高**：`line_for_y` 会把落在
    /// 图片下半部的点算到下一行去。复选框只有 0.68 个行高，撑不出这种情况。
    ///
    /// 而串进来不只是多余：[`BlockHit::image`] 会因此把一次复选框点击报成
    /// 「点在一张图上」，FFI 那一层照着它给平台一个图片区间。多余的代码顺手
    /// 说了个谎，这是留着它更坏的那一半。
    pub(crate) fn placed(&self) -> impl Iterator<Item = PlacedWidget> + '_ {
        self.images.iter().map(|image| image.placed())
    }
}

/// 把排好的 widget 盒接回它那段 Markdown。
///
/// 顺序与 [`yu_layout::BlockLayout::widgets`] 一致，也就是与装饰集合里的
/// widget 一一对应、同序。查不到的 id 是错误：`BlockLayout` 已经因为
/// 同一个 id 报过一次 `UnknownWidget`，这里再查不到说明两张表分叉了。
///
/// **没有「跳过不认识的那一种」这条路。** 静静跳过一个 widget 的后果是画面
/// 上少一个盒子而所有断言全绿——第七刀那个「锚不到簇起点的 widget 被 while
/// 静静跳过」就是这个形状。种类是穷举的，加一种就得在这里说清楚它去哪。
pub(crate) fn build_widget_placements(view: &BlockView) -> Result<WidgetPlacements, LayoutError> {
    let decorations = view.decorations();
    let mut placements = WidgetPlacements::default();
    for placed in view.layout().widgets() {
        let box_geometry = PlacedWidget {
            source: TextRange::empty(yu_core::ByteOffset::ZERO),
            visual: VisualRange::empty(placed.visual()),
            line: placed.line(),
            bounds: placed.bounds(),
        };
        match decorations.widget(placed.widget()) {
            Some(BlockWidget::Image(image)) => placements.images.push(ImagePlacement {
                placed: PlacedWidget {
                    source: image.source(),
                    ..box_geometry
                },
                label: image.label(),
            }),
            Some(BlockWidget::Checkbox(checkbox)) => {
                placements.checkboxes.push(CheckboxPlacement {
                    placed: PlacedWidget {
                        source: checkbox.source(),
                        ..box_geometry
                    },
                    state: checkbox.state(),
                });
            }
            None => return Err(LayoutError::UnknownWidget(placed.widget())),
        }
    }
    Ok(placements)
}

/// 表格块里的图片盒子来自**格内**的那一份布局。
///
/// 此前是排完之后把文字流里的盒子按源码区间挪进格子、再按格宽裁一刀。
/// 现在每一格自己排一次，widget 在格内就按那一列的宽度量过了——盒子不需要
/// 事后裁，位置也不需要事后挪。
pub(crate) fn build_table_widget_placements(
    view: &BlockView,
    table: &TableLayout,
) -> Result<WidgetPlacements, LayoutError> {
    let decorations = view.decorations();
    let mut placements = WidgetPlacements::default();
    for (index, cell) in table.cells().iter().copied().enumerate() {
        let Some(layout) = table.cell_layouts().get(index) else {
            return Err(LayoutError::Upstream("table cell has no layout".into()));
        };
        for placed in layout.widgets() {
            let visual = VisualOffset::new(
                placed
                    .visual()
                    .get()
                    .saturating_add(cell.visual().start().get()),
            );
            let box_geometry = PlacedWidget {
                source: TextRange::empty(yu_core::ByteOffset::ZERO),
                visual: VisualRange::empty(visual),
                line: cell.row(),
                bounds: LayoutRect::new(
                    cell.content_x() + placed.bounds().x(),
                    cell.bounds().y() + placed.bounds().y(),
                    placed.bounds().width(),
                    placed.bounds().height(),
                )?,
            };
            match decorations.widget(placed.widget()) {
                Some(BlockWidget::Image(image)) => placements.images.push(ImagePlacement {
                    placed: PlacedWidget {
                        source: image.source(),
                        ..box_geometry
                    },
                    label: image.label(),
                }),
                // 格子里不会有复选框——表格块的 `BlockKind` 是表格，task
                // extension 的定义域进不来。真出现了也要按同一套几何排，
                // 不能悄悄丢掉。
                Some(BlockWidget::Checkbox(checkbox)) => {
                    placements.checkboxes.push(CheckboxPlacement {
                        placed: PlacedWidget {
                            source: checkbox.source(),
                            ..box_geometry
                        },
                        state: checkbox.state(),
                    });
                }
                None => return Err(LayoutError::UnknownWidget(placed.widget())),
            }
        }
    }
    Ok(placements)
}
