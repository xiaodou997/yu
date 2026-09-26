//! Source-mapped paragraph projection for the structural HTML layout path.
use super::{
    HtmlBlockModel, HtmlElementKind as Kind, HtmlFlowKind, HtmlFlowPartition, HtmlNodeKind,
};
use crate::extension::{ExtensionOutput, entity, reveals};
use yu_core::{ByteOffset, TextAttrs, TextRange, TextRole, TextScript, TextStyle};

impl HtmlBlockModel {
    /// Only resolved, text-only math leaves can become native resources.
    /// Both ranges address canonical HTML bytes, never a decoded buffer.
    pub fn inline_math_span(&self, id: usize) -> Option<crate::EmbeddedSpan> {
        if !self
            .resolution
            .elements
            .get(id)?
            .as_ref()?
            .attributes
            .inline_math
        {
            return None;
        }
        let node = self.fragment.nodes.get(id)?;
        let HtmlNodeKind::Element {
            opening,
            closing: Some(closing),
        } = &node.kind
        else {
            return None;
        };
        Some(crate::EmbeddedSpan {
            source: node.source,
            content: TextRange::new(opening.source.end(), closing.source.start())?,
            kind: crate::EmbeddedKind::Math,
            display: false,
        })
    }

    pub fn inline_math_spans(&self) -> Vec<crate::EmbeddedSpan> {
        (0..self.fragment.nodes.len())
            .filter_map(|id| self.inline_math_span(id))
            .collect()
    }

    /// Presentation and cell editing must expose the same active TeX bytes.
    /// The canonical decorator remains useful for resource discovery/navigation.
    pub fn decorate_table_active(
        &self,
        source: &str,
        part: &HtmlFlowPartition,
        active: Option<TextRange>,
        out: &mut ExtensionOutput,
    ) -> bool {
        if !self.decorate_table(source, part, out) {
            return false;
        }
        out.reveal_html_math(source, &self.inline_math_spans(), active);
        true
    }

    /// Produce decorations for one model-owned paragraph. Tables and raw leaves
    /// need their own layout; active paragraphs expose their original markup.
    /// The caller supplies the canonical document, never a reserialized DOM.
    pub fn decorate_paragraph(
        &self,
        source: &str,
        part: &HtmlFlowPartition,
        active: Option<TextRange>,
        out: &mut ExtensionOutput,
    ) -> bool {
        if !matches!(
            part.content.kind,
            HtmlFlowKind::Paragraph | HtmlFlowKind::Heading(_)
        ) || !self.partitions.contains(part)
            || source
                .get(part.source.start().get() as usize..part.source.end().get() as usize)
                .is_none()
        {
            return false;
        }
        let disclosure_header = self.disclosures_ref().iter().any(|details| {
            details.opening.start() >= part.source.start()
                && details.opening.end() <= part.source.end()
        });
        if reveals(active, part.source) && !disclosure_header {
            return true;
        }
        self.decorate_text_part(source, part, out)
    }

    pub(super) fn decorate_text_part(
        &self,
        source: &str,
        part: &HtmlFlowPartition,
        out: &mut ExtensionOutput,
    ) -> bool {
        self.decorate_text_part_with_breaks(source, part, out, &[])
    }

    pub(super) fn decorate_text_part_with_breaks(
        &self,
        source: &str,
        part: &HtmlFlowPartition,
        out: &mut ExtensionOutput,
        breaks: &[TextRange],
    ) -> bool {
        let disclosures = self.disclosures_ref();
        let hidden = disclosures
            .iter()
            .filter(|details| !details.open && !details.body.is_empty())
            .filter_map(|details| intersection(details.body, part.source))
            .collect::<Vec<_>>();
        for range in &hidden {
            // Outer closed details owns nested hidden ranges as one atom.
            if !hidden.iter().any(|outer| {
                outer != range && outer.start() <= range.start() && range.end() <= outer.end()
            }) {
                out.replace(*range);
            }
        }
        let mut text = TextProjection::default();
        let mut opaque_end = ByteOffset::new(0);
        for (id, node) in self.fragment.nodes.iter().enumerate() {
            if node.source.end() <= part.source.start() || node.source.start() >= part.source.end()
            {
                continue;
            }
            if hidden.iter().any(|range| {
                range.start() <= node.source.start() && node.source.end() <= range.end()
            }) {
                continue;
            }
            if node.source.start() < opaque_end {
                continue;
            }
            match &node.kind {
                HtmlNodeKind::Element { opening, closing } => {
                    let resolved = self.resolution.elements[id].as_ref();
                    // Images require the resource/widget path. Until that path
                    // consumes them, preserve the complete element as source.
                    if resolved.is_none_or(|element| matches!(element.kind, Kind::Image)) {
                        text.visible(out);
                        opaque_end = node.source.end();
                        continue;
                    }
                    if let Some(span) = self.inline_math_span(id) {
                        text.visible(out);
                        let widget = out.widget(crate::BlockWidget::Embedded(span));
                        out.place_widget(span.source, widget, yu_core::WidgetSide::Before);
                        opaque_end = node.source.end();
                        continue;
                    }
                    let kind = resolved.expect("checked resolution").kind;
                    if self.footnote_span(id).is_some() {
                        // The document index supplies number/diagnostic and
                        // editing projection; never display a stale saved number.
                        text.visible(out);
                        opaque_end = node.source.end();
                        continue;
                    }
                    if kind == Kind::Break {
                        text.discard_space(out);
                        if let Some(range) = intersection(opening.source, part.source) {
                            out.substitute(range, '\n');
                        }
                        text.has_text = false;
                        continue;
                    }
                    if let Some(range) = intersection(opening.source, part.source) {
                        if kind == Kind::Details
                            && let Some(details) = disclosures
                                .iter()
                                .find(|details| details.opening == opening.source)
                        {
                            let marker = match (details.open, details.summary.is_some()) {
                                (false, true) => "▸ ",
                                (true, true) => "▾ ",
                                (false, false) => "▸ Details",
                                (true, false) => "▾ Details",
                            };
                            out.substitute_text(
                                range,
                                yu_decoration::ReplacementText::Literal(marker),
                            );
                        } else if breaks.contains(&range) {
                            out.substitute(range, '\n');
                        } else {
                            out.replace(range);
                        }
                    }
                    if let Some(closing) = closing
                        && let Some(range) = intersection(closing.source, part.source)
                    {
                        if breaks.contains(&range) {
                            out.substitute(range, '\n');
                        } else {
                            out.replace(range);
                        }
                    }
                    let attrs = match kind {
                        Kind::Link
                            if resolved.is_some_and(|element| {
                                element
                                    .attributes
                                    .destination
                                    .as_ref()
                                    .is_some_and(|url| !url.is_empty())
                            }) =>
                        {
                            Some(TextAttrs::new(TextStyle::Plain).with_role(TextRole::Link))
                        }
                        Kind::Strike => Some(TextAttrs::new(TextStyle::Plain).with_struck(true)),
                        Kind::Underline => {
                            Some(TextAttrs::new(TextStyle::Plain).with_underlined(true))
                        }
                        Kind::Strong => Some(TextAttrs::new(TextStyle::Strong)),
                        Kind::Emphasis => Some(TextAttrs::new(TextStyle::Emphasis)),
                        Kind::Code => Some(TextAttrs::new(TextStyle::Code)),
                        Kind::Highlight => {
                            Some(TextAttrs::new(TextStyle::Plain).with_highlighted(true))
                        }
                        Kind::Superscript => Some(
                            TextAttrs::new(TextStyle::Plain).with_script(TextScript::Superscript),
                        ),
                        Kind::Subscript => Some(
                            TextAttrs::new(TextStyle::Plain).with_script(TextScript::Subscript),
                        ),
                        _ => None,
                    };
                    if let Some(attrs) = attrs
                        && let Some(range) = intersection(node.source, part.source)
                    {
                        let style = out.style(attrs);
                        out.mark(range, style);
                    }
                }
                HtmlNodeKind::Comment => {
                    if let Some(range) = intersection(node.source, part.source) {
                        out.replace(range);
                    }
                }
                HtmlNodeKind::Text => {
                    let Some(range) = intersection(node.source, part.source) else {
                        continue;
                    };
                    if let Some(content) = intersection(range, part.content.source) {
                        if range.start() < content.start() {
                            out.replace(
                                TextRange::new(range.start(), content.start()).expect("prefix"),
                            );
                        }
                        text.append(source, content, out);
                        if content.end() < range.end() {
                            out.replace(
                                TextRange::new(content.end(), range.end()).expect("suffix"),
                            );
                        }
                    } else {
                        out.replace(range);
                    }
                }
            }
        }
        text.discard_space(out);
        true
    }
}

fn intersection(a: TextRange, b: TextRange) -> Option<TextRange> {
    let start = a.start().max(b.start());
    let end = a.end().min(b.end());
    (start < end).then(|| TextRange::new(start, end).expect("intersection"))
}

#[derive(Default)]
struct TextProjection {
    has_text: bool,
    spaces: Vec<TextRange>,
}
impl TextProjection {
    fn discard_space(&mut self, out: &mut ExtensionOutput) {
        for range in self.spaces.drain(..) {
            out.replace(range);
        }
    }
    fn visible(&mut self, out: &mut ExtensionOutput) {
        if self.has_text && !self.spaces.is_empty() {
            out.substitute(self.spaces.remove(0), ' ');
        }
        self.discard_space(out);
        self.has_text = true;
    }
    fn append(&mut self, source: &str, range: TextRange, out: &mut ExtensionOutput) {
        let mut cursor = range.start().get() as usize;
        let end = range.end().get() as usize;
        while cursor < end {
            let tail = &source[cursor..end];
            let c = tail.chars().next().expect("nonempty text");
            let mut size = c.len_utf8();
            let mut replacement = None;
            if c == '&'
                && let Some(stop) = tail
                    .as_bytes()
                    .iter()
                    .take(33)
                    .position(|&byte| byte == b';')
            {
                replacement = entity::decode(&tail[..=stop]);
                if replacement.is_some() {
                    size = stop + 1;
                }
            }
            let span = TextRange::new(
                ByteOffset::new(cursor as u64),
                ByteOffset::new((cursor + size) as u64),
            )
            .expect("character");
            let mut decoded = String::new();
            if let Some(value) = &replacement {
                value.append_to(&mut decoded);
            }
            let collapsible = if replacement.is_some() {
                !decoded.is_empty() && decoded.chars().all(html_space)
            } else {
                html_space(c)
            };
            if collapsible {
                self.spaces.push(span);
            } else {
                self.visible(out);
                if let Some(replacement) = replacement {
                    out.substitute_text(span, replacement);
                }
            }
            cursor += size;
        }
    }
}

fn html_space(c: char) -> bool {
    matches!(c, ' ' | '\t' | '\r' | '\n' | '\x0c')
}
