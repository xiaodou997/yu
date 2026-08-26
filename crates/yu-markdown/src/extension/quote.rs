//! `> 引用`。
//!
//! 竖条画在缩进让出来的那条 gutter 里。竖条多宽、gutter 多深都是几何，
//! 不在这里；这里只说「这是几层引用」，以及哪几段 `> ` 不进视觉文本。
//!
//! 续行的 `>` 是嵌在 `Paragraph` 里的，不是 `Blockquote` 的直接子节点
//! （`> a\n> b` 只有一个 `Blockquote`）。所以层数按**嵌套的 `Blockquote`**
//! 数，而要隐藏的标记按块内**全部** `QuoteMark` 取——两者数得不一样，
//! 混用会让 `> a\n> b` 报成两层。

use yu_core::TextRange;
use yu_syntax::NodeKind;

use super::{BlockContext, BlockOrnament, Extension, ExtensionOutput, SyntaxNode};

pub struct Quote;

impl Extension for Quote {
    fn name(&self) -> &'static str {
        "quote"
    }

    fn decorate(&self, cx: &BlockContext<'_>, out: &mut ExtensionOutput) {
        let Some(node) = cx.block_node(|kind| kind == NodeKind::Blockquote) else {
            return;
        };
        let depth = nesting_depth(node);
        if depth == 0 {
            return;
        }

        if !cx.is_focus() {
            for mark in cx.nodes().filter(|node| node.kind() == NodeKind::QuoteMark) {
                let content_start = cx.skip_spaces(mark.range().end());
                if let Some(prefix) = TextRange::new(mark.range().start(), content_start) {
                    out.replace(prefix);
                }
            }
        }

        let style = out.line_style(BlockOrnament::QuoteBar { depth });
        out.line(cx.range(), style);
    }
}

/// 一路往里数连续嵌套的 `Blockquote`。
fn nesting_depth(node: SyntaxNode<'_>) -> u8 {
    let mut depth = 0_u8;
    let mut current = node;
    while current.kind() == NodeKind::Blockquote {
        depth = depth.saturating_add(1);
        let Some(inner) = current
            .children()
            .find(|child| child.kind() == NodeKind::Blockquote)
        else {
            break;
        };
        current = inner;
    }
    depth
}
