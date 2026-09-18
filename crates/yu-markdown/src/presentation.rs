//! Hierarchical, source-backed document presentation. Containers come from
//! the syntax tree, never from guessed indentation in a rendering backend.

use yu_core::{ByteOffset, TextRange};
use yu_syntax::{NodeKind, Tree};
use yu_text::TextSnapshot;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PresentationKind {
    Document,
    Paragraph,
    /// An intentional empty paragraph between top-level blocks. Its source is
    /// one blank line; the surrounding separator lines remain unpresented.
    EmptyParagraph,
    Heading(u8),
    List {
        ordered: bool,
        tight: bool,
        start: u64,
    },
    ListItem,
    Quote,
    Code,
    Rule,
    Html,
    Reference,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PresentationNode {
    pub kind: PresentationKind,
    pub source: TextRange,
    pub parent: Option<usize>,
    pub children: Vec<usize>,
    pub item_number: Option<u64>,
    context_key: u64,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PresentationTree {
    nodes: Vec<PresentationNode>,
    leaves: Vec<usize>,
}

impl PresentationTree {
    pub fn from_syntax(tree: Option<&Tree>, source: &TextSnapshot) -> Self {
        let mut result = Self::default();
        if let Some(tree) = tree {
            result.append(tree, 0, None, source);
            result.append_empty_paragraphs(source);
        }
        result
    }

    fn append_empty_paragraphs(&mut self, source: &TextSnapshot) {
        let Some(root) = self
            .nodes
            .iter()
            .position(|node| node.kind == PresentationKind::Document)
        else {
            return;
        };
        let original = self.nodes[root].children.clone();
        let mut children = Vec::with_capacity(original.len());
        for (index, &id) in original.iter().enumerate() {
            children.push(id);
            let start = self.nodes[id].source.start().get() as usize;
            let end = original
                .get(index + 1)
                .map_or(source.len_bytes().get() as usize, |next| {
                    self.nodes[*next].source.start().get() as usize
                });
            let Ok(chunks) = source.chunk_cursor(ByteOffset::new(start as u64)) else {
                continue;
            };
            let mut newlines = Vec::new();
            'chunks: for chunk in chunks {
                for (index, byte) in chunk.text().bytes().enumerate() {
                    let offset = chunk.start().get() as usize + index;
                    if offset >= end {
                        break 'chunks;
                    }
                    if offset < start {
                        continue;
                    }
                    match byte {
                        b'\n' => newlines.push(offset),
                        b'\r' | b' ' | b'\t' => {}
                        _ => newlines.clear(),
                    }
                }
            }
            // Two or three newlines only separate blocks. Each additional pair
            // represents an empty paragraph, not two independently tall rows.
            for line in (2..newlines.len().saturating_sub(1)).step_by(2) {
                let range = TextRange::new(
                    ByteOffset::new((newlines[line - 1] + 1) as u64),
                    ByteOffset::new((newlines[line] + 1) as u64),
                )
                .expect("ordered newline range");
                // Whitespace owned by a code/HTML/container node is not a
                // document-level separator, notably in an unfinished fence.
                if range.start() < self.nodes[id].source.end() {
                    continue;
                }
                let empty = self.nodes.len();
                self.nodes.push(PresentationNode {
                    kind: PresentationKind::EmptyParagraph,
                    source: range,
                    parent: Some(root),
                    children: Vec::new(),
                    item_number: None,
                    context_key: self.nodes[root].context_key ^ 0x656d707479706172,
                });
                self.leaves.push(empty);
                children.push(empty);
            }
        }
        self.nodes[root].children = children;
        self.leaves.sort_by_key(|id| self.nodes[*id].source.start());
    }

    pub fn nodes(&self) -> &[PresentationNode] {
        &self.nodes
    }

    fn append(&mut self, tree: &Tree, start: u32, parent: Option<usize>, source: &TextSnapshot) {
        let kind = match tree.kind() {
            NodeKind::Document => PresentationKind::Document,
            NodeKind::Blockquote => PresentationKind::Quote,
            NodeKind::BulletList | NodeKind::OrderedList => PresentationKind::List {
                ordered: tree.kind() == NodeKind::OrderedList,
                tight: !has_separating_blank_line(tree, start, source),
                start: ordered_start(tree, start, source),
            },
            NodeKind::ListItem => PresentationKind::ListItem,
            NodeKind::Paragraph | NodeKind::Task => PresentationKind::Paragraph,
            NodeKind::CodeBlock | NodeKind::FencedCode => PresentationKind::Code,
            NodeKind::HorizontalRule => PresentationKind::Rule,
            NodeKind::LinkReference => PresentationKind::Reference,
            NodeKind::HtmlBlock | NodeKind::CommentBlock | NodeKind::ProcessingInstructionBlock => {
                PresentationKind::Html
            }
            node if node.atx_heading_level().is_some() => {
                PresentationKind::Heading(node.atx_heading_level().expect("heading"))
            }
            NodeKind::SetextHeading1 => PresentationKind::Heading(1),
            NodeKind::SetextHeading2 => PresentationKind::Heading(2),
            _ => return,
        };
        let Some(range) = TextRange::new(
            ByteOffset::new(start as u64),
            ByteOffset::new(start as u64 + tree.len_bytes() as u64),
        ) else {
            return;
        };
        let id = self.nodes.len();
        let item_number = parent.and_then(|parent| {
            if kind != PresentationKind::ListItem {
                return None;
            }
            let list = &self.nodes[parent];
            match list.kind {
                PresentationKind::List {
                    ordered: true,
                    start,
                    ..
                } => Some(start.saturating_add(list.children.len() as u64)),
                _ => None,
            }
        });
        use std::hash::{Hash, Hasher};
        let mut hash = std::collections::hash_map::DefaultHasher::new();
        parent.map(|id| self.nodes[id].context_key).hash(&mut hash);
        kind.hash(&mut hash);
        item_number.hash(&mut hash);
        // A top-level heading changes its leading box margin when another
        // semantic block is inserted ahead of it. Invalidate only that node.
        if matches!(kind, PresentationKind::Heading(_)) {
            parent
                .is_some_and(|id| {
                    self.nodes[id].kind == PresentationKind::Document
                        && self.nodes[id].children.is_empty()
                })
                .hash(&mut hash);
        }
        self.nodes.push(PresentationNode {
            context_key: hash.finish(),
            item_number,
            kind,
            source: range,
            parent,
            children: Vec::new(),
        });
        if let Some(parent) = parent {
            self.nodes[parent].children.push(id);
        }
        if !tree.kind().is_block_context() {
            self.leaves.push(id);
        }
        if tree.kind().is_block_context() {
            for index in 0..tree.child_count() {
                if let Some((child, offset)) = tree.child(index) {
                    self.append(child, start + offset, Some(id), source);
                }
            }
        }
    }

    /// Match a lossless layout block (which includes line prefixes/newlines)
    /// to its semantic leaf. Leaves are ordered and do not overlap.
    pub fn leaf_for_range(&self, range: TextRange) -> Option<usize> {
        let position = self
            .leaves
            .partition_point(|id| self.nodes[*id].source.end() <= range.start());
        self.leaves
            .get(position)
            .copied()
            .filter(|id| self.nodes[*id].source.start() < range.end())
    }

    /// Trailing container markers can extend beyond the last content leaf.
    /// They do not extend the visible box or its external margin.
    pub fn content_end(&self, id: usize) -> ByteOffset {
        let range = self.nodes[id].source;
        let end = self
            .leaves
            .partition_point(|leaf| self.nodes[*leaf].source.start() < range.end());
        end.checked_sub(1)
            .and_then(|index| self.leaves.get(index))
            .map(|leaf| &self.nodes[*leaf])
            .filter(|leaf| leaf.source.end() > range.start())
            .map_or(range.end(), |leaf| leaf.source.end())
    }

    /// Root-to-leaf ancestry, including containers whose marker precedes the
    /// leaf's own source start. No guessed source indentation enters geometry.
    pub fn path_for_range(&self, range: TextRange) -> Vec<usize> {
        let Some(leaf) = self.leaf_for_range(range) else {
            return Vec::new();
        };
        let mut path = Vec::new();
        let mut current = Some(leaf);
        while let Some(id) = current {
            path.push(id);
            current = self.nodes[id].parent;
        }
        path.reverse();
        path
    }

    /// Structural layout dependencies, excluding byte positions and sibling
    /// text. Renumbering or moving a paragraph into another container must
    /// invalidate it; an unrelated prefix insertion must not.
    pub fn context_key(&self, range: TextRange) -> u64 {
        self.leaf_for_range(range)
            .map_or(0, |id| self.nodes[id].context_key)
    }

    /// Only direct paragraphs of a tight list item lose paragraph margins.
    /// Paragraphs inside a nested quote keep their own block spacing.
    pub fn is_tight_paragraph(&self, leaf: usize) -> bool {
        let node = &self.nodes[leaf];
        if node.kind != PresentationKind::Paragraph {
            return false;
        }
        let Some(item) = node
            .parent
            .filter(|id| self.nodes[*id].kind == PresentationKind::ListItem)
        else {
            return false;
        };
        self.nodes[item].parent.is_some_and(|id| {
            matches!(
                self.nodes[id].kind,
                PresentationKind::List { tight: true, .. }
            )
        })
    }
}

fn ordered_start(tree: &Tree, start: u32, source: &TextSnapshot) -> u64 {
    if tree.kind() != NodeKind::OrderedList {
        return 1;
    }
    for index in 0..tree.child_count() {
        let Some((item, item_offset)) = tree.child(index) else {
            continue;
        };
        if item.kind() != NodeKind::ListItem {
            continue;
        }
        for mark_index in 0..item.child_count() {
            let Some((mark, mark_offset)) = item.child(mark_index) else {
                continue;
            };
            if mark.kind() == NodeKind::ListMark {
                let begin = (start + item_offset + mark_offset) as usize;
                let end = begin + mark.len_bytes().saturating_sub(1) as usize;
                return source
                    .as_str()
                    .get(begin..end)
                    .and_then(|text| text.parse().ok())
                    .unwrap_or(1);
            }
        }
    }
    1
}

fn has_separating_blank_line(tree: &Tree, start: u32, source: &TextSnapshot) -> bool {
    let mut previous_end = None;
    for index in 0..tree.child_count() {
        let Some((child, offset)) = tree.child(index) else {
            continue;
        };
        if !child.kind().is_block() {
            continue;
        }
        let begin = (start + offset) as usize;
        if let Some(end) = previous_end {
            let gap = source.as_str().get(end..begin).unwrap_or("");
            if gap.bytes().filter(|byte| *byte == b'\n').count() > 1 {
                return true;
            }
        }
        if child.kind() == NodeKind::ListItem
            && has_separating_blank_line(child, start + offset, source)
        {
            return true;
        }
        previous_end = Some(begin + child.len_bytes() as usize);
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use yu_text::TextBuffer;

    #[test]
    fn first_heading_context_changes_without_invalidating_later_headings() {
        fn heading_keys(text: &str) -> Vec<u64> {
            let source = TextBuffer::new(text).snapshot();
            let parsed = crate::parse(&source);
            parsed
                .presentation()
                .nodes()
                .iter()
                .filter(|node| matches!(node.kind, PresentationKind::Heading(_)))
                .map(|node| node.context_key)
                .collect()
        }
        let original = heading_keys("# first\n\nbody\n\n# later\n");
        let prefixed = heading_keys("prefix\n\n# first\n\nbody\n\n# later\n");
        let blank_prefixed = heading_keys("\n\n# first\n\nbody\n\n# later\n");
        assert_ne!(original[0], prefixed[0], "first-heading margin changed");
        assert_eq!(original[1], prefixed[1], "unrelated heading stays cached");
        assert_eq!(
            original, blank_prefixed,
            "source blank lines add no visible block"
        );
    }

    #[test]
    fn intentional_empty_paragraphs_preserve_separator_source_ranges() {
        for breaks in 2..=8 {
            let text = format!("before{}after\n", "\n".repeat(breaks));
            let source = TextBuffer::new(&text).snapshot();
            let parsed = crate::parse(&source);
            let tree = parsed.presentation();
            let empty: Vec<_> = tree
                .nodes()
                .iter()
                .filter(|node| node.kind == PresentationKind::EmptyParagraph)
                .collect();
            assert_eq!(empty.len(), breaks / 2 - 1, "{text:?}");
            for node in empty {
                assert_eq!(
                    &text[node.source.start().get() as usize..node.source.end().get() as usize],
                    "\n"
                );
                let leaf = tree.leaf_for_range(node.source).expect("source mapping");
                assert_eq!(tree.nodes()[leaf].kind, PresentationKind::EmptyParagraph);
            }
            assert_eq!(source.as_str(), text);
        }
        let source = TextBuffer::new("```\na\n\n\n\nb\n```\n").snapshot();
        assert!(
            !crate::parse(&source)
                .presentation()
                .nodes()
                .iter()
                .any(|node| node.kind == PresentationKind::EmptyParagraph)
        );
        for text in ["```\na\n\n\n\n", "~~~\na\n\n\n\n"] {
            let source = TextBuffer::new(text).snapshot();
            let parsed = crate::parse(&source);
            assert!(
                !parsed
                    .presentation()
                    .nodes()
                    .iter()
                    .any(|node| node.kind == PresentationKind::EmptyParagraph),
                "{text:?}"
            );
        }
    }

    #[test]
    fn containers_preserve_nested_parents_and_list_spacing_semantics() {
        let source = TextBuffer::new(
            "> - first\n>   - child\n> - last\n\n- loose\n\n  second paragraph\n\n- end\n",
        )
        .snapshot();
        let parsed = crate::parse(&source);
        let nodes = parsed.presentation().nodes();
        assert!(
            nodes
                .iter()
                .any(|node| node.kind == PresentationKind::Quote)
        );
        assert!(
            nodes
                .iter()
                .any(|node| matches!(node.kind, PresentationKind::List { tight: true, .. }))
        );
        assert!(
            nodes
                .iter()
                .any(|node| matches!(node.kind, PresentationKind::List { tight: false, .. }))
        );
        for (id, node) in nodes.iter().enumerate().skip(1) {
            let parent = &nodes[node.parent.expect("parent")];
            assert!(parent.children.contains(&id));
            assert!(parent.source.start() <= node.source.start());
            assert!(node.source.end() <= parent.source.end());
        }
    }
}
