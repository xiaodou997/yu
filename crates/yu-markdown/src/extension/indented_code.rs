//! 四空格（或一个制表符）缩进的代码块。
//!
//! 它与围栏是**两种拼法、一种东西**：整块按等宽排，内容不解析行内语法。后半
//! 件事同样不需要任何人判断——树里 `CodeBlock` 的内容是 `CodeText` 叶子，里面
//! 没有行内标记节点，遍历不到就产不出装饰。
//!
//! 底色也不在这里：它由 `BlockKind` 直接决定（`yu_workspace::viewport_block_background`），
//! 与围栏走同一张表。这个文件只说「这一块的字排成等宽」。
//!
//! # 为什么与 `fenced_code.rs` 分成两个文件
//!
//! 与 `BlockKind` 把它们分成两个变体是同一条理由。两者在实现上没有一个字节
//! 共享：围栏要找 `CodeMark`、算两段隐藏区间、认语言名、按语言名着色，缩进
//! 代码一样都没有。合成一个文件的话每一段都要先问一句「我现在是哪一种」。
//!
//! # 行首那四个空格留在画面上
//!
//! 树给的 `CodeBlock` 节点**只跳过第一行的缩进**（`    only\n` 里块是 `0..9`，
//! 节点是 `4..8`），后面每一行的四个空格都在节点里面。要藏就得自己逐行扫一遍
//! 缩进——把行扫描器的活儿在这里重写一遍，而那正是「块的边界由谁定」那件已经
//! 登记的事。按源码原样画是不变量 I5 的缺省，代价只是正文往右四格。
//!
//! # 不着色
//!
//! 缩进代码没有语言名可问，于是一个字都不着色。`fenced_code.rs` 里没有 info
//! 的围栏是同一个结果（见那个文件的 `highlight`），这是同一条规矩，不是遗漏。
//!
//! # 定义域为什么是 `BlockKind`
//!
//! 与 `heading.rs` / `thematic_break.rs` 同一条。这里还有一个自己的形状：
//! 缩进代码块是唯一能跨空行的块，`    a\n\n    b\n` 在行扫描器眼里是三块、
//! 在树里是一个 `CodeBlock`，三块谁也不完整，于是 `classify` 全部退回
//! `Paragraph`。跟着 `BlockKind` 走的话这三块一块都不排等宽（现状，与这一刀
//! 之前一致）；自己找节点的话会画出三段各排一半的代码。

use yu_core::{TextAttrs, TextStyle};

use super::{BlockContext, Extension, ExtensionOutput};
use crate::block_sequence::BlockKind;

pub struct IndentedCode;

impl Extension for IndentedCode {
    fn name(&self) -> &'static str {
        "indented-code"
    }

    fn decorate(&self, cx: &BlockContext<'_>, out: &mut ExtensionOutput) {
        if !matches!(cx.block().kind(), BlockKind::IndentedCode) {
            return;
        }
        let style = out.style(TextAttrs::new(TextStyle::Code));
        out.mark(cx.range(), style);
    }
}
