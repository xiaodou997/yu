//! `[文字](目标)`、`[文字][引用]` 与 `<autolink>`。
//!
//! 三种写法在树里是同一个形状：头两个 `LinkMark` 夹住正文，其余（`](url)`、
//! `[label]`、autolink 的尖括号与 `Url`）都在正文之外，整段是语法。
//! 引用式链接不需要 definition 索引——`[a][b]` 的 `LinkLabel` 是语法树给的
//! 结构，v1 那边则要先查表才知道它是不是一个链接。

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
