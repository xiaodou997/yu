use std::mem;
use std::ops::Range;
use std::sync::Arc;

use super::{AllocationCollector, StorageBackend, StorageStats};

const LEAF_BYTES: usize = 4 * 1024;
type Link = Option<Arc<Node>>;

#[derive(Debug)]
struct Node {
    kind: NodeKind,
    bytes: usize,
    leaves: usize,
    height: usize,
}

#[derive(Debug)]
enum NodeKind {
    Leaf(Arc<str>),
    Branch { left: Arc<Node>, right: Arc<Node> },
}

fn leaf(text: Arc<str>) -> Arc<Node> {
    Arc::new(Node {
        bytes: text.len(),
        leaves: 1,
        height: 1,
        kind: NodeKind::Leaf(text),
    })
}

fn branch(left: Arc<Node>, right: Arc<Node>) -> Arc<Node> {
    Arc::new(Node {
        bytes: left.bytes + right.bytes,
        leaves: left.leaves + right.leaves,
        height: 1 + left.height.max(right.height),
        kind: NodeKind::Branch { left, right },
    })
}

fn concat(left: Link, right: Link) -> Link {
    let (Some(left_text), Some(right_text)) = (last_leaf(&left), first_leaf(&right)) else {
        return concat_raw(left, right);
    };
    if left_text.len() + right_text.len() > LEAF_BYTES {
        return concat_raw(left, right);
    }

    let (left_remainder, left_text) = pop_last(left.expect("boundary requires a left rope"));
    let (right_text, right_remainder) = pop_first(right.expect("boundary requires a right rope"));
    let mut merged = String::with_capacity(left_text.len() + right_text.len());
    merged.push_str(&left_text);
    merged.push_str(&right_text);
    concat_raw(
        concat_raw(left_remainder, Some(leaf(Arc::from(merged)))),
        right_remainder,
    )
}

fn concat_raw(left: Link, right: Link) -> Link {
    match (left, right) {
        (None, right) => right,
        (left, None) => left,
        (Some(left), Some(right)) => Some(join(left, right)),
    }
}

fn first_leaf(root: &Link) -> Option<&Arc<str>> {
    let mut node = root.as_deref()?;
    loop {
        match &node.kind {
            NodeKind::Leaf(text) => return Some(text),
            NodeKind::Branch { left, .. } => node = left,
        }
    }
}

fn last_leaf(root: &Link) -> Option<&Arc<str>> {
    let mut node = root.as_deref()?;
    loop {
        match &node.kind {
            NodeKind::Leaf(text) => return Some(text),
            NodeKind::Branch { right, .. } => node = right,
        }
    }
}

fn pop_first(node: Arc<Node>) -> (Arc<str>, Link) {
    match &node.kind {
        NodeKind::Leaf(text) => (Arc::clone(text), None),
        NodeKind::Branch { left, right } => {
            let (text, remainder) = pop_first(Arc::clone(left));
            (text, concat_raw(remainder, Some(Arc::clone(right))))
        }
    }
}

fn pop_last(node: Arc<Node>) -> (Link, Arc<str>) {
    match &node.kind {
        NodeKind::Leaf(text) => (None, Arc::clone(text)),
        NodeKind::Branch { left, right } => {
            let (remainder, text) = pop_last(Arc::clone(right));
            (concat_raw(Some(Arc::clone(left)), remainder), text)
        }
    }
}

fn join(left: Arc<Node>, right: Arc<Node>) -> Arc<Node> {
    if left.height > right.height + 1 {
        let NodeKind::Branch {
            left: left_left,
            right: left_right,
        } = &left.kind
        else {
            return branch(left, right);
        };
        return balance(Arc::clone(left_left), join(Arc::clone(left_right), right));
    }
    if right.height > left.height + 1 {
        let NodeKind::Branch {
            left: right_left,
            right: right_right,
        } = &right.kind
        else {
            return branch(left, right);
        };
        return balance(join(left, Arc::clone(right_left)), Arc::clone(right_right));
    }
    branch(left, right)
}

fn balance(left: Arc<Node>, right: Arc<Node>) -> Arc<Node> {
    if left.height > right.height + 1 {
        let NodeKind::Branch {
            left: left_left,
            right: left_right,
        } = &left.kind
        else {
            return branch(left, right);
        };
        if left_left.height >= left_right.height {
            return branch(Arc::clone(left_left), branch(Arc::clone(left_right), right));
        }
        let NodeKind::Branch {
            left: middle_left,
            right: middle_right,
        } = &left_right.kind
        else {
            return branch(left, right);
        };
        return branch(
            branch(Arc::clone(left_left), Arc::clone(middle_left)),
            branch(Arc::clone(middle_right), right),
        );
    }

    if right.height > left.height + 1 {
        let NodeKind::Branch {
            left: right_left,
            right: right_right,
        } = &right.kind
        else {
            return branch(left, right);
        };
        if right_right.height >= right_left.height {
            return branch(
                branch(left, Arc::clone(right_left)),
                Arc::clone(right_right),
            );
        }
        let NodeKind::Branch {
            left: middle_left,
            right: middle_right,
        } = &right_left.kind
        else {
            return branch(left, right);
        };
        return branch(
            branch(left, Arc::clone(middle_left)),
            branch(Arc::clone(middle_right), Arc::clone(right_right)),
        );
    }

    branch(left, right)
}

fn from_text(text: Arc<str>) -> Link {
    if text.is_empty() {
        return None;
    }

    let mut leaves = Vec::new();
    let mut start = 0;
    while start < text.len() {
        let mut end = (start + LEAF_BYTES).min(text.len());
        while !text.is_char_boundary(end) {
            end -= 1;
        }
        leaves.push(leaf(Arc::from(&text[start..end])));
        start = end;
    }
    build_balanced(&leaves)
}

fn build_balanced(leaves: &[Arc<Node>]) -> Link {
    match leaves.len() {
        0 => None,
        1 => Some(Arc::clone(&leaves[0])),
        _ => {
            let middle = leaves.len() / 2;
            concat(
                build_balanced(&leaves[..middle]),
                build_balanced(&leaves[middle..]),
            )
        }
    }
}

fn split(root: Link, offset: usize) -> (Link, Link) {
    let Some(node) = root else {
        return (None, None);
    };
    match &node.kind {
        NodeKind::Leaf(text) => {
            let before = (!text[..offset].is_empty()).then(|| leaf(Arc::from(&text[..offset])));
            let after = (!text[offset..].is_empty()).then(|| leaf(Arc::from(&text[offset..])));
            (before, after)
        }
        NodeKind::Branch { left, right } if offset < left.bytes => {
            let (before, remainder) = split(Some(Arc::clone(left)), offset);
            (before, concat(remainder, Some(Arc::clone(right))))
        }
        NodeKind::Branch { left, right } => {
            let (remainder, after) = split(Some(Arc::clone(right)), offset - left.bytes);
            (concat(Some(Arc::clone(left)), remainder), after)
        }
    }
}

fn append_range(root: &Link, range: Range<usize>, output: &mut String) {
    let Some(node) = root else { return };
    match &node.kind {
        NodeKind::Leaf(text) => output.push_str(&text[range]),
        NodeKind::Branch { left, right } => {
            if range.start < left.bytes {
                append_range(
                    &Some(Arc::clone(left)),
                    range.start..range.end.min(left.bytes),
                    output,
                );
            }
            if range.end > left.bytes {
                append_range(
                    &Some(Arc::clone(right)),
                    range.start.saturating_sub(left.bytes)..range.end - left.bytes,
                    output,
                );
            }
        }
    }
}

fn write_all(root: &Link, output: &mut String) {
    let Some(node) = root else { return };
    match &node.kind {
        NodeKind::Leaf(text) => output.push_str(text),
        NodeKind::Branch { left, right } => {
            write_all(&Some(Arc::clone(left)), output);
            write_all(&Some(Arc::clone(right)), output);
        }
    }
}

fn is_char_boundary(root: &Link, offset: usize) -> bool {
    let Some(node) = root else { return offset == 0 };
    if offset == 0 || offset == node.bytes {
        return true;
    }
    match &node.kind {
        NodeKind::Leaf(text) => text.is_char_boundary(offset),
        NodeKind::Branch { left, right } if offset < left.bytes => {
            is_char_boundary(&Some(Arc::clone(left)), offset)
        }
        NodeKind::Branch { left, right } if offset == left.bytes => true,
        NodeKind::Branch { left, right } => {
            is_char_boundary(&Some(Arc::clone(right)), offset - left.bytes)
        }
    }
}

fn collect_allocations(root: &Link, collector: &mut AllocationCollector) {
    let Some(node) = root else { return };
    if !collector.add_node(node, mem::size_of::<Node>()) {
        return;
    }
    match &node.kind {
        NodeKind::Leaf(text) => collector.add_text(text),
        NodeKind::Branch { left, right } => {
            collect_allocations(&Some(Arc::clone(left)), collector);
            collect_allocations(&Some(Arc::clone(right)), collector);
        }
    }
}

#[derive(Debug)]
pub(crate) struct RopeStore {
    root: Link,
}

impl RopeStore {
    pub(super) fn new(text: String) -> Self {
        Self {
            root: from_text(Arc::from(text)),
        }
    }

    pub(super) fn len_bytes(&self) -> usize {
        self.root.as_ref().map_or(0, |node| node.bytes)
    }

    pub(super) fn is_char_boundary(&self, offset: usize) -> bool {
        offset <= self.len_bytes() && is_char_boundary(&self.root, offset)
    }

    pub(super) fn slice(&self, range: Range<usize>) -> String {
        let mut output = String::with_capacity(range.len());
        append_range(&self.root, range, &mut output);
        output
    }

    pub(super) fn replace_range(&mut self, range: Range<usize>, inserted: Arc<str>) {
        let (through_end, after) = split(self.root.clone(), range.end);
        let (before, _) = split(through_end, range.start);
        self.root = concat(concat(before, from_text(inserted)), after);
    }

    pub(super) fn snapshot(&self) -> RopeSnapshot {
        RopeSnapshot {
            root: self.root.clone(),
        }
    }

    pub(super) fn stats(&self) -> StorageStats {
        self.snapshot().stats()
    }
}

#[derive(Clone, Debug)]
pub(crate) struct RopeSnapshot {
    root: Link,
}

impl RopeSnapshot {
    pub(super) fn write_to(&self, output: &mut String) {
        write_all(&self.root, output);
    }

    pub(super) fn stats(&self) -> StorageStats {
        let (bytes, leaves, height) = self
            .root
            .as_ref()
            .map_or((0, 0, 0), |node| (node.bytes, node.leaves, node.height));
        let nodes = leaves
            .saturating_mul(2)
            .saturating_sub(usize::from(leaves > 0));
        StorageStats::new(StorageBackend::PersistentRope, bytes, leaves, nodes, height)
    }

    pub(super) fn collect_allocations(&self, collector: &mut AllocationCollector) {
        collect_allocations(&self.root, collector);
    }
}
