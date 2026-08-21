//! 语法节点类型。
//!
//! 编号与 `@lezer/markdown` 的 `Type` 枚举一一对应，顺序也保持一致：算法里
//! 有几处依赖「id 落在某个区间」的判断（`ATXHeading1 - 1 + size`、
//! `id >= Escape` 表示行内节点），改动顺序会静默地改变解析结果。

/// 一个语法节点的类型。
///
/// 变体分三段，段内顺序不可调整：
///
/// 1. `Document` 与块级节点（`Document` ..= `ProcessingInstructionBlock`）；
/// 2. 行内节点（`Escape` ..= `Autolink`）；
/// 3. 标记节点（`HeaderMark` ..= `Url`），语法字符本身。
///
/// 第 3 段是不变量 C2「lossless」得以成立的关键：`#`、`>`、`*` 这些字符都有
/// 自己的节点，装饰阶段据此隐藏语法而不触碰 source。
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(u8)]
pub enum NodeKind {
    // 从 1 开始与上游 `Type` 对齐：0 在 lezer 里是「无类型」，而 context hash
    // 的计算把类型编号直接加了进去，用 0 会让根节点的 hash 与「未设置」撞上。
    Document = 1,

    // 块级。
    CodeBlock,
    FencedCode,
    Blockquote,
    HorizontalRule,
    BulletList,
    OrderedList,
    ListItem,
    AtxHeading1,
    AtxHeading2,
    AtxHeading3,
    AtxHeading4,
    AtxHeading5,
    AtxHeading6,
    SetextHeading1,
    SetextHeading2,
    HtmlBlock,
    LinkReference,
    Paragraph,
    CommentBlock,
    ProcessingInstructionBlock,

    // 行内。
    Escape,
    Entity,
    HardBreak,
    Emphasis,
    StrongEmphasis,
    Link,
    Image,
    InlineCode,
    HtmlTag,
    Comment,
    ProcessingInstruction,
    Autolink,

    // 标记。
    HeaderMark,
    QuoteMark,
    ListMark,
    LinkMark,
    EmphasisMark,
    CodeMark,
    CodeText,
    CodeInfo,
    LinkTitle,
    LinkLabel,
    Url,
}

impl NodeKind {
    /// 稳定的名字。用于测试断言与调试输出，不参与解析判断。
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Document => "Document",
            Self::CodeBlock => "CodeBlock",
            Self::FencedCode => "FencedCode",
            Self::Blockquote => "Blockquote",
            Self::HorizontalRule => "HorizontalRule",
            Self::BulletList => "BulletList",
            Self::OrderedList => "OrderedList",
            Self::ListItem => "ListItem",
            Self::AtxHeading1 => "ATXHeading1",
            Self::AtxHeading2 => "ATXHeading2",
            Self::AtxHeading3 => "ATXHeading3",
            Self::AtxHeading4 => "ATXHeading4",
            Self::AtxHeading5 => "ATXHeading5",
            Self::AtxHeading6 => "ATXHeading6",
            Self::SetextHeading1 => "SetextHeading1",
            Self::SetextHeading2 => "SetextHeading2",
            Self::HtmlBlock => "HTMLBlock",
            Self::LinkReference => "LinkReference",
            Self::Paragraph => "Paragraph",
            Self::CommentBlock => "CommentBlock",
            Self::ProcessingInstructionBlock => "ProcessingInstructionBlock",
            Self::Escape => "Escape",
            Self::Entity => "Entity",
            Self::HardBreak => "HardBreak",
            Self::Emphasis => "Emphasis",
            Self::StrongEmphasis => "StrongEmphasis",
            Self::Link => "Link",
            Self::Image => "Image",
            Self::InlineCode => "InlineCode",
            Self::HtmlTag => "HTMLTag",
            Self::Comment => "Comment",
            Self::ProcessingInstruction => "ProcessingInstruction",
            Self::Autolink => "Autolink",
            Self::HeaderMark => "HeaderMark",
            Self::QuoteMark => "QuoteMark",
            Self::ListMark => "ListMark",
            Self::LinkMark => "LinkMark",
            Self::EmphasisMark => "EmphasisMark",
            Self::CodeMark => "CodeMark",
            Self::CodeText => "CodeText",
            Self::CodeInfo => "CodeInfo",
            Self::LinkTitle => "LinkTitle",
            Self::LinkLabel => "LinkLabel",
            Self::Url => "URL",
        }
    }

    /// 块级节点：`Document` 与它到 `ProcessingInstructionBlock` 之间的全部。
    ///
    /// 增量复用只在块边界上发生（`FragmentCursor::take_nodes`），这个判断决定
    /// 哪些节点可以充当边界。
    #[must_use]
    pub const fn is_block(self) -> bool {
        (self as u8) <= (Self::ProcessingInstructionBlock as u8)
    }

    /// 容器块：可以嵌套其他块，并在每行开头需要跳过自己的标记。
    #[must_use]
    pub const fn is_block_context(self) -> bool {
        matches!(
            self,
            Self::Document
                | Self::Blockquote
                | Self::ListItem
                | Self::OrderedList
                | Self::BulletList
        )
    }

    /// 能跨越空行的块。它们只有在**后一个兄弟也被复用**时才可以复用，
    /// 否则一个紧随其后的空行会改变它的边界而不改变它自身的字节。
    #[must_use]
    pub const fn spans_blank_lines(self) -> bool {
        matches!(
            self,
            Self::CodeBlock | Self::ListItem | Self::OrderedList | Self::BulletList
        )
    }

    /// ATX 标题的层级（1..=6），非标题返回 `None`。
    #[must_use]
    pub const fn atx_heading_level(self) -> Option<u8> {
        match self {
            Self::AtxHeading1 => Some(1),
            Self::AtxHeading2 => Some(2),
            Self::AtxHeading3 => Some(3),
            Self::AtxHeading4 => Some(4),
            Self::AtxHeading5 => Some(5),
            Self::AtxHeading6 => Some(6),
            _ => None,
        }
    }

    /// `level` 级 ATX 标题，`level` 必须在 1..=6。
    ///
    /// 对应 lezer 的 `Type.ATXHeading1 - 1 + size`。写成显式匹配而不是算术，
    /// 是因为算术版本在枚举顺序被改动时不会报错，只会静默地产出错误层级。
    #[must_use]
    pub const fn atx_heading(level: u8) -> Option<Self> {
        match level {
            1 => Some(Self::AtxHeading1),
            2 => Some(Self::AtxHeading2),
            3 => Some(Self::AtxHeading3),
            4 => Some(Self::AtxHeading4),
            5 => Some(Self::AtxHeading5),
            6 => Some(Self::AtxHeading6),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::NodeKind;

    #[test]
    fn block_range_covers_exactly_the_block_section() {
        assert!(NodeKind::Document.is_block());
        assert!(NodeKind::ProcessingInstructionBlock.is_block());
        assert!(!NodeKind::Escape.is_block());
        assert!(!NodeKind::HeaderMark.is_block());
    }

    #[test]
    fn atx_heading_round_trips_through_level() {
        for level in 1..=6 {
            let kind = NodeKind::atx_heading(level).expect("1..=6 都是合法层级");
            assert_eq!(kind.atx_heading_level(), Some(level));
            assert!(kind.is_block());
        }
        assert_eq!(NodeKind::atx_heading(0), None);
        assert_eq!(NodeKind::atx_heading(7), None);
    }

    #[test]
    fn block_contexts_are_the_five_lezer_skip_markup_types() {
        let contexts: Vec<&str> = [
            NodeKind::Document,
            NodeKind::CodeBlock,
            NodeKind::FencedCode,
            NodeKind::Blockquote,
            NodeKind::HorizontalRule,
            NodeKind::BulletList,
            NodeKind::OrderedList,
            NodeKind::ListItem,
            NodeKind::Paragraph,
        ]
        .into_iter()
        .filter(|kind| kind.is_block_context())
        .map(NodeKind::name)
        .collect();
        assert_eq!(
            contexts,
            [
                "Document",
                "Blockquote",
                "BulletList",
                "OrderedList",
                "ListItem"
            ]
        );
    }
}
