use yu_core::{TextAttrs, TextScript, TextStyle};
use yu_syntax::NodeKind;

use super::{BlockContext, DelimitedSpan, Extension, ExtensionOutput, reveals};

pub struct Script;

impl Extension for Script {
    fn name(&self) -> &'static str {
        "script"
    }

    fn decorate(&self, cx: &BlockContext<'_>, out: &mut ExtensionOutput) {
        for node in cx.nodes() {
            let script = match node.kind() {
                NodeKind::Superscript => TextScript::Superscript,
                NodeKind::Subscript => TextScript::Subscript,
                _ => continue,
            };
            let Some(span) = DelimitedSpan::of(node, |kind| kind == NodeKind::ScriptMark) else {
                continue;
            };
            // During editing, expose ordinary full-size source. Composition and
            // caret queries therefore consume the same active projection.
            if reveals(cx.active(), node.range()) {
                continue;
            }
            let style = out.style(TextAttrs::new(TextStyle::Plain).with_script(script));
            out.mark(span.content, style);
            out.replace(span.opening);
            out.replace(span.closing);
        }
    }
}
