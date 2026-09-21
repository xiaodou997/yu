//! Source-preserving inline HTML formatting. Structural HTML is handled separately.
use yu_core::{TextAttrs, TextRange, TextScript, TextStyle};
use yu_syntax::NodeKind;

use super::{BlockContext, Extension, ExtensionOutput, reveals};
use crate::reference::read_range;

pub struct HtmlFormat;

struct OpenTag {
    name: String,
    range: TextRange,
    attrs: Option<TextAttrs>,
}

fn attrs(kind: Option<crate::html::HtmlElementKind>) -> Option<TextAttrs> {
    use crate::html::HtmlElementKind::*;
    Some(match kind? {
        Strike => TextAttrs::new(TextStyle::Plain).with_struck(true),
        Underline => TextAttrs::new(TextStyle::Plain).with_underlined(true),
        Strong => TextAttrs::new(TextStyle::Strong),
        Emphasis => TextAttrs::new(TextStyle::Emphasis),
        Highlight => TextAttrs::new(TextStyle::Plain).with_highlighted(true),
        Superscript => TextAttrs::new(TextStyle::Plain).with_script(TextScript::Superscript),
        Subscript => TextAttrs::new(TextStyle::Plain).with_script(TextScript::Subscript),
        _ => return None,
    })
}

impl Extension for HtmlFormat {
    fn name(&self) -> &'static str {
        "html-format"
    }

    fn decorate(&self, cx: &BlockContext<'_>, out: &mut ExtensionOutput) {
        let mut stack: Vec<OpenTag> = Vec::new();
        for node in cx.nodes().filter(|node| node.kind() == NodeKind::HtmlTag) {
            let Some(bytes) = read_range(cx.source(), node.range()) else {
                continue;
            };
            let Ok(raw) = std::str::from_utf8(&bytes) else {
                continue;
            };
            let Ok(tag) = crate::html::HtmlTag::parse(raw, node.range().start()) else {
                continue;
            };
            let name = tag.name.clone();
            if tag.closing {
                let Some(open) = stack.pop() else { continue };
                if open.name != name {
                    // Broken nesting is edited as source, never repaired into a
                    // different document or matched across an unknown container.
                    stack.clear();
                    continue;
                }
                let Some(attrs) = open.attrs else { continue };
                let whole =
                    TextRange::new(open.range.start(), node.range().end()).expect("ordered tags");
                if reveals(cx.active(), whole) {
                    continue;
                }
                let content =
                    TextRange::new(open.range.end(), node.range().start()).expect("tag content");
                let style = out.style(attrs);
                out.mark(content, style);
                out.replace(open.range);
                out.replace(node.range());
            } else if !tag.self_closing && !tag.is_void() {
                let resolved = tag.resolve_attributes(cx.source().as_str()).ok();
                let supported =
                    resolved.is_some() && stack.last().is_none_or(|entry| entry.attrs.is_some());
                let formatting =
                    if tag.semantic_kind() == Some(crate::html::HtmlElementKind::Link) {
                        let role = if resolved.as_ref().is_some_and(|a| {
                            a.destination.as_ref().is_some_and(|url| !url.is_empty())
                        }) {
                            yu_core::TextRole::Link
                        } else {
                            yu_core::TextRole::Plain
                        };
                        Some(TextAttrs::new(TextStyle::Plain).with_role(role))
                    } else {
                        attrs(tag.semantic_kind())
                    };
                stack.push(OpenTag {
                    attrs: supported.then_some(formatting).flatten(),
                    name,
                    range: node.range(),
                });
            }
        }
    }
}
