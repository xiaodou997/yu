//! A task item is one source-backed checkbox followed by editable text.
//! Container/list prefixes disappear in the same projection as the task marker.
//! Toggling still edits only the original three-byte `[ ]` / `[x]` range.

use yu_core::{TextRange, WidgetSide};
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

        // Keep ancestor list markers even when the innermost marker is a
        // checkbox. Hide each list prefix separately so quote ranges retain
        // their source-backed projection and never leak outer '-' characters.
        if let Some(mark) = cx
            .block_node(|kind| kind == NodeKind::ListItem)
            .and_then(|item| {
                item.children()
                    .find(|child| child.kind() == NodeKind::ListMark)
            })
        {
            super::list::hide_list_prefixes(cx, out, mark.range());
        }
        super::list::emit_list_markers(cx, out, cx.list_depth().saturating_sub(1));

        // The measured checkbox object includes the theme's trailing gap.
        // Hide source separator whitespace instead of counting it a second time.
        let content_start = cx.skip_spaces(marker.range().end());
        if content_start > marker.range().end() {
            out.replace(
                TextRange::new(marker.range().end(), content_start).expect("ordered separator"),
            );
        }

        // 缩进：嵌套的任务项要往右让，而它没有标记装饰替它说这句话。
        let indent = out.line_style(BlockOrnament::Indent {
            columns: cx.list_depth().saturating_sub(1).saturating_mul(2),
        });
        out.line(cx.range(), indent);
    }
}
