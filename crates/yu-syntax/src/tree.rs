//! 语法树。
//!
//! 与 `@lezer/markdown` 的移植关系见 crate 文档。这里记录三处**有意与上游
//! 不同**的实现选择，它们都不改变解析结果：
//!
//! 1. **子节点位置是相对父节点的**（与 lezer 一致）。这是子树能被增量复用的
//!    前提：一棵子树不知道自己在文档里的绝对位置，因此换一个位置仍然有效。
//!    绝对位置由 [`TreeCursor`] 在遍历时累加得出，不存进节点。
//! 2. **没有 `TreeBuffer`。** lezer 用一个扁平 uint16 数组存小的叶子节点，
//!    这是 JS 的内存优化。Rust 里 `Arc<TreeData>` 的开销本来就低，两种表示
//!    并存只会让每个遍历点多一个分支。
//! 3. **不做 balance，也没有匿名节点。** lezer 的 `.balance()` 把宽扁的子节点
//!    列表折成二叉结构，好让 `childAfter` 不必线性扫描；代价是引入没有类型的
//!    中间节点。这里改为在有序的 `positions` 上二分查找，效果相同而树里不出现
//!    用户看不懂的节点——不变量 C2 要求 gap 可由 position 推导，匿名节点会让
//!    这个推导多一层。

use std::fmt;
use std::sync::Arc;

use crate::node::NodeKind;

/// 一棵不可变语法树。克隆是 `Arc` 克隆，增量复用据此成立。
#[derive(Clone)]
pub struct Tree(Arc<TreeData>);

struct TreeData {
    kind: NodeKind,
    /// 本子树覆盖的字节数。
    length: u32,
    /// 解析出本节点时所处的容器上下文的哈希。
    ///
    /// 增量复用时用它回答「这个节点当初是不是在同样的容器里解析出来的」。
    /// 同样的字节在 `> ` 里面和外面解析结果不同，只比较字节是不够的。
    context_hash: u32,
    children: Box<[Tree]>,
    /// 每个子节点相对本子树起点的偏移，升序。
    positions: Box<[u32]>,
}

impl Tree {
    /// 构造一个节点。`positions` 必须与 `children` 等长且升序。
    ///
    /// # Panics
    ///
    /// 两个数组长度不一致时 panic。这是构造期的内部约束，不是输入校验。
    #[must_use]
    pub(crate) fn new(
        kind: NodeKind,
        length: u32,
        context_hash: u32,
        children: Vec<Tree>,
        positions: Vec<u32>,
    ) -> Self {
        assert_eq!(
            children.len(),
            positions.len(),
            "每个子节点都必须有一个位置"
        );
        debug_assert!(
            positions.windows(2).all(|pair| pair[0] <= pair[1]),
            "子节点位置必须升序，否则 child_after 的二分查找会给出错误答案"
        );
        Self(Arc::new(TreeData {
            kind,
            length,
            context_hash,
            children: children.into_boxed_slice(),
            positions: positions.into_boxed_slice(),
        }))
    }

    /// 一个没有子节点的叶子。
    #[must_use]
    pub(crate) fn leaf(kind: NodeKind, length: u32, context_hash: u32) -> Self {
        Self::new(kind, length, context_hash, Vec::new(), Vec::new())
    }

    /// 换一个 context hash，其余不变。子节点是 `Arc` 克隆，不深拷贝。
    #[must_use]
    pub(crate) fn with_context_hash(&self, context_hash: u32) -> Self {
        if self.0.context_hash == context_hash {
            return self.clone();
        }
        Self(Arc::new(TreeData {
            kind: self.0.kind,
            length: self.0.length,
            context_hash,
            children: self.0.children.clone(),
            positions: self.0.positions.clone(),
        }))
    }

    #[must_use]
    pub fn kind(&self) -> NodeKind {
        self.0.kind
    }

    /// 本子树覆盖的字节数。
    #[must_use]
    pub fn len_bytes(&self) -> u32 {
        self.0.length
    }

    #[must_use]
    pub(crate) fn context_hash(&self) -> u32 {
        self.0.context_hash
    }

    #[must_use]
    pub fn child_count(&self) -> usize {
        self.0.children.len()
    }

    /// 第 `index` 个子节点及其相对本子树起点的偏移。
    #[must_use]
    pub fn child(&self, index: usize) -> Option<(&Tree, u32)> {
        let child = self.0.children.get(index)?;
        let position = self.0.positions[index];
        Some((child, position))
    }

    /// 第一个「结束位置严格晚于 `pos`」的子节点下标，`pos` 相对本子树起点。
    ///
    /// 对应 lezer 的 `childAfter` / `Side.After`（判据是 `to > pos`，不是
    /// `from >= pos`）。增量复用的定位完全依赖这个语义。
    #[must_use]
    pub(crate) fn first_child_ending_after(&self, pos: u32) -> Option<usize> {
        // positions 升序，但 `to = position + length` 不一定单调（子节点可以
        // 长度为 0，也可以互相包含），因此先用二分缩小到「起点 <= pos」的最后
        // 一个，再线性向前找。实际树里重叠不发生，这个回退最多走几步。
        let start = self.0.positions.partition_point(|&p| p <= pos);
        let scan_from = start.saturating_sub(1);
        (scan_from..self.0.children.len())
            .find(|&index| self.0.positions[index] + self.0.children[index].0.length > pos)
    }

    /// 以本树为根、绝对起点为 `from` 的游标。
    #[must_use]
    pub fn cursor(&self, from: u32) -> TreeCursor<'_> {
        TreeCursor {
            stack: vec![Frame {
                tree: self,
                from,
                index_in_parent: 0,
            }],
        }
    }

    /// `Document(Paragraph(Emphasis))` 形式的结构串，只给测试和调试用。
    #[must_use]
    pub fn to_sexp(&self) -> String {
        let mut out = String::new();
        self.write_sexp(&mut out);
        out
    }

    fn write_sexp(&self, out: &mut String) {
        out.push_str(self.0.kind.name());
        if self.0.children.is_empty() {
            return;
        }
        out.push('(');
        for (index, child) in self.0.children.iter().enumerate() {
            if index > 0 {
                out.push(',');
            }
            child.write_sexp(out);
        }
        out.push(')');
    }
}

impl fmt::Debug for Tree {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}[{}]", self.to_sexp(), self.0.length)
    }
}

/// 结构相等：类型、长度、子节点位置与子节点全部相同。
///
/// **不比较 `context_hash`**。它是增量复用的辅助信息，不是解析结果的一部分；
/// 把它算进相等性会让不变量 C3 的差分测试对一个与语义无关的字段报警。
impl PartialEq for Tree {
    fn eq(&self, other: &Self) -> bool {
        if Arc::ptr_eq(&self.0, &other.0) {
            return true;
        }
        self.0.kind == other.0.kind
            && self.0.length == other.0.length
            && self.0.positions == other.0.positions
            && self.0.children == other.0.children
    }
}

impl Eq for Tree {}

struct Frame<'a> {
    tree: &'a Tree,
    /// 本节点的绝对起点。
    from: u32,
    index_in_parent: usize,
}

/// 在一棵树上移动的游标，报告绝对位置。
///
/// 位置在节点里是相对的（见本模块文档），游标在下降时累加、上升时丢弃，
/// 因此 `from()` / `to()` 永远是 document-relative 的——这正是选择移植
/// lezer 的理由之一。
pub struct TreeCursor<'a> {
    stack: Vec<Frame<'a>>,
}

impl<'a> TreeCursor<'a> {
    fn top(&self) -> &Frame<'a> {
        self.stack.last().expect("游标至少持有根节点")
    }

    #[must_use]
    pub fn kind(&self) -> NodeKind {
        self.top().tree.0.kind
    }

    /// 当前节点的绝对起点。
    #[must_use]
    pub fn from(&self) -> u32 {
        self.top().from
    }

    /// 当前节点的绝对终点。
    #[must_use]
    pub fn to(&self) -> u32 {
        let frame = self.top();
        frame.from + frame.tree.0.length
    }

    #[must_use]
    pub fn tree(&self) -> &'a Tree {
        self.top().tree
    }

    /// 下降到第一个子节点。没有子节点时返回 `false` 且不移动。
    pub fn first_child(&mut self) -> bool {
        self.enter_child(0)
    }

    /// 下降到第一个结束位置晚于绝对位置 `pos` 的子节点。
    pub fn child_ending_after(&mut self, pos: u32) -> bool {
        let frame = self.top();
        let relative = pos.saturating_sub(frame.from);
        match frame.tree.first_child_ending_after(relative) {
            Some(index) => self.enter_child(index),
            None => false,
        }
    }

    fn enter_child(&mut self, index: usize) -> bool {
        let frame = self.top();
        let Some((child, position)) = frame.tree.child(index) else {
            return false;
        };
        let from = frame.from + position;
        self.stack.push(Frame {
            tree: child,
            from,
            index_in_parent: index,
        });
        true
    }

    /// 移动到下一个兄弟节点。没有父节点或已是最后一个时返回 `false` 且不移动。
    pub fn next_sibling(&mut self) -> bool {
        if self.stack.len() < 2 {
            return false;
        }
        let index = self.top().index_in_parent + 1;
        let popped = self.stack.pop().expect("刚判断过深度至少为 2");
        if self.enter_child(index) {
            true
        } else {
            self.stack.push(popped);
            false
        }
    }

    /// 上升到父节点。已在根节点时返回 `false` 且不移动。
    pub fn parent(&mut self) -> bool {
        if self.stack.len() < 2 {
            return false;
        }
        self.stack.pop();
        true
    }
}

#[cfg(test)]
mod tests {
    use super::Tree;
    use crate::node::NodeKind;

    /// `Document[10]( Paragraph@0[4], Paragraph@6[4] )`
    fn sample() -> Tree {
        let first = Tree::leaf(NodeKind::Paragraph, 4, 0);
        let second = Tree::new(
            NodeKind::Paragraph,
            4,
            0,
            vec![Tree::leaf(NodeKind::Emphasis, 2, 0)],
            vec![1],
        );
        Tree::new(NodeKind::Document, 10, 0, vec![first, second], vec![0, 6])
    }

    #[test]
    fn cursor_reports_absolute_positions() {
        let tree = sample();
        let mut cursor = tree.cursor(100);
        assert_eq!((cursor.from(), cursor.to()), (100, 110));
        assert!(cursor.first_child());
        assert_eq!((cursor.from(), cursor.to()), (100, 104));
        assert!(cursor.next_sibling());
        assert_eq!((cursor.from(), cursor.to()), (106, 110));
        assert!(cursor.first_child());
        assert_eq!(cursor.kind(), NodeKind::Emphasis);
        assert_eq!((cursor.from(), cursor.to()), (107, 109));
        assert!(cursor.parent());
        assert_eq!(cursor.kind(), NodeKind::Paragraph);
        assert!(!cursor.next_sibling());
        assert!(cursor.parent());
        assert_eq!(cursor.kind(), NodeKind::Document);
        assert!(!cursor.parent());
    }

    /// `child_ending_after` 的判据是 `to > pos`，不是 `from >= pos`。
    /// 两者在「pos 落在某个子节点内部」时给出不同答案，而增量复用的定位
    /// 恰恰总是这种情况。
    #[test]
    fn child_ending_after_enters_the_node_containing_the_position() {
        let tree = sample();
        // 位置 2 落在第一个 Paragraph(0..4) 内部。
        let mut cursor = tree.cursor(0);
        assert!(cursor.child_ending_after(2));
        assert_eq!((cursor.from(), cursor.to()), (0, 4));

        // 位置 4 是第一个 Paragraph 的终点，它不再「结束于 4 之后」。
        let mut cursor = tree.cursor(0);
        assert!(cursor.child_ending_after(4));
        assert_eq!((cursor.from(), cursor.to()), (6, 10));

        // 越过末尾则找不到。
        let mut cursor = tree.cursor(0);
        assert!(!cursor.child_ending_after(10));
    }

    #[test]
    fn failed_moves_leave_the_cursor_where_it_was() {
        let tree = sample();
        let mut cursor = tree.cursor(0);
        assert!(cursor.first_child());
        assert!(!cursor.first_child());
        assert_eq!((cursor.kind(), cursor.from()), (NodeKind::Paragraph, 0));
        assert!(cursor.next_sibling());
        assert!(!cursor.next_sibling());
        assert_eq!(cursor.from(), 6);
    }

    #[test]
    fn structural_equality_ignores_context_hash() {
        let plain = Tree::leaf(NodeKind::Paragraph, 4, 0);
        let stamped = plain.with_context_hash(0xABCD);
        assert_eq!(plain, stamped);
        assert_ne!(plain.context_hash(), stamped.context_hash());
    }

    #[test]
    fn with_context_hash_shares_children() {
        let tree = sample();
        let stamped = tree.with_context_hash(7);
        assert_eq!(stamped.context_hash(), 7);
        assert_eq!(stamped.to_sexp(), tree.to_sexp());
        // 子节点是共享的，不是深拷贝。
        let (original_child, _) = tree.child(1).expect("有第二个子节点");
        let (stamped_child, _) = stamped.child(1).expect("有第二个子节点");
        assert!(std::sync::Arc::ptr_eq(&original_child.0, &stamped_child.0));
    }
}
