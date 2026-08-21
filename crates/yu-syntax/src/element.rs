//! 解析中间态：还没进树的节点。
//!
//! 对应 lezer 的 `Element`。块解析与行内解析都先产出 `Element` 森林，最后由
//! [`Element::into_tree`] 一次性折成 [`Tree`]。
//!
//! 上游在这一步走了一趟扁平的 uint16 `Buffer` 再用 `Tree.build` 还原成树，
//! 那是 JS 里为了少建对象。这里 `Element` 本身就是树，直接折。

use crate::node::NodeKind;
use crate::tree::Tree;

/// 一个待入树的节点，位置是**绝对**的（相对文档起点）。
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Element {
    pub kind: NodeKind,
    pub from: u32,
    pub to: u32,
    pub children: Vec<Element>,
}

impl Element {
    pub(crate) fn leaf(kind: NodeKind, from: u32, to: u32) -> Self {
        Self {
            kind,
            from,
            to,
            children: Vec::new(),
        }
    }

    pub(crate) fn new(kind: NodeKind, from: u32, to: u32, children: Vec<Element>) -> Self {
        Self {
            kind,
            from,
            to,
            children,
        }
    }

    /// 折成一棵树。`context_hash` 只盖在根上，子节点不需要——增量复用只在块
    /// 边界发生，块内部的节点不会单独被复用。
    pub(crate) fn into_tree(self, context_hash: u32) -> Tree {
        let base = self.from;
        let (children, positions) = children_into_tree(self.children, base);
        Tree::new(
            self.kind,
            self.to.saturating_sub(self.from),
            context_hash,
            children,
            positions,
        )
    }
}

/// 把一组 `Element` 折成 `(子树, 相对 `base` 的位置)`。
fn children_into_tree(children: Vec<Element>, base: u32) -> (Vec<Tree>, Vec<u32>) {
    let mut trees = Vec::with_capacity(children.len());
    let mut positions = Vec::with_capacity(children.len());
    for child in children {
        positions.push(child.from.saturating_sub(base));
        trees.push(child.into_tree(0));
    }
    (trees, positions)
}

/// 用 `kind` 把一组元素包成一棵树，范围是 `from..to`。
///
/// 对应上游的 `Buffer.writeElements(...).finish(type, length)`。
pub(crate) fn wrap(
    kind: NodeKind,
    from: u32,
    to: u32,
    children: Vec<Element>,
    context_hash: u32,
) -> Tree {
    Element::new(kind, from, to, children).into_tree(context_hash)
}

/// 把容器块的标记（`>`、列表缩进等）插进一串行内元素里。
///
/// 对应上游的 `injectMarks`。块级标记的位置散落在行内内容中间——`> *a*` 里的
/// `>` 在 Emphasis 之前，而多行引用里的 `>` 可能落在某个 Emphasis **内部**。
/// 后一种情况必须递归下去插，否则标记会与它所在的节点交叉，破坏树形。
pub(crate) fn inject_marks(elements: Vec<Element>, marks: Vec<Element>) -> Vec<Element> {
    if marks.is_empty() {
        return elements;
    }
    if elements.is_empty() {
        return marks;
    }
    let mut elements = elements;
    let mut index = 0_usize;
    for mark in marks {
        while index < elements.len() && elements[index].to < mark.to {
            index += 1;
        }
        if index < elements.len() && elements[index].from < mark.from {
            let existing = &mut elements[index];
            let children = std::mem::take(&mut existing.children);
            existing.children = inject_marks(children, vec![mark]);
        } else {
            elements.insert(index, mark);
            index += 1;
        }
    }
    elements
}

#[cfg(test)]
mod tests {
    use super::{Element, inject_marks};
    use crate::node::NodeKind;

    #[test]
    fn into_tree_makes_child_positions_relative() {
        let element = Element::new(
            NodeKind::Paragraph,
            100,
            110,
            vec![Element::leaf(NodeKind::Emphasis, 103, 107)],
        );
        let tree = element.into_tree(0);
        assert_eq!(tree.len_bytes(), 10);
        let (child, position) = tree.child(0).expect("有一个子节点");
        assert_eq!(position, 3);
        assert_eq!(child.len_bytes(), 4);
        // 放回绝对位置后与原始 Element 一致。
        let mut cursor = tree.cursor(100);
        assert!(cursor.first_child());
        assert_eq!((cursor.from(), cursor.to()), (103, 107));
    }

    #[test]
    fn marks_landing_inside_an_element_are_injected_recursively() {
        // `*aa` / `>` / `bb*`：QuoteMark 落在 Emphasis 内部。
        let emphasis = Element::new(NodeKind::Emphasis, 0, 10, vec![]);
        let mark = Element::leaf(NodeKind::QuoteMark, 4, 5);
        let result = inject_marks(vec![emphasis], vec![mark]);
        assert_eq!(result.len(), 1, "标记不该变成 Emphasis 的兄弟");
        assert_eq!(result[0].kind, NodeKind::Emphasis);
        assert_eq!(result[0].children.len(), 1);
        assert_eq!(result[0].children[0].kind, NodeKind::QuoteMark);
    }

    #[test]
    fn marks_before_an_element_become_siblings() {
        let emphasis = Element::new(NodeKind::Emphasis, 6, 10, vec![]);
        let mark = Element::leaf(NodeKind::QuoteMark, 0, 1);
        let result = inject_marks(vec![emphasis], vec![mark]);
        assert_eq!(result.len(), 2);
        assert_eq!(result[0].kind, NodeKind::QuoteMark);
        assert_eq!(result[1].kind, NodeKind::Emphasis);
    }
}
