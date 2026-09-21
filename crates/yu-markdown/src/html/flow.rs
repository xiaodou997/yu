use super::{HtmlElementKind as Kind, HtmlFragment, HtmlNodeKind, HtmlResolution};
use yu_core::TextRange;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HtmlFlowKind {
    Paragraph,
    Heading(u8),
    Table,
    Source,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HtmlFlowBlock {
    pub kind: HtmlFlowKind,
    pub source: TextRange,
    /// Node providing container ancestry, attributes and list identity.
    pub owner: Option<usize>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HtmlFlowPartition {
    /// Lossless editing range, including container syntax between visible leaves.
    pub source: TextRange,
    pub content: HtmlFlowBlock,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HtmlFlowError {
    OutsideSource,
    OverlappingLeaves,
}

/// Assign every source byte to exactly one editable block. Semantic geometry
/// continues to use `content.source`; separators do not create extra paragraphs.
pub fn partition_flow(
    leaves: &[HtmlFlowBlock],
    source: TextRange,
) -> Result<Vec<HtmlFlowPartition>, HtmlFlowError> {
    if leaves.is_empty() {
        return Ok(vec![HtmlFlowPartition {
            source,
            content: HtmlFlowBlock {
                kind: HtmlFlowKind::Source,
                source,
                owner: None,
            },
        }]);
    }
    let mut cursor = source.start();
    let mut result = Vec::with_capacity(leaves.len());
    for (index, leaf) in leaves.iter().enumerate() {
        if leaf.source.start() < source.start() || leaf.source.end() > source.end() {
            return Err(HtmlFlowError::OutsideSource);
        }
        if leaf.source.start() < cursor {
            return Err(HtmlFlowError::OverlappingLeaves);
        }
        let end = if index + 1 == leaves.len() {
            source.end()
        } else {
            leaf.source.end()
        };
        result.push(HtmlFlowPartition {
            source: TextRange::new(cursor, end).expect("ordered partition"),
            content: leaf.clone(),
        });
        cursor = end;
    }
    Ok(result)
}

impl HtmlFragment {
    /// Nonoverlapping semantic leaves. Parent tags remain source-owned metadata;
    /// callers must project them rather than serialize these leaves as Markdown.
    pub fn flow_blocks(&self, resolved: &HtmlResolution, source: &str) -> Vec<HtmlFlowBlock> {
        if !self.diagnostics.is_empty() {
            return self
                .roots
                .iter()
                .map(|&id| HtmlFlowBlock {
                    kind: HtmlFlowKind::Source,
                    source: self.nodes[id].source,
                    owner: Some(id),
                })
                .collect();
        }
        self.flow_blocks_in(resolved, source, &self.roots, None)
    }

    pub(super) fn flow_blocks_in(
        &self,
        resolved: &HtmlResolution,
        source: &str,
        roots: &[usize],
        parent: Option<usize>,
    ) -> Vec<HtmlFlowBlock> {
        let mut pending = vec![(roots, parent)];
        let mut blocks = Vec::new();
        let mut list_items = Vec::new();
        while let Some((children, parent)) = pending.pop() {
            let mut inline: Option<TextRange> = None;
            let mut visible = false;
            for &id in children {
                let node = &self.nodes[id];
                let kind = resolved
                    .elements
                    .get(id)
                    .and_then(Option::as_ref)
                    .map(|e| e.kind);
                if kind == Some(Kind::ListItem) {
                    list_items.push(id);
                }
                let boundary = match &node.kind {
                    HtmlNodeKind::Element { opening, .. } => {
                        kind.is_none()
                            || opening.name == "div"
                            || matches!(
                                kind,
                                Some(
                                    Kind::Paragraph
                                        | Kind::Heading(_)
                                        | Kind::Table
                                        | Kind::UnorderedList
                                        | Kind::OrderedList
                                        | Kind::ListItem
                                        | Kind::Details
                                        | Kind::Summary
                                )
                            )
                    }
                    _ => false,
                };
                if boundary {
                    if let Some(range) = inline.take()
                        && visible
                    {
                        blocks.push(HtmlFlowBlock {
                            kind: HtmlFlowKind::Paragraph,
                            source: range,
                            owner: parent,
                        });
                    }
                    visible = false;
                    if kind == Some(Kind::Details) {
                        let open = resolved.elements[id]
                            .as_ref()
                            .is_some_and(|element| element.attributes.open);
                        if !open {
                            blocks.push(HtmlFlowBlock {
                                kind: HtmlFlowKind::Paragraph,
                                source: node.source,
                                owner: Some(id),
                            });
                            continue;
                        }
                        let has_summary = node.children.iter().any(|&child| {
                            resolved.elements[child]
                                .as_ref()
                                .is_some_and(|element| element.kind == Kind::Summary)
                        });
                        if !has_summary && let HtmlNodeKind::Element { opening, .. } = &node.kind {
                            blocks.push(HtmlFlowBlock {
                                kind: HtmlFlowKind::Paragraph,
                                source: opening.source,
                                owner: Some(id),
                            });
                        }
                    }
                    let leaf = match kind {
                        Some(Kind::Paragraph | Kind::Summary) => Some(HtmlFlowKind::Paragraph),
                        Some(Kind::Heading(level)) => Some(HtmlFlowKind::Heading(level)),
                        Some(Kind::Table) => Some(HtmlFlowKind::Table),
                        None => Some(HtmlFlowKind::Source),
                        _ => None,
                    };
                    if let Some(kind) = leaf {
                        blocks.push(HtmlFlowBlock {
                            kind,
                            source: node.source,
                            owner: Some(id),
                        });
                    } else {
                        pending.push((node.children.as_slice(), Some(id)));
                    }
                } else {
                    inline = Some(inline.map_or(node.source, |range| {
                        TextRange::new(range.start(), node.source.end()).expect("ordered nodes")
                    }));
                    visible |= match node.kind {
                        HtmlNodeKind::Comment => false,
                        HtmlNodeKind::Text => source
                            .get(
                                node.source.start().get() as usize
                                    ..node.source.end().get() as usize,
                            )
                            .is_some_and(|text| !text.trim().is_empty()),
                        _ => true,
                    };
                }
            }
            if let Some(range) = inline
                && visible
            {
                blocks.push(HtmlFlowBlock {
                    kind: HtmlFlowKind::Paragraph,
                    source: range,
                    owner: parent,
                });
            }
        }
        // An empty item is still an editable list entry and consumes a number.
        // Visit descendants first so a nested empty item owns its own leaf;
        // its ancestors must not receive an overlapping synthetic paragraph.
        if !list_items.is_empty() {
            let mut has_leaf = std::collections::HashSet::new();
            let mark_ancestors =
                |mut owner: Option<usize>, covered: &mut std::collections::HashSet<usize>| {
                    while let Some(id) = owner {
                        if !covered.insert(id) {
                            break;
                        }
                        owner = self.nodes[id].parent;
                    }
                };
            for block in &blocks {
                mark_ancestors(block.owner, &mut has_leaf);
            }
            for id in list_items.into_iter().rev() {
                if !has_leaf.contains(&id) {
                    blocks.push(HtmlFlowBlock {
                        kind: HtmlFlowKind::Paragraph,
                        source: self.nodes[id].source,
                        owner: Some(id),
                    });
                    mark_ancestors(Some(id), &mut has_leaf);
                }
            }
        }
        blocks.sort_by_key(|block| block.source.start());
        blocks
    }
}
