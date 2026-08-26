//! `- 项目` 与 `1. 项目`。
//!
//! 标记 `•` 不在 source 里，它是 `-` 的**替代呈现**：被替代掉的那段源码
//! 由 [`MarkerOrnament::source`] 指着，选中与编辑仍然走它（不变量 A2）。
//! 有序列表相反——`1.` 本来就是要给人看的，原样搬过去。
//!
//! # 任务项不归这里管
//!
//! `- [ ] 待办` 画成 `- ☐ 待办`，`- 项目` 画成 `• 项目`：任务项的 `- `
//! 原样留着，不换成 `•`。
//!
//! 实现这件事的方式**不是**让本 extension 去问「有没有 task extension」
//! ——那正是不变量 D6 禁止的相互感知。归属按**块类型**划分：`ListItem` 归
//! 这里，`TaskListItem` 归 `task.rs`，两个集合不相交，各自读的都是同一份
//! 共享输入。谁都不需要知道对方存在。
//!
//! 语法树帮不上这个忙：`- [ ] x` 在树里就是一个普通的 `ListItem`，`[ ]`
//! 连节点都没有。所以这一处的块类型只能来自 `block_sequence`。
//!
//! [`MarkerOrnament::source`]: super::MarkerOrnament::source

use yu_core::TextRange;
use yu_syntax::NodeKind;

use super::{BlockContext, BlockOrnament, Extension, ExtensionOutput, MarkerOrnament, SyntaxNode};
use crate::block_sequence::BlockKind;
use crate::reference::read_range;

pub struct List;

impl Extension for List {
    fn name(&self) -> &'static str {
        "list"
    }

    fn decorate(&self, cx: &BlockContext<'_>, out: &mut ExtensionOutput) {
        if !matches!(cx.block().kind(), BlockKind::ListItem { .. }) {
            return;
        }
        let Some(item) = cx.block_node(|kind| kind == NodeKind::ListItem) else {
            return;
        };
        // 焦点块把标记原样露出来，连替代呈现一起撤掉——否则光标停在一个
        // 看不见的 `-` 上，用户按退格会删掉一个他没看见的字符。
        if cx.is_focus() {
            return;
        }
        let Some(mark) = item
            .children()
            .find(|child| child.kind() == NodeKind::ListMark)
        else {
            return;
        };

        // 行首缩进也是语法：缩进量由 `MarkerOrnament::indent` 报给上一层去
        // 排版，原样留在视觉文本里就会缩进两次。
        let line_start = cx.first_line_start();
        let content_start = cx.skip_spaces(mark.range().end());
        if let Some(prefix) = TextRange::new(line_start, content_start) {
            out.replace(prefix);
        }

        let indent = mark
            .range()
            .start()
            .get()
            .saturating_sub(line_start.get())
            .min(u64::from(u8::MAX)) as u8;
        let style = out.line_style(BlockOrnament::Marker(MarkerOrnament::new(
            mark.range(),
            marker_text(cx, mark),
            indent,
        )));
        out.line(cx.range(), style);
    }
}

/// 有序列表原样用它的 `1.`；无序列表一律画 `•`。
///
/// 「是不是有序」按标记的第一个字节判，不问父节点：`ListItem` 的父节点是
/// `BulletList` 还是 `OrderedList` 要有父指针才知道，而标记本身已经说清楚了。
fn marker_text(cx: &BlockContext<'_>, mark: SyntaxNode<'_>) -> String {
    let ordered = read_range(cx.source(), mark.range())
        .and_then(|bytes| bytes.first().copied())
        .is_some_and(|byte| byte.is_ascii_digit());
    if ordered {
        read_range(cx.source(), mark.range())
            .and_then(|bytes| String::from_utf8(bytes).ok())
            .unwrap_or_else(|| "\u{2022}".to_owned())
    } else {
        "\u{2022}".to_owned()
    }
}
