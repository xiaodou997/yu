//! `# 标题` 与 `标题\n===`。
//!
//! 产出两样：把结构标记从视觉文本里拿掉，以及一条说明「这是几级标题」的行级
//! 装饰。**字号倍率不在这里**——那要 `LayoutConfig` 才算得出来，属于
//! `yu-editor`。这一层说的是语义。
//!
//! # 两种拼法，一个语义
//!
//! ATX 的标记在**前面**（`## `，偶尔还有收尾的 ` ##`），Setext 的标记是**下
//! 面那一整行**（`===` / `---`）。两种在树里都是 `HeaderMark` 子节点，区别只
//! 在它长在哪一头，所以这里按节点类型分两条路取区间；[`BlockKind::Heading`]
//! 不带拼法，上面几层看到的是同一个「几级标题」。
//!
//! Setext 藏的那一段**连前面那个换行符一起**。只藏 `===` 的话，标题会画成
//! 两行——第二行是空的。这与「隐藏区间要一直隐藏到下一段内容的起点」是同一
//! 条规则，围栏代码块的开围栏也是这么处理的。
//!
//! # 定义域为什么是 `BlockKind`
//!
//! 块是不是一个标题由 [`crate::classify`] 问树定下来，这里跟着它走，而不是
//! 自己再判一次「块里有没有标题节点」。两者不等价：`a\n===\nb` 在块序列里是
//! **一个**块（行扫描器不认得 Setext 下划线），树里却是
//! `SetextHeading1` 加一个 `Paragraph`。自己找节点的话，那个块会被整块放大
//! ——包括不属于标题的 `b`。

use yu_core::{ByteOffset, TextRange};
use yu_syntax::NodeKind;

use super::{BlockContext, BlockOrnament, Extension, ExtensionOutput, SyntaxNode};
use crate::block_sequence::BlockKind;

pub struct Heading;

impl Extension for Heading {
    fn name(&self) -> &'static str {
        "heading"
    }

    fn decorate(&self, cx: &BlockContext<'_>, out: &mut ExtensionOutput) {
        let BlockKind::Heading { level } = cx.block().kind() else {
            return;
        };
        let Some(anatomy) = anatomy(cx) else {
            return;
        };

        // 焦点块的结构标记整个露出来：光标在这一行时用户要能看见 `##`，也要
        // 能看见 `===`——否则他按退格会删掉一个他没看见的字符。
        if !cx.is_focus() {
            for range in anatomy.hidden {
                out.replace(range);
            }
        }

        let style = out.line_style(BlockOrnament::Heading { level });
        out.line(cx.range(), style);
    }
}

/// 一个标题在源码里的解剖：结构标记占哪几段，正文占哪一段。
///
/// **装饰要藏的，与大纲、导出、可访问性要读的，是同一件事的两面**——正文
/// 就是节点范围减去 `hidden`，所以这里一次算出两者，谁也不可能与谁分叉。
///
/// 分叉过一次，而且三个消费者一起错：`heading_content_range` 曾经自己扫行，
/// 认得 ATX 的 `#` 前缀却不认得收尾的 ` ##`（那条规则只写在
/// `yu-syntax` 的 `HeaderMark` 里）。于是同一个 `## a ##`，编辑器里显示
/// `a`，导出成 `<h2>a ##</h2>`，VoiceOver 读作「a ##」。不报错、不 panic，
/// 三条都绿。
pub(crate) struct HeadingAnatomy {
    /// 正文：大纲显示的、导出进 `<h2>` 的、VoiceOver 读的那一段。
    pub(crate) content: TextRange,
    /// 不进视觉文本的那几段。
    pub(crate) hidden: Vec<TextRange>,
}

/// 解剖这个块对应的标题节点，树里找不到标题节点时为 `None`。
///
/// **它只问树，不问 [`BlockKind`]**——两者不等价（`a\n===\nb` 在块序列里是
/// 一个块，树里是 `SetextHeading1` 加一个 `Paragraph`），所以调用方要先自己
/// 确认这个块真的是标题，理由见本模块「定义域为什么是 `BlockKind`」一节。
pub(crate) fn anatomy(cx: &BlockContext<'_>) -> Option<HeadingAnatomy> {
    let node = cx.block_node(is_heading)?;
    let mut marks = node
        .children()
        .filter(|child| child.kind() == NodeKind::HeaderMark);
    let first = marks.next()?;

    if is_setext(node.kind()) {
        // 下划线那一行是 Setext 唯一的标记，它在**后面**，正文是它之前的
        // 全部（可能不止一行）。
        let underline = underline_range(cx, node, first)?;
        return Some(HeadingAnatomy {
            content: TextRange::new(node.range().start(), underline.start())?,
            hidden: vec![underline],
        });
    }

    let mut hidden = Vec::new();
    let content_start = cx.skip_spaces(first.range().end());
    if let Some(prefix) = TextRange::new(node.range().start(), content_start) {
        hidden.push(prefix);
    }
    // 收尾 `#` 连着它前面那个空格一起走。没有收尾标记的标题（绝大多数）
    // 正文一直到节点末尾。
    let mut content_end = node.range().end();
    if let Some(closing) = marks.next() {
        content_end = cx.skip_spaces_back(closing.range().start());
        if let Some(suffix) = TextRange::new(content_end, node.range().end()) {
            hidden.push(suffix);
        }
    }
    Some(HeadingAnatomy {
        content: TextRange::new(content_start, content_end.max(content_start))?,
        hidden,
    })
}

/// `===` 那一行，连同它前面的换行符与行首缩进。
fn underline_range(
    cx: &BlockContext<'_>,
    node: SyntaxNode<'_>,
    mark: SyntaxNode<'_>,
) -> Option<TextRange> {
    let after_content = cx.skip_spaces_back(mark.range().start());
    let start = line_break_start(cx, after_content).unwrap_or(after_content);
    TextRange::new(start, node.range().end())
}

/// `offset` 前面那个换行符的起点。`\r\n` 算一个。
///
/// 换行符不在标记节点里（`HeaderMark` 从下划线的第一个字符开始），也不是
/// 空白扫描跳得过去的（[`BlockContext::skip_spaces`] 只认空格与制表符），
/// 所以只能自己读回来一两个字节。读不到就把它留在视觉文本里——那是一个多出
/// 来的空行，看得见；猜一个位置藏掉才是静默地做错事。
fn line_break_start(cx: &BlockContext<'_>, offset: ByteOffset) -> Option<ByteOffset> {
    let before = |at: ByteOffset| -> Option<(ByteOffset, String)> {
        let start = ByteOffset::new(at.get().checked_sub(1)?);
        Some((start, cx.text(TextRange::new(start, at)?)?))
    };
    let (line_feed, text) = before(offset)?;
    if text != "\n" {
        return None;
    }
    match before(line_feed) {
        Some((carriage_return, text)) if text == "\r" => Some(carriage_return),
        _ => Some(line_feed),
    }
}

const fn is_heading(kind: NodeKind) -> bool {
    is_setext(kind) || kind.atx_heading_level().is_some()
}

const fn is_setext(kind: NodeKind) -> bool {
    matches!(kind, NodeKind::SetextHeading1 | NodeKind::SetextHeading2)
}
