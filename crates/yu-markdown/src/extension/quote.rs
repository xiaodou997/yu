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

use super::{BlockContext, BlockOrnament, Extension, ExtensionOutput};

pub struct Quote;

impl Extension for Quote {
    fn name(&self) -> &'static str {
        "quote"
    }

    fn decorate(&self, cx: &BlockContext<'_>, out: &mut ExtensionOutput) {
        let depth = cx.quote_depth();
        if depth == 0 {
            return;
        }

        if !cx.is_focus() {
            let mut cursor = cx.range().start();
            for mark in cx.nodes().filter(|node| node.kind() == NodeKind::QuoteMark) {
                // Whitespace before an explicit quote marker belongs to the
                // enclosing container, including continuation lines. Inspect
                // each source slice once; never scan the whole prefix per line.
                if let Some(prefix) = cx
                    .source()
                    .as_str()
                    .get(cursor.get() as usize..mark.range().start().get() as usize)
                {
                    let start = prefix.rfind('\n').map_or(0, |index| index + 1);
                    if prefix[start..]
                        .bytes()
                        .all(|byte| matches!(byte, b' ' | b'\t'))
                    {
                        let start = yu_core::ByteOffset::new(cursor.get() + start as u64);
                        if start < mark.range().start() {
                            out.replace(
                                TextRange::new(start, mark.range().start())
                                    .expect("ordered prefix"),
                            );
                        }
                    }
                }
                let content_start = cx.skip_spaces(mark.range().end());
                cursor = content_start;
                if let Some(prefix) = TextRange::new(mark.range().start(), content_start) {
                    out.replace(prefix);
                }
            }
        }

        let style = out.line_style(BlockOrnament::QuoteBar {
            depth,
            outside_list: cx.quote_depth_outside_lists(),
        });
        out.line(cx.range(), style);
    }
}
