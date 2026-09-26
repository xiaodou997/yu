//! Source-backed HTML footnote leaves. Saved labels, never saved display numbers.
use super::{HtmlBlockModel, HtmlNodeKind};
use crate::{FootnoteIndex, MarkdownDocument, TableBlock};
use yu_core::{ByteOffset, TextAttrs, TextRange, TextRole, TextScript, TextStyle};
use yu_decoration::{Decoration, ReplacementText};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HtmlFootnoteSpan {
    pub source: TextRange,
    pub content: TextRange,
}

impl HtmlFootnoteSpan {
    pub fn marker_text(self, source: &str) -> Option<String> {
        let body =
            source.get(self.content.start().get() as usize..self.content.end().get() as usize)?;
        Some(crate::image_markup::decode_image_text(body, true))
    }
}

impl HtmlBlockModel {
    pub fn footnote_span(&self, id: usize) -> Option<HtmlFootnoteSpan> {
        if !self
            .resolution
            .elements
            .get(id)?
            .as_ref()?
            .attributes
            .footnote_reference
        {
            return None;
        }
        let node = self.fragment.nodes.get(id)?;
        let HtmlNodeKind::Element {
            opening,
            closing: Some(closing),
        } = &node.kind
        else {
            return None;
        };
        Some(HtmlFootnoteSpan {
            source: node.source,
            content: TextRange::new(opening.source.end(), closing.source.start())?,
        })
    }

    pub fn footnote_spans(&self) -> Vec<HtmlFootnoteSpan> {
        (0..self.fragment.nodes.len())
            .filter_map(|id| self.footnote_span(id))
            .collect()
    }
}

impl FootnoteIndex {
    pub(crate) fn decorate_html(
        &self,
        source: &str,
        range: TextRange,
        active: Option<TextRange>,
        out: &mut crate::ExtensionOutput,
    ) {
        for reference in self.references().iter().filter(|reference| {
            range.start() <= reference.source.start() && reference.source.end() <= range.end()
        }) {
            let Some(content) = reference.content else {
                continue;
            };
            // Closed details already owns this entire range. Never substitute
            // a hidden descendant back into the visible table projection.
            if out.ranges().iter().any(|entry| {
                matches!(entry.decoration, Decoration::Replace)
                    && entry.range.start() <= reference.source.start()
                    && reference.source.end() <= entry.range.end()
            }) {
                continue;
            }
            if let Some(number) = reference.number
                && !crate::reveals(active, reference.source)
            {
                out.substitute_text(reference.source, ReplacementText::Number(number));
                let style = out.style(
                    TextAttrs::new(TextStyle::Plain)
                        .with_script(TextScript::Superscript)
                        .with_role(TextRole::Link),
                );
                out.mark(reference.source, style);
            } else {
                // Missing/duplicate definitions stay editable and diagnostic.
                out.reveal_html_leaf(source, reference.source, content);
            }
        }
    }
}

impl MarkdownDocument {
    /// Structured clipboard references may bind existing unique definitions;
    /// importing or renaming foreign definitions is a separate operation.
    pub fn html_footnote_targets_resolve(&self, model: &HtmlBlockModel, source: &str) -> bool {
        model.footnote_spans().into_iter().all(|span| {
            span.marker_text(source)
                .is_some_and(|marker| self.footnotes().definition_for_marker(&marker).is_some())
        })
    }

    /// Reconcile every reference in its original cell before any transaction.
    /// Definitions remain untouched outside the table. A missing, ambiguous or
    /// misassociated reference rejects conversion rather than flattening to text.
    pub fn preserve_table_footnote_source(&self, html: &str, table: &TableBlock) -> Option<String> {
        let source = self.source().as_str();
        let range = TextRange::new(
            ByteOffset::new(table.source_range().start() as u64),
            ByteOffset::new(table.source_range().end() as u64),
        )?;
        let originals: Vec<_> = self
            .footnotes()
            .references()
            .iter()
            .filter(|reference| {
                range.start() <= reference.source.start() && reference.source.end() <= range.end()
            })
            .collect();
        let proposed = crate::parse(&yu_text::TextBuffer::new(html).snapshot());
        let index = proposed.html_regions();
        if index.regions.len() != 1 {
            return None;
        }
        let model = index.regions[0].model.as_ref().ok()?;
        if model.partitions.len() != 1 {
            return None;
        }
        let generated = model.footnote_spans();
        if originals.is_empty() && generated.is_empty() {
            return Some(html.to_owned());
        }
        let converted = model
            .native_table(model.partitions[0].content.owner?)
            .ok()?;
        if converted.columns != table.column_count()
            || converted.rows.len() != table.visible_row_count()
        {
            return None;
        }
        let mut edits = Vec::new();
        for (before, after) in std::iter::once(table.first_row())
            .chain(table.rows().iter().map(Vec::as_slice))
            .zip(&converted.rows)
        {
            if before.len() != after.cells.len() {
                return None;
            }
            for (cell, converted_cell) in before.iter().zip(&after.cells) {
                let expected: Vec<_> = originals
                    .iter()
                    .filter(|reference| {
                        reference.source.start().get() as usize >= cell.start()
                            && reference.source.end().get() as usize <= cell.end()
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
                for (reference, span) in expected.into_iter().zip(actual) {
                    if reference.content.is_some() {
                        return None;
                    }
                    let marker = source.get(
                        reference.source.start().get() as usize
                            ..reference.source.end().get() as usize,
                    )?;
                    let definition = self.footnotes().definition_for_marker(marker)?;
                    if definition.source.start() < range.end()
                        && range.start() < definition.source.end()
                    {
                        return None;
                    }
                    // Compare labels, not rendered numbers or overall counts.
                    let generated_marker = span.marker_text(html)?;
                    if self
                        .footnotes()
                        .definition_for_marker(&generated_marker)?
                        .source
                        != definition.source
                    {
                        return None;
                    }
                    let encoded = marker
                        .replace('&', "&amp;")
                        .replace('<', "&lt;")
                        .replace('>', "&gt;");
                    edits.push((span.content, encoded));
                }
            }
        }
        if edits.len() != originals.len() || edits.len() != generated.len() {
            return None;
        }
        edits.sort_by_key(|(range, _)| range.start());
        let mut result = html.to_owned();
        for (range, replacement) in edits.into_iter().rev() {
            result.replace_range(
                range.start().get() as usize..range.end().get() as usize,
                &replacement,
            );
        }
        Some(result)
    }
}
