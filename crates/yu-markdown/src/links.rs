//! Revision-bound link targets shared by native mouse and accessibility actions.
use crate::{
    BlockContext, BlockKind, InlineSpanKind, MarkdownDocument, parse_inline_with_definitions,
};
use yu_core::{ByteOffset, Revision, TextRange};
use yu_syntax::NodeKind;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DocumentLink {
    pub revision: Revision,
    pub source: TextRange,
    pub label: TextRange,
    /// Decoded destination; the native host must resolve relative paths and
    /// apply its URL activation policy before opening anything.
    pub destination: String,
}

/// A validated, parser-owned inline HTML element. Metadata is only published
/// after its enclosing inline elements close correctly.
#[derive(Clone, Debug, PartialEq)]
pub struct InlineHtmlElement {
    pub source: TextRange,
    pub label: TextRange,
    pub kind: crate::html::HtmlElementKind,
    pub attributes: crate::html::HtmlAttributes,
}

impl MarkdownDocument {
    pub fn link_at(&self, at: ByteOffset) -> Option<DocumentLink> {
        let blocks = self.semantic_blocks();
        let block = blocks.get(blocks.block_index_for_offset(at)?)?;
        if matches!(
            block.kind(),
            BlockKind::FencedCodeBlock { .. } | BlockKind::IndentedCode | BlockKind::FrontMatter
        ) {
            return None;
        }
        if let Some(region) = self.html_regions().region_for(block.range()) {
            return region
                .model
                .as_ref()
                .ok()?
                .links()
                .into_iter()
                .find(|link| contains(link.label, at))
                .map(|link| DocumentLink {
                    revision: self.revision(),
                    source: link.source,
                    label: link.label,
                    destination: link.destination_text,
                });
        }
        let cx = BlockContext::for_block(
            self.source(),
            self.tree()?,
            self.reference_definitions(),
            self.semantic_presentation(),
            block,
        );
        if cx.nodes().any(|node| {
            matches!(
                node.kind(),
                NodeKind::InlineMath | NodeKind::MathBlock | NodeKind::InlineCode
            ) && contains(node.range(), at)
        }) {
            return None;
        }
        let inline = parse_inline_with_definitions(
            self.source(),
            block.range(),
            Some(self.reference_definitions()),
        )
        .ok()?;
        for span in inline.spans() {
            if !matches!(
                span.kind(),
                InlineSpanKind::Link | InlineSpanKind::ReferenceLink | InlineSpanKind::Autolink
            ) || !contains(span.content(), at)
            {
                continue;
            }
            let destination = span.destination().or_else(|| {
                span.reference().and_then(|label| {
                    self.reference_definitions()
                        .lookup(self.source(), label)
                        .map(|definition| definition.destination())
                })
            })?;
            let raw = self
                .source()
                .as_str()
                .get(destination.start().get() as usize..destination.end().get() as usize)?;
            let mut target = crate::image_markup::decode_image_text(raw, false);
            if span.kind() == InlineSpanKind::Autolink
                && target.contains('@')
                && !target.contains(':')
            {
                target.insert_str(0, "mailto:");
            }
            if !target.is_empty() {
                return Some(DocumentLink {
                    revision: self.revision(),
                    source: span.source_range(),
                    label: span.content(),
                    destination: target,
                });
            }
        }
        self.inline_html_elements(block)
            .into_iter()
            .find_map(|element| {
                if element.kind != crate::html::HtmlElementKind::Link
                    || !contains(element.label, at)
                {
                    return None;
                }
                let destination = element
                    .attributes
                    .destination
                    .filter(|value| !value.is_empty())?;
                Some(DocumentLink {
                    revision: self.revision(),
                    source: element.source,
                    label: element.label,
                    destination,
                })
            })
    }
    pub fn inline_html_elements(&self, block: crate::Block) -> Vec<InlineHtmlElement> {
        let Some(tree) = self.tree() else {
            return Vec::new();
        };
        let cx = BlockContext::for_block(
            self.source(),
            tree,
            self.reference_definitions(),
            self.semantic_presentation(),
            block,
        );
        struct Frame {
            opening: crate::html::HtmlTag,
            attributes: Option<crate::html::HtmlAttributes>,
            children: Vec<InlineHtmlElement>,
        }
        let mut stack: Vec<Frame> = Vec::new();
        let mut output = Vec::new();
        for node in cx.nodes().filter(|node| node.kind() == NodeKind::HtmlTag) {
            let Some(raw) = self
                .source()
                .as_str()
                .get(node.range().start().get() as usize..node.range().end().get() as usize)
            else {
                continue;
            };
            let Ok(tag) = crate::html::HtmlTag::parse(raw, node.range().start()) else {
                continue;
            };
            if tag.closing {
                let Some(frame) = stack.pop() else {
                    continue;
                };
                if frame.opening.name != tag.name {
                    stack.clear();
                    continue;
                }
                let Some(attributes) = frame.attributes else {
                    continue;
                };
                let Some(kind) = frame.opening.semantic_kind() else {
                    continue;
                };
                let element = InlineHtmlElement {
                    source: TextRange::new(frame.opening.source.start(), tag.source.end())
                        .expect("tag order"),
                    label: TextRange::new(frame.opening.source.end(), tag.source.start())
                        .expect("tag body"),
                    kind,
                    attributes,
                };
                let destination = stack
                    .last_mut()
                    .map_or(&mut output, |parent| &mut parent.children);
                destination.push(element);
                destination.extend(frame.children);
            } else {
                let attributes = if stack
                    .last()
                    .is_none_or(|parent| parent.attributes.is_some())
                {
                    tag.resolve_attributes(self.source().as_str()).ok()
                } else {
                    None
                };
                if tag.is_void() {
                    if let (Some(attributes), Some(kind)) = (attributes, tag.semantic_kind()) {
                        let element = InlineHtmlElement {
                            source: tag.source,
                            label: TextRange::empty(tag.source.end()),
                            kind,
                            attributes,
                        };
                        stack
                            .last_mut()
                            .map_or(&mut output, |parent| &mut parent.children)
                            .push(element);
                    }
                } else if !tag.self_closing {
                    if stack.len() >= 128 {
                        return Vec::new();
                    }
                    stack.push(Frame {
                        opening: tag,
                        attributes,
                        children: Vec::new(),
                    });
                }
            }
        }
        output.sort_by_key(|element| element.source.start());
        output
    }
}

fn contains(range: TextRange, at: ByteOffset) -> bool {
    range.start() <= at && at < range.end()
}
