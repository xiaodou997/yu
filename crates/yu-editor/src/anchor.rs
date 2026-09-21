//! Source-backed fragment navigation; no document serialization or UI parsing.
use crate::{DecorationCache, DecorationError, VisualText};
use std::collections::HashSet;
use yu_core::{ByteOffset, TextRange};
use yu_markdown::{BlockKind, MarkdownDocument};

pub fn document_anchor_target(
    markdown: &MarkdownDocument,
    destination: &str,
) -> Result<Option<TextRange>, DecorationError> {
    let Some(fragment) = destination.strip_prefix('#').and_then(decode_fragment) else {
        return Ok(None);
    };
    if fragment.is_empty() {
        return Ok(markdown
            .source()
            .as_str()
            .chars()
            .next()
            .and_then(|c| TextRange::new(ByteOffset::ZERO, ByteOffset::new(c.len_utf8() as u64))));
    }
    let mut explicit: Option<TextRange> = None;
    // Explicit HTML ids take precedence over generated heading identifiers.
    for region in &markdown.html_regions().regions {
        let Ok(model) = &region.model else {
            continue;
        };
        for (id, element) in model.resolution.elements.iter().enumerate() {
            if element
                .as_ref()
                .is_some_and(|element| element.attributes.id.as_deref() == Some(fragment.as_str()))
            {
                let range = model.fragment.nodes[id].source;
                if explicit.is_none_or(|old| old.start() > range.start()) {
                    explicit = Some(range);
                }
            }
        }
    }
    for block in markdown.semantic_blocks().iter() {
        if markdown.html_regions().region_for(block.range()).is_some() {
            continue;
        }
        for element in markdown.inline_html_elements(block) {
            if element.attributes.id.as_deref() == Some(fragment.as_str())
                && explicit.is_none_or(|old| old.start() > element.source.start())
            {
                explicit = Some(element.source);
            }
        }
    }
    if explicit.is_some() {
        return Ok(explicit);
    }
    let mut used = HashSet::new();
    let mut cache = DecorationCache::default();
    for block in markdown
        .semantic_blocks()
        .iter()
        .filter(|block| matches!(block.kind(), BlockKind::Heading { .. }))
    {
        let decorations = cache.decorate_navigation(markdown, block)?;
        let visual = VisualText::new(markdown.source(), block.range(), decorations.set().clone())?;
        let base = heading_identifier(visual.text());
        let mut identifier = base.clone();
        let mut suffix = 1u64;
        while used.contains(&identifier) {
            identifier = format!("{base}-{suffix}");
            suffix += 1;
        }
        used.insert(identifier.clone());
        if identifier == fragment {
            return Ok(Some(yu_markdown::heading_content_range(markdown, block)));
        }
    }
    Ok(None)
}

fn heading_identifier(text: &str) -> String {
    let mut id = String::new();
    let mut gap = false;
    for c in text.trim().chars().flat_map(char::to_lowercase) {
        if c.is_whitespace() {
            gap = !id.is_empty();
        } else if c.is_alphanumeric() || matches!(c, '-' | '_') {
            if gap {
                id.push('-');
                gap = false;
            }
            id.push(c);
        }
    }
    id
}

fn decode_fragment(fragment: &str) -> Option<String> {
    let bytes = fragment.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut at = 0;
    while at < bytes.len() {
        if bytes[at] == b'%' {
            let hi = char::from(*bytes.get(at + 1)?).to_digit(16)?;
            let lo = char::from(*bytes.get(at + 2)?).to_digit(16)?;
            decoded.push((hi * 16 + lo) as u8);
            at += 3;
        } else {
            decoded.push(bytes[at]);
            at += 1;
        }
    }
    String::from_utf8(decoded).ok()
}
