//! Source-backed identities for semantic layout leaves.
//!
//! `presentation_kind` combines a syntax leaf with its list/quote ancestry.
//! The normal parser uses this hierarchy for both block identity and boundaries.
//! Lexical `BlockShape` classification below is retained only for source sizes
//! beyond the syntax parser's 32-bit range; such source has no native preview.

use yu_core::TextRange;
use yu_syntax::{NodeKind, Tree};
use yu_text::TextSnapshot;

use crate::block_sequence::{BlockKind, TaskState};
use crate::extension::{SyntaxNode, block_node};
use crate::reference::read_range;

/// Semantic leaves determine boundaries; their ancestors supply container
/// meaning. Byte ranges remain source backed, including editable prefixes.
pub(crate) fn presentation_kind(
    path: &[SyntaxNode<'_>],
    source: &TextSnapshot,
    range: TextRange,
) -> BlockKind {
    let leaf = path.last().expect("flow includes root");
    if let Some(level) = heading_level(leaf.kind()) {
        return BlockKind::Heading { level };
    }
    match leaf.kind() {
        NodeKind::FencedCode => {
            let marks: Vec<_> = leaf
                .children()
                .filter(|child| child.kind() == NodeKind::CodeMark)
                .collect();
            let marker = marks
                .first()
                .and_then(|mark| read_range(source, mark.range()))
                .and_then(|text| text.first().copied())
                .unwrap_or(b'`') as char;
            return BlockKind::FencedCodeBlock {
                marker,
                closed: marks.len() > 1,
            };
        }
        NodeKind::CodeBlock => return BlockKind::IndentedCode,
        NodeKind::FrontMatter => return BlockKind::FrontMatter,
        NodeKind::HorizontalRule => return BlockKind::ThematicBreak,
        NodeKind::LinkReference => return BlockKind::ReferenceDefinition,
        NodeKind::HtmlBlock | NodeKind::CommentBlock | NodeKind::ProcessingInstructionBlock => {
            return BlockKind::HtmlBlock;
        }
        _ => {}
    }
    if let Some(item) = path
        .iter()
        .rev()
        .find(|node| node.kind() == NodeKind::ListItem)
        && let Some(mark) = item
            .children()
            .find(|node| node.kind() == NodeKind::ListMark)
        && range.start() <= mark.range().start()
        && mark.range().end() <= range.end()
    {
        let text = read_range(source, mark.range()).unwrap_or_default();
        let ordered = text.first().is_some_and(u8::is_ascii_digit);
        let marker = text.last().copied().unwrap_or(b'-') as char;
        let start = if ordered {
            std::str::from_utf8(&text[..text.len().saturating_sub(1)])
                .ok()
                .and_then(|text| text.parse().ok())
                .unwrap_or(1)
        } else {
            1
        };
        let depth = path
            .iter()
            .filter(|node| node.kind() == NodeKind::ListItem)
            .count()
            .saturating_sub(1)
            .min(255) as u8;
        return match task_state(*item, source) {
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
        };
    }
    let depth = path
        .iter()
        .filter(|node| node.kind() == NodeKind::Blockquote)
        .count()
        .min(255) as u8;
    if depth > 0 {
        BlockKind::BlockQuote { depth }
    } else {
        BlockKind::Paragraph
    }
}

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
        // 这三种此前都落进下面那个 `_ => Paragraph`，于是 `---` 画成字面的三个
        // 减号、缩进代码与 HTML 块混在段落里被行内语法解析一遍。它们与上面几种
        // 的区别只有一点：**没有行首结构**，所以行扫描器给的形状一律是
        // `Plain`，分类完全由树做。
        (NodeKind::HorizontalRule, BlockShape::Plain) => BlockKind::ThematicBreak,
        // `NodeKind::CodeBlock` 是缩进代码；围栏是 `FencedCode`，在上面。
        (NodeKind::CodeBlock, BlockShape::Plain) => BlockKind::IndentedCode,
        (NodeKind::FrontMatter, BlockShape::Plain) => BlockKind::FrontMatter,
        (NodeKind::HtmlBlock, BlockShape::Plain) => BlockKind::HtmlBlock,
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

    /// 三种「没有行首结构、只有树分得出来」的块。
    ///
    /// 它们此前一律落进 `_ => Paragraph`，于是 `---` 画成字面的三个减号。
    /// 判据落在 `classify` 上而不是画面上：这一刀只让它们**说得出自己是谁**，
    /// 怎么画是下一刀的事。
    #[test]
    fn the_tree_tells_the_three_shapes_that_have_no_line_prefix_apart() {
        for (source, expected) in [
            ("---\n", BlockKind::FrontMatter),
            ("***\n", BlockKind::ThematicBreak),
            ("___\n", BlockKind::ThematicBreak),
            ("    code\n", BlockKind::IndentedCode),
            ("\tcode\n", BlockKind::IndentedCode),
            ("<div>x</div>\n", BlockKind::HtmlBlock),
        ] {
            let snapshot = TextBuffer::new(source).snapshot();
            let parse = yu_syntax::parse(&snapshot).expect("解析得出树");
            let tree = parse.tree();
            let range = TextRange::new(ByteOffset::ZERO, snapshot.len_bytes()).expect("整篇一块");
            assert_eq!(
                classify(Some(tree), &snapshot, range, BlockShape::Plain),
                expected,
                "{source:?}"
            );
        }
    }

    /// **拼法不进块的身份**：`---` / `***` / `___` 是同一个变体，与 Setext 和
    /// ATX 落在同一个 `Heading` 是同一条规矩。上面那条用例已经压住了它，这里
    /// 记下理由。
    ///
    /// 而**缩进代码与围栏是两个变体**：围栏带着 `marker` 与 `closed` 两样负载，
    /// 缩进代码一样都没有。合成一个变体会让每个消费者去匹配一个对另一半没有
    /// 意义的字段。
    #[test]
    fn an_indented_code_block_is_not_a_fenced_one() {
        let snapshot = TextBuffer::new("    code\n").snapshot();
        let parse = yu_syntax::parse(&snapshot).expect("解析得出树");
        let tree = parse.tree();
        let range = TextRange::new(ByteOffset::ZERO, snapshot.len_bytes()).expect("整篇一块");
        let kind = classify(Some(tree), &snapshot, range, BlockShape::Plain);
        assert_eq!(kind, BlockKind::IndentedCode);
        assert_ne!(
            kind.viewport_tag(),
            BlockKind::FencedCodeBlock {
                marker: '`',
                closed: true
            }
            .viewport_tag()
        );
    }

    /// 叶子节点横跨块边界时这个块只是它的一个片段，退回 `Paragraph`。
    ///
    /// **缩进代码块是唯一能跨空行的那种**（`NodeKind::spans_blank_lines`），
    /// 所以它是这条既有规则第一个真正撞上的形状：中间夹一个空行时，行扫描器
    /// 切成三块而树只有一个 `CodeBlock`。三块谁也不完整，都退回段落。
    /// **这不是缺陷，是那条规则在起作用**——认领半个代码块会让画面上出现两段
    /// 各画一半的代码。
    #[test]
    fn a_fragment_of_an_indented_code_block_is_nobody() {
        let source = "    a\n\n    b\n";
        let snapshot = TextBuffer::new(source).snapshot();
        let parse = yu_syntax::parse(&snapshot).expect("解析得出树");
        let tree = parse.tree();
        let first = TextRange::new(ByteOffset::ZERO, ByteOffset::new(6)).expect("第一行");
        assert_eq!(
            classify(Some(tree), &snapshot, first, BlockShape::Plain),
            BlockKind::Paragraph
        );
    }
}
