//! `![替代文字](目标)` 与它的引用形式。
//!
//! 图片是一个 widget（第 3 节的对照表）：不聚焦时整段 `![替代](目标)` 由
//! [`Decoration::Widget`] 覆盖，从视觉文本里消失，位置上留一个盒子。盒子
//! **多大**不在这里——那要资源解码后的固有尺寸与 `LayoutConfig`，是
//! `yu-editor` 与 workspace 的事。这里只说「这一段是一张图，它的替代文字
//! 与目标各在哪」。
//!
//! # 光标进来时 widget 让位
//!
//! 与行内语法的定界符同一条规则（[`reveals`]）：光标碰到这一段时整段源码
//! 原样露出来，可编辑。不变量 D7 要求 widget 的资源失败时「保留可编辑的
//! 源码回退」——回退就是这一条，不是第二套呈现。
//!
//! [`Decoration::Widget`]: yu_decoration::Decoration::Widget

use yu_core::{TextAttrs, TextRange, TextStyle, WidgetSide};
use yu_syntax::NodeKind;

use super::SyntaxNode;
use super::{
    BlockContext, BlockWidget, DelimitedSpan, Extension, ExtensionOutput, ImageSpan, reveals,
};

pub struct Image;

impl Extension for Image {
    fn name(&self) -> &'static str {
        "image"
    }

    fn decorate(&self, cx: &BlockContext<'_>, out: &mut ExtensionOutput) {
        for node in cx.nodes() {
            if node.kind() != NodeKind::Image {
                continue;
            }
            let Some(span) = DelimitedSpan::of(node, |kind| kind == NodeKind::LinkMark) else {
                continue;
            };
            if reveals(cx.active(), node.range()) {
                // 与链接同一个理由：替代文字按正文字型排，不继承外层。
                let style = out.style(TextAttrs::new(TextStyle::Plain));
                out.mark(span.content, style);
                continue;
            }
            // 引用式的候选查不到定义就不是图片（不变量 C6：parser 只产出
            // 候选）。此前不查表，于是一段解析不出目标的 `![替代][没定义]`
            // 也占一个 widget，画面上是一个空框——第七刀登记的那条行为变化。
            let reference = span.reference_label(node);
            if reference.is_some_and(|label| !cx.resolves(label)) {
                continue;
            }
            let destination = child_range(node, NodeKind::Url);
            let widget = out.widget(BlockWidget::Image(ImageSpan::new(
                node.range(),
                span.content,
                destination,
                reference,
            )));
            // 非空 range 的 widget 覆盖并隐藏这一段，`side` 没有歧义。
            out.place_widget(node.range(), widget, WidgetSide::Before);
        }
    }
}

fn child_range(node: SyntaxNode<'_>, kind: NodeKind) -> Option<TextRange> {
    node.children()
        .find(|child| child.kind() == kind)
        .map(SyntaxNode::range)
}
