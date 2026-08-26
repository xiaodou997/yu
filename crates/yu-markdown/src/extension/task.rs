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
//! `[ ]` 在语法树里没有节点（`- [ ] x` 就是一个普通 `ListItem`），所以这里
//! 的块类型与标记范围只能来自 `block_sequence`。这是 extension 层里仅剩的
//! 一处 v1 依赖。

use super::{BlockContext, Extension, ExtensionOutput};
use crate::block_sequence::BlockKind;
use crate::task_marker;

pub struct Task;

impl Extension for Task {
    fn name(&self) -> &'static str {
        "task"
    }

    fn decorate(&self, cx: &BlockContext<'_>, out: &mut ExtensionOutput) {
        if !matches!(cx.block().kind(), BlockKind::TaskListItem { .. }) {
            return;
        }
        let Some(marker) = task_marker(cx.source(), cx.block()) else {
            return;
        };
        out.replace(marker.range());
    }
}
