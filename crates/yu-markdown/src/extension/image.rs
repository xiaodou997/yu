//! `![替代文字](目标)` 与它的引用形式。
//!
//! 图片的**几何**（盒子多大、排在哪一行）不在这里，它要 `LayoutConfig`
//! 才算得出来。这里只说「这一段是一张图，它的替代文字与目标各在哪」。

use yu_core::{TextAttrs, TextStyle};
use yu_syntax::NodeKind;

use super::{BlockContext, DelimitedSpan, Extension, ExtensionOutput, reveals};

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
        }
    }
}
