//! Decoration 的类型族。
//!
//! 对应 `docs/architecture/overview-v2.md` 第 5.1 节。不变量 D1 规定视觉表现
//! 的唯一来源是 DecorationSet：任何「隐藏语法字符」「替换为控件」「改变样式」
//! 都必须表达成这里的一个值，不得在 layout 或 scene 里开特殊分支。
//!
//! # 三个 id 现在住在 `yu-core`
//!
//! `StyleId` / `LineStyleId` / `WidgetId` / `WidgetSide` 原本定义在这个文件里。
//! S5 把它们挪进了 `yu-core::style`：它们是**装饰层与布局层之间的共用词汇**，
//! 而这两个 crate 互不依赖（不变量 E2），共用词汇只能住在共同下游。
//! 这里原样再导出，本 crate 的公开面不变。含义仍然由上层解释，见
//! `yu_core::style` 的模块文档。

use yu_core::TextRange;
pub use yu_core::{LineStyleId, StyleId, WidgetId, WidgetSide};

/// 一条装饰。
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Decoration {
    /// 不改变字符数量，只改变呈现样式。可叠加。
    Mark { style: StyleId },
    /// 从视觉上移除这段 source 字符。source 不变、长度不变、可被光标穿越
    /// 与选中（不变量 D5）。这是「隐藏未聚焦的 Markdown 语法」的唯一机制。
    Replace,
    /// 在该 range 位置放置一个视觉物件。range 为空则是插入，
    /// 非空则同时隐藏被覆盖的 source。
    Widget { widget: WidgetId, side: WidgetSide },
    /// 作用于整行/整块的样式（缩进、背景、行高、前缀装饰）。
    Line { style: LineStyleId },
}

impl Decoration {
    /// 这条装饰是否让它覆盖的 source 在视觉字节流里消失。
    ///
    /// 视觉坐标是 UTF-8 字节偏移（见 `docs/specs/coordinates.md`），所以
    /// 「widget 有多宽」在这一层没有意义——widget 不是文本，它的 source
    /// 在字节流里就是不占位。宽度是 layout 的事。
    #[must_use]
    pub const fn hides_source(self) -> bool {
        matches!(self, Self::Replace | Self::Widget { .. })
    }
}

/// 一条装饰及其覆盖的 source range。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DecorationRange {
    pub range: TextRange,
    pub decoration: Decoration,
    /// 合并顺序的最后一级依据。数值大的排在后面。
    pub priority: i32,
}

impl DecorationRange {
    #[must_use]
    pub const fn new(range: TextRange, decoration: Decoration) -> Self {
        Self {
            range,
            decoration,
            priority: 0,
        }
    }

    #[must_use]
    pub const fn with_priority(mut self, priority: i32) -> Self {
        self.priority = priority;
        self
    }

    /// 合并多个 extension 的产出时的定序键（不变量 D6）。
    ///
    /// `(from, side, priority)`。`side` 在这里由 widget 的 side 提供，
    /// 其余装饰按 0 处理——它们的位置没有「贴在光标哪一边」的歧义。
    ///
    /// 定序必须是全序且只依赖装饰自身的值：extension 之间不得相互感知，
    /// 所以合并结果不能依赖它们被加进来的先后。
    #[must_use]
    pub fn order_key(&self) -> (u64, u64, i8, i32, Decoration) {
        let side = match self.decoration {
            Decoration::Widget {
                side: WidgetSide::Before,
                ..
            } => -1,
            Decoration::Widget {
                side: WidgetSide::After,
                ..
            } => 1,
            _ => 0,
        };
        (
            self.range.start().get(),
            self.range.end().get(),
            side,
            self.priority,
            self.decoration,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::{Decoration, DecorationRange, StyleId, WidgetId, WidgetSide};
    use yu_core::{ByteOffset, TextRange};

    fn range(start: u64, end: u64) -> TextRange {
        TextRange::new(ByteOffset::new(start), ByteOffset::new(end)).expect("有序")
    }

    #[test]
    fn only_replace_and_widget_hide_source() {
        assert!(Decoration::Replace.hides_source());
        assert!(
            Decoration::Widget {
                widget: WidgetId(0),
                side: WidgetSide::Before
            }
            .hides_source()
        );
        assert!(!Decoration::Mark { style: StyleId(0) }.hides_source());
        assert!(
            !Decoration::Line {
                style: super::LineStyleId(0)
            }
            .hides_source()
        );
    }

    /// 定序只能依赖装饰自身的值。同一组装饰无论以什么顺序加进来，
    /// 排完必须一模一样——不变量 D6 的「extension 之间不得相互感知」。
    #[test]
    fn ordering_does_not_depend_on_insertion_order() {
        let a = DecorationRange::new(range(0, 2), Decoration::Replace);
        let b = DecorationRange::new(range(0, 2), Decoration::Mark { style: StyleId(1) });
        let c = DecorationRange::new(range(1, 5), Decoration::Replace).with_priority(3);
        let d = DecorationRange::new(range(0, 2), Decoration::Replace).with_priority(9);

        let mut forward = vec![a, b, c, d];
        let mut backward = vec![d, c, b, a];
        forward.sort_by_key(DecorationRange::order_key);
        backward.sort_by_key(DecorationRange::order_key);
        assert_eq!(forward, backward);
        // priority 是最后一级依据：同 range 同 side 时小的在前。
        assert_eq!(
            forward[0].decoration,
            Decoration::Mark { style: StyleId(1) }
        );
    }

    #[test]
    fn widget_side_orders_before_and_after_around_the_same_offset() {
        let before = DecorationRange::new(
            range(4, 4),
            Decoration::Widget {
                widget: WidgetId(1),
                side: WidgetSide::Before,
            },
        );
        let after = DecorationRange::new(
            range(4, 4),
            Decoration::Widget {
                widget: WidgetId(1),
                side: WidgetSide::After,
            },
        );
        let mut set = vec![after, before];
        set.sort_by_key(DecorationRange::order_key);
        assert_eq!(set, vec![before, after]);
    }
}
