//! `![替代文字](目标)` 与它的引用形式。
//!
//! 图片的**几何**（盒子多大、排在哪一行）不在这里，它要 `LayoutConfig`
//! 才算得出来。这里只说「这一段是一张图，它的替代文字与目标各在哪」。

use yu_core::{TextAttrs, TextRange, TextStyle};
use yu_syntax::NodeKind;

use super::SyntaxNode;
use super::{BlockAnnotation, BlockContext, DelimitedSpan, Extension, ExtensionOutput, reveals};

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
            // 与链接同一个理由：替代文字按正文字型排，不继承外层。
            let style = out.style(TextAttrs::new(TextStyle::Plain));
            out.mark(span.content, style);
            if !reveals(cx.active(), node.range()) {
                out.replace(span.opening);
                out.replace(span.closing);
            }
            // 图片盒子画在替代文字上面，而画它的那一层要先知道「这一段是
            // 一张图」。装饰说不出这句话——上面三条改的是字型与可见性，
            // 没有一条是「这里有张图」。理由见 `BlockAnnotation`。
            let destination = child_range(node, NodeKind::Url);
            out.annotate(BlockAnnotation::Image {
                source: node.range(),
                label: span.content,
                destination,
                // 引用式的标签：`![替代][引用]` 取 `LinkLabel`，shortcut
                // `![替代]` 没有 `LinkLabel`，标签就是替代文字本身。
                reference: destination
                    .is_none()
                    .then(|| child_range(node, NodeKind::LinkLabel).unwrap_or(span.content)),
            });
        }
    }
}

fn child_range(node: SyntaxNode<'_>, kind: NodeKind) -> Option<TextRange> {
    node.children()
        .find(|child| child.kind() == kind)
        .map(SyntaxNode::range)
}
