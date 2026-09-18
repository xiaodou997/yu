//! Nonprinting code characters have visible, source-backed placeholders.
//! Tabs and line endings keep their normal layout semantics; prose and inline
//! code are outside this extension's block-code domain.
use yu_core::{ByteOffset, TextAttrs, TextRange, TextRole, TextStyle};
use yu_decoration::ReplacementText;
use yu_syntax::NodeKind;

use super::{BlockContext, Extension, ExtensionOutput};

pub struct CodeControls;

impl Extension for CodeControls {
    fn name(&self) -> &'static str {
        "code-controls"
    }

    fn decorate(&self, cx: &BlockContext<'_>, out: &mut ExtensionOutput) {
        for node in cx.nodes().filter(|node| node.kind() == NodeKind::CodeText) {
            let Some(text) = cx.text(node.range()) else {
                continue;
            };
            for (offset, ch) in text.char_indices() {
                if !is_nonprinting(ch) {
                    continue;
                }
                let start = node.range().start().get() + offset as u64;
                let range = TextRange::new(
                    ByteOffset::new(start),
                    ByteOffset::new(start + ch.len_utf8() as u64),
                )
                .expect("character range");
                out.substitute_text(range, ReplacementText::Scalar('•'));
                let style = out
                    .style(TextAttrs::new(TextStyle::Code).with_role(TextRole::ControlCharacter));
                out.mark(range, style);
            }
        }
    }
}

fn is_nonprinting(ch: char) -> bool {
    !matches!(ch, '\t' | '\n' | '\r')
        && matches!(ch,
        '\u{0000}'..='\u{001f}' | '\u{007f}'..='\u{009f}' |
        '\u{00ad}' | '\u{061c}' | '\u{200b}'..='\u{200f}' |
        '\u{2028}' | '\u{2029}' | '\u{feff}')
}
