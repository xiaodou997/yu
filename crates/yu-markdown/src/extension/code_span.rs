//! `` `行内代码` ``。

use yu_core::{TextAttrs, TextStyle};
use yu_syntax::NodeKind;

use super::{BlockContext, DelimitedSpan, Extension, ExtensionOutput, reveals};

pub struct CodeSpan;

impl Extension for CodeSpan {
    fn name(&self) -> &'static str {
        "code-span"
    }

    fn decorate(&self, cx: &BlockContext<'_>, out: &mut ExtensionOutput) {
        for node in cx.nodes() {
            if node.kind() != NodeKind::InlineCode {
                continue;
            }
            let Some(span) = DelimitedSpan::of(node, |kind| kind == NodeKind::CodeMark) else {
                continue;
            };
            let style = out.style(TextAttrs::new(TextStyle::Code));
            out.mark(span.content, style);
            if !reveals(cx.active(), node.range()) {
                out.replace(span.opening);
                out.replace(span.closing);
            }
        }
    }
}
