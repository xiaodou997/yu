//! List markers and indentation come from syntax container ancestry.
//! Continuation paragraphs inherit indentation without repeating the marker.
//! Source indentation is hidden by projection and is never normalized on save.

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
        if cx.list_depth() == 0 || matches!(cx.block().kind(), BlockKind::TaskListItem { .. }) {
            return;
        }
        let Some(item) = cx.block_node(|kind| kind == NodeKind::ListItem) else {
            return;
        };
        // 焦点块把标记原样露出来，连替代呈现一起撤掉——否则光标停在一个
        // 看不见的 `-` 上，用户按退格会删掉一个他没看见的字符。
        let Some(mark) = item
            .children()
            .find(|child| child.kind() == NodeKind::ListMark)
        else {
            return;
        };

        if mark.range().start() < cx.range().start() {
            // A second paragraph belongs to the same item but has no repeated
            // marker. Preserve the container's indentation in visual geometry.
            let start = cx.range().start();
            if let Some(prefix) = TextRange::new(start, cx.skip_spaces(start)) {
                out.replace(prefix);
            }
            let indent = out.line_style(BlockOrnament::Indent {
                columns: cx.list_depth().saturating_mul(2),
            });
            out.line(cx.range(), indent);
            return;
        }

        if cx.is_focus() {
            return;
        }

        hide_list_prefixes(cx, out, mark.range());

        let indent = out.line_style(BlockOrnament::Indent {
            columns: cx.list_depth().saturating_sub(1).saturating_mul(2),
        });
        out.line(cx.range(), indent);
        emit_list_markers(cx, out, cx.list_depth());
    }
}

/// Shared prefix projection for ordinary items and task checkbox leaves.
pub(super) fn hide_list_prefixes(
    cx: &BlockContext<'_>,
    out: &mut ExtensionOutput,
    last_mark: TextRange,
) {
    // 行首缩进也是语法：缩进量由 `BlockOrnament::Indent` 报给上一层去
    // 排版，原样留在视觉文本里就会缩进两次。
    // 块从行首开始（块序列铺满源码，不在行中间切），所以「第一行的起点」
    // 就是块的起点。此前这里调一个 `first_line_start()`，它把整块切成行
    // 再取第一条的起点——同一个答案，代价是 O(块长度)。**那是变异验证抓
    // 出来的**：把它换成块的起点，一条用例都不红。
    let mut quote_ends: Vec<_> = cx
        .nodes()
        .filter(|node| {
            node.kind() == NodeKind::QuoteMark && node.range().end() <= last_mark.start()
        })
        .map(|node| node.range().end())
        .collect();
    quote_ends.sort_unstable();
    for prefix_mark in cx.nodes().filter(|node| {
        node.kind() == NodeKind::ListMark
            && node.range().start() >= cx.range().start()
            && node.range().end() <= last_mark.end()
    }) {
        let preceding = quote_ends.partition_point(|end| *end <= prefix_mark.range().start());
        let line_start = preceding
            .checked_sub(1)
            .map_or(cx.range().start(), |index| {
                cx.skip_spaces(quote_ends[index])
            });
        let content_start = cx.skip_spaces(prefix_mark.range().end());
        if let Some(prefix) = TextRange::new(line_start, content_start) {
            out.replace(prefix);
        }
    }
}

pub(super) fn emit_list_markers(cx: &BlockContext<'_>, out: &mut ExtensionOutput, max_depth: u8) {
    for (item, quotes, depth) in cx.list_marker_ancestors() {
        if depth > max_depth {
            continue;
        }
        let Some(mark) = item
            .children()
            .find(|child| child.kind() == NodeKind::ListMark)
        else {
            continue;
        };
        if mark.range().start() < cx.range().start() || mark.range().end() > cx.range().end() {
            continue;
        }
        let style = out.line_style(BlockOrnament::Marker(
            MarkerOrnament::new(mark.range(), marker_text(cx, item, mark))
                .with_quote_depth(quotes)
                .with_list_depth(depth),
        ));
        out.line(cx.range(), style);
    }
}

/// Ordered lists number items from the first marker, independently of the
/// spelling of subsequent source markers. Saving preserves those source bytes.
fn marker_text(cx: &BlockContext<'_>, item: SyntaxNode<'_>, mark: SyntaxNode<'_>) -> String {
    let text = read_range(cx.source(), mark.range()).unwrap_or_default();
    if !text.first().is_some_and(u8::is_ascii_digit) {
        return "•".to_owned();
    }
    if let Some(number) = cx.ordered_number_for_item(item.range()) {
        return format!("{}{}", number, text.last().copied().unwrap_or(b'.') as char);
    }
    String::from_utf8(text).unwrap_or_else(|_| "•".to_owned())
}
