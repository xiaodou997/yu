//! 块是什么，由语法树说。
//!
//! # 这一层要回答的问题
//!
//! `block_sequence` 是一个按行走的扁平扫描器，它一个人干两件事：**块从哪到
//! 哪**（边界），以及**块是什么**（`BlockKind`）。第二件事语法树也在做，于是
//! 同一句 Markdown 语法在两处各有一个实现——`# ` 的判断在 `Line::
//! atx_heading_level` 与 `yu-syntax` 的 ATX 解析器里各写了一遍，`[x]` 在
//! `task::parse_task_marker` 与 GFM 的 `TaskList` extension 里各写了一遍。
//!
//! 两份实现不一致的地方就是产品上的缺陷：`标题\n===` 树里是
//! `SetextHeading1`，行扫描器里是一个普通段落，于是 Setext 标题既不放大也不
//! 加粗；`foo\n[a]: /x` 的第二行在 CommonMark 里是段落的延续，行扫描器却把它
//! 登记成一条引用定义。
//!
//! 这个模块把第二件事整个交给树。**边界仍然归行扫描器**——块是编辑器的缓存
//! 与布局单位，换成树的嵌套粒度意味着改嵌套列表里的一个字要重排整个外层项，
//! 那是另一刀的事（见 overview 的「块结构合并：调查结论」）。
//!
//! # 树给变体，行扫描器给负载
//!
//! `BlockKind` 的每一个变体现在都由树的节点类型决定。变体上挂的那些字段
//! （列表标记是 `-` 还是 `*`、序号从几起、围栏闭没闭合）不是分类，是**从源码
//! 字节里读出来的负载**，行扫描器扫边界的时候顺手就读到了，树反而说不出
//! 「这个围栏没有收尾」。所以负载仍然由 [`BlockShape`] 带过来。
//!
//! 判据是「这一句话是不是 Markdown 语法的分类」：是的归树，不是的归谁读到
//! 归谁。
//!
//! # 一个块只是节点的一个片段时，谁也不是
//!
//! 行扫描器的边界与树的块边界不保证对齐，于是**一个树节点可能横跨两个块**。
//! 这时要分两种情况：
//!
//! - **容器节点**（`Blockquote` / `ListItem`）横跨是正常的：块就是容器里的
//!   一组行，`  - 内` 那一块是外层列表项的一部分，说它是一个列表项没有错。
//! - **叶子节点**横跨说明这个块只拿到了它的一半。`foo\n-` 是一个二级 Setext
//!   标题，而行扫描器在 `-` 那一行另起了一块（它看上去像一个列表标记）——
//!   两块都说自己是二级标题的话，画面上会出现两个放大的行，其中一个只有一个
//!   `-`。这种块退回 `Paragraph`：它确实不是任何一种完整的块。
//!
//! 判据是 [`NodeKind::is_block_context`]，不是「节点比块大还是小」。
//!
//! # 树说不是那种块的时候以树为准
//!
//! `2. bar` 跟在一个段落后面时，行扫描器认得那个 `2.` 并开一个新块，而
//! CommonMark 说序号不是 1 的有序列表**不能打断段落**——树给的是一个跨两行的
//! `Paragraph`。这时 kind 取 `Paragraph`：行扫描器在边界上仍然有发言权（那一
//! 行确实另起了一个缓存单位），在「它是什么」上没有。
//!
//! 块**横跨好几个树块**时同理，只是这回连一个候选节点都没有——查询一路退到
//! `Document`。`- a\n<div>\nx` 就是一个：行扫描器把 `<div>` 当成列表项的惰性
//! 延续收进同一个块，树把它拆成 `ListItem` 与一个 `HTMLBlock`。这种块按源码
//! 原样画（不变量 I5「未支持的语法按普通段落源码绘制」）。
//!
//! # 树不在的时候
//!
//! `MarkdownDocument::tree` 只有一种成因会是 `None`：源码超过 4 GiB。那种文档
//! 一条装饰都产不出来，所以这里退化成 [`BlockShape`] 自己的结构形状——`# 标题`
//! 会变成一个普通段落。**这是登记在案的降级**，不是兜底：那份文档本来就是
//! 按纯文本画的。

use yu_core::TextRange;
use yu_syntax::{NodeKind, Tree};
use yu_text::TextSnapshot;

use crate::block_sequence::{BlockKind, TaskState};
use crate::extension::{SyntaxNode, block_node};
use crate::reference::read_range;

/// 行扫描器认出的块结构：它定边界，也带着树表示不了的那部分负载。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum BlockShape {
    /// 没有行首结构的块。段落、Setext 标题、ATX 标题、缩进代码、分隔线、
    /// HTML 块、引用定义都长这样——它们之间的区别全部由树来分。
    Plain,
    Fence {
        marker: char,
        closed: bool,
    },
    Quote {
        depth: u8,
    },
    List {
        ordered: bool,
        depth: u8,
        marker: char,
        start: u32,
    },
}

impl BlockShape {
    /// 没有树的时候的结构形状。见模块文档「树不在的时候」。
    const fn without_tree(self) -> BlockKind {
        match self {
            Self::Plain => BlockKind::Paragraph,
            Self::Fence { marker, closed } => BlockKind::FencedCodeBlock { marker, closed },
            Self::Quote { depth } => BlockKind::BlockQuote { depth },
            Self::List {
                ordered,
                depth,
                marker,
                start,
            } => BlockKind::ListItem {
                ordered,
                depth,
                marker,
                start,
            },
        }
    }
}

/// 一个块的 `BlockKind`。
pub(crate) fn classify(
    tree: Option<&Tree>,
    source: &TextSnapshot,
    range: TextRange,
    shape: BlockShape,
) -> BlockKind {
    let Some(tree) = tree else {
        return shape.without_tree();
    };
    let node = block_node(tree, source, range);
    // 叶子节点横跨块边界时，这个块只是它的一个片段。见模块文档。
    if !node.kind().is_block_context()
        && (node.range().start() < range.start() || range.end() < node.range().end())
    {
        return BlockKind::Paragraph;
    }
    if let Some(level) = heading_level(node.kind()) {
        return BlockKind::Heading { level };
    }
    match (node.kind(), shape) {
        (NodeKind::FencedCode, BlockShape::Fence { marker, closed }) => {
            BlockKind::FencedCodeBlock { marker, closed }
        }
        (NodeKind::Blockquote, BlockShape::Quote { depth }) => BlockKind::BlockQuote { depth },
        (
            NodeKind::ListItem,
            BlockShape::List {
                ordered,
                depth,
                marker,
                start,
            },
        ) => match task_state(node, source) {
            Some(state) => BlockKind::TaskListItem {
                ordered,
                depth,
                marker,
                start,
                state,
            },
            None => BlockKind::ListItem {
                ordered,
                depth,
                marker,
                start,
            },
        },
        (NodeKind::LinkReference, _) => BlockKind::ReferenceDefinition,
        // 块横跨了好几个树块，树说不出它是什么。`- a\n<div>\nx` 就是一个：
        // 行扫描器把 `<div>` 当成列表项的惰性延续收进同一个块，树把它拆成
        // `ListItem` 与一个 `HTMLBlock`，谁也装不下这个块，于是落到
        // `Document`。按源码原样画（不变量 I5），不按行扫描器的形状画——那样
        // 会给这一块画一个列表标记，而它的后半段根本不是列表。
        (NodeKind::Document, _) => BlockKind::Paragraph,
        // 形状与节点对不上：那一行确实另起了一个缓存单位，但它不是那种块。
        _ => BlockKind::Paragraph,
    }
}

/// ATX 与 Setext 映射到同一个「几级标题」。
///
/// **拼法不进 `BlockKind`。** 一个二级标题是不是用下划线写的，只有隐藏区间
/// 需要知道（`extension/heading.rs` 问树），可访问性、导出、字号都不需要。
/// 把它记进块的身份，意味着每一个消费者都要多匹配一个自己不关心的字段。
const fn heading_level(kind: NodeKind) -> Option<u8> {
    match kind {
        NodeKind::SetextHeading1 => Some(1),
        NodeKind::SetextHeading2 => Some(2),
        _ => kind.atx_heading_level(),
    }
}

/// 列表项的复选框状态，没有复选框时是 `None`。
///
/// 勾没勾上从**树给的那三个字节**里读，与 `extension/task.rs` 同一个判据
/// （[`crate::task::checkbox_state`]）：区间与状态出自同一次查询，错不开。
fn task_state(item: SyntaxNode<'_>, source: &TextSnapshot) -> Option<TaskState> {
    let task = item
        .children()
        .find(|child| child.kind() == NodeKind::Task)?;
    let marker = task
        .children()
        .find(|child| child.kind() == NodeKind::TaskMarker)?;
    crate::task::checkbox_state(&read_range(source, marker.range())?)
}

#[cfg(test)]
mod tests {
    use super::{BlockShape, classify};
    use crate::block_sequence::BlockKind;
    use yu_core::{ByteOffset, TextRange};
    use yu_text::TextBuffer;

    /// 没有树的时候块退化成行扫描器的结构形状。
    ///
    /// 唯一的成因是源码超过 4 GiB，测试造不出那种文档，所以直接给
    /// `classify` 一个 `None`。这条断言钉的是**登记在案的降级**：`# 标题`
    /// 变成一个普通段落，而不是随手挑一个变体。
    #[test]
    fn without_a_tree_a_block_falls_back_to_its_line_shape() {
        let snapshot = TextBuffer::new("# 标题\n").snapshot();
        let range = TextRange::new(ByteOffset::ZERO, snapshot.len_bytes()).expect("整篇是一段");
        assert_eq!(
            classify(None, &snapshot, range, BlockShape::Plain),
            BlockKind::Paragraph
        );
        assert_eq!(
            classify(
                None,
                &snapshot,
                range,
                BlockShape::Fence {
                    marker: '`',
                    closed: true
                }
            ),
            BlockKind::FencedCodeBlock {
                marker: '`',
                closed: true
            }
        );
        assert_eq!(
            classify(None, &snapshot, range, BlockShape::Quote { depth: 1 }),
            BlockKind::BlockQuote { depth: 1 }
        );
    }
}
