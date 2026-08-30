use std::error::Error;
use std::fmt;

use yu_core::{ByteOffset, LineIndex, Revision, TextRange, Utf16Offset, Utf16Range};
use yu_markdown::{
    Block, BlockKind, InlineParseError, InlineSpanKind, MarkdownDocument, TaskState,
    parse_inline_with_definitions,
};
use yu_text::{TextPositionError, TextSnapshot};

use crate::{EditorDocument, EditorSelection};

/// A native UTF-16 position bound to one immutable document revision.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AccessibilityTextPosition {
    revision: Revision,
    offset: Utf16Offset,
}

impl AccessibilityTextPosition {
    #[must_use]
    pub const fn revision(self) -> Revision {
        self.revision
    }

    #[must_use]
    pub const fn offset(self) -> Utf16Offset {
        self.offset
    }
}

/// A native UTF-16 range bound to one immutable document revision.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AccessibilityTextRange {
    revision: Revision,
    range: Utf16Range,
}

impl AccessibilityTextRange {
    #[must_use]
    pub const fn revision(self) -> Revision {
        self.revision
    }

    #[must_use]
    pub const fn range(self) -> Utf16Range {
        self.range
    }
}

/// Semantic roles that can be exposed to a native accessibility tree.
///
/// Node source and label ranges are always bound to the snapshot Revision;
/// native adapters must query their text through the existing range ABI rather
/// than retaining a second Markdown string.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum AccessibilitySemanticKind {
    Document,
    Heading,
    Paragraph,
    CodeBlock,
    BlockQuote,
    ListItem,
    TaskListItem,
    Emphasis,
    Strong,
    CodeSpan,
    Link,
    Image,
    Autolink,
    ReferenceLink,
    ReferenceImage,
}

impl AccessibilitySemanticKind {
    /// Stable scalar for native Accessibility adapters.
    #[must_use]
    pub const fn tag(self) -> u8 {
        match self {
            Self::Document => 1,
            Self::Heading => 2,
            Self::Paragraph => 3,
            Self::CodeBlock => 4,
            Self::BlockQuote => 5,
            Self::ListItem => 6,
            Self::TaskListItem => 7,
            Self::Emphasis => 8,
            Self::Strong => 9,
            Self::CodeSpan => 10,
            Self::Link => 11,
            Self::Image => 12,
            Self::Autolink => 13,
            Self::ReferenceLink => 14,
            Self::ReferenceImage => 15,
        }
    }
}

/// Flags carried by [`AccessibilitySemanticNode`].
pub const ACCESSIBILITY_SEMANTIC_FLAG_ORDERED: u8 = 1 << 0;
pub const ACCESSIBILITY_SEMANTIC_FLAG_TASK_DONE: u8 = 1 << 1;

/// One source-backed semantic node in document order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AccessibilitySemanticNode {
    index: u32,
    parent: Option<u32>,
    kind: AccessibilitySemanticKind,
    flags: u8,
    level: u8,
    source_range: AccessibilityTextRange,
    label_range: AccessibilityTextRange,
    destination_range: Option<AccessibilityTextRange>,
    action_block: Option<usize>,
}

impl AccessibilitySemanticNode {
    #[must_use]
    pub const fn index(self) -> u32 {
        self.index
    }

    #[must_use]
    pub const fn parent(self) -> Option<u32> {
        self.parent
    }

    #[must_use]
    pub const fn kind(self) -> AccessibilitySemanticKind {
        self.kind
    }

    #[must_use]
    pub const fn flags(self) -> u8 {
        self.flags
    }

    #[must_use]
    pub const fn level(self) -> u8 {
        self.level
    }

    #[must_use]
    pub const fn source_range(self) -> AccessibilityTextRange {
        self.source_range
    }

    #[must_use]
    pub const fn label_range(self) -> AccessibilityTextRange {
        self.label_range
    }

    /// Returns the source-backed URL destination for links/images whose
    /// destination was resolved by the Markdown parser. Reference links use
    /// the resolved definition range in the same Revision.
    #[must_use]
    pub const fn destination_range(self) -> Option<AccessibilityTextRange> {
        self.destination_range
    }

    /// Returns the Markdown block index accepted by the editor's
    /// `toggle_task` command, when this node is an actionable task item.
    #[must_use]
    pub const fn action_block(self) -> Option<usize> {
        self.action_block
    }
}

/// Revision-bound semantic tree metadata for VoiceOver/native adapters.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AccessibilitySemanticSnapshot {
    revision: Revision,
    nodes: Vec<AccessibilitySemanticNode>,
}

impl AccessibilitySemanticSnapshot {
    /// Builds a fresh tree from the canonical Markdown blocks and inline spans.
    /// The tree owns only compact node metadata and source ranges.
    pub fn from_document(document: &EditorDocument) -> Result<Self, AccessibilityTextError> {
        let source = document.snapshot();
        let revision = source.revision();
        let full_source = TextRange::new(ByteOffset::ZERO, source.len_bytes())
            .expect("snapshot bounds must form a range");
        let mut nodes = Vec::new();
        push_semantic_node(
            &source,
            &mut nodes,
            SemanticNodeSpec {
                parent: None,
                kind: AccessibilitySemanticKind::Document,
                flags: 0,
                level: 0,
                source_range: full_source,
                label_range: full_source,
                destination_range: None,
                action_block: None,
            },
        )?;

        for (block_index, block) in document.markdown().blocks().into_iter().enumerate() {
            let Some((kind, flags, level)) = semantic_block_kind(block.kind()) else {
                continue;
            };
            let source_range = block.range();
            let label_range = semantic_block_label_range(document.markdown(), block);
            let parent = Some(0);
            let semantic_block_node_index = u32::try_from(nodes.len())
                .map_err(|_| AccessibilityTextError::SemanticNodeOverflow)?;
            push_semantic_node(
                &source,
                &mut nodes,
                SemanticNodeSpec {
                    parent,
                    kind,
                    flags,
                    level,
                    source_range,
                    label_range,
                    destination_range: None,
                    action_block: (kind == AccessibilitySemanticKind::TaskListItem)
                        .then_some(block_index),
                },
            )?;

            if matches!(kind, AccessibilitySemanticKind::CodeBlock) {
                continue;
            }
            let inline = parse_inline_with_definitions(
                &source,
                source_range,
                Some(document.markdown().reference_definitions()),
            )
            .map_err(AccessibilityTextError::SemanticParse)?;
            for span in inline.spans() {
                let Some(span_kind) = semantic_inline_kind(span.kind()) else {
                    continue;
                };
                let label_range = span.content();
                let destination_range = span.destination().or_else(|| {
                    span.reference().and_then(|reference| {
                        document
                            .markdown()
                            .reference_definitions()
                            .lookup(&source, reference)
                            .map(|definition| definition.destination())
                    })
                });
                push_semantic_node(
                    &source,
                    &mut nodes,
                    SemanticNodeSpec {
                        parent: Some(semantic_block_node_index),
                        kind: span_kind,
                        flags: 0,
                        level: 0,
                        source_range: span.source_range(),
                        label_range,
                        destination_range,
                        action_block: None,
                    },
                )?;
            }
        }

        Ok(Self { revision, nodes })
    }

    #[must_use]
    pub const fn revision(&self) -> Revision {
        self.revision
    }

    #[must_use]
    pub fn nodes(&self) -> &[AccessibilitySemanticNode] {
        &self.nodes
    }

    #[must_use]
    pub fn node(&self, index: u32) -> Option<AccessibilitySemanticNode> {
        self.nodes.get(usize::try_from(index).ok()?).copied()
    }
}

/// Synchronous text queries exposed to a platform accessibility adapter.
///
/// The adapter must create a fresh instance for the revision it is serving.
/// Geometry and visible ranges remain layout responsibilities and are not part
/// of this source-coordinate model.
#[derive(Clone, Debug)]
pub struct AccessibilityTextSnapshot {
    source: TextSnapshot,
    selection: TextRange,
    selection_utf16: Utf16Range,
}

impl AccessibilityTextSnapshot {
    /// Creates an accessibility snapshot from the document's canonical source
    /// and revision-bound selection.
    pub fn from_document(document: &EditorDocument) -> Result<Self, AccessibilityTextError> {
        let source = document.snapshot();
        let selection = document.selection();
        if selection.revision() != source.revision() {
            return Err(AccessibilityTextError::StaleRevision {
                expected: source.revision(),
                actual: selection.revision(),
            });
        }
        Self::new(source, selection.ordered_range())
    }

    /// Creates an accessibility snapshot from a typed source selection.
    pub fn from_selection(
        source: TextSnapshot,
        selection: EditorSelection,
    ) -> Result<Self, AccessibilityTextError> {
        if selection.revision() != source.revision() {
            return Err(AccessibilityTextError::StaleRevision {
                expected: source.revision(),
                actual: selection.revision(),
            });
        }
        Self::new(source, selection.ordered_range())
    }

    pub fn new(source: TextSnapshot, selection: TextRange) -> Result<Self, AccessibilityTextError> {
        let selection_utf16 = source_range_to_utf16(&source, selection)?;
        Ok(Self {
            source,
            selection,
            selection_utf16,
        })
    }

    #[must_use]
    pub fn revision(&self) -> Revision {
        self.source.revision()
    }

    #[must_use]
    pub fn number_of_characters(&self) -> Utf16Offset {
        self.source.summary().utf16_units()
    }

    #[must_use]
    pub fn full_range(&self) -> AccessibilityTextRange {
        AccessibilityTextRange {
            revision: self.revision(),
            range: Utf16Range::new(Utf16Offset::ZERO, self.number_of_characters())
                .expect("zero must not exceed the Snapshot UTF-16 length"),
        }
    }

    #[must_use]
    pub fn selected_range(&self) -> AccessibilityTextRange {
        AccessibilityTextRange {
            revision: self.revision(),
            range: self.selection_utf16,
        }
    }

    #[must_use]
    pub fn selected_source_range(&self) -> TextRange {
        self.selection
    }

    pub fn bind_position(
        &self,
        offset: Utf16Offset,
    ) -> Result<AccessibilityTextPosition, AccessibilityTextError> {
        self.source.byte_offset_for_utf16(offset)?;
        Ok(AccessibilityTextPosition {
            revision: self.revision(),
            offset,
        })
    }

    pub fn bind_range(
        &self,
        range: Utf16Range,
    ) -> Result<AccessibilityTextRange, AccessibilityTextError> {
        utf16_range_to_source(&self.source, range)?;
        Ok(AccessibilityTextRange {
            revision: self.revision(),
            range,
        })
    }

    pub fn range_for_source(
        &self,
        range: TextRange,
    ) -> Result<AccessibilityTextRange, AccessibilityTextError> {
        Ok(AccessibilityTextRange {
            revision: self.revision(),
            range: source_range_to_utf16(&self.source, range)?,
        })
    }

    pub fn source_range(
        &self,
        range: AccessibilityTextRange,
    ) -> Result<TextRange, AccessibilityTextError> {
        self.validate_revision(range.revision)?;
        utf16_range_to_source(&self.source, range.range)
    }

    pub fn text_for_range(
        &self,
        range: AccessibilityTextRange,
    ) -> Result<String, AccessibilityTextError> {
        let source_range = self.source_range(range)?;
        collect_text(&self.source, source_range)
    }

    pub fn line_for_position(
        &self,
        position: AccessibilityTextPosition,
    ) -> Result<LineIndex, AccessibilityTextError> {
        self.validate_revision(position.revision)?;
        let byte = self.source.byte_offset_for_utf16(position.offset)?;
        Ok(self.source.line_index(byte)?)
    }

    pub fn range_for_line(
        &self,
        line: LineIndex,
    ) -> Result<AccessibilityTextRange, AccessibilityTextError> {
        let start = self.source.line_start(line)?;
        let line_count = self.source.summary().line_count();
        let end = if line.get().saturating_add(1) < line_count {
            self.source
                .line_start(LineIndex::new(line.get().saturating_add(1)))?
        } else {
            self.source.len_bytes()
        };
        self.range_for_source(
            TextRange::new(start, end).expect("ordered line boundaries must form a range"),
        )
    }

    fn validate_revision(&self, actual: Revision) -> Result<(), AccessibilityTextError> {
        let expected = self.revision();
        if actual != expected {
            return Err(AccessibilityTextError::StaleRevision { expected, actual });
        }
        Ok(())
    }
}

fn semantic_block_kind(kind: BlockKind) -> Option<(AccessibilitySemanticKind, u8, u8)> {
    Some(match kind {
        BlockKind::BlankLine | BlockKind::ReferenceDefinition => return None,
        BlockKind::Paragraph => (AccessibilitySemanticKind::Paragraph, 0, 0),
        BlockKind::Heading { level } => (AccessibilitySemanticKind::Heading, 0, level),
        BlockKind::FencedCodeBlock { .. } => (AccessibilitySemanticKind::CodeBlock, 0, 0),
        BlockKind::BlockQuote { depth } => (AccessibilitySemanticKind::BlockQuote, 0, depth),
        BlockKind::ListItem { ordered, depth, .. } => (
            AccessibilitySemanticKind::ListItem,
            if ordered {
                ACCESSIBILITY_SEMANTIC_FLAG_ORDERED
            } else {
                0
            },
            depth,
        ),
        BlockKind::TaskListItem {
            ordered,
            depth,
            state,
            ..
        } => (
            AccessibilitySemanticKind::TaskListItem,
            (if ordered {
                ACCESSIBILITY_SEMANTIC_FLAG_ORDERED
            } else {
                0
            }) | if state == TaskState::Done {
                ACCESSIBILITY_SEMANTIC_FLAG_TASK_DONE
            } else {
                0
            },
            depth,
        ),
    })
}

fn semantic_inline_kind(kind: InlineSpanKind) -> Option<AccessibilitySemanticKind> {
    Some(match kind {
        InlineSpanKind::Emphasis => AccessibilitySemanticKind::Emphasis,
        InlineSpanKind::Strong => AccessibilitySemanticKind::Strong,
        InlineSpanKind::CodeSpan => AccessibilitySemanticKind::CodeSpan,
        InlineSpanKind::Link => AccessibilitySemanticKind::Link,
        InlineSpanKind::Image => AccessibilitySemanticKind::Image,
        InlineSpanKind::ReferenceLink => AccessibilitySemanticKind::ReferenceLink,
        InlineSpanKind::ReferenceImage => AccessibilitySemanticKind::ReferenceImage,
        InlineSpanKind::Autolink => AccessibilitySemanticKind::Autolink,
    })
}

struct SemanticNodeSpec {
    parent: Option<u32>,
    kind: AccessibilitySemanticKind,
    flags: u8,
    level: u8,
    source_range: TextRange,
    label_range: TextRange,
    destination_range: Option<TextRange>,
    action_block: Option<usize>,
}

fn push_semantic_node(
    source: &TextSnapshot,
    nodes: &mut Vec<AccessibilitySemanticNode>,
    spec: SemanticNodeSpec,
) -> Result<(), AccessibilityTextError> {
    let index =
        u32::try_from(nodes.len()).map_err(|_| AccessibilityTextError::SemanticNodeOverflow)?;
    nodes.push(AccessibilitySemanticNode {
        index,
        parent: spec.parent,
        kind: spec.kind,
        flags: spec.flags,
        level: spec.level,
        source_range: AccessibilityTextRange {
            revision: source.revision(),
            range: source_range_to_utf16(source, spec.source_range)?,
        },
        label_range: AccessibilityTextRange {
            revision: source.revision(),
            range: source_range_to_utf16(source, spec.label_range)?,
        },
        destination_range: spec
            .destination_range
            .map(|range| {
                source_range_to_utf16(source, range).map(|range| AccessibilityTextRange {
                    revision: source.revision(),
                    range,
                })
            })
            .transpose()?,
        action_block: spec.action_block,
    });
    Ok(())
}

/// 语义块的标签区间。
///
/// 标题只报正文，`#` 前缀、收尾的 ` ##`、Setext 的下划线都不进 VoiceOver 的
/// 朗读——那是语法，不是标题的内容。哪一段是正文由 `yu-markdown` 说（它才
/// 认识 Markdown 语法）；此前这里自己扫了一遍 `#`，于是 Setext 标题会带着
/// 一行 `===` 被读出来。
fn semantic_block_label_range(markdown: &MarkdownDocument, block: Block) -> TextRange {
    yu_markdown::heading_content_range(markdown, block)
}

fn source_range_to_utf16(
    source: &TextSnapshot,
    range: TextRange,
) -> Result<Utf16Range, AccessibilityTextError> {
    let start = source.utf16_offset(range.start())?;
    let end = source.utf16_offset(range.end())?;
    Utf16Range::new(start, end).ok_or(AccessibilityTextError::InvalidSourceRange(range))
}

fn utf16_range_to_source(
    source: &TextSnapshot,
    range: Utf16Range,
) -> Result<TextRange, AccessibilityTextError> {
    let start = source.byte_offset_for_utf16(range.start())?;
    let end = source.byte_offset_for_utf16(range.end())?;
    TextRange::new(start, end).ok_or(AccessibilityTextError::InvalidUtf16Range(range))
}

fn collect_text(source: &TextSnapshot, range: TextRange) -> Result<String, AccessibilityTextError> {
    let start =
        usize::try_from(range.start()).map_err(|_| AccessibilityTextError::OffsetOverflow)?;
    let end = usize::try_from(range.end()).map_err(|_| AccessibilityTextError::OffsetOverflow)?;
    let capacity = end.saturating_sub(start);
    let mut result = String::with_capacity(capacity);

    for chunk in source.chunk_cursor(range.start())? {
        let chunk_start =
            usize::try_from(chunk.start()).map_err(|_| AccessibilityTextError::OffsetOverflow)?;
        if chunk_start >= end {
            break;
        }
        let chunk_end = chunk_start.saturating_add(chunk.text().len());
        let local_start = start.max(chunk_start).saturating_sub(chunk_start);
        let local_end = end.min(chunk_end).saturating_sub(chunk_start);
        if local_start < local_end {
            result.push_str(&chunk.text()[local_start..local_end]);
        }
    }
    Ok(result)
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AccessibilityTextError {
    StaleRevision {
        expected: Revision,
        actual: Revision,
    },
    InvalidSourceRange(TextRange),
    InvalidUtf16Range(Utf16Range),
    Position(TextPositionError),
    OffsetOverflow,
    SemanticNodeOverflow,
    SemanticParse(InlineParseError),
}

impl fmt::Display for AccessibilityTextError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::StaleRevision { expected, actual } => write!(
                formatter,
                "accessibility query revision {actual:?} does not match {expected:?}"
            ),
            Self::InvalidSourceRange(range) => {
                write!(formatter, "invalid accessibility source range {range:?}")
            }
            Self::InvalidUtf16Range(range) => {
                write!(formatter, "invalid accessibility UTF-16 range {range:?}")
            }
            Self::Position(error) => error.fmt(formatter),
            Self::OffsetOverflow => formatter.write_str("accessibility offset overflow"),
            Self::SemanticNodeOverflow => {
                formatter.write_str("accessibility semantic node overflow")
            }
            Self::SemanticParse(error) => {
                write!(formatter, "accessibility semantic parse failed: {error}")
            }
        }
    }
}

impl Error for AccessibilityTextError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Position(error) => Some(error),
            Self::SemanticParse(error) => Some(error),
            Self::StaleRevision { .. }
            | Self::InvalidSourceRange(_)
            | Self::InvalidUtf16Range(_)
            | Self::OffsetOverflow
            | Self::SemanticNodeOverflow => None,
        }
    }
}

impl From<TextPositionError> for AccessibilityTextError {
    fn from(error: TextPositionError) -> Self {
        Self::Position(error)
    }
}

impl From<InlineParseError> for AccessibilityTextError {
    fn from(error: InlineParseError) -> Self {
        Self::SemanticParse(error)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{CaretAffinity, EditorCommand};
    use yu_core::ByteOffset;
    use yu_text::{Edit, TextBuffer, Transaction, retained_snapshot_stats};

    fn source_range(start: u64, end: u64) -> TextRange {
        TextRange::new(ByteOffset::new(start), ByteOffset::new(end))
            .expect("test source range must be ordered")
    }

    fn utf16_range(start: u64, end: u64) -> Utf16Range {
        Utf16Range::new(Utf16Offset::new(start), Utf16Offset::new(end))
            .expect("test UTF-16 range must be ordered")
    }

    #[test]
    fn selection_and_text_queries_bridge_utf8_and_utf16() {
        let buffer = TextBuffer::new("a😊\n羽\r\nlast");
        let accessibility = AccessibilityTextSnapshot::new(buffer.snapshot(), source_range(1, 5))
            .expect("emoji selection should be valid");

        assert_eq!(accessibility.number_of_characters(), Utf16Offset::new(11));
        assert_eq!(accessibility.selected_range().range(), utf16_range(1, 3));
        assert_eq!(
            accessibility
                .text_for_range(accessibility.selected_range())
                .expect("selected text query should succeed"),
            "😊"
        );
        assert_eq!(
            accessibility
                .source_range(accessibility.selected_range())
                .expect("selected source range should resolve"),
            source_range(1, 5)
        );
    }

    #[test]
    fn document_accessibility_snapshot_uses_the_canonical_selection_revision() {
        let mut document = EditorDocument::new("a😊b");
        let selection = EditorSelection::range(
            &document.snapshot(),
            ByteOffset::new(1),
            ByteOffset::new(5),
            CaretAffinity::Downstream,
        )
        .expect("emoji selection should be valid");
        document
            .set_selection(selection)
            .expect("selection should belong to document");

        let accessibility = AccessibilityTextSnapshot::from_document(&document)
            .expect("document selection should be exposed");
        assert_eq!(accessibility.revision(), document.revision());
        assert_eq!(accessibility.selected_range().range(), utf16_range(1, 3));
        assert_eq!(
            accessibility
                .text_for_range(accessibility.selected_range())
                .expect("selected text query should succeed"),
            "😊"
        );

        document
            .execute(EditorCommand::insert_text("羽"))
            .expect("command should advance the document");
        assert_eq!(accessibility.revision().get(), 0);
    }

    #[test]
    fn typed_selection_from_an_old_revision_is_rejected() {
        let source = TextBuffer::new("old").snapshot();
        let selection =
            EditorSelection::cursor(&source, ByteOffset::new(3), CaretAffinity::Downstream)
                .expect("caret should be valid");
        let mut next = TextBuffer::new("old");
        next.apply(&Transaction::new(
            next.revision(),
            [Edit::new(source_range(0, 3), "new")],
        ))
        .expect("replacement should advance the revision");

        assert!(matches!(
            AccessibilityTextSnapshot::from_selection(next.snapshot(), selection),
            Err(AccessibilityTextError::StaleRevision { .. })
        ));
    }

    #[test]
    fn logical_line_queries_include_the_terminating_lf() {
        let buffer = TextBuffer::new("a😊\n羽\r\nlast");
        let accessibility = AccessibilityTextSnapshot::new(buffer.snapshot(), source_range(0, 0))
            .expect("empty selection should be valid");

        let first = accessibility
            .range_for_line(LineIndex::ZERO)
            .expect("first line should exist");
        let second = accessibility
            .range_for_line(LineIndex::new(1))
            .expect("second line should exist");
        let last = accessibility
            .range_for_line(LineIndex::new(2))
            .expect("last line should exist");

        assert_eq!(first.range(), utf16_range(0, 4));
        assert_eq!(second.range(), utf16_range(4, 7));
        assert_eq!(last.range(), utf16_range(7, 11));
        assert_eq!(
            accessibility
                .line_for_position(
                    accessibility
                        .bind_position(Utf16Offset::new(5))
                        .expect("line position should bind")
                )
                .expect("line query should succeed"),
            LineIndex::new(1)
        );
    }

    #[test]
    fn stale_ranges_are_rejected_after_the_document_changes() {
        let mut buffer = TextBuffer::new("old");
        let old = AccessibilityTextSnapshot::new(buffer.snapshot(), source_range(0, 0))
            .expect("old snapshot should be valid");
        let old_range = old.full_range();
        let transaction =
            Transaction::new(buffer.revision(), [Edit::new(source_range(0, 3), "new")]);
        buffer
            .apply(&transaction)
            .expect("replacement should apply");
        let new = AccessibilityTextSnapshot::new(buffer.snapshot(), source_range(0, 0))
            .expect("new snapshot should be valid");

        assert!(matches!(
            new.text_for_range(old_range),
            Err(AccessibilityTextError::StaleRevision { .. })
        ));
    }

    #[test]
    fn native_range_cannot_split_a_surrogate_pair() {
        let buffer = TextBuffer::new("😊");
        let accessibility = AccessibilityTextSnapshot::new(buffer.snapshot(), source_range(0, 0))
            .expect("empty selection should be valid");

        assert!(matches!(
            accessibility.bind_range(utf16_range(1, 1)),
            Err(AccessibilityTextError::Position(
                TextPositionError::Utf16InsideScalar(_)
            ))
        ));
    }

    #[test]
    fn text_query_does_not_materialize_a_snapshot() {
        let mut buffer = TextBuffer::new("alpha");
        let transaction = Transaction::new(
            buffer.revision(),
            [Edit::new(source_range(5, 5), "😊\nomega")],
        );
        buffer.apply(&transaction).expect("append should apply");
        let snapshot = buffer.snapshot();
        let accessibility = AccessibilityTextSnapshot::new(snapshot.clone(), source_range(0, 0))
            .expect("snapshot should be valid");
        let materialized_before =
            retained_snapshot_stats(std::slice::from_ref(&snapshot)).materialized_buffers();
        let range = accessibility
            .bind_range(utf16_range(5, 8))
            .expect("cross-piece range should bind");

        assert_eq!(
            accessibility
                .text_for_range(range)
                .expect("cross-piece query should succeed"),
            "😊\n"
        );
        assert_eq!(
            retained_snapshot_stats(&[snapshot]).materialized_buffers(),
            materialized_before
        );
    }

    /// Setext 标题报成标题，标签只有正文——`===` 那一行不朗读。
    ///
    /// 块的身份由语法树给之前，`标题\n===` 在块序列里是一个普通段落，
    /// VoiceOver 那边既不是标题，标签也带着下划线。
    #[test]
    fn a_setext_heading_reads_as_a_heading_without_its_underline() {
        let document = EditorDocument::new("Setext 二级\n---\n\n段落\n");
        let semantic =
            AccessibilitySemanticSnapshot::from_document(&document).expect("语义树该建得起来");
        let text = AccessibilityTextSnapshot::from_document(&document).expect("文本快照该建得起来");
        let heading = semantic
            .nodes()
            .iter()
            .find(|node| node.kind() == AccessibilitySemanticKind::Heading)
            .copied()
            .expect("Setext 也是标题");
        assert_eq!(heading.level(), 2);
        assert_eq!(
            text.text_for_range(heading.label_range())
                .expect("标题的标签"),
            "Setext 二级"
        );
    }

    #[test]
    fn semantic_snapshot_exposes_revision_bound_block_and_inline_nodes() {
        let mut document = EditorDocument::new(
            "# 标题\n\n段落 **粗体** [链接](https://example.com) [参考][rust]\n\n- [x] 完成\n\n[rust]: https://www.rust-lang.org/\n\n```rust\n<&>\n```\n",
        );
        let semantic = AccessibilitySemanticSnapshot::from_document(&document)
            .expect("semantic tree should build");
        assert_eq!(semantic.revision(), document.revision());
        assert_eq!(
            semantic.node(0).expect("document root").kind(),
            AccessibilitySemanticKind::Document
        );
        assert_eq!(semantic.node(0).expect("document root").parent(), None);

        let text = AccessibilityTextSnapshot::from_document(&document)
            .expect("text accessibility snapshot should build");
        let heading = semantic
            .nodes()
            .iter()
            .find(|node| node.kind() == AccessibilitySemanticKind::Heading)
            .copied()
            .expect("heading node");
        assert_eq!(
            text.text_for_range(heading.label_range())
                .expect("heading label text"),
            "标题"
        );
        assert!(
            semantic
                .nodes()
                .iter()
                .any(|node| node.kind() == AccessibilitySemanticKind::Strong)
        );
        assert!(
            semantic
                .nodes()
                .iter()
                .any(|node| node.kind() == AccessibilitySemanticKind::Link)
        );
        let link = semantic
            .nodes()
            .iter()
            .find(|node| node.kind() == AccessibilitySemanticKind::Link)
            .copied()
            .expect("link node");
        assert_eq!(
            text.text_for_range(link.destination_range().expect("link destination range"))
                .expect("link destination text"),
            "https://example.com"
        );
        let reference_link = semantic
            .nodes()
            .iter()
            .find(|node| node.kind() == AccessibilitySemanticKind::ReferenceLink)
            .copied()
            .expect("reference link node");
        assert_eq!(
            text.text_for_range(
                reference_link
                    .destination_range()
                    .expect("reference destination range")
            )
            .expect("reference destination text"),
            "https://www.rust-lang.org/"
        );
        let task = semantic
            .nodes()
            .iter()
            .find(|node| node.kind() == AccessibilitySemanticKind::TaskListItem)
            .copied()
            .expect("task node");
        assert_ne!(task.flags() & ACCESSIBILITY_SEMANTIC_FLAG_TASK_DONE, 0);
        assert_eq!(task.parent(), Some(0));
        let task_block = task.action_block().expect("task action block");
        assert!(document.command_available(&EditorCommand::toggle_task(task_block)));
        assert!(
            semantic
                .nodes()
                .iter()
                .any(|node| node.kind() == AccessibilitySemanticKind::CodeBlock)
        );

        document
            .execute(EditorCommand::insert_text("!"))
            .expect("edit should advance the revision");
        let next = AccessibilitySemanticSnapshot::from_document(&document)
            .expect("new semantic tree should build");
        assert_ne!(semantic.revision(), next.revision());
        assert_eq!(
            semantic
                .node(0)
                .expect("old root")
                .source_range()
                .revision()
                .get(),
            0
        );
        assert_eq!(
            next.node(0)
                .expect("new root")
                .source_range()
                .revision()
                .get(),
            1
        );
    }
}
