//! Source-owned heading relationships shared by the sidebar and document TOC.
use yu_core::{ByteOffset, TextRange};
use yu_syntax::NodeKind;

use crate::{BlockKind, MarkdownDocument};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TocHeading {
    pub parent: Option<usize>,
    pub level: u8,
    pub block: usize,
    pub source: TextRange,
    pub label: TextRange,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TocIndex {
    headings: Vec<TocHeading>,
    markers: Vec<TextRange>,
}

impl TocIndex {
    fn from_document(document: &MarkdownDocument) -> Self {
        let mut result = Self::default();
        let mut ancestors: Vec<(usize, u8)> = Vec::new();
        for (block, value) in document.semantic_blocks().into_iter().enumerate() {
            let BlockKind::Heading { level } = value.kind() else {
                continue;
            };
            while ancestors
                .last()
                .is_some_and(|(_, previous)| *previous >= level)
            {
                ancestors.pop();
            }
            let index = result.headings.len();
            result.headings.push(TocHeading {
                parent: ancestors.last().map(|(index, _)| *index),
                level,
                block,
                source: value.range(),
                label: crate::heading_content_range(document, value),
            });
            ancestors.push((index, level));
        }
        if let Some(tree) = document.tree() {
            let mut pending = vec![(tree, 0_u32)];
            while let Some((node, offset)) = pending.pop() {
                if node.kind() == NodeKind::Paragraph {
                    let from = offset as usize;
                    let text = &document.source().as_str()[from..from + node.len_bytes() as usize];
                    let trimmed = text.trim_end_matches([' ', '\t', '\r', '\n']);
                    if trimmed.eq_ignore_ascii_case("[toc]") {
                        result.markers.push(
                            TextRange::new(
                                ByteOffset::new(offset as u64),
                                ByteOffset::new(offset as u64 + trimmed.len() as u64),
                            )
                            .expect("syntax range"),
                        );
                    }
                } else if node.kind().is_block_context() {
                    for index in (0..node.child_count()).rev() {
                        if let Some((child, relative)) = node.child(index) {
                            pending.push((child, offset + relative));
                        }
                    }
                }
            }
        }
        result
    }

    pub fn headings(&self) -> &[TocHeading] {
        &self.headings
    }
    pub fn markers(&self) -> &[TextRange] {
        &self.markers
    }
}

impl MarkdownDocument {
    pub fn toc_marker_in(&self, range: TextRange) -> Option<TextRange> {
        let markers = self.table_of_contents().markers();
        let index = markers.partition_point(|marker| marker.end() <= range.start());
        markers
            .get(index)
            .copied()
            .filter(|marker| marker.start() >= range.start() && marker.end() <= range.end())
    }

    /// Generated TOC text and jump targets depend on headings outside its block.
    pub fn presentation_context_key(&self, range: TextRange) -> u64 {
        let key = self.presentation().context_key(range);
        if self.source_mode() || self.toc_marker_in(range).is_none() {
            return key;
        }
        use std::hash::{Hash, Hasher};
        let mut hash = std::collections::hash_map::DefaultHasher::new();
        key.hash(&mut hash);
        self.revision().hash(&mut hash);
        hash.finish()
    }

    pub fn table_of_contents(&self) -> &TocIndex {
        self.toc
            .get_or_init(|| std::sync::Arc::new(TocIndex::from_document(self)))
    }
}
