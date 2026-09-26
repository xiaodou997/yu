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

impl ExtensionOutput {
    /// Reveal the editable TeX body of active HTML formulas, not the table or
    /// their identity tags. Entity substitutions retain canonical byte ranges.
    /// Inactive formulas keep their ordinary EmbeddedSpan resource/geometry path.
    pub(crate) fn reveal_html_math(
        &mut self,
        source: &str,
        owned: &[EmbeddedSpan],
        active: Option<TextRange>,
    ) {
        use yu_core::WidgetId;
        use yu_decoration::Decoration;

        let revealed: Vec<_> = self
            .widgets
            .iter()
            .filter_map(|widget| match widget {
                BlockWidget::Embedded(span)
                    if owned.contains(span) && super::reveals(active, span.source) =>
                {
                    Some(*span)
                }
                _ => None,
            })
            .collect();
        if revealed.is_empty() {
            return;
        }
        let mut remap = Vec::with_capacity(self.widgets.len());
        let mut retained = Vec::new();
        for widget in &self.widgets {
            if matches!(widget, BlockWidget::Embedded(span) if revealed.contains(span)) {
                remap.push(None);
            } else {
                remap.push(Some(WidgetId(retained.len() as u32)));
                retained.push(*widget);
            }
        }
        self.ranges.retain_mut(|entry| {
            if let Decoration::Widget { widget, side } = entry.decoration {
                let Some(Some(mapped)) = remap.get(widget.0 as usize) else {
                    return false;
                };
                entry.decoration = Decoration::Widget {
                    widget: *mapped,
                    side,
                };
            }
            true
        });
        self.widgets = retained;
        for span in revealed {
            self.reveal_html_leaf(source, span.source, span.content);
        }
    }

    /// Math and footnote bodies share one entity-to-source editing projection.
    pub(crate) fn reveal_html_leaf(&mut self, source: &str, full: TextRange, content: TextRange) {
        use yu_core::{ByteOffset, TextAttrs, TextStyle};
        self.replace(TextRange::new(full.start(), content.start()).expect("leaf opening"));
        self.replace(TextRange::new(content.end(), full.end()).expect("leaf closing"));
        let style = self.style(TextAttrs::new(TextStyle::Code));
        self.mark(content, style);
        let mut cursor = content.start().get() as usize;
        let end = content.end().get() as usize;
        while cursor < end {
            let Some(tail) = source.get(cursor..end) else {
                break;
            };
            if tail.starts_with('&')
                && let Some(stop) = tail
                    .as_bytes()
                    .iter()
                    .take(34)
                    .position(|&byte| byte == b';')
                && let Some(decoded) = super::entity::decode(&tail[..=stop])
            {
                self.substitute_text(
                    TextRange::new(
                        ByteOffset::new(cursor as u64),
                        ByteOffset::new((cursor + stop + 1) as u64),
                    )
                    .expect("HTML entity"),
                    decoded,
                );
                cursor += stop + 1;
            } else {
                cursor += tail.chars().next().expect("nonempty leaf").len_utf8();
            }
        }
    }
}
