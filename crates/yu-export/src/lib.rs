#![forbid(unsafe_code)]

//! Source-backed clipboard payloads for Yu.
//!
//! Markdown remains the canonical payload. The HTML fragment is generated from
//! the same selected source range by the current lossless Markdown parser; it
//! is deliberately conservative for syntax that the parser does not yet
//! classify instead of pretending that a TextKit projection is semantic HTML.

use std::error::Error;
use std::fmt;

use yu_core::{ByteOffset, Revision, TextRange};
use yu_markdown::{
    Block, BlockKind, InlineDocument, InlineNodeKind, InlineSpan, InlineSpanKind,
    ReferenceDefinitionIndex, TaskState, parse, parse_inline_with_definitions,
};
use yu_text::{TextBuffer, TextPositionError, TextSnapshot};

/// Pasteboard MIME/UTI names shared by native adapters.
pub const MARKDOWN_MIME: &str = "text/markdown";
pub const MARKDOWN_UTI: &str = "net.daringfireball.markdown";
pub const PLAIN_TEXT_MIME: &str = "text/plain;charset=utf-8";
pub const HTML_MIME: &str = "text/html";

/// The three payloads published for one canonical source selection.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClipboardPayload {
    revision: Revision,
    source_range: TextRange,
    markdown: String,
    plain_text: String,
    html: String,
}

impl ClipboardPayload {
    #[must_use]
    pub fn revision(&self) -> Revision {
        self.revision
    }

    #[must_use]
    pub const fn source_range(&self) -> TextRange {
        self.source_range
    }

    /// Canonical Markdown source. This is the payload preferred by Yu and
    /// other Markdown-aware applications.
    #[must_use]
    pub fn markdown(&self) -> &str {
        &self.markdown
    }

    /// Plain-text fallback. It intentionally remains the selected source so a
    /// plain-text-only paste does not silently discard Markdown syntax.
    #[must_use]
    pub fn plain_text(&self) -> &str {
        &self.plain_text
    }

    /// Conservative semantic HTML fragment generated from the same source.
    #[must_use]
    pub fn html(&self) -> &str {
        &self.html
    }
}

/// Errors raised while exporting a revision-bound source range.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ExportError {
    RevisionMismatch {
        snapshot: Revision,
        expected: Revision,
    },
    SourcePosition(TextPositionError),
    InlineParse(yu_markdown::InlineParseError),
}

impl fmt::Display for ExportError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::RevisionMismatch { snapshot, expected } => write!(
                formatter,
                "clipboard snapshot revision {snapshot:?} does not match expected {expected:?}"
            ),
            Self::SourcePosition(error) => error.fmt(formatter),
            Self::InlineParse(error) => {
                write!(formatter, "cannot parse clipboard inline source: {error}")
            }
        }
    }
}

impl Error for ExportError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::SourcePosition(error) => Some(error),
            Self::InlineParse(error) => Some(error),
            Self::RevisionMismatch { .. } => None,
        }
    }
}

impl From<TextPositionError> for ExportError {
    fn from(error: TextPositionError) -> Self {
        Self::SourcePosition(error)
    }
}

impl From<yu_markdown::InlineParseError> for ExportError {
    fn from(error: yu_markdown::InlineParseError) -> Self {
        Self::InlineParse(error)
    }
}

/// Exports one revision-bound source selection for a native clipboard.
pub fn export_clipboard(
    snapshot: &TextSnapshot,
    expected_revision: Revision,
    source_range: TextRange,
) -> Result<ClipboardPayload, ExportError> {
    if snapshot.revision() != expected_revision {
        return Err(ExportError::RevisionMismatch {
            snapshot: snapshot.revision(),
            expected: expected_revision,
        });
    }
    let markdown = slice(snapshot, source_range)?.to_owned();
    let plain_text = markdown.clone();
    let html = export_html_fragment(&markdown)?;
    Ok(ClipboardPayload {
        revision: expected_revision,
        source_range,
        markdown,
        plain_text,
        html,
    })
}

/// Exports a source string as an HTML fragment. This helper is useful for
/// tests and for future non-editor clipboard providers.
pub fn export_html_fragment(source: &str) -> Result<String, ExportError> {
    let buffer = TextBuffer::new(source);
    let snapshot = buffer.snapshot();
    let document = parse(&snapshot);
    let mut html = String::new();
    render_blocks(
        &snapshot,
        document.reference_definitions(),
        document.blocks().iter(),
        &mut html,
    )?;
    Ok(html)
}

fn slice(snapshot: &TextSnapshot, range: TextRange) -> Result<&str, ExportError> {
    snapshot.utf16_offset(range.start())?;
    snapshot.utf16_offset(range.end())?;
    let start = usize::try_from(range.start().get()).expect("validated source offset fits usize");
    let end = usize::try_from(range.end().get()).expect("validated source offset fits usize");
    Ok(&snapshot.as_str()[start..end])
}

fn render_blocks(
    snapshot: &TextSnapshot,
    definitions: &ReferenceDefinitionIndex,
    blocks: impl IntoIterator<Item = Block>,
    output: &mut String,
) -> Result<(), ExportError> {
    let mut first = true;
    for block in blocks {
        let fragment = render_block(snapshot, definitions, block)?;
        if fragment.is_empty() {
            continue;
        }
        if !first {
            output.push('\n');
        }
        first = false;
        output.push_str(&fragment);
    }
    Ok(())
}

fn render_block(
    snapshot: &TextSnapshot,
    definitions: &ReferenceDefinitionIndex,
    block: Block,
) -> Result<String, ExportError> {
    let source = slice(snapshot, block.range())?;
    match block.kind() {
        BlockKind::BlankLine | BlockKind::ReferenceDefinition => Ok(String::new()),
        BlockKind::Paragraph => {
            let mut html = String::from("<p>");
            render_inline(snapshot, definitions, block.range(), &mut html)?;
            html.push_str("</p>");
            Ok(html)
        }
        BlockKind::AtxHeading { level } => {
            let content = heading_content_range(snapshot, block.range());
            let mut html = format!("<h{level}>");
            render_inline(snapshot, definitions, content, &mut html)?;
            html.push_str(&format!("</h{level}>"));
            Ok(html)
        }
        BlockKind::FencedCodeBlock { marker, closed } => {
            Ok(render_fenced_code(source, marker, closed))
        }
        BlockKind::BlockQuote { .. } => {
            let stripped = strip_blockquote(source);
            let inner = export_html_fragment(&stripped)?;
            Ok(format!("<blockquote>{inner}</blockquote>"))
        }
        BlockKind::ListItem { ordered, .. } | BlockKind::TaskListItem { ordered, .. } => {
            render_list_item(source, block, ordered)
        }
    }
}

fn render_inline(
    snapshot: &TextSnapshot,
    definitions: &ReferenceDefinitionIndex,
    range: TextRange,
    output: &mut String,
) -> Result<(), ExportError> {
    let inline = parse_inline_with_definitions(snapshot, range, Some(definitions))?;
    let renderer = InlineRenderer {
        snapshot,
        inline: &inline,
        definitions,
    };
    renderer.render_range(range, output)
}

struct InlineRenderer<'a> {
    snapshot: &'a TextSnapshot,
    inline: &'a InlineDocument,
    definitions: &'a ReferenceDefinitionIndex,
}

impl InlineRenderer<'_> {
    fn render_range(&self, range: TextRange, output: &mut String) -> Result<(), ExportError> {
        let mut cursor = range.start();
        while cursor < range.end() {
            let Some(span) = self.next_span(cursor, range.end()) else {
                self.render_plain(cursor, range.end(), output);
                break;
            };
            if span.source_range().start() > cursor {
                self.render_plain(cursor, span.source_range().start(), output);
            }
            self.render_span(span, output)?;
            cursor = span.source_range().end();
        }
        Ok(())
    }

    fn next_span(&self, cursor: ByteOffset, end: ByteOffset) -> Option<InlineSpan> {
        self.inline
            .spans()
            .iter()
            .copied()
            .filter(|span| {
                span.source_range().start() >= cursor && span.source_range().end() <= end
            })
            .min_by_key(|span| {
                (
                    span.source_range().start(),
                    std::cmp::Reverse(span.source_range().end()),
                )
            })
    }

    fn render_span(&self, span: InlineSpan, output: &mut String) -> Result<(), ExportError> {
        match span.kind() {
            InlineSpanKind::Emphasis | InlineSpanKind::Strong => {
                let (open, close) = if span.kind() == InlineSpanKind::Emphasis {
                    ("<em>", "</em>")
                } else {
                    ("<strong>", "</strong>")
                };
                output.push_str(open);
                self.render_range(span.content(), output)?;
                output.push_str(close);
            }
            InlineSpanKind::CodeSpan => {
                output.push_str("<code>");
                self.render_escaped(span.content(), output);
                output.push_str("</code>");
            }
            InlineSpanKind::Link | InlineSpanKind::Autolink => {
                let destination = span.destination().unwrap_or(span.content());
                output.push_str("<a href=\"");
                self.render_attribute(destination, output);
                output.push_str("\">");
                self.render_range(span.content(), output)?;
                output.push_str("</a>");
            }
            InlineSpanKind::Image => {
                output.push_str("<img src=\"");
                if let Some(destination) = span.destination() {
                    self.render_attribute(destination, output);
                }
                output.push_str("\" alt=\"");
                self.render_attribute(span.content(), output);
                output.push_str("\">");
            }
            InlineSpanKind::ReferenceLink | InlineSpanKind::ReferenceImage => {
                let label = span
                    .reference()
                    .filter(|range| !range.is_empty())
                    .unwrap_or(span.content());
                let Some(definition) = self.definitions.lookup(self.snapshot, label) else {
                    self.render_plain(span.content().start(), span.content().end(), output);
                    return Ok(());
                };
                if span.kind() == InlineSpanKind::ReferenceImage {
                    output.push_str("<img src=\"");
                    self.render_attribute(definition.destination(), output);
                    output.push_str("\" alt=\"");
                    self.render_attribute(span.content(), output);
                    output.push_str("\">");
                } else {
                    output.push_str("<a href=\"");
                    self.render_attribute(definition.destination(), output);
                    output.push_str("\">");
                    self.render_range(span.content(), output)?;
                    output.push_str("</a>");
                }
            }
        }
        Ok(())
    }

    fn render_plain(&self, start: ByteOffset, end: ByteOffset, output: &mut String) {
        for node in self.inline.nodes() {
            let node_range = node.range();
            if node_range.end() <= start || node_range.start() >= end {
                continue;
            }
            let node_start = node_range.start().max(start);
            let node_end = node_range.end().min(end);
            match node.kind() {
                InlineNodeKind::LineBreak { hard } => {
                    if hard {
                        output.push_str("<br>\n");
                    } else {
                        output.push('\n');
                    }
                }
                InlineNodeKind::Escaped if node_start == node_range.start() => {
                    let unescaped_start = ByteOffset::new(node_range.start().get() + 1);
                    let unescaped_range = TextRange::new(unescaped_start, node_range.end())
                        .expect("escaped node has a scalar after its marker");
                    self.render_escaped_range(unescaped_range, output);
                }
                _ => self.render_escaped_range(
                    TextRange::new(node_start, node_end).expect("clipped node range is ordered"),
                    output,
                ),
            }
        }
    }

    fn render_escaped(&self, range: TextRange, output: &mut String) {
        self.render_escaped_range(range, output);
    }

    fn render_escaped_range(&self, range: TextRange, output: &mut String) {
        let text = slice(self.snapshot, range).expect("parser-owned range is valid");
        escape_html_text(text, output);
    }

    fn render_attribute(&self, range: TextRange, output: &mut String) {
        let text = slice(self.snapshot, range).expect("parser-owned range is valid");
        escape_html_attribute(text, output);
    }
}

fn heading_content_range(snapshot: &TextSnapshot, range: TextRange) -> TextRange {
    let source = slice(snapshot, range).expect("parser-owned heading range is valid");
    let line_end = source.trim_end_matches(['\r', '\n']).len();
    let bytes = source.as_bytes();
    let mut index = 0;
    while index < line_end && bytes[index] == b' ' {
        index += 1;
    }
    while index < line_end && bytes[index] == b'#' {
        index += 1;
    }
    if index < line_end && matches!(bytes[index], b' ' | b'\t') {
        index += 1;
        while index < line_end && matches!(bytes[index], b' ' | b'\t') {
            index += 1;
        }
    }
    TextRange::new(
        ByteOffset::new(range.start().get() + index as u64),
        ByteOffset::new(range.start().get() + line_end as u64),
    )
    .expect("heading content range is ordered")
}

fn render_fenced_code(source: &str, marker: char, closed: bool) -> String {
    let mut lines = source.split_inclusive('\n').collect::<Vec<_>>();
    if lines.is_empty() {
        lines.push(source);
    }
    let info = lines
        .first()
        .map(|line| line.trim_end_matches(['\r', '\n']))
        .map(|line| line.trim_start_matches([' ', '\t', marker]))
        .unwrap_or("")
        .trim();
    let language = info.split_whitespace().next().unwrap_or("");
    let body_end = if closed && lines.len() > 1 {
        lines.len() - 1
    } else {
        lines.len()
    };
    let body = lines
        .get(1..body_end)
        .unwrap_or_default()
        .iter()
        .copied()
        .collect::<String>();
    let mut output = String::from("<pre><code");
    if !language.is_empty() {
        output.push_str(" class=\"language-");
        escape_html_attribute(language, &mut output);
        output.push('"');
    }
    output.push('>');
    escape_html_text(&body, &mut output);
    output.push_str("</code></pre>");
    output
}

fn strip_blockquote(source: &str) -> String {
    source
        .lines()
        .map(|line| {
            let line = line.trim_start_matches([' ', '\t']);
            let line = line.strip_prefix('>').unwrap_or(line);
            line.strip_prefix([' ', '\t']).unwrap_or(line)
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn render_list_item(source: &str, block: Block, ordered: bool) -> Result<String, ExportError> {
    let mut text = source.to_owned();
    let first_line_end = text.find('\n').unwrap_or(text.len());
    let prefix_end = list_prefix_end(&text[..first_line_end]);
    text.replace_range(..prefix_end, "");
    let mut task = None;
    if let BlockKind::TaskListItem { state, .. } = block.kind() {
        task = Some(state);
        let marker_start = text.find('[').unwrap_or(0);
        if text[marker_start..].starts_with("[ ]")
            || text[marker_start..].starts_with("[x]")
            || text[marker_start..].starts_with("[X]")
        {
            text.replace_range(marker_start..marker_start + 3, "");
            while text.starts_with([' ', '\t']) {
                text.remove(0);
            }
        }
    }
    let mut html = if ordered {
        String::from("<ol><li>")
    } else {
        String::from("<ul><li>")
    };
    if let Some(state) = task {
        html.push_str("<input type=\"checkbox\" disabled");
        if state == TaskState::Done {
            html.push_str(" checked");
        }
        html.push_str("> ");
    }
    html.push_str(&export_html_fragment(&text)?);
    html.push_str(if ordered { "</li></ol>" } else { "</li></ul>" });
    Ok(html)
}

fn list_prefix_end(line: &str) -> usize {
    let bytes = line.as_bytes();
    let mut index = 0;
    while index < bytes.len() && bytes[index] == b' ' {
        index += 1;
    }
    if index < bytes.len() && matches!(bytes[index], b'-' | b'+' | b'*') {
        index += 1;
    } else {
        while index < bytes.len() && bytes[index].is_ascii_digit() {
            index += 1;
        }
        if index < bytes.len() && matches!(bytes[index], b'.' | b')') {
            index += 1;
        }
    }
    while index < bytes.len() && matches!(bytes[index], b' ' | b'\t') {
        index += 1;
    }
    index
}

fn escape_html_text(text: &str, output: &mut String) {
    for character in text.chars() {
        match character {
            '&' => output.push_str("&amp;"),
            '<' => output.push_str("&lt;"),
            '>' => output.push_str("&gt;"),
            _ => output.push(character),
        }
    }
}

fn escape_html_attribute(text: &str, output: &mut String) {
    for character in text.chars() {
        match character {
            '&' => output.push_str("&amp;"),
            '<' => output.push_str("&lt;"),
            '>' => output.push_str("&gt;"),
            '"' => output.push_str("&quot;"),
            '\'' => output.push_str("&#39;"),
            _ => output.push(character),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn whole_range(snapshot: &TextSnapshot) -> TextRange {
        TextRange::new(ByteOffset::ZERO, snapshot.len_bytes()).expect("whole range")
    }

    #[test]
    fn clipboard_payload_keeps_source_and_exports_semantic_html() {
        let source =
            "# Yu\n\n**羽** [Rust](https://example.com)\n\n- [x] done\n\n```rust\n<&>\n```\n";
        let buffer = TextBuffer::new(source);
        let snapshot = buffer.snapshot();
        let payload = export_clipboard(&snapshot, snapshot.revision(), whole_range(&snapshot))
            .expect("clipboard export");

        assert_eq!(payload.markdown(), source);
        assert_eq!(payload.plain_text(), source);
        assert!(payload.html().contains("<h1>Yu</h1>"));
        assert!(payload.html().contains("<strong>羽</strong>"));
        assert!(
            payload
                .html()
                .contains("<a href=\"https://example.com\">Rust</a>")
        );
        assert!(
            payload
                .html()
                .contains("<input type=\"checkbox\" disabled checked>")
        );
        assert!(payload.html().contains("&lt;&amp;&gt;"));
    }

    #[test]
    fn clipboard_export_is_revision_and_utf8_boundary_bound() {
        let buffer = TextBuffer::new("羽🙂");
        let snapshot = buffer.snapshot();
        let whole = whole_range(&snapshot);
        assert!(matches!(
            export_clipboard(
                &snapshot,
                Revision::INITIAL.next().expect("next revision"),
                whole
            ),
            Err(ExportError::RevisionMismatch { .. })
        ));
        let invalid = TextRange::new(ByteOffset::new(1), ByteOffset::new(2)).expect("ordered");
        assert!(matches!(
            export_clipboard(&snapshot, snapshot.revision(), invalid),
            Err(ExportError::SourcePosition(
                TextPositionError::NotUtf8Boundary(_)
            ))
        ));
    }

    #[test]
    fn unresolved_reference_stays_visible_instead_of_creating_a_broken_link() {
        let source = "[label][missing]";
        let html = export_html_fragment(source).expect("export unresolved reference");
        assert!(html.contains("label"));
        assert!(!html.contains("href"));
        assert!(!html.contains("[missing]"));
    }

    #[test]
    fn escaped_markdown_punctuation_is_plain_html_text() {
        let html = export_html_fragment(r"\*literal\*").expect("export escaped text");
        assert_eq!(html, "<p>*literal*</p>");
    }
}
