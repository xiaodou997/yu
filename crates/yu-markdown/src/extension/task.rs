//! `- [ ] 待办`。
//!
//! 任务项的 `[ ]` **永远**不露出来，焦点块也不例外：复选框是替代它的控件，
//! 让它在光标经过时闪出一个 `[ ]` 只会让人以为多了两个字符。
//!
//! `- ` 前缀原样留着，不换成 `•`——任务项画成 `- ☐ 待办`，普通列表项画成
//! `• 项目`。前缀归 `list.rs` 管，而它按块类型只认 `ListItem`，认不到
//! `TaskListItem`。两个 extension 因此不相交，谁也不需要知道对方存在
//! （不变量 D6）。
//!
//! # 标记范围来自语法树
//!
//! 第九刀之前，`[ ]` 在语法树里没有节点（`- [ ] x` 就是一个普通
//! `ListItem`），块类型与标记范围**都**只能问 `block_sequence`——那是
//! extension 层里仅剩的一处 v1 依赖。GFM 的 TaskList 移进 `yu-syntax` 之后
//! 标记是一个 `TaskMarker` 节点，这里与其余十个 extension 变成同一个形状：
//! **块的身份问 `BlockKind`，区间问树。**
//!
//! 顺带修掉的是一个**恰好不出事**的重叠：`[x]` 此前会被行内解析器认成一个
//! shortcut `Link`，于是 link 与 task 盖在同一段 source 上，隐藏区间靠取
//! 并集恰好收敛到整个 `[x]`。见
//! `yu-markdown/tests/extension_decorations.rs::a_checked_box_is_hidden_by_exactly_one_decoration`。
//!
//! # 两份判断还没并成一份
//!
//! `BlockKind::TaskListItem` 与树的 `Task` 是同一个问题的两个实现，它们**不
//! 完全一致**：`block_sequence` 不下降到引用块里，`> - [x] q` 在它眼里是一个
//! `BlockQuote`。这里的定义域按 `BlockKind` 取，所以那种块不产装饰——与这一刀
//! 之前逐字节相同。两条路由 `tests/task_identity.rs` 锁在一起。

use yu_syntax::NodeKind;

use super::{BlockContext, Extension, ExtensionOutput};
use crate::block_sequence::BlockKind;

pub struct Task;

impl Extension for Task {
    fn name(&self) -> &'static str {
        "task"
    }

    fn decorate(&self, cx: &BlockContext<'_>, out: &mut ExtensionOutput) {
        if !matches!(cx.block().kind(), BlockKind::TaskListItem { .. }) {
            return;
        }
        let Some(marker) = cx.nodes().find(|node| node.kind() == NodeKind::TaskMarker) else {
            return;
        };
        out.replace(marker.range());
    }
}
