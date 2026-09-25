//! Extract renderer input using syntax-owned container marks, never a second
//! Markdown parser or string-based removal of meaningful TeX/diagram operators.
use crate::{EmbeddedKind, EmbeddedSpan, MarkdownDocument, SyntaxNode};
use yu_core::{ByteOffset, TextRange};
use yu_syntax::NodeKind;

impl MarkdownDocument {
    pub fn embedded_source(&self, span: EmbeddedSpan) -> Option<String> {
        let text = self.source().as_str();
        let from = usize::try_from(span.content.start().get()).ok()?;
        let to = usize::try_from(span.content.end().get()).ok()?;
        let body = text.get(from..to)?;
        if span.kind == EmbeddedKind::Math
            && !span.display
            && self
                .html_regions()
                .region_for(span.source)
                .and_then(|region| region.model.as_ref().ok())
                .is_some_and(|model| model.inline_math_spans().contains(&span))
        {
            // Decode exactly once, only for a model-owned HTML math leaf.
            // Its source/content offsets continue to address the saved bytes;
            // Markdown TeX backslashes and literal entities are not rewritten.
            return Some(crate::image_markup::decode_image_text(body, true));
        }
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

    /// Restore syntax-owned TeX into a proposed HTML table, before any edit.
    /// Matching is per cell, never a global count that could move a formula to
    /// another cell. A disagreement between parsers safely rejects promotion.
    pub fn preserve_table_math_source(
        &self,
        html: &str,
        table: &crate::TableBlock,
    ) -> Option<String> {
        let range = TextRange::new(
            ByteOffset::new(table.source_range().start() as u64),
            ByteOffset::new(table.source_range().end() as u64),
        )?;
        let tree = self.tree()?;
        let mut original = Vec::new();
        for node in SyntaxNode::new(tree, 0).descendants_in(range) {
            if node.kind() == NodeKind::MathBlock {
                return None; // This batch supports inline dollar math only.
            }
            if node.kind() != NodeKind::InlineMath {
                continue;
            }
            if node.range().start() < range.start() || node.range().end() > range.end() {
                return None;
            }
            let marks: Vec<_> = node
                .children()
                .filter(|child| child.kind() == NodeKind::MathMark)
                .map(|child| child.range())
                .collect();
            if marks.len() != 2 {
                return None;
            }
            for mark in &marks {
                if self.source().as_str().get(
                    mark.start().get() as usize..mark.end().get() as usize,
                )? != "$" {
                    return None;
                }
            }
            original.push(EmbeddedSpan {
                source: node.range(),
                content: TextRange::new(marks[0].end(), marks[1].start())?,
                kind: EmbeddedKind::Math,
                display: false,
            });
        }
        original.sort_by_key(|span| span.source.start());
        let proposed = crate::parse(&yu_text::TextBuffer::new(html).snapshot());
        let index = proposed.html_regions();
        if index.regions.len() != 1 {
            return None;
        }
        let model = index.regions.first()?.model.as_ref().ok()?;
        if model.partitions.len() != 1 {
            return None;
        }
        let generated = model.inline_math_spans();
        // Do not impose new shape rules on pre-existing non-math promotion.
        // The caller still applies the unchanged native-table validity gate.
        if original.is_empty() && generated.is_empty() {
            return Some(html.to_owned());
        }
        let converted = model.native_table(model.partitions[0].content.owner?).ok()?;
        if converted.columns != table.column_count()
            || converted.rows.len() != table.visible_row_count()
        {
            return None;
        }
        let mut replacements = Vec::new();
        for (before, after) in std::iter::once(table.first_row())
            .chain(table.rows().iter().map(Vec::as_slice))
            .zip(&converted.rows)
        {
            if before.len() != after.cells.len() {
                return None;
            }
            for (cell, converted_cell) in before.iter().zip(&after.cells) {
                let expected: Vec<_> = original
                    .iter()
                    .filter(|span| {
                        span.source.start().get() as usize >= cell.start()
                            && span.source.end().get() as usize <= cell.end()
                    })
                    .collect();
                let actual: Vec<_> = generated
                    .iter()
                    .filter(|span| {
                        converted_cell.content.start() <= span.source.start()
                            && span.source.end() <= converted_cell.content.end()
                    })
                    .collect();
                if expected.len() != actual.len() {
                    return None;
                }
                for (before, after) in expected.into_iter().zip(actual) {
                    let tex = self.embedded_source(*before)?;
                    if tex.contains(['\r', '\n', '\0']) {
                        return None;
                    }
                    let encoded = tex.replace('&', "&amp;")
                        .replace('<', "&lt;")
                        .replace('>', "&gt;");
                    replacements.push((after.content, encoded));
                }
            }
        }
        if replacements.len() != original.len() || replacements.len() != generated.len() {
            return None;
        }
        replacements.sort_by_key(|(range, _)| range.start());
        let mut result = html.to_owned();
        for (range, encoded) in replacements.into_iter().rev() {
            result.replace_range(
                range.start().get() as usize..range.end().get() as usize,
                &encoded,
            );
        }
        Some(result)
    }
}
