//! `- [ ] 待办`。
//!
//! 复选框是一个 widget：`[x]` / `[ ]` 那三个字节由 [`Decoration::Widget`]
//! 覆盖，从视觉文本里消失，位置上留一个盒子。盒子**多大**不在这里——那要
//! `LayoutConfig` 的行高，是 `yu-editor` 的事。这里只说「这一段是一个复选
//! 框，勾没勾上」。
//!
//! # 为什么它可以是 widget，而表格不行
//!
//! 判据是**有没有内部位置**。`[x]` 没有：光标不需要停在方括号中间，切换
//! 状态走的是整段替换（不变量 B6），VoiceOver 也是按块 press。表格有——
//! 单元格内容一旦从视觉字节流里消失，光标就进不了任何一格。
//!
//! # 它此前是 `Replace`，那是一个画得不对的形状
//!
//! `Replace` 让 `[x]` 的视觉宽度变成零，于是整段塌成**一个点**，而复选框
//! 是画在那个点上的一个**有宽度**的覆盖物——正文的第一个字被压在框底下。
//! 这与第七刀「widget 有宽度，所以同一个视觉偏移在它两侧是两个 x」是同一
//! 件事：有宽度的东西必须在排版里占位，事后贴上去一定会压到别人。
//!
//! 隐藏区间没有变：非空 range 的 `Decoration::Widget` 同样
//! [`hides_source`]，藏的还是那三个字节。变的只是它在行里占不占位。
//!
//! # 复选框永远不露出来
//!
//! 焦点块也不例外。让它在光标经过时闪出一个 `[ ]`，用户会以为凭空多了两个
//! 字符。图片那边相反（[`reveals`]），因为图片的替代文字与目标是要编辑的；
//! `[x]` 里没有可编辑的内容，只有一个二选一的状态。
//!
//! `- ` 前缀原样留着，不换成 `•`——任务项画成 `- ☐ 待办`，普通列表项画成
//! `• 项目`。前缀归 `list.rs` 管，而它按块类型只认 `ListItem`，认不到
//! `TaskListItem`。两个 extension 因此不相交，谁也不需要知道对方存在
//! （不变量 D6）。
//!
//! # 定义域按 `BlockKind` 取，而它就是树的答案
//!
//! `BlockKind::TaskListItem` 曾经是一份独立的判断（`task::parse_task_marker`
//! 扫行首），与树的 `Task` 节点各说各话，由一个 `task_identity.rs` 把两边锁
//! 在一起。现在块的身份由 [`crate::classify`] 问树要，两份判断合成了一份，
//! 那个测试文件也就没有可锁的东西了。
//!
//! 已登记的那处不一致还在，但它不再是「两份判断」的问题，而是**块边界**的
//! 问题：`block_sequence` 不下降到容器里，`> - [x] q` 整块是一个
//! `BlockQuote`，于是引用块里的任务项没有复选框。要收掉它得让块的边界也由树
//! 定，那是块结构合并的第二刀（overview 的「块结构合并：调查结论」）。
//!
//! [`Decoration::Widget`]: yu_decoration::Decoration::Widget
//! [`hides_source`]: yu_decoration::Decoration::hides_source
//! [`reveals`]: super::reveals

use yu_core::WidgetSide;
use yu_syntax::NodeKind;

use super::{BlockContext, BlockOrnament, BlockWidget, CheckboxSpan, Extension, ExtensionOutput};
use crate::block_sequence::BlockKind;
use crate::task::checkbox_state;

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
        // 勾没勾上从**树给的那三个字节**里读，不从 `BlockKind` 里读：区间与
        // 状态出自同一次查询，错不开。
        let Some(state) = cx
            .text(marker.range())
            .as_deref()
            .and_then(|text| checkbox_state(text.as_bytes()))
        else {
            return;
        };
        let widget = out.widget(BlockWidget::Checkbox(CheckboxSpan::new(
            marker.range(),
            state,
        )));
        // 非空 range 的 widget 覆盖并隐藏这一段，`side` 没有歧义。
        out.place_widget(marker.range(), widget, WidgetSide::Before);

        // 缩进：嵌套的任务项要往右让，而它没有标记装饰替它说这句话。
        let indent = out.line_style(BlockOrnament::Indent {
            columns: cx.indent_columns(),
        });
        out.line(cx.range(), indent);
    }
}
