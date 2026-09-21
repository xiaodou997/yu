//! Source-backed details/summary metadata for native disclosure interaction.
use super::{HtmlBlockModel, HtmlElementKind, HtmlNodeKind};
use yu_core::{ByteOffset, TextRange};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HtmlDisclosure {
    pub source: TextRange,
    pub opening: TextRange,
    pub summary: Option<TextRange>,
    pub summary_content: Option<TextRange>,
    pub body: TextRange,
    pub open: bool,
    pub open_attribute: Option<TextRange>,
}

impl HtmlBlockModel {
    /// Read a semantic label from the already-resolved HTML tree. This does not
    /// change source ranges, selection, projection state or saved source bytes.
    pub fn accessible_text(&self, source: &str, range: TextRange) -> Option<String> {
        source.get(range.start().get() as usize..range.end().get() as usize)?;
        let mut text = String::new();
        for (index, node) in self.fragment.nodes.iter().enumerate() {
            let start = node.source.start().max(range.start());
            let end = node.source.end().min(range.end());
            if start >= end {
                continue;
            }
            match &node.kind {
                HtmlNodeKind::Text => text.push_str(&crate::image_markup::decode_image_text(
                    source.get(start.get() as usize..end.get() as usize)?,
                    true,
                )),
                HtmlNodeKind::Comment => {}
                HtmlNodeKind::Element { opening, closing } => {
                    // Attribute labels and partial tags still refer to literal
                    // source; only complete markup may be removed here.
                    for tag in std::iter::once(opening.as_ref()).chain(closing.as_deref()) {
                        if tag.source.start() < range.end()
                            && range.start() < tag.source.end()
                            && !(range.start() <= tag.source.start()
                                && tag.source.end() <= range.end())
                        {
                            return None;
                        }
                    }
                    let element = self.resolution.elements.get(index)?.as_ref()?;
                    if range.start() <= opening.source.start()
                        && opening.source.end() <= range.end()
                    {
                        match element.kind {
                            HtmlElementKind::Break => text.push(' '),
                            HtmlElementKind::Image => {
                                text.push_str(element.attributes.alternate.as_deref().unwrap_or(""))
                            }
                            _ => {}
                        }
                    }
                }
            }
        }
        Some(text.split_whitespace().collect::<Vec<_>>().join(" "))
    }

    /// Only visible disclosure headers are interactive. Hidden descendants
    /// retain source metadata but must not expose a clickable target.
    pub fn disclosure_header_at(&self, at: ByteOffset) -> Option<HtmlDisclosure> {
        if let Some(table) = self.partitions.iter().find(|part| {
            part.content.kind == super::HtmlFlowKind::Table
                && part.source.start() <= at
                && at < part.source.end()
        }) && self.native_table(table.content.owner?).is_err()
        {
            return None;
        }
        let disclosures = self.disclosures_ref();
        if disclosures
            .iter()
            .any(|details| !details.open && details.body.start() <= at && at < details.body.end())
        {
            return None;
        }
        disclosures
            .iter()
            .filter(|details| {
                let end = details
                    .summary
                    .map_or(details.opening.end(), |summary| summary.end());
                details.opening.start() <= at && at < end
            })
            .min_by_key(|details| details.source.len())
            .cloned()
    }

    pub fn disclosures(&self) -> Vec<HtmlDisclosure> {
        self.cached_disclosures.clone()
    }

    pub(super) fn disclosures_ref(&self) -> &[HtmlDisclosure] {
        &self.cached_disclosures
    }

    pub(super) fn collect_disclosures(&self) -> Vec<HtmlDisclosure> {
        self.fragment
            .nodes
            .iter()
            .enumerate()
            .filter_map(|(id, node)| {
                let element = self.resolution.elements[id].as_ref()?;
                if element.kind != HtmlElementKind::Details {
                    return None;
                }
                let HtmlNodeKind::Element {
                    opening,
                    closing: Some(closing),
                } = &node.kind
                else {
                    return None;
                };
                let summary = node
                    .children
                    .iter()
                    .copied()
                    .find(|&child| {
                        self.resolution.elements[child]
                            .as_ref()
                            .is_some_and(|element| element.kind == HtmlElementKind::Summary)
                    })
                    .map(|child| &self.fragment.nodes[child]);
                let summary_content = summary.and_then(|node| match &node.kind {
                    HtmlNodeKind::Element {
                        opening,
                        closing: Some(closing),
                    } => TextRange::new(opening.source.end(), closing.source.start()),
                    _ => None,
                });
                let summary = summary.map(|node| node.source);
                Some(HtmlDisclosure {
                    source: node.source,
                    opening: opening.source,
                    summary,
                    summary_content,
                    body: TextRange::new(
                        summary.map_or(opening.source.end(), |range| range.end()),
                        closing.source.start(),
                    )?,
                    open: element.attributes.open,
                    open_attribute: opening
                        .attributes
                        .iter()
                        .find(|attribute| attribute.name == "open")
                        .map(|attribute| attribute.source),
                })
            })
            .collect()
    }
}
