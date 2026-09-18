//! Parser-owned character references share the canonical source projection.
//! Unknown names, escaped ampersands and code remain literal source.
use std::collections::HashMap;
use std::sync::LazyLock;

use yu_core::TextRange;
use yu_decoration::ReplacementText;
use yu_syntax::NodeKind;

use super::{BlockContext, Extension, ExtensionOutput, reveals};

static NAMES: LazyLock<HashMap<&'static str, &'static str>> = LazyLock::new(|| {
    entities::ENTITIES
        .iter()
        .filter(|entry| entry.entity.ends_with(';'))
        .map(|entry| (entry.entity, entry.characters))
        .collect()
});

pub struct Entity;

impl Extension for Entity {
    fn name(&self) -> &'static str {
        "entity"
    }

    fn decorate(&self, cx: &BlockContext<'_>, out: &mut ExtensionOutput) {
        // Table syntax stays projected during cell editing, including atoms.
        let table = cx.active().is_some() && super::table::container_table(cx).is_some();
        for (range, text) in atoms(cx) {
            if table || !reveals(cx.active(), range) {
                out.substitute_text(range, text);
            }
        }
    }
}

pub(crate) fn atoms(cx: &BlockContext<'_>) -> Vec<(TextRange, ReplacementText)> {
    cx.nodes()
        .filter(|node| node.kind() == NodeKind::Entity)
        .filter_map(|node| Some((node.range(), decode(&cx.text(node.range())?)?)))
        .collect()
}

fn decode(source: &str) -> Option<ReplacementText> {
    let inner = source.strip_prefix("&#").and_then(|s| s.strip_suffix(';'));
    if let Some(inner) = inner {
        let value = if let Some(hex) = inner.strip_prefix('x').or_else(|| inner.strip_prefix('X')) {
            u32::from_str_radix(hex, 16).ok()?
        } else {
            inner.parse::<u32>().ok()?
        };
        // CommonMark replaces null and invalid Unicode with U+FFFD.
        let character = char::from_u32(value)
            .filter(|&c| c != '\0')
            .unwrap_or('\u{fffd}');
        Some(ReplacementText::Scalar(character))
    } else {
        NAMES
            .get(source)
            .map(|&text| ReplacementText::Literal(text))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_named_reference_is_decodable_and_parser_owned() {
        for entry in entities::ENTITIES
            .iter()
            .filter(|e| e.entity.ends_with(';'))
        {
            let mut value = String::new();
            decode(entry.entity)
                .expect("known name")
                .append_to(&mut value);
            assert_eq!(value, entry.characters);
            let snapshot = yu_text::TextBuffer::new(entry.entity).snapshot();
            let document = crate::parse(&snapshot);
            let tree = document.tree().expect("valid entity fixture");
            let cx = BlockContext::for_block(
                &snapshot,
                tree,
                document.reference_definitions(),
                document.presentation(),
                document.blocks().get(0).expect("valid entity fixture"),
            );
            let atoms = atoms(&cx);
            assert_eq!(atoms.len(), 1, "{}", entry.entity);
            assert_eq!(atoms[0].0.len(), entry.entity.len() as u64);
        }
    }
}
