//! `---` / `***` / `___`。
//!
//! 源码里是三个字符，屏幕上是一条横线。这个文件产出两样：把那三个字符从视觉
//! 文本里拿掉，以及一条说明「这一块是分隔线」的行级装饰。**线画多粗、多长、
//! 什么颜色不在这里**——那要 `LayoutConfig` 与主题才说得出来，分别属于
//! `yu-editor` 与 `yu-workspace`。这一层说的是语义。
//!
//! # 隐藏与装饰是同一个条件的两面：没有焦点
//!
//! 这与标题不一样：标题的 `#` 藏起来了，字号照样放大，因为放大与露出标记
//! 互不妨碍。分隔线不是——那条线占的**就是**正文那一行。焦点块要露出 `---`
//! 让用户改得动（否则他按退格会删掉一个他没看见的字符，与 `heading.rs` 同一
//! 条理由），而线如果仍然画着，它正好穿过那三个减号，看上去是一条删除线。
//! 所以两样一起由 [`BlockContext::is_focus`] 决定：**光标进来是源码，光标
//! 出去是一条线**。
//!
//! # 为什么不连行尾那个换行符一起藏
//!
//! 藏掉整块的话视觉文本是空的，而块高由排版从视觉文本算。留着那个 `\n`，
//! 这一块与一个空行**逐字节同形**（`BlankLine` 块的视觉文本就是 `"\n"`）：
//! 高度是一行，线画在这一行的正中。树给的 `HorizontalRule` 节点正好就是不
//! 含换行符的那三个字符（`---\n` 的块是 `11..15`，节点是 `11..14`），所以
//! 这件事不需要在这里数字节。
//!
//! # 定义域为什么是 `BlockKind`
//!
//! 与 `heading.rs` 同一条：块是不是分隔线由 [`crate::classify`] 问树定下来，
//! 这里跟着它走。自己找 `HorizontalRule` 节点的话，横跨几个树块的块会被认领
//! **半个**——那种块 `classify` 一律退回 `Paragraph`，按源码原样画（不变量
//! I5），而这里会给它藏掉中间的三个减号再画一条线。

use yu_syntax::NodeKind;

use super::{BlockContext, BlockOrnament, Extension, ExtensionOutput};
use crate::block_sequence::BlockKind;

pub struct ThematicBreak;

impl Extension for ThematicBreak {
    fn name(&self) -> &'static str {
        "thematic-break"
    }

    fn decorate(&self, cx: &BlockContext<'_>, out: &mut ExtensionOutput) {
        if !matches!(cx.block().kind(), BlockKind::ThematicBreak) {
            return;
        }
        // 光标在这一块里：什么都不产出，`---` 原样留在画面上等着被编辑。
        if cx.is_focus() {
            return;
        }
        let Some(node) = cx.block_node(|kind| kind == NodeKind::HorizontalRule) else {
            return;
        };
        out.replace(node.range());
        let style = out.line_style(BlockOrnament::ThematicBreak);
        out.line(cx.range(), style);
    }
}
