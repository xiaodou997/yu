//! `# 标题`。
//!
//! 产出两样：把 `#` 前缀（以及可能存在的收尾 `#`）从视觉文本里拿掉，以及
//! 一条说明「这是几级标题」的行级装饰。**字号倍率不在这里**——那要
//! `LayoutConfig` 才算得出来，属于 `yu-editor`。这一层说的是语义。
//!
//! Setext 标题（`标题\n===`）不在这里：`block_sequence` 不认识它，那一行
//! 会当成段落。树认识它，两边不一致的地方以块序列为准——块的身份是编辑器
//! 的缓存单位，不能由装饰层单方面改。

use yu_syntax::NodeKind;

use super::{BlockContext, BlockOrnament, Extension, ExtensionOutput};
use yu_core::TextRange;

pub struct Heading;

impl Extension for Heading {
    fn name(&self) -> &'static str {
        "heading"
    }

    fn decorate(&self, cx: &BlockContext<'_>, out: &mut ExtensionOutput) {
        let Some(node) = cx.block_node(|kind| atx_level(kind).is_some()) else {
            return;
        };
        let Some(level) = atx_level(node.kind()) else {
            return;
        };

        // 焦点块的结构前缀整个露出来：光标在这一行时用户要能看见 `##`。
        if !cx.is_focus() {
            let mut marks = node
                .children()
                .filter(|child| child.kind() == NodeKind::HeaderMark);
            if let Some(opening) = marks.next() {
                let content_start = cx.skip_spaces(opening.range().end());
                if let Some(prefix) = TextRange::new(node.range().start(), content_start) {
                    out.replace(prefix);
                }
                // 收尾 `#` 连着它前面那个空格一起走。没有收尾标记的标题
                // （绝大多数）走不到这里。
                if let Some(closing) = marks.next() {
                    let content_end = cx.skip_spaces_back(closing.range().start());
                    if let Some(suffix) = TextRange::new(content_end, node.range().end()) {
                        out.replace(suffix);
                    }
                }
            }
        }

        let style = out.line_style(BlockOrnament::Heading { level });
        out.line(cx.range(), style);
    }
}

const fn atx_level(kind: NodeKind) -> Option<u8> {
    Some(match kind {
        NodeKind::AtxHeading1 => 1,
        NodeKind::AtxHeading2 => 2,
        NodeKind::AtxHeading3 => 3,
        NodeKind::AtxHeading4 => 4,
        NodeKind::AtxHeading5 => 5,
        NodeKind::AtxHeading6 => 6,
        _ => return None,
    })
}
