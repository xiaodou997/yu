#![forbid(unsafe_code)]

//! Lossless Markdown syntax experiments.
//!
//! Phase 1 provides a deliberately small block scanner. It preserves every
//! source byte through ranges, but it is not yet a CommonMark implementation.

use yu_core::{ByteOffset, Revision, TextRange};
use yu_text::TextSnapshot;

/// The block shapes recognized by the Phase 1 scanner.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BlockKind {
    BlankLine,
    Paragraph,
    AtxHeading { level: u8 },
    FencedCodeBlock { marker: char, closed: bool },
}

/// A block that refers to source without owning or normalizing its text.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Block {
    kind: BlockKind,
    range: TextRange,
}

impl Block {
    #[must_use]
    pub fn kind(self) -> BlockKind {
        self.kind
    }

    #[must_use]
    pub fn range(self) -> TextRange {
        self.range
    }
}

/// A lossless block view of one immutable text revision.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MarkdownDocument {
    revision: Revision,
    source_len: ByteOffset,
    blocks: Vec<Block>,
}

impl MarkdownDocument {
    #[must_use]
    pub fn revision(&self) -> Revision {
        self.revision
    }

    #[must_use]
    pub fn source_len(&self) -> ByteOffset {
        self.source_len
    }

    #[must_use]
    pub fn blocks(&self) -> &[Block] {
        &self.blocks
    }

    /// Confirms that ordered block ranges cover the source exactly once.
    #[must_use]
    pub fn has_lossless_coverage(&self) -> bool {
        let mut expected_start = ByteOffset::ZERO;
        for block in &self.blocks {
            if block.range.start() != expected_start {
                return false;
            }
            expected_start = block.range.end();
        }
        expected_start == self.source_len
    }
}

#[derive(Clone, Copy, Debug)]
struct Line<'a> {
    start: usize,
    end: usize,
    body: &'a str,
}

#[derive(Clone, Copy, Debug)]
struct Fence {
    marker: char,
    count: usize,
}

/// Scans the snapshot into a lossless Phase 1 block structure.
#[must_use]
pub fn parse(snapshot: &TextSnapshot) -> MarkdownDocument {
    let source = snapshot.as_str();
    let lines = lines(source);
    let mut blocks = Vec::new();
    let mut index = 0;

    while index < lines.len() {
        let line = lines[index];

        if line.body.trim().is_empty() {
            blocks.push(block(BlockKind::BlankLine, line.start, line.end));
            index += 1;
            continue;
        }

        if let Some(fence) = opening_fence(line.body) {
            let start = line.start;
            index += 1;
            let mut closed = false;
            while index < lines.len() {
                let candidate = lines[index];
                index += 1;
                if is_closing_fence(candidate.body, fence) {
                    closed = true;
                    break;
                }
            }
            let end = lines[index - 1].end;
            blocks.push(block(
                BlockKind::FencedCodeBlock {
                    marker: fence.marker,
                    closed,
                },
                start,
                end,
            ));
            continue;
        }

        if let Some(level) = atx_heading_level(line.body) {
            blocks.push(block(BlockKind::AtxHeading { level }, line.start, line.end));
            index += 1;
            continue;
        }

        let start = line.start;
        index += 1;
        while index < lines.len() {
            let candidate = lines[index];
            if candidate.body.trim().is_empty()
                || opening_fence(candidate.body).is_some()
                || atx_heading_level(candidate.body).is_some()
            {
                break;
            }
            index += 1;
        }
        let end = lines[index - 1].end;
        blocks.push(block(BlockKind::Paragraph, start, end));
    }

    MarkdownDocument {
        revision: snapshot.revision(),
        source_len: ByteOffset::try_from(source.len()).unwrap_or(ByteOffset::new(u64::MAX)),
        blocks,
    }
}

fn block(kind: BlockKind, start: usize, end: usize) -> Block {
    let start = ByteOffset::try_from(start).unwrap_or(ByteOffset::new(u64::MAX));
    let end = ByteOffset::try_from(end).unwrap_or(ByteOffset::new(u64::MAX));
    let range = TextRange::new(start, end).unwrap_or_else(|| TextRange::empty(start));
    Block { kind, range }
}

fn lines(source: &str) -> Vec<Line<'_>> {
    let mut result = Vec::new();
    let mut start = 0;

    while start < source.len() {
        let relative_end = source[start..].find('\n');
        let end = relative_end.map_or(source.len(), |offset| start + offset + 1);
        let mut body_end = end;
        if source.as_bytes().get(body_end.wrapping_sub(1)) == Some(&b'\n') {
            body_end -= 1;
        }
        if source.as_bytes().get(body_end.wrapping_sub(1)) == Some(&b'\r') {
            body_end -= 1;
        }
        result.push(Line {
            start,
            end,
            body: &source[start..body_end],
        });
        start = end;
    }

    result
}

fn content_after_indent(line: &str) -> Option<&str> {
    let spaces = line.bytes().take_while(|byte| *byte == b' ').count();
    (spaces <= 3).then(|| &line[spaces..])
}

fn opening_fence(line: &str) -> Option<Fence> {
    let content = content_after_indent(line)?;
    let marker = content.chars().next()?;
    if marker != '`' && marker != '~' {
        return None;
    }
    let count = content
        .chars()
        .take_while(|candidate| *candidate == marker)
        .count();
    (count >= 3).then_some(Fence { marker, count })
}

fn is_closing_fence(line: &str, opening: Fence) -> bool {
    let Some(content) = content_after_indent(line) else {
        return false;
    };
    let count = content
        .chars()
        .take_while(|candidate| *candidate == opening.marker)
        .count();
    if count < opening.count {
        return false;
    }
    content[count..].chars().all(char::is_whitespace)
}

fn atx_heading_level(line: &str) -> Option<u8> {
    let content = content_after_indent(line)?;
    let count = content
        .chars()
        .take_while(|candidate| *candidate == '#')
        .count();
    if !(1..=6).contains(&count) {
        return None;
    }

    let tail = &content[count..];
    if !tail.is_empty() && !tail.starts_with([' ', '\t']) {
        return None;
    }

    u8::try_from(count).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use yu_text::TextBuffer;

    #[test]
    fn scanner_covers_source_without_gaps() {
        let source = "# 羽\n\nparagraph\ncontinued\n\n```rust\nfn main() {}\n```\n";
        let buffer = TextBuffer::new(source);
        let document = parse(&buffer.snapshot());

        assert!(document.has_lossless_coverage());
        assert_eq!(document.source_len().get(), source.len() as u64);
        assert_eq!(document.blocks().len(), 5);
        assert_eq!(
            document.blocks()[0].kind(),
            BlockKind::AtxHeading { level: 1 }
        );
        assert_eq!(document.blocks()[1].kind(), BlockKind::BlankLine);
        assert_eq!(document.blocks()[2].kind(), BlockKind::Paragraph);
        assert_eq!(
            document.blocks()[4].kind(),
            BlockKind::FencedCodeBlock {
                marker: '`',
                closed: true
            }
        );
    }

    #[test]
    fn unclosed_fence_owns_the_remaining_source() {
        let buffer = TextBuffer::new("before\n\n~~~\ninside\n");
        let document = parse(&buffer.snapshot());

        assert!(document.has_lossless_coverage());
        assert_eq!(
            document.blocks()[2].kind(),
            BlockKind::FencedCodeBlock {
                marker: '~',
                closed: false
            }
        );
    }

    #[test]
    fn empty_source_has_lossless_coverage() {
        let buffer = TextBuffer::new("");
        let document = parse(&buffer.snapshot());

        assert!(document.blocks().is_empty());
        assert!(document.has_lossless_coverage());
    }

    #[test]
    fn crlf_bytes_are_preserved_in_ranges() {
        let source = "# title\r\n\r\ntext\r\n";
        let buffer = TextBuffer::new(source);
        let snapshot = buffer.snapshot();
        let document = parse(&snapshot);
        let reconstructed: String = document
            .blocks()
            .iter()
            .map(|block| {
                let start =
                    usize::try_from(block.range().start()).expect("test offset should fit usize");
                let end =
                    usize::try_from(block.range().end()).expect("test offset should fit usize");
                &snapshot.as_str()[start..end]
            })
            .collect();

        assert_eq!(reconstructed, source);
    }
}
