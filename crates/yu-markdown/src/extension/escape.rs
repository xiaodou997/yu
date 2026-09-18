//! Parser-recognized punctuation escapes share the ordinary inline projection.
//! Code and unrecognized backslashes have no Escape nodes and remain literal.

use yu_core::{ByteOffset, TextRange};
use yu_syntax::NodeKind;

use super::{BlockContext, Extension, ExtensionOutput, reveals};

pub struct Escape;

impl Extension for Escape {
    fn name(&self) -> &'static str {
        "escape"
    }

    fn decorate(&self, cx: &BlockContext<'_>, out: &mut ExtensionOutput) {
        for node in cx.nodes().filter(|node| node.kind() == NodeKind::Escape) {
            if !reveals(cx.active(), node.range())
                && let Some(prefix) = TextRange::new(
                    node.range().start(),
                    ByteOffset::new(node.range().start().get() + 1),
                )
            {
                out.replace(prefix);
            }
        }
    }
}
