//! `[文字](目标)`、`[文字][引用]` 与 `<autolink>`。
//!
//! 三种写法在树里是同一个形状：头两个 `LinkMark` 夹住正文，其余（`](url)`、
//! `[label]`、autolink 的尖括号与 `Url`）都在正文之外，整段是语法。
//!
//! # 引用式的要先查表
//!
//! 不变量 C6 规定 parser 只产出**候选**引用：`[文字][标签]` 在树里是一个
//! `Link` 节点，无论 `标签` 有没有被定义过。查不到定义的候选按 CommonMark
//! **根本不是链接**，是一段普通文字——定界符原样留着，`[文字][没定义]` 就画
//! 成 `[文字][没定义]`。
//!
//! 这一句此前写的是「引用式链接不需要 definition 索引」，那是错的：不查表就
//! 把每一个候选都画成链接，画面上是一个哪儿也去不了的链接，不报错。

use yu_core::{TextAttrs, TextStyle};
use yu_syntax::NodeKind;

use super::{BlockContext, DelimitedSpan, Extension, ExtensionOutput, reveals};

pub struct Link;

impl Extension for Link {
    fn name(&self) -> &'static str {
        "link"
    }

    fn decorate(&self, cx: &BlockContext<'_>, out: &mut ExtensionOutput) {
        for node in cx.nodes() {
            if !matches!(node.kind(), NodeKind::Link | NodeKind::Autolink) {
                continue;
            }
            let Some(span) = DelimitedSpan::of(node, |kind| kind == NodeKind::LinkMark) else {
                continue;
            };
            // 引用式的候选查不到定义就不是链接。行内式与 autolink 给 `None`，
            // 不查表。
            if span
                .reference_label(node)
                .is_some_and(|label| !cx.resolves(label))
            {
                continue;
            }
            // 链接正文按**正文**字型排，不继承外层。
            //
            // 这一条看起来是多余的（不产出 mark 也会落到默认字型），它压的
            // 是嵌套：`**[文字](url)**` 里最内层是链接，v1 的 `style_for`
            // 取最内层，于是链接正文不是粗的。不显式说出来的话，装配层的
            // 「窄的赢」会让外层的 Strong 赢，画面就变了——而这种变化不报错。
            let style = out.style(TextAttrs::new(TextStyle::Plain));
            out.mark(span.content, style);
            if !reveals(cx.active(), node.range()) {
                out.replace(span.opening);
                out.replace(span.closing);
            }
        }
    }
}
