//! 语法树的借用视图。
//!
//! `yu_syntax::Tree` 的节点只知道自己的长度，位置由父节点在下降时累加
//! （`Tree::child` 返回的是**相对**偏移）。extension 要的是绝对的
//! [`TextRange`]，每个 extension 自己累加一遍就会有十份同样的算术——其中
//! 一份写错就是「某种语法的隐藏区间整体偏了几个字节」，不 panic 不报错。
//!
//! 所以累加只在这里做一次。[`SyntaxNode`] 是「一个节点 + 它的绝对起点」，
//! 遍历时把偏移一路带下去。

use yu_core::{ByteOffset, TextRange};
use yu_syntax::{NodeKind, Tree};

/// 语法树里的一个节点，带着它在文档里的绝对位置。
#[derive(Clone, Copy, Debug)]
pub struct SyntaxNode<'a> {
    tree: &'a Tree,
    from: u32,
}

impl<'a> SyntaxNode<'a> {
    pub(crate) const fn new(tree: &'a Tree, from: u32) -> Self {
        Self { tree, from }
    }

    #[must_use]
    pub fn kind(self) -> NodeKind {
        self.tree.kind()
    }

    #[must_use]
    pub const fn start(self) -> u32 {
        self.from
    }

    #[must_use]
    pub fn end(self) -> u32 {
        self.from.saturating_add(self.tree.len_bytes())
    }

    /// 这个节点覆盖的源码区间。
    ///
    /// 终点由起点 `saturating_add` 得来，`start <= end` 因此是构造保证的，
    /// 这里不返回 `Option`：把一个不可能的分支交给十个调用方，只会让它们各写
    /// 一遍无法触发的兜底，而那种兜底通常是「跳过这一段」——真出了事就是
    /// 静默地少隐藏一段语法。宁可响。
    #[must_use]
    pub fn range(self) -> TextRange {
        TextRange::new(ByteOffset::from(self.from), ByteOffset::from(self.end()))
            .expect("节点终点由起点 saturating_add 得来，不可能落在起点之前")
    }

    /// 直接子节点，按源码顺序。
    pub fn children(self) -> impl Iterator<Item = SyntaxNode<'a>> {
        (0..self.tree.child_count()).filter_map(move |index| {
            self.tree
                .child(index)
                .map(|(child, offset)| Self::new(child, self.from.saturating_add(offset)))
        })
    }

    /// 前序遍历自己与全部后代。
    #[must_use]
    pub fn descendants(self) -> Descendants<'a> {
        Descendants { stack: vec![self] }
    }

    /// 完整包含 `range` 的最深**块级**节点。
    ///
    /// 块的边界由 `block_sequence` 定，语法树的块结构由 `yu-syntax` 定，
    /// 两者不保证逐字节相同。取「最深的完整包含者」是唯一在两边都成立的
    /// 说法：它至少覆盖整个块，且不会把邻块的语法也带进来。
    ///
    /// # 为什么停在块级节点上
    ///
    /// 下降不设限的话，内容恰好等于某一个标记或行内节点的块会停在那上面：
    /// `+` 单独一行是一个空列表项，块修剪完两头就是 `ListMark` 那一个字节，
    /// 于是「这个块是什么」的答案变成 `ListMark`——一个块级问题拿到了一个
    /// 行内答案。停在块级节点上，答案的值域才与问题对得上。
    ///
    /// # 为什么按位置下降，不逐个子节点扫
    ///
    /// 这个查询现在每个块都要做一次**两遍**：解析时定块的身份
    /// （`crate::classify`），装饰时定块的语法节点。逐个子节点扫的话，
    /// `Document` 下面有多少个块就扫多少次，一篇两万行的文档是
    /// O(块数²)——不报错、不画错，唯一的症状是慢。
    ///
    /// [`TreeCursor::child_ending_after`] 在有序的 `positions` 上二分。
    /// 兄弟节点不重叠，所以「第一个终点晚于 `from` 的子节点」就是唯一有可能
    /// 包含 `range` 的那一个：它不包含，后面的更不可能（它们起点更靠后）。
    ///
    /// **空 range 是这条推理的例外**：终点正好等于 `from` 的子节点会被
    /// `child_ending_after` 跳过，而它本来包含得住。空 range 只有全空白的块
    /// （空行块）会给出来，那种块里一个语法节点都没有，两种答案产出的装饰
    /// 都是空集。
    #[must_use]
    pub fn deepest_block_containing(self, range: TextRange) -> Self {
        let (from, to) = (range.start().get(), range.end().get());
        let Ok(from_u32) = u32::try_from(from) else {
            return self;
        };
        let mut cursor = self.tree.cursor(self.from);
        let mut best = self;
        while cursor.child_ending_after(from_u32) {
            if u64::from(cursor.from()) > from || to > u64::from(cursor.to()) {
                break;
            }
            if !cursor.kind().is_block() {
                break;
            }
            best = Self::new(cursor.tree(), cursor.from());
        }
        best
    }
}

/// [`SyntaxNode::descendants`] 的迭代器。
pub struct Descendants<'a> {
    stack: Vec<SyntaxNode<'a>>,
}

impl<'a> Iterator for Descendants<'a> {
    type Item = SyntaxNode<'a>;

    fn next(&mut self) -> Option<Self::Item> {
        let node = self.stack.pop()?;
        // 压栈顺序反过来，弹出来才是源码顺序。
        let mut children: Vec<_> = node.children().collect();
        children.reverse();
        self.stack.extend(children);
        Some(node)
    }
}

/// 成对行内语法的三段：开定界符、内容、闭定界符。
///
/// 六种成对语法（强调、加粗、行内代码、链接、图片、autolink）在树里是同一个
/// 形状：**头两个标记子节点夹住内容**，节点里除内容之外的部分全是语法。
///
/// ```text
///   [文字](目标)        Link(LinkMark 0..1, LinkMark 7..8, LinkMark, Url, LinkMark)
///   ├┤    ├──────┤      opening = 0..1   closing = 7..16
///     ├──┤                content = 1..7
/// ```
///
/// 链接的 `](目标)` 里还有三个标记子节点，但它们全落在 `closing` 里——按
/// 「内容之外都是语法」取，比逐个列举子节点少一份会漏的清单。autolink 的
/// 第二个标记前面隔着一个 `Url` 子节点，同样落在 `closing` 里。
///
/// # `opening` 为什么从节点起点算起
///
/// 现在这六种语法的第一个标记**都**恰好在节点起点上（`![` 也是），所以
/// `node.start()..open.end()` 与 `open.range()` 逐字节相同——变异验证里把它
/// 换掉是一个等价变异，没有测试会红，也不该有。
///
/// 留着一般式是因为这一句说的是规则本身：**内容之外的都是语法**。哪天树的
/// 形状变了（比如某种语法在第一个标记前面还有别的东西），一般式仍然对，
/// 而 `open.range()` 会静静地把那一段留在视觉文本里。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DelimitedSpan {
    pub opening: TextRange,
    pub content: TextRange,
    pub closing: TextRange,
}

impl DelimitedSpan {
    /// 按「头两个标记子节点」拆分。标记不足两个就不是一段完整的语法。
    #[must_use]
    pub fn of(node: SyntaxNode<'_>, is_mark: impl Fn(NodeKind) -> bool) -> Option<Self> {
        let mut marks = node.children().filter(|child| is_mark(child.kind()));
        let open = marks.next()?;
        let close = marks.next()?;
        let node = node.range();
        Some(Self {
            opening: TextRange::new(node.start(), open.range().end())?,
            content: TextRange::new(open.range().end(), close.range().start())?,
            closing: TextRange::new(close.range().start(), node.end())?,
        })
    }
}
