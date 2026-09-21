use yu_core::{TextAttrs, TextRole, TextScript, TextStyle};
use yu_decoration::ReplacementText;
use yu_syntax::NodeKind;

use super::{BlockContext, Extension, ExtensionOutput, reveals};

pub struct Footnote;
impl Extension for Footnote {
    fn name(&self) -> &'static str {
        "footnote"
    }

    fn decorate(&self, cx: &BlockContext<'_>, out: &mut ExtensionOutput) {
        let index = cx.presentation.footnotes();
        let references = index.references();
        let first = references.partition_point(|entry| entry.source.end() <= cx.range().start());
        for reference in references[first..]
            .iter()
            .take_while(|entry| entry.source.start() < cx.range().end())
        {
            let Some(number) = reference.number else {
                continue;
            };
            if reveals(cx.active(), reference.source) {
                continue;
            }
            out.substitute_text(reference.source, ReplacementText::Number(number));
            let style = out.style(
                TextAttrs::new(TextStyle::Plain)
                    .with_script(TextScript::Superscript)
                    .with_role(TextRole::Link),
            );
            out.mark(reference.source, style);
        }
        let definitions = index.definitions();
        let first = definitions.partition_point(|entry| entry.marker.end() <= cx.range().start());
        for definition in definitions[first..]
            .iter()
            .take_while(|entry| entry.marker.start() < cx.range().end())
        {
            let Some(number) = definition.number else {
                continue;
            };
            if reveals(cx.active(), definition.marker) {
                continue;
            }
            out.substitute_text(definition.marker, ReplacementText::Number(number));
            let style = out.style(TextAttrs::new(TextStyle::Strong).with_role(TextRole::Link));
            out.mark(definition.marker, style);
        }
        for node in cx
            .nodes()
            .filter(|node| node.kind() == NodeKind::FootnoteIndent)
        {
            if !reveals(cx.active(), node.range()) {
                out.replace(node.range());
            }
        }
    }
}
