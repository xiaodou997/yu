use yu_core::{TextAttrs, TextStyle};
use yu_syntax::NodeKind;

use super::{BlockContext, DelimitedSpan, Extension, ExtensionOutput, reveals};

pub struct Highlight;

impl Extension for Highlight {
    fn name(&self) -> &'static str {
        "highlight"
    }

    fn decorate(&self, cx: &BlockContext<'_>, out: &mut ExtensionOutput) {
        for node in cx.nodes().filter(|node| node.kind() == NodeKind::Highlight) {
            let Some(span) = DelimitedSpan::of(node, |kind| kind == NodeKind::HighlightMark) else {
                continue;
            };
            let style = out.style(TextAttrs::new(TextStyle::Plain).with_highlighted(true));
            out.mark(span.content, style);
            if !reveals(cx.active(), node.range()) {
                out.replace(span.opening);
                out.replace(span.closing);
            }
        }
    }
}
