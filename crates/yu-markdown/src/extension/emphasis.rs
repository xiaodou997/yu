//! `*斜体*` 与 `**粗体**`。
//!
//! 一种语法一个文件：它只认识强调，只产出强调的装饰。

use yu_core::{TextAttrs, TextStyle};
use yu_syntax::NodeKind;

use super::{BlockContext, DelimitedSpan, Extension, ExtensionOutput, reveals};

pub struct Emphasis;

impl Extension for Emphasis {
    fn name(&self) -> &'static str {
        "emphasis"
    }

    fn decorate(&self, cx: &BlockContext<'_>, out: &mut ExtensionOutput) {
        for node in cx.nodes() {
            let style = match node.kind() {
                NodeKind::Emphasis => TextStyle::Emphasis,
                NodeKind::StrongEmphasis => TextStyle::Strong,
                _ => continue,
            };
            let Some(span) = DelimitedSpan::of(node, |kind| kind == NodeKind::EmphasisMark) else {
                continue;
            };
            let style = out.style(TextAttrs::new(style));
            out.mark(span.content, style);
            // 光标碰到这一段时定界符要露出来，否则用户没法编辑自己写的 `**`。
            if !reveals(cx.active(), node.range()) {
                out.replace(span.opening);
                out.replace(span.closing);
            }
        }
    }
}
