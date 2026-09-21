//! Source-backed prose ranges for platform spell checkers. Never spell-check
//! Markdown destinations, code or markup, even while their syntax is revealed.
use crate::{MarkdownDocument, SyntaxNode};
use yu_core::{ByteOffset, TextRange};
use yu_syntax::NodeKind;

impl MarkdownDocument {
    /// Returns disjoint UTF-8 source ranges, bounded by the requested region.
    /// Invalid boundaries return no text. Hosts bind diagnostics to revision
    /// and suspend publication while an IME composition is active.
    #[must_use]
    pub fn spelling_ranges(&self, requested: TextRange) -> Vec<TextRange> {
        let text = self.source.as_str();
        let (start, end) = (
            requested.start().get() as usize,
            requested.end().get() as usize,
        );
        if text.get(start..end).is_none() {
            return Vec::new();
        }
        let requested_start = start;
        let requested_end = end;
        // Expand only edge tokens before classifying bare URLs. A viewport
        // query starting inside an address must not spell-check its suffix.
        let start = text[..start].rfind(char::is_whitespace).map_or(0, |i| {
            i + text[i..].chars().next().expect("boundary").len_utf8()
        });
        let end = text[end..]
            .find(char::is_whitespace)
            .map_or(text.len(), |i| end + i);
        let expanded = TextRange::new(ByteOffset::new(start as u64), ByteOffset::new(end as u64))
            .expect("expanded range");
        let Some(tree) = self.tree() else {
            return Vec::new();
        };
        let mut excluded = SyntaxNode::new(tree, 0)
            .descendants_in(expanded)
            .filter(|node| {
                matches!(
                    node.kind(),
                    NodeKind::FootnoteReference
                        | NodeKind::FootnoteMark
                        | NodeKind::FrontMatter
                        | NodeKind::MathBlock
                        | NodeKind::InlineMath
                        | NodeKind::EquationReference
                        | NodeKind::CodeBlock
                        | NodeKind::FencedCode
                        | NodeKind::InlineCode
                        | NodeKind::HtmlBlock
                        | NodeKind::HtmlTag
                        | NodeKind::CommentBlock
                        | NodeKind::Comment
                        | NodeKind::ProcessingInstruction
                        | NodeKind::ProcessingInstructionBlock
                        | NodeKind::LinkReference
                        | NodeKind::Image
                        | NodeKind::Autolink
                        | NodeKind::HeaderMark
                        | NodeKind::QuoteMark
                        | NodeKind::ListMark
                        | NodeKind::LinkMark
                        | NodeKind::EmphasisMark
                        | NodeKind::CodeMark
                        | NodeKind::CodeText
                        | NodeKind::CodeInfo
                        | NodeKind::LinkTitle
                        | NodeKind::LinkLabel
                        | NodeKind::Url
                        | NodeKind::TaskMarker
                        | NodeKind::Entity
                        | NodeKind::Escape
                        | NodeKind::HorizontalRule
                )
            })
            .map(|node| {
                (
                    node.range().start().get() as usize,
                    node.range().end().get() as usize,
                )
            })
            .collect::<Vec<_>>();
        excluded.sort_unstable();
        let mut result = Vec::new();
        let mut cursor = start;
        for (from, to) in excluded {
            let from = from.clamp(start, end);
            let to = to.clamp(start, end);
            if cursor < from {
                push_prose(text, cursor, from, &mut result);
            }
            cursor = cursor.max(to);
        }
        if cursor < end {
            push_prose(text, cursor, end, &mut result);
        }
        result
            .into_iter()
            .filter_map(|range| {
                let start = range.start().get().max(requested_start as u64);
                let end = range.end().get().min(requested_end as u64);
                (start < end).then(|| {
                    TextRange::new(ByteOffset::new(start), ByteOffset::new(end))
                        .expect("clipped range")
                })
            })
            .collect()
    }
}

fn push_prose(text: &str, start: usize, end: usize, output: &mut Vec<TextRange>) {
    // Only inspect syntax-free gaps. This keeps link labels independent of
    // destinations without another Markdown recognizer or host-side parser.
    let mut cursor = start;
    let mut segment = start;
    for token in text[start..end].split_whitespace() {
        let Some(relative) = text[cursor..end].find(token) else {
            break;
        };
        let offset = cursor + relative;
        cursor = offset + token.len();
        let candidate = token.trim_matches(|c: char| {
            matches!(c, '(' | ')' | '[' | ']' | '<' | '>' | '"' | ',' | ';')
        });
        if candidate.contains("://")
            || candidate.starts_with("www.")
            || candidate.starts_with("mailto:")
            || (candidate.contains('@') && candidate.contains('.'))
        {
            push_range(text, segment, offset, output);
            segment = cursor;
        }
    }
    push_range(text, segment, end, output);
}

fn push_range(text: &str, start: usize, end: usize, output: &mut Vec<TextRange>) {
    if text[start..end].chars().any(char::is_alphabetic) {
        output.push(
            TextRange::new(ByteOffset::new(start as u64), ByteOffset::new(end as u64))
                .expect("ordered prose range"),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use yu_text::TextBuffer;

    fn prose(source: &str) -> String {
        let snapshot = TextBuffer::new(source).snapshot();
        let doc = crate::parse(&snapshot);
        doc.spelling_ranges(
            TextRange::new(ByteOffset::new(0), snapshot.len_bytes()).expect("range"),
        )
        .iter()
        .map(|r| &source[r.start().get() as usize..r.end().get() as usize])
        .collect::<Vec<_>>()
        .join("|")
    }

    #[test]
    fn metadata_is_excluded_without_hiding_body_prose() {
        let source = "---\ntitle: missspelled\n# metadata comment\n---\n\nBody prose\n";
        let buffer = yu_text::TextBuffer::new(source);
        let doc = crate::parse(&buffer.snapshot());
        let ranges = doc.spelling_ranges(
            yu_core::TextRange::new(
                yu_core::ByteOffset::ZERO,
                yu_core::ByteOffset::new(source.len() as u64),
            )
            .expect("range"),
        );
        let prose: String = ranges
            .iter()
            .map(|range| &source[range.start().get() as usize..range.end().get() as usize])
            .collect();
        assert_eq!(prose.trim(), "Body prose");
    }

    #[test]
    fn spelling_keeps_prose_and_link_labels_but_excludes_code_and_addresses() {
        let source = "# Helo\n\nA **wrld** and [labl](https://exampel.com) `codde` <https://exampel.com>\n\n```swift\nlet codde = 1\n```\n\nwww.exampel.com mail@exampel.com https://exampel.com Tail\n";
        let value = prose(source);
        for word in ["Helo", "wrld", "labl", "Tail"] {
            assert!(value.contains(word), "missing {word}: {value}");
        }
        for word in ["codde", "exampel", "swift", "**", "#"] {
            assert!(!value.contains(word), "unexpected {word}: {value}");
        }
    }

    #[test]
    fn clipped_requests_inside_code_and_markup_remain_excluded() {
        let source = "prefix\n\n```swift\ncodde\n```\n\n<!-- commment -->\n\n![altt](image.png)\n";
        let snapshot = TextBuffer::new(source).snapshot();
        let doc = crate::parse(&snapshot);
        for word in ["codde", "commment", "altt", "image.png"] {
            let start = source.find(word).expect("fixture");
            let requested = TextRange::new(
                ByteOffset::new(start as u64),
                ByteOffset::new((start + word.len()) as u64),
            )
            .expect("range");
            assert!(doc.spelling_ranges(requested).is_empty(), "{word}");
        }
    }

    #[test]
    fn clipped_bare_addresses_do_not_become_prose() {
        let source = "before https://exampel.com/path mail@exampel.com after";
        let doc = crate::parse(&TextBuffer::new(source).snapshot());
        for start in source.match_indices("exampel").map(|(i, _)| i) {
            let range = TextRange::new(
                ByteOffset::new(start as u64),
                ByteOffset::new((start + 7) as u64),
            )
            .expect("range");
            assert!(doc.spelling_ranges(range).is_empty());
        }
    }

    #[test]
    fn spelling_preserves_unicode_source_coordinates_and_rejects_split_scalars() {
        let source = "中文 🙂 **wrld** e\u{301}\n";
        let snapshot = TextBuffer::new(source).snapshot();
        let doc = crate::parse(&snapshot);
        assert!(prose(source).contains("中文 🙂"));
        assert!(
            doc.spelling_ranges(
                TextRange::new(ByteOffset::new(1), snapshot.len_bytes()).expect("range")
            )
            .is_empty()
        );
        assert_eq!(snapshot.as_str(), source);
    }
}
