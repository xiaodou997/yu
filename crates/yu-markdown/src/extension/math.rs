//! Dollar math uses opaque syntax nodes; source/history remain authoritative.
use super::{BlockContext, BlockWidget, EmbeddedKind, EmbeddedSpan, Extension, ExtensionOutput};
use yu_core::{TextRange, WidgetSide};
use yu_syntax::NodeKind;

pub struct Math;
impl Extension for Math {
    fn name(&self) -> &'static str {
        "math"
    }
    fn decorate(&self, cx: &BlockContext<'_>, out: &mut ExtensionOutput) {
        for node in cx.nodes() {
            let display = match node.kind() {
                NodeKind::MathBlock => true,
                NodeKind::InlineMath | NodeKind::EquationReference => false,
                _ => continue,
            };
            if node.kind() == NodeKind::EquationReference {
                if !super::reveals(cx.active(), node.range()) {
                    let widget = out.widget(BlockWidget::Embedded(EmbeddedSpan {
                        source: node.range(),
                        content: node.range(),
                        kind: EmbeddedKind::Math,
                        display: false,
                    }));
                    out.place_widget(node.range(), widget, WidgetSide::Before);
                }
                continue;
            }
            let marks: Vec<_> = node
                .children()
                .filter(|child| child.kind() == NodeKind::MathMark)
                .collect();
            if marks.len() != 2 || super::reveals(cx.active(), node.range()) {
                continue;
            }
            let Some(content) = TextRange::new(marks[0].range().end(), marks[1].range().start())
            else {
                continue;
            };
            let source = if display { cx.range() } else { node.range() };
            let widget = out.widget(BlockWidget::Embedded(EmbeddedSpan {
                source,
                content,
                kind: EmbeddedKind::Math,
                display,
            }));
            out.place_widget(source, widget, WidgetSide::Before);
        }
    }
}
