use super::{
    HtmlFlowError, HtmlFlowPartition, HtmlFragment, HtmlResolution, HtmlTagError, partition_flow,
};
use crate::block_sequence::{BlockRecord, SourceHash};
use crate::{Block, BlockKind, BlockSequence, BlockState, MarkdownDocument};
use yu_core::{Revision, TextRange};

#[derive(Clone, Debug, PartialEq)]
pub struct HtmlBlockModel {
    pub fragment: HtmlFragment,
    pub resolution: HtmlResolution,
    pub partitions: Vec<HtmlFlowPartition>,
    pub(super) cached_disclosures: Vec<super::HtmlDisclosure>,
    pub(super) boundary_tags: Vec<(TextRange, bool)>,
    pub(super) list_items: std::collections::BTreeMap<usize, (Option<u64>, bool)>,
    pub cell_partitions: std::collections::BTreeMap<usize, Vec<HtmlFlowPartition>>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HtmlLink {
    pub source: TextRange,
    pub label: TextRange,
    pub destination: TextRange,
    pub destination_text: String,
}

impl HtmlBlockModel {
    /// Cell flow and disclosure metadata must observe the same resolved open state.
    pub(super) fn refresh_cell_partitions(&mut self, source: &str) -> Result<(), HtmlModelError> {
        self.cached_disclosures = self.collect_disclosures();
        let mut cell_partitions = std::collections::BTreeMap::new();
        for (id, node) in self.fragment.nodes.iter().enumerate() {
            if !self.resolution.elements[id].as_ref().is_some_and(|e| {
                matches!(
                    e.kind,
                    super::HtmlElementKind::HeaderCell | super::HtmlElementKind::DataCell
                )
            }) {
                continue;
            }
            let super::HtmlNodeKind::Element {
                opening,
                closing: Some(closing),
            } = &node.kind
            else {
                continue;
            };
            let content = TextRange::new(opening.source.end(), closing.source.start())
                .ok_or(HtmlModelError::InvalidSource)?;
            let mut leaves =
                self.fragment
                    .flow_blocks_in(&self.resolution, source, &node.children, Some(id));
            if leaves.is_empty() {
                leaves.push(super::HtmlFlowBlock {
                    kind: super::HtmlFlowKind::Paragraph,
                    source: content,
                    owner: Some(id),
                });
            }
            cell_partitions.insert(
                id,
                partition_flow(&leaves, content).map_err(HtmlModelError::Partition)?,
            );
        }
        self.cell_partitions = cell_partitions;
        Ok(())
    }

    pub fn alignment(&self, part: &super::HtmlFlowPartition) -> Option<super::HtmlAlignment> {
        self.alignment_for_node(part.content.owner?)
    }

    pub(crate) fn alignment_for_node(&self, node: usize) -> Option<super::HtmlAlignment> {
        let mut owner = Some(node);
        while let Some(id) = owner {
            let element = self.resolution.elements[id].as_ref()?;
            if element.attributes.alignment.is_some() {
                return element.attributes.alignment;
            }
            owner = self.fragment.nodes[id].parent;
        }
        None
    }

    /// Link metadata remains bound to the canonical document ranges. URL
    /// activation policy belongs to the native host, not this parser.
    pub fn links(&self) -> Vec<HtmlLink> {
        self.fragment
            .nodes
            .iter()
            .enumerate()
            .filter_map(|(id, node)| {
                let element = self.resolution.elements[id].as_ref()?;
                if element.kind != super::HtmlElementKind::Link {
                    return None;
                }
                let destination_text = element.attributes.destination.as_ref()?.clone();
                if destination_text.is_empty() {
                    return None;
                }
                let super::HtmlNodeKind::Element {
                    opening,
                    closing: Some(closing),
                } = &node.kind
                else {
                    return None;
                };
                Some(HtmlLink {
                    source: node.source,
                    label: TextRange::new(opening.source.end(), closing.source.start())?,
                    destination: opening.unique_attribute("href").ok()??.value?,
                    destination_text,
                })
            })
            .collect()
    }

    /// Resource discovery and painting share the resolved finite HTML tree.
    /// Unknown/malformed ancestors never expose embedded image resources.
    pub fn image_spans(&self, source: &str) -> Vec<crate::ImageSpan> {
        self.fragment
            .nodes
            .iter()
            .enumerate()
            .filter_map(|(id, node)| {
                if self.resolution.elements[id].as_ref()?.kind != super::HtmlElementKind::Image {
                    return None;
                }
                let super::HtmlNodeKind::Element { opening, .. } = &node.kind else {
                    return None;
                };
                let raw = source.get(
                    opening.source.start().get() as usize..opening.source.end().get() as usize,
                )?;
                crate::extension::image::html_image_span(raw, opening.source.start().get())
            })
            .collect()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HtmlModelError {
    InvalidSource,
    Parse(HtmlTagError),
    Partition(HtmlFlowError),
}

#[derive(Clone, Debug, PartialEq)]
pub struct HtmlRegion {
    pub source: TextRange,
    pub model: Result<HtmlBlockModel, HtmlModelError>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct HtmlIndex {
    pub revision: Revision,
    pub regions: Vec<HtmlRegion>,
    disclosure_fingerprint: u64,
    projected_blocks: BlockSequence,
    presentation: std::sync::Arc<crate::PresentationTree>,
}

impl HtmlIndex {
    pub(crate) fn from_document(document: &MarkdownDocument) -> Self {
        if !document
            .blocks
            .iter()
            .any(|block| block.kind() == BlockKind::HtmlBlock)
        {
            return Self {
                revision: document.revision(),
                regions: Vec::new(),
                disclosure_fingerprint: 0,
                projected_blocks: document.blocks.clone(),
                presentation: document.presentation.clone(),
            };
        }
        let source = document.source().as_str();
        let regions: Vec<HtmlRegion> = document
            .blocks
            .iter()
            .filter(|block| block.kind() == BlockKind::HtmlBlock)
            .map(|block| {
                let range = block.range();
                let model = (|| {
                    let raw = source
                        .get(range.start().get() as usize..range.end().get() as usize)
                        .ok_or(HtmlModelError::InvalidSource)?;
                    let fragment =
                        HtmlFragment::parse(raw, range.start()).map_err(HtmlModelError::Parse)?;
                    let resolution = fragment.resolve(source);
                    let leaves = fragment.flow_blocks(&resolution, source);
                    let partitions =
                        partition_flow(&leaves, range).map_err(HtmlModelError::Partition)?;
                    let mut list_items = std::collections::BTreeMap::new();
                    for (id, node) in fragment.nodes.iter().enumerate() {
                        let Some(element) = &resolution.elements[id] else {
                            continue;
                        };
                        if !matches!(
                            element.kind,
                            super::HtmlElementKind::OrderedList
                                | super::HtmlElementKind::UnorderedList
                        ) {
                            continue;
                        }
                        let items: Vec<_> = node
                            .children
                            .iter()
                            .copied()
                            .filter(|item| {
                                resolution.elements[*item]
                                    .as_ref()
                                    .is_some_and(|e| e.kind == super::HtmlElementKind::ListItem)
                            })
                            .collect();
                        let tight = !items.iter().any(|item| {
                            fragment.nodes[*item].children.iter().any(|child| {
                                resolution.elements[*child]
                                    .as_ref()
                                    .is_some_and(|e| e.kind == super::HtmlElementKind::Paragraph)
                            })
                        });
                        for (offset, item) in items.into_iter().enumerate() {
                            let number = (element.kind == super::HtmlElementKind::OrderedList)
                                .then(|| {
                                    element
                                        .attributes
                                        .start
                                        .unwrap_or(1)
                                        .saturating_add(offset as u64)
                                });
                            list_items.insert(item, (number, tight));
                        }
                    }
                    let mut boundary_tags = Vec::new();
                    for node in &fragment.nodes {
                        if let super::HtmlNodeKind::Element { opening, closing } = &node.kind {
                            boundary_tags.push((opening.source, false));
                            if let Some(closing) = closing {
                                boundary_tags.push((closing.source, true));
                            }
                        }
                    }
                    boundary_tags.sort_unstable_by_key(|(range, _)| range.start());
                    let mut model = HtmlBlockModel {
                        cached_disclosures: Vec::new(),
                        boundary_tags,
                        list_items,
                        cell_partitions: Default::default(),
                        fragment,
                        resolution,
                        partitions,
                    };
                    model.refresh_cell_partitions(source)?;
                    Ok(model)
                })();
                HtmlRegion {
                    source: range,
                    model,
                }
            })
            .collect();
        Self::from_regions(document, regions)
    }

    fn from_regions(document: &MarkdownDocument, regions: Vec<HtmlRegion>) -> Self {
        let source = document.source().as_str();
        let replacements = regions
            .iter()
            .filter_map(|region| {
                let model = region.model.as_ref().ok()?;
                let index = document
                    .blocks
                    .block_index_for_offset(region.source.start())?;
                let records = model
                    .partitions
                    .iter()
                    .map(|part| {
                        let kind = match part.content.kind {
                            super::HtmlFlowKind::Paragraph => BlockKind::Paragraph,
                            super::HtmlFlowKind::Heading(level) => BlockKind::Heading { level },
                            super::HtmlFlowKind::Table | super::HtmlFlowKind::Source => {
                                BlockKind::HtmlBlock
                            }
                        };
                        let bytes = &source.as_bytes()
                            [part.source.start().get() as usize..part.source.end().get() as usize];
                        BlockRecord {
                            block: Block {
                                kind,
                                range: part.source,
                            },
                            start_state: BlockState::Normal,
                            end_state: BlockState::Normal,
                            source_hash: crate::extend_hash(SourceHash(0), bytes),
                        }
                    })
                    .collect();
                Some((index, records))
            })
            .collect();
        let projected_blocks = document.blocks.expand_records(replacements);
        let presentation = if regions.is_empty() {
            document.presentation.clone()
        } else {
            std::sync::Arc::new(
                document
                    .presentation
                    .with_html_regions(&regions, document.source()),
            )
        };
        use std::hash::{Hash, Hasher};
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        for details in regions
            .iter()
            .filter_map(|region| region.model.as_ref().ok())
            .flat_map(|model| model.disclosures())
        {
            details.opening.hash(&mut hasher);
            details.open.hash(&mut hasher);
        }
        Self {
            revision: document.revision(),
            disclosure_fingerprint: hasher.finish(),
            regions,
            projected_blocks,
            presentation,
        }
    }

    /// Reveal closed ancestors of a navigation target in presentation only.
    /// The canonical HTML attributes and source bytes remain untouched.
    fn revealing(&self, document: &MarkdownDocument, target: TextRange) -> Option<Self> {
        let index = self.regions.iter().position(|region| {
            region.source.start() <= target.start() && target.start() < region.source.end()
        })?;
        let model = self.regions[index].model.as_ref().ok()?;
        let closed = model
            .disclosures()
            .into_iter()
            .filter(|details| {
                !details.open
                    && details.body.start() <= target.start()
                    && (target.start() < details.body.end()
                        || target.is_empty() && target.start() == details.body.end())
            })
            .map(|details| details.source)
            .collect::<Vec<_>>();
        if closed.is_empty() {
            return None;
        }
        // Normal history restoration usually needs no reveal. Clone only once
        // a closed ancestor at the restored position has actually been found.
        let mut regions = self.regions.clone();
        let region = &mut regions[index];
        let model = region.model.as_mut().ok()?;
        for (id, node) in model.fragment.nodes.iter().enumerate() {
            if closed.contains(&node.source)
                && let Some(element) = &mut model.resolution.elements[id]
            {
                element.attributes.open = true;
            }
        }
        let leaves = model
            .fragment
            .flow_blocks(&model.resolution, document.source().as_str());
        model.partitions = partition_flow(&leaves, region.source).ok()?;
        model
            .refresh_cell_partitions(document.source().as_str())
            .ok()?;
        Some(Self::from_regions(document, regions))
    }

    fn with_disclosure_state(
        &self,
        document: &MarkdownDocument,
        openings: &[TextRange],
        open: bool,
    ) -> Option<Self> {
        let mut regions = self.regions.clone();
        let mut changed = false;
        for region in &mut regions {
            let Ok(model) = &mut region.model else {
                continue;
            };
            let targets = model
                .disclosures()
                .into_iter()
                .filter(|details| {
                    details.open != open
                        && details.open_attribute.is_none()
                        && openings.contains(&details.opening)
                })
                .map(|details| details.source)
                .collect::<Vec<_>>();
            if targets.is_empty() {
                continue;
            }
            for (id, node) in model.fragment.nodes.iter().enumerate() {
                if targets.contains(&node.source)
                    && let Some(element) = &mut model.resolution.elements[id]
                {
                    element.attributes.open = open;
                }
            }
            let leaves = model
                .fragment
                .flow_blocks(&model.resolution, document.source().as_str());
            model.partitions = partition_flow(&leaves, region.source).ok()?;
            model
                .refresh_cell_partitions(document.source().as_str())
                .ok()?;
            changed = true;
        }
        changed.then(|| Self::from_regions(document, regions))
    }

    /// Preview layout sequence; the parser's original sequence is unchanged.
    pub fn projected_blocks(&self) -> &BlockSequence {
        &self.projected_blocks
    }

    pub fn presentation(&self) -> &crate::PresentationTree {
        &self.presentation
    }

    pub fn decorate(
        &self,
        document: &MarkdownDocument,
        block: Block,
        active: Option<TextRange>,
    ) -> Option<crate::BlockDecorations> {
        let snapshot = document.source();
        let model = self.region_for(block.range())?.model.as_ref().ok()?;
        let part = self.partition_for(block.range())?;
        if part.source != block.range() {
            return None;
        }
        // Cross-item selections keep the same visible list geometry. Revealing
        // a focus item's tags while dragging from another item moves its text
        // out from under the pointer and turns a valid range into raw markup.
        // Local caret/range editing and explicit source mode keep their existing
        // reveal behavior; tables already have an independent native projection.
        let active = active.filter(|selection| {
            selection.is_empty()
                || (part.source.start() <= selection.start()
                    && selection.end() <= part.source.end())
                || !self
                    .presentation
                    .path_for_range(part.source)
                    .iter()
                    .any(|id| {
                        self.presentation.nodes()[*id].kind == crate::PresentationKind::ListItem
                    })
        });
        let mut out = crate::ExtensionOutput::default();
        if part.content.kind == super::HtmlFlowKind::Table {
            return Some(
                if model.decorate_table_active(snapshot.as_str(), part, active, &mut out) {
                    document.footnotes().decorate_html(
                        snapshot.as_str(),
                        part.source,
                        active,
                        &mut out,
                    );
                    crate::BlockDecorations::from_output(snapshot, part.source, out)
                } else {
                    crate::BlockDecorations::source(snapshot, block)
                },
            );
        }
        if !model.decorate_paragraph(snapshot.as_str(), part, active, &mut out) {
            return Some(crate::BlockDecorations::source(snapshot, block));
        }
        if !crate::reveals(active, part.source) {
            document
                .footnotes()
                .decorate_html(snapshot.as_str(), part.source, None, &mut out);
        }
        if !crate::reveals(active, part.source)
            && let Some(alignment) = model.alignment(part)
        {
            let style = out.line_style(crate::BlockOrnament::Alignment { alignment });
            out.line(part.source, style);
        }
        if let super::HtmlFlowKind::Heading(level) = part.content.kind {
            let style = out.line_style(crate::BlockOrnament::Heading { level });
            out.line(part.source, style);
        }
        if !crate::reveals(active, part.source) {
            let path = self.presentation.path_for_range(part.source);
            let mut depth = 0u8;
            let mut has_marker = false;
            for id in path {
                let node = &self.presentation.nodes()[id];
                if matches!(node.kind, crate::PresentationKind::List { .. }) {
                    depth = depth.saturating_add(1);
                }
                if node.kind != crate::PresentationKind::ListItem {
                    continue;
                }
                let opening = model.fragment.nodes.iter().find_map(|html| {
                    if html.source != node.source {
                        return None;
                    }
                    match &html.kind {
                        super::HtmlNodeKind::Element { opening, .. } => Some(opening.source),
                        _ => None,
                    }
                });
                if let Some(mark) = opening
                    .filter(|r| r.start() >= part.source.start() && r.end() <= part.source.end())
                {
                    let text = node
                        .item_number
                        .map_or_else(|| "•".to_owned(), |n| format!("{n}."));
                    let style = out.line_style(crate::BlockOrnament::Marker(
                        crate::MarkerOrnament::new(mark, text).with_list_depth(depth),
                    ));
                    out.line(part.source, style);
                    has_marker = true;
                }
            }
            if depth > 0 {
                let style = out.line_style(crate::BlockOrnament::Indent {
                    columns: depth.saturating_sub(u8::from(has_marker)).saturating_mul(2),
                });
                out.line(part.source, style);
            }
        }
        for image in crate::image_spans(document, snapshot, Some(part.source)) {
            if image.source().start() >= part.source.start()
                && image.source().end() <= part.source.end()
                && !crate::reveals(active, part.source)
                && !model.disclosures().iter().any(|details| {
                    !details.open
                        && details.body.start() <= image.source().start()
                        && image.source().end() <= details.body.end()
                })
            {
                let widget = out.widget(crate::BlockWidget::Image(image));
                out.place_widget(image.source(), widget, yu_core::WidgetSide::Before);
            }
        }
        Some(crate::BlockDecorations::from_output(
            snapshot,
            part.source,
            out,
        ))
    }

    pub fn region_for(&self, range: TextRange) -> Option<&HtmlRegion> {
        let index = self
            .regions
            .partition_point(|region| region.source.end() <= range.start());
        self.regions.get(index).filter(|region| {
            region.source.start() <= range.start() && region.source.end() >= range.end()
        })
    }

    /// Half-open lookup. At an internal boundary the next editable partition
    /// owns the position; a range crossing two leaves has no single partition.
    pub fn partition_for(&self, range: TextRange) -> Option<&HtmlFlowPartition> {
        let partitions = &self.region_for(range)?.model.as_ref().ok()?.partitions;
        let index = partitions.partition_point(|part| part.source.end() <= range.start());
        partitions
            .get(index)
            .filter(|part| part.source.start() <= range.start() && part.source.end() >= range.end())
    }
}

impl MarkdownDocument {
    /// Expands the ancestors of a source target without changing the document.
    /// Returns whether the presentation changed; callers must invalidate geometry.
    pub fn reveal_html_range(&mut self, target: TextRange) -> bool {
        if self.source_mode() || target.end() > self.source_len {
            return false;
        }
        let Some(index) = self.html_regions().revealing(self, target) else {
            return false;
        };
        self.html = std::sync::OnceLock::from(std::sync::Arc::new(index));
        true
    }

    pub fn close_revealed_html(&mut self, opening: TextRange) -> bool {
        let Some(index) = self
            .html_regions()
            .with_disclosure_state(self, &[opening], false)
        else {
            return false;
        };
        self.html = std::sync::OnceLock::from(std::sync::Arc::new(index));
        true
    }

    pub(crate) fn inherit_html_reveals(&mut self, previous: &Self, changes: &yu_text::ChangeSet) {
        let Some(index) = previous.html.get() else {
            return;
        };
        let openings = index
            .regions
            .iter()
            .filter_map(|region| region.model.as_ref().ok())
            .flat_map(|model| model.disclosures())
            .filter(|details| details.open && details.open_attribute.is_none())
            .filter(|details| {
                !changes.changes().iter().any(|change| {
                    let range = change.old_range();
                    range.start() < details.opening.end() && details.opening.start() < range.end()
                        || range.is_empty()
                            && details.opening.start() < range.start()
                            && range.start() < details.opening.end()
                })
            })
            .filter_map(|details| {
                crate::map_unchanged_range(previous.revision(), details.opening, changes).ok()
            })
            .collect::<Vec<_>>();
        if openings.is_empty() {
            return;
        }
        if let Some(index) = self
            .html_regions()
            .with_disclosure_state(self, &openings, true)
        {
            self.html = std::sync::OnceLock::from(std::sync::Arc::new(index));
        }
    }

    pub fn html_disclosure_fingerprint(&self) -> u64 {
        self.html_regions().disclosure_fingerprint
    }

    /// Navigation sees every semantic heading, independent of fold state.
    /// This index shares the same source and is never used for visible layout.
    pub fn semantic_html_regions(&self) -> &HtmlIndex {
        self.semantic_html.get_or_init(|| {
            let index = self.html_regions();
            let openings = index
                .regions
                .iter()
                .filter_map(|region| region.model.as_ref().ok())
                .flat_map(|model| model.disclosures())
                .filter(|details| !details.open)
                .map(|details| details.opening)
                .collect::<Vec<_>>();
            if openings.is_empty() {
                return self
                    .html
                    .get()
                    .expect("HTML index initialized above")
                    .clone();
            }
            std::sync::Arc::new(
                index
                    .with_disclosure_state(self, &openings, true)
                    .unwrap_or_else(|| index.clone()),
            )
        })
    }

    /// Lazy revision-owned HTML metadata shared by presentation snapshots.
    pub fn html_regions(&self) -> &HtmlIndex {
        self.html
            .get_or_init(|| std::sync::Arc::new(HtmlIndex::from_document(self)))
    }
}
