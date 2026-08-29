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
    ReferenceDefinitionIndex, TableAlignment, TableBlock, TaskState, parse,
    parse_inline_with_definitions, parse_table,
};
use yu_text::{TextBuffer, TextPositionError, TextSnapshot};

mod html_import;

pub use html_import::{HtmlImportError, import_html_fragment};

/// Pasteboard MIME/UTI names shared by native adapters.
pub const MARKDOWN_MIME: &str = "text/markdown";
pub const MARKDOWN_UTI: &str = "net.daringfireball.markdown";
pub const PLAIN_TEXT_MIME: &str = "text/plain;charset=utf-8";
pub const PLAIN_TEXT_UTI: &str = "public.utf8-plain-text";
pub const HTML_MIME: &str = "text/html";
pub const HTML_UTI: &str = "public.html";

/// Stable source formats that every native clipboard adapter must understand.
///
/// The MIME name is used by Windows/Linux/web-facing adapters; `uti()` is the
/// corresponding macOS pasteboard identifier. The payload order is deliberate:
/// Markdown is canonical, plain text is the lossless fallback, and HTML is a
/// derived semantic fragment.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ClipboardFormat {
    Markdown,
    PlainText,
    Html,
}

impl ClipboardFormat {
    /// Formats published for every canonical source selection.
    pub const ALL: [Self; 3] = [Self::Markdown, Self::PlainText, Self::Html];

    #[must_use]
    pub const fn mime(self) -> &'static str {
        match self {
            Self::Markdown => MARKDOWN_MIME,
            Self::PlainText => PLAIN_TEXT_MIME,
            Self::Html => HTML_MIME,
        }
    }

    #[must_use]
    pub const fn uti(self) -> &'static str {
        match self {
            Self::Markdown => MARKDOWN_UTI,
            Self::PlainText => PLAIN_TEXT_UTI,
            Self::Html => HTML_UTI,
        }
    }
}

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

    /// Returns the payload value for a stable native clipboard format.
    #[must_use]
    pub fn value(&self, format: ClipboardFormat) -> &str {
        match format {
            ClipboardFormat::Markdown => self.markdown(),
            ClipboardFormat::PlainText => self.plain_text(),
            ClipboardFormat::Html => self.html(),
        }
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
    let mut blocks = blocks.into_iter().peekable();
    let mut first = true;
    while let Some(block) = blocks.next() {
        let fragment = if list_signature(block.kind()).is_some() {
            let mut run = vec![block];
            while let Some(next) = blocks.peek().copied() {
                if list_signature(next.kind()).is_some() {
                    run.push(blocks.next().expect("peeked list block must be available"));
                } else {
                    break;
                }
            }
            render_list_run(snapshot, &run)?
        } else {
            render_block(snapshot, definitions, block)?
        };
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

fn list_signature(kind: BlockKind) -> Option<(bool, u8)> {
    match kind {
        BlockKind::ListItem { ordered, depth, .. }
        | BlockKind::TaskListItem { ordered, depth, .. } => Some((ordered, depth)),
        _ => None,
    }
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
            if let Some(table) = parse_table(source) {
                return render_table(source, &table);
            }
            let mut html = String::from("<p>");
            render_inline(snapshot, definitions, block.range(), &mut html)?;
            html.push_str("</p>");
            Ok(html)
        }
        BlockKind::Heading { level } => {
            let content = yu_markdown::heading_content_range(snapshot, block);
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
        BlockKind::ListItem { .. } | BlockKind::TaskListItem { .. } => {
            render_list_run(snapshot, &[block])
        }
    }
}

fn render_table(source: &str, table: &TableBlock) -> Result<String, ExportError> {
    let mut html = String::from("<table><thead><tr>");
    for (cell, alignment) in table.header().iter().zip(table.alignments()) {
        html.push_str("<th");
        html.push_str(table_alignment_attribute(*alignment));
        html.push('>');
        render_table_cell(source, *cell, &mut html)?;
        html.push_str("</th>");
    }
    html.push_str("</tr></thead>");
    if !table.rows().is_empty() {
        html.push_str("<tbody>");
        for row in table.rows() {
            html.push_str("<tr>");
            for (cell, alignment) in row.iter().zip(table.alignments()) {
                html.push_str("<td");
                html.push_str(table_alignment_attribute(*alignment));
                html.push('>');
                render_table_cell(source, *cell, &mut html)?;
                html.push_str("</td>");
            }
            html.push_str("</tr>");
        }
        html.push_str("</tbody>");
    }
    html.push_str("</table>");
    Ok(html)
}

fn render_table_cell(
    source: &str,
    cell: yu_markdown::TableCellRange,
    output: &mut String,
) -> Result<(), ExportError> {
    let value = &source[cell.start()..cell.end()];
    let inner = export_html_fragment(value)?;
    append_tight_fragment_content(&inner, output);
    Ok(())
}

fn table_alignment_attribute(alignment: TableAlignment) -> &'static str {
    match alignment {
        TableAlignment::Default => "",
        TableAlignment::Left => " style=\"text-align: left\"",
        TableAlignment::Center => " style=\"text-align: center\"",
        TableAlignment::Right => " style=\"text-align: right\"",
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

fn render_list_run(snapshot: &TextSnapshot, blocks: &[Block]) -> Result<String, ExportError> {
    let mut cursor = 0;
    let mut output = String::new();
    while cursor < blocks.len() {
        let (ordered, depth) =
            list_signature(blocks[cursor].kind()).expect("list run contains only list blocks");
        if !output.is_empty() {
            output.push('\n');
        }
        output.push_str(&render_list_level(
            snapshot,
            blocks,
            &mut cursor,
            depth,
            ordered,
        )?);
    }
    Ok(output)
}

fn render_list_level(
    snapshot: &TextSnapshot,
    blocks: &[Block],
    cursor: &mut usize,
    depth: u8,
    ordered: bool,
) -> Result<String, ExportError> {
    let start = list_start(blocks[*cursor].kind());
    let mut html = if ordered {
        if start > 1 {
            format!("<ol start=\"{start}\">")
        } else {
            String::from("<ol>")
        }
    } else {
        String::from("<ul>")
    };

    while *cursor < blocks.len() {
        let Some((item_ordered, item_depth)) = list_signature(blocks[*cursor].kind()) else {
            break;
        };
        if item_depth != depth || item_ordered != ordered {
            break;
        }
        let block = blocks[*cursor];
        *cursor += 1;
        html.push_str("<li>");
        let source = slice(snapshot, block.range())?;
        let (text, task) = list_item_content(source, block);
        if let Some(state) = task {
            html.push_str("<input type=\"checkbox\" disabled");
            if state == TaskState::Done {
                html.push_str(" checked");
            }
            html.push_str("> ");
        }
        let inner = export_html_fragment(&text)?;
        append_tight_fragment_content(&inner, &mut html);

        while *cursor < blocks.len() {
            let Some((child_ordered, child_depth)) = list_signature(blocks[*cursor].kind()) else {
                break;
            };
            if child_depth <= depth {
                break;
            }
            html.push('\n');
            html.push_str(&render_list_level(
                snapshot,
                blocks,
                cursor,
                child_depth,
                child_ordered,
            )?);
        }
        html.push_str("</li>");
    }

    html.push_str(if ordered { "</ol>" } else { "</ul>" });
    Ok(html)
}

fn list_start(kind: BlockKind) -> u32 {
    match kind {
        BlockKind::ListItem { start, .. } | BlockKind::TaskListItem { start, .. } => start,
        _ => 1,
    }
}

fn list_item_content(source: &str, block: Block) -> (String, Option<TaskState>) {
    let first_line_end = source.find('\n').unwrap_or(source.len());
    let prefix_end = list_prefix_end(&source[..first_line_end]);
    let mut text = source[prefix_end..].to_owned();
    if text.ends_with("\r\n") {
        text.truncate(text.len().saturating_sub(2));
    } else if text.ends_with(['\n', '\r']) {
        text.truncate(text.len().saturating_sub(1));
    }
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
    (text, task)
}

fn append_tight_fragment_content(inner: &str, output: &mut String) {
    if let Some(content) = inner
        .strip_prefix("<p>")
        .and_then(|value| value.strip_suffix("</p>"))
    {
        output.push_str(content);
    } else {
        output.push_str(inner);
    }
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

    /// Setext 标题导出成 `<h1>` / `<h2>`，下划线那一行不进正文。
    ///
    /// 块的身份由语法树给（`yu-markdown::classify`）之前，`标题\n===` 在块序列
    /// 里是一个普通段落，这里导出的是 `<p>标题\n===</p>`。
    #[test]
    fn setext_headings_export_as_headings_without_their_underline() {
        for (source, expected) in [
            ("Setext 一级\n===\n", "<h1>Setext 一级</h1>"),
            ("Setext 二级\n---\n", "<h2>Setext 二级</h2>"),
            ("# ATX\n", "<h1>ATX</h1>"),
        ] {
            let html = export_html_fragment(source).expect("导出不该失败");
            assert_eq!(html, expected, "source {source:?}");
        }
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
    fn clipboard_format_contract_maps_mime_uti_and_payloads() {
        assert_eq!(
            ClipboardFormat::ALL,
            [
                ClipboardFormat::Markdown,
                ClipboardFormat::PlainText,
                ClipboardFormat::Html
            ]
        );
        assert_eq!(ClipboardFormat::Markdown.mime(), MARKDOWN_MIME);
        assert_eq!(ClipboardFormat::Markdown.uti(), MARKDOWN_UTI);
        assert_eq!(ClipboardFormat::PlainText.mime(), PLAIN_TEXT_MIME);
        assert_eq!(ClipboardFormat::PlainText.uti(), PLAIN_TEXT_UTI);
        assert_eq!(ClipboardFormat::Html.mime(), HTML_MIME);
        assert_eq!(ClipboardFormat::Html.uti(), HTML_UTI);

        let buffer = TextBuffer::new("# Yu");
        let snapshot = buffer.snapshot();
        let payload = export_clipboard(&snapshot, snapshot.revision(), whole_range(&snapshot))
            .expect("clipboard export");
        assert_eq!(payload.value(ClipboardFormat::Markdown), "# Yu");
        assert_eq!(payload.value(ClipboardFormat::PlainText), "# Yu");
        assert_eq!(payload.value(ClipboardFormat::Html), "<h1>Yu</h1>");
    }

    #[test]
    fn consecutive_lists_share_containers_and_preserve_ordered_start() {
        let source = "- one\n- **two**\n\n3. three\n4. four\n";
        let html = export_html_fragment(source).expect("export list fragment");

        assert_eq!(
            html,
            "<ul><li>one</li><li><strong>two</strong></li></ul>\n<ol start=\"3\"><li>three</li><li>four</li></ol>"
        );
    }

    #[test]
    fn nested_lists_follow_source_depth_inside_the_parent_item() {
        let source = "- parent\n  - child\n  - **second**\n- sibling\n";
        let html = export_html_fragment(source).expect("export nested list fragment");

        assert_eq!(
            html,
            "<ul><li>parent\n<ul><li>child</li><li><strong>second</strong></li></ul></li><li>sibling</li></ul>"
        );
    }

    #[test]
    fn gfm_tables_export_header_body_and_alignment_semantics() {
        let source = "| Name | Count | Note |\n| :--- | ---: | :---: |\n| **Yu** | 2 | `a|b` |\n";
        let html = export_html_fragment(source).expect("export table fragment");

        assert_eq!(
            html,
            "<table><thead><tr><th style=\"text-align: left\">Name</th><th style=\"text-align: right\">Count</th><th style=\"text-align: center\">Note</th></tr></thead><tbody><tr><td style=\"text-align: left\"><strong>Yu</strong></td><td style=\"text-align: right\">2</td><td style=\"text-align: center\"><code>a|b</code></td></tr></tbody></table>"
        );
    }

    #[test]
    fn exported_html_round_trips_through_strict_import_policy() {
        let source = "# Yu\n\n- [x] done\n\n| A | B |\n| :--- | ---: |\n| 1 | 2 |";
        let html = export_html_fragment(source).expect("export source fragment");

        assert_eq!(
            import_html_fragment(&html).expect("Yu HTML should be accepted by its importer"),
            source
        );
    }

    #[test]
    fn pipe_text_without_a_valid_delimiter_remains_a_paragraph() {
        let html = export_html_fragment("a | b\nc | d\n").expect("export paragraph fragment");
        assert_eq!(html, "<p>a | b\nc | d\n</p>");
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
