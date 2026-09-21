//! Extract renderer input using syntax-owned container marks, never a second
//! Markdown parser or string-based removal of meaningful TeX/diagram operators.
use crate::{EmbeddedSpan, MarkdownDocument, SyntaxNode};
use yu_syntax::NodeKind;
impl MarkdownDocument {
    pub fn embedded_source(&self, span: EmbeddedSpan) -> Option<String> {
        let text = self.source().as_str();
        let from = usize::try_from(span.content.start().get()).ok()?;
        let to = usize::try_from(span.content.end().get()).ok()?;
        text.get(from..to)?;
        let tree = self.tree()?;
        let mut marks: Vec<_> = SyntaxNode::new(tree, 0)
            .descendants_in(span.content)
            .filter(|node| matches!(node.kind(), NodeKind::QuoteMark | NodeKind::ListMark))
            .map(|node| node.range())
            .collect();
        marks.sort_unstable_by_key(|range| (range.start(), range.end()));
        let mut output = String::new();
        let mut cursor = from;
        for mark in marks {
            let start = (mark.start().get() as usize).max(from);
            let end = (mark.end().get() as usize).min(to);
            if start >= cursor && end >= start {
                output.push_str(text.get(cursor..start)?);
                cursor = end;
            }
        }
        output.push_str(text.get(cursor..to)?);
        Some(output)
    }
}
