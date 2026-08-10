#![forbid(unsafe_code)]

//! Source-to-visual projection primitives.
//!
//! This phase intentionally implements only a small, lossless inline
//! projection. Matched Markdown emphasis, strong-emphasis, and code-span
//! delimiters from `yu-markdown::InlineDocument` become zero-width visual
//! runs; all other source bytes remain visible and continue to point into the
//! canonical TextSnapshot.

use std::error::Error;
use std::fmt;
use yu_core::{Affinity, ByteOffset, Revision, TextAnchor, TextRange};
use yu_markdown::{
    InlineDelimiter, InlineDocument, InlineNodeKind, InlineParseError, parse_inline,
};
use yu_text::{AnchorMapError, ChangeSet, TextChange, TextPositionError, TextSnapshot};

/// An offset in the projected UTF-8 visual stream.
///
/// It is not a source byte offset. A visual offset is only meaningful for the
/// projection revision and range that produced it.
#[derive(Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct VisualOffset(u64);

impl VisualOffset {
    pub const ZERO: Self = Self(0);

    #[must_use]
    pub const fn new(value: u64) -> Self {
        Self(value)
    }

    #[must_use]
    pub const fn get(self) -> u64 {
        self.0
    }

    #[must_use]
    pub const fn checked_add(self, bytes: u64) -> Option<Self> {
        match self.0.checked_add(bytes) {
            Some(value) => Some(Self(value)),
            None => None,
        }
    }
}

impl fmt::Debug for VisualOffset {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "VisualOffset({})", self.0)
    }
}

impl TryFrom<usize> for VisualOffset {
    type Error = std::num::TryFromIntError;

    fn try_from(value: usize) -> Result<Self, Self::Error> {
        Ok(Self(value.try_into()?))
    }
}

/// A half-open range in projected UTF-8 visual bytes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct VisualRange {
    start: VisualOffset,
    end: VisualOffset,
}

impl VisualRange {
    #[must_use]
    pub const fn new(start: VisualOffset, end: VisualOffset) -> Option<Self> {
        if start.get() <= end.get() {
            Some(Self { start, end })
        } else {
            None
        }
    }

    #[must_use]
    pub const fn empty(at: VisualOffset) -> Self {
        Self { start: at, end: at }
    }

    #[must_use]
    pub const fn start(self) -> VisualOffset {
        self.start
    }

    #[must_use]
    pub const fn end(self) -> VisualOffset {
        self.end
    }

    #[must_use]
    pub const fn len(self) -> u64 {
        self.end.get() - self.start.get()
    }

    #[must_use]
    pub const fn is_empty(self) -> bool {
        self.start.get() == self.end.get()
    }
}

/// Which source spans contribute to a projected visual run.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum VisualRunKind {
    /// Source bytes remain visible and map linearly to visual bytes.
    Visible,
    /// Source syntax remains in the canonical source but occupies no visual bytes.
    HiddenSyntax,
}

/// A source-backed visual run.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct VisualRun {
    source: TextRange,
    visual: VisualRange,
    kind: VisualRunKind,
}

impl VisualRun {
    #[must_use]
    pub const fn source(self) -> TextRange {
        self.source
    }

    #[must_use]
    pub const fn visual(self) -> VisualRange {
        self.visual
    }

    #[must_use]
    pub const fn kind(self) -> VisualRunKind {
        self.kind
    }
}

/// Bias for resolving a visual caret at a hidden syntax boundary.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum ProjectionBias {
    /// Resolve to the source position before hidden syntax.
    Before,
    /// Resolve to the source position after hidden syntax.
    #[default]
    After,
}

/// Errors raised while creating or querying a projection.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ProjectionError {
    SourcePosition(TextPositionError),
    InlineParse(InlineParseError),
    AnchorMap(AnchorMapError),
    BeforeRevisionMismatch {
        expected: Revision,
        actual: Revision,
    },
    AfterRevisionMismatch {
        expected: Revision,
        actual: Revision,
    },
    SourceOutsideRange {
        offset: ByteOffset,
        range: TextRange,
    },
    VisualOutOfBounds {
        offset: VisualOffset,
        len: VisualOffset,
    },
    OffsetOverflow,
}

impl fmt::Display for ProjectionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::SourcePosition(error) => error.fmt(formatter),
            Self::InlineParse(error) => error.fmt(formatter),
            Self::AnchorMap(error) => error.fmt(formatter),
            Self::BeforeRevisionMismatch { expected, actual } => write!(
                formatter,
                "projection revision {actual:?} does not match change-set input {expected:?}"
            ),
            Self::AfterRevisionMismatch { expected, actual } => write!(
                formatter,
                "projection target revision {expected:?} does not match snapshot {actual:?}"
            ),
            Self::SourceOutsideRange { offset, range } => {
                write!(
                    formatter,
                    "source offset {offset:?} is outside projection {range:?}"
                )
            }
            Self::VisualOutOfBounds { offset, len } => {
                write!(
                    formatter,
                    "visual offset {offset:?} is outside length {len:?}"
                )
            }
            Self::OffsetOverflow => formatter.write_str("projection offset overflow"),
        }
    }
}

impl Error for ProjectionError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::SourcePosition(error) => Some(error),
            Self::InlineParse(error) => Some(error),
            Self::AnchorMap(error) => Some(error),
            Self::SourceOutsideRange { .. }
            | Self::BeforeRevisionMismatch { .. }
            | Self::AfterRevisionMismatch { .. }
            | Self::VisualOutOfBounds { .. }
            | Self::OffsetOverflow => None,
        }
    }
}

impl From<TextPositionError> for ProjectionError {
    fn from(error: TextPositionError) -> Self {
        Self::SourcePosition(error)
    }
}

impl From<InlineParseError> for ProjectionError {
    fn from(error: InlineParseError) -> Self {
        Self::InlineParse(error)
    }
}

impl From<AnchorMapError> for ProjectionError {
    fn from(error: AnchorMapError) -> Self {
        Self::AnchorMap(error)
    }
}

/// A source-backed inline projection for one immutable revision and range.
#[derive(Clone, Debug)]
pub struct Projection {
    source: TextSnapshot,
    source_range: TextRange,
    runs: Vec<VisualRun>,
    visual_len: VisualOffset,
}

impl Projection {
    /// Builds a minimal Markdown inline projection.
    ///
    /// Matched emphasis, strong-emphasis, and backtick delimiters are hidden.
    /// The parser is deliberately conservative: unmatched or escaped
    /// delimiters remain visible, and source bytes are never rewritten.
    pub fn inline(source: &TextSnapshot, source_range: TextRange) -> Result<Self, ProjectionError> {
        let inline = parse_inline(source, source_range)?;
        Self::from_inline(&inline)
    }

    /// Builds a projection from the parser-owned lossless inline token stream.
    ///
    /// Keeping this constructor separate makes the source of delimiter ranges
    /// explicit: the projection never rescans or owns a second inline syntax
    /// representation.
    pub fn from_inline(inline: &InlineDocument) -> Result<Self, ProjectionError> {
        let source = inline.source();
        let source_range = inline.source_range();
        source.utf16_offset(source_range.start())?;
        source.utf16_offset(source_range.end())?;
        let hidden = find_hidden_ranges(inline)?;
        let runs = build_runs(source_range, &hidden)?;
        let visual_len = runs
            .last()
            .map_or(VisualOffset::ZERO, |run| run.visual.end());
        Ok(Self {
            source: source.clone(),
            source_range,
            runs,
            visual_len,
        })
    }

    /// Carries an unchanged projection through an edit that is strictly
    /// outside its source range.
    ///
    /// `None` means that an edit touched the range or one of its boundaries;
    /// the caller must parse a fresh inline token stream in that case. Runs
    /// retain their visual offsets while source ranges are mapped through the
    /// change set, so a prefix insertion does not force a reparse.
    pub fn map_through(
        &self,
        changes: &ChangeSet,
        snapshot: &TextSnapshot,
    ) -> Result<Option<Self>, ProjectionError> {
        if self.revision() != changes.before() {
            return Err(ProjectionError::BeforeRevisionMismatch {
                expected: changes.before(),
                actual: self.revision(),
            });
        }
        if snapshot.revision() != changes.after() {
            return Err(ProjectionError::AfterRevisionMismatch {
                expected: changes.after(),
                actual: snapshot.revision(),
            });
        }
        if changes
            .changes()
            .iter()
            .any(|change| change_affects_range(*change, self.source_range))
        {
            return Ok(None);
        }

        let source_range = map_range(self.source_range, changes)?;
        let mut runs = Vec::with_capacity(self.runs.len());
        for run in &self.runs {
            runs.push(VisualRun {
                source: map_range(run.source, changes)?,
                visual: run.visual,
                kind: run.kind,
            });
        }
        snapshot.utf16_offset(source_range.start())?;
        snapshot.utf16_offset(source_range.end())?;
        Ok(Some(Self {
            source: snapshot.clone(),
            source_range,
            runs,
            visual_len: self.visual_len,
        }))
    }

    #[must_use]
    pub fn revision(&self) -> yu_core::Revision {
        self.source.revision()
    }

    #[must_use]
    pub const fn source_range(&self) -> TextRange {
        self.source_range
    }

    #[must_use]
    pub fn runs(&self) -> &[VisualRun] {
        &self.runs
    }

    #[must_use]
    pub const fn visual_len(&self) -> VisualOffset {
        self.visual_len
    }

    /// Maps a source boundary into the projected visual byte stream.
    ///
    /// Hidden syntax has zero visual width, so all boundaries inside one
    /// hidden run resolve to that run's visual boundary.
    pub fn source_to_visual(
        &self,
        source: ByteOffset,
        _bias: ProjectionBias,
    ) -> Result<VisualOffset, ProjectionError> {
        self.validate_source(source)?;
        if self.runs.is_empty() {
            return Ok(VisualOffset::ZERO);
        }
        for run in &self.runs {
            if run.source.start() <= source && source <= run.source.end() {
                if run.kind == VisualRunKind::HiddenSyntax {
                    return Ok(run.visual.start());
                }
                let delta = source
                    .get()
                    .checked_sub(run.source.start().get())
                    .ok_or(ProjectionError::OffsetOverflow)?;
                return run
                    .visual
                    .start()
                    .checked_add(delta)
                    .ok_or(ProjectionError::OffsetOverflow);
            }
        }
        Err(ProjectionError::SourceOutsideRange {
            offset: source,
            range: self.source_range,
        })
    }

    /// Maps a projected visual boundary back to a source boundary.
    pub fn visual_to_source(
        &self,
        visual: VisualOffset,
        bias: ProjectionBias,
    ) -> Result<ByteOffset, ProjectionError> {
        if visual > self.visual_len {
            return Err(ProjectionError::VisualOutOfBounds {
                offset: visual,
                len: self.visual_len,
            });
        }
        if self.runs.is_empty() {
            return Ok(self.source_range.start());
        }

        for (index, run) in self.runs.iter().enumerate() {
            if run.kind == VisualRunKind::HiddenSyntax && run.visual.start() == visual {
                let mut candidate = match bias {
                    ProjectionBias::Before => run.source.start(),
                    ProjectionBias::After => run.source.end(),
                };
                if bias == ProjectionBias::After {
                    for following in &self.runs[index + 1..] {
                        if following.kind != VisualRunKind::HiddenSyntax
                            || following.visual.start() != visual
                        {
                            break;
                        }
                        candidate = following.source.end();
                    }
                }
                return Ok(candidate);
            }

            if run.kind == VisualRunKind::Visible
                && run.visual.start() <= visual
                && visual <= run.visual.end()
            {
                if visual == run.visual.end()
                    && bias == ProjectionBias::After
                    && self.runs.get(index + 1).is_some_and(|following| {
                        following.kind == VisualRunKind::HiddenSyntax
                            && following.visual.start() == visual
                    })
                {
                    continue;
                }
                let delta = visual
                    .get()
                    .checked_sub(run.visual.start().get())
                    .ok_or(ProjectionError::OffsetOverflow)?;
                return run
                    .source
                    .start()
                    .checked_add(delta)
                    .ok_or(ProjectionError::OffsetOverflow);
            }
        }

        Err(ProjectionError::VisualOutOfBounds {
            offset: visual,
            len: self.visual_len,
        })
    }

    fn validate_source(&self, source: ByteOffset) -> Result<(), ProjectionError> {
        self.source.utf16_offset(source)?;
        if source < self.source_range.start() || source > self.source_range.end() {
            return Err(ProjectionError::SourceOutsideRange {
                offset: source,
                range: self.source_range,
            });
        }
        Ok(())
    }
}

fn change_affects_range(change: TextChange, range: TextRange) -> bool {
    let old = change.old_range();
    old.start() <= range.end() && old.end() >= range.start()
}

fn map_range(range: TextRange, changes: &ChangeSet) -> Result<TextRange, ProjectionError> {
    let start = changes
        .map_anchor(TextAnchor::new(
            changes.before(),
            range.start(),
            Affinity::Before,
        ))?
        .offset();
    let end = changes
        .map_anchor(TextAnchor::new(
            changes.before(),
            range.end(),
            Affinity::After,
        ))?
        .offset();
    TextRange::new(start, end).ok_or(ProjectionError::OffsetOverflow)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Delimiter {
    marker: InlineDelimiter,
    len: usize,
    range: TextRange,
}

fn find_hidden_ranges(inline: &InlineDocument) -> Result<Vec<TextRange>, ProjectionError> {
    let delimiters = inline
        .nodes()
        .iter()
        .filter_map(|node| match node.kind() {
            InlineNodeKind::Delimiter { marker } => Some((marker, node.range())),
            InlineNodeKind::Text | InlineNodeKind::Escaped => None,
        })
        .map(|(marker, range)| {
            Ok(Delimiter {
                marker,
                len: usize::try_from(range.len()).map_err(|_| ProjectionError::OffsetOverflow)?,
                range,
            })
        })
        .collect::<Result<Vec<_>, ProjectionError>>()?;
    let code_pairs = pair_delimiters(&delimiters, InlineDelimiter::Code);
    let mut hidden = code_pairs
        .iter()
        .flat_map(|pair| [pair.start(), pair.end()])
        .collect::<Vec<_>>();

    let inline_delimiters = delimiters
        .iter()
        .copied()
        .filter(|delimiter| delimiter.marker != InlineDelimiter::Code)
        .filter(|delimiter| {
            !code_pairs.iter().any(|pair| {
                pair.start().end() < delimiter.range.start()
                    && delimiter.range.end() < pair.end().start()
            })
        })
        .collect::<Vec<_>>();
    for pair in pair_delimiters(&inline_delimiters, InlineDelimiter::Star) {
        hidden.extend([pair.start(), pair.end()]);
    }
    for pair in pair_delimiters(&inline_delimiters, InlineDelimiter::Underscore) {
        hidden.extend([pair.start(), pair.end()]);
    }
    hidden.sort_by_key(|range| (range.start(), range.end()));
    Ok(hidden)
}

fn pair_delimiters(delimiters: &[Delimiter], marker: InlineDelimiter) -> Vec<TextRangePair> {
    let mut openings = Vec::new();
    let mut pairs = Vec::new();
    for delimiter in delimiters
        .iter()
        .copied()
        .filter(|item| item.marker == marker)
    {
        if let Some(opening_index) = openings
            .iter()
            .rposition(|opening: &Delimiter| opening.len == delimiter.len)
        {
            let opening = openings.remove(opening_index);
            if opening.range.end() < delimiter.range.start() {
                pairs.push(TextRangePair {
                    start: opening.range,
                    end: delimiter.range,
                });
                continue;
            }
        }
        openings.push(delimiter);
    }
    pairs
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct TextRangePair {
    start: TextRange,
    end: TextRange,
}

impl TextRangePair {
    #[must_use]
    const fn start(self) -> TextRange {
        self.start
    }

    #[must_use]
    const fn end(self) -> TextRange {
        self.end
    }
}

fn build_runs(
    source_range: TextRange,
    hidden: &[TextRange],
) -> Result<Vec<VisualRun>, ProjectionError> {
    let mut runs = Vec::new();
    let mut source_cursor =
        usize::try_from(source_range.start()).map_err(|_| ProjectionError::OffsetOverflow)?;
    let source_end =
        usize::try_from(source_range.end()).map_err(|_| ProjectionError::OffsetOverflow)?;
    let mut visual_cursor = VisualOffset::ZERO;
    for hidden_range in hidden {
        if hidden_range.start() < source_range.start() || hidden_range.end() > source_range.end() {
            return Err(ProjectionError::SourceOutsideRange {
                offset: hidden_range.start(),
                range: source_range,
            });
        }
        let hidden_start =
            usize::try_from(hidden_range.start()).map_err(|_| ProjectionError::OffsetOverflow)?;
        let hidden_end =
            usize::try_from(hidden_range.end()).map_err(|_| ProjectionError::OffsetOverflow)?;
        if source_cursor < hidden_start {
            let visible = byte_range(source_cursor, hidden_start)?;
            let visual_end = visual_cursor
                .checked_add(visible.len())
                .ok_or(ProjectionError::OffsetOverflow)?;
            runs.push(VisualRun {
                source: visible,
                visual: VisualRange::new(visual_cursor, visual_end)
                    .ok_or(ProjectionError::OffsetOverflow)?,
                kind: VisualRunKind::Visible,
            });
            visual_cursor = visual_end;
        }
        if hidden_start < source_cursor {
            continue;
        }
        runs.push(VisualRun {
            source: *hidden_range,
            visual: VisualRange::empty(visual_cursor),
            kind: VisualRunKind::HiddenSyntax,
        });
        source_cursor = hidden_end;
    }
    if source_cursor < source_end {
        let visible = byte_range(source_cursor, source_end)?;
        let visual_end = visual_cursor
            .checked_add(visible.len())
            .ok_or(ProjectionError::OffsetOverflow)?;
        runs.push(VisualRun {
            source: visible,
            visual: VisualRange::new(visual_cursor, visual_end)
                .ok_or(ProjectionError::OffsetOverflow)?,
            kind: VisualRunKind::Visible,
        });
    }
    Ok(runs)
}

fn byte_range(start: usize, end: usize) -> Result<TextRange, ProjectionError> {
    let start = ByteOffset::try_from(start).map_err(|_| ProjectionError::OffsetOverflow)?;
    let end = ByteOffset::try_from(end).map_err(|_| ProjectionError::OffsetOverflow)?;
    TextRange::new(start, end).ok_or(ProjectionError::OffsetOverflow)
}

#[cfg(test)]
mod tests {
    use super::*;
    use yu_text::{Edit, StorageBackend, TextBuffer, Transaction, retained_snapshot_stats};

    fn projection(source: &str) -> Projection {
        let buffer = TextBuffer::new(source);
        let snapshot = buffer.snapshot();
        Projection::inline(
            &snapshot,
            TextRange::new(ByteOffset::ZERO, snapshot.len_bytes())
                .expect("source range should be ordered"),
        )
        .expect("projection should build")
    }

    #[test]
    fn strong_delimiters_become_zero_width_runs_with_bidirectional_mapping() {
        let source = "before **羽🙂** after";
        let projection = projection(source);
        let hidden = projection
            .runs()
            .iter()
            .filter(|run| run.kind() == VisualRunKind::HiddenSyntax)
            .collect::<Vec<_>>();

        assert_eq!(hidden.len(), 2);
        assert_eq!(
            projection.visual_len().get(),
            u64::try_from(source.len() - 4).expect("test length should fit")
        );
        let open_start = ByteOffset::new(7);
        let open_end = ByteOffset::new(9);
        let content_end = ByteOffset::new(16);
        assert_eq!(
            projection
                .source_to_visual(open_start, ProjectionBias::Before)
                .expect("open delimiter should map"),
            VisualOffset::new(7)
        );
        assert_eq!(
            projection
                .source_to_visual(open_end, ProjectionBias::After)
                .expect("content start should map"),
            VisualOffset::new(7)
        );
        assert_eq!(
            projection
                .visual_to_source(VisualOffset::new(7), ProjectionBias::Before)
                .expect("visual boundary should map before syntax"),
            open_start
        );
        assert_eq!(
            projection
                .visual_to_source(VisualOffset::new(7), ProjectionBias::After)
                .expect("visual boundary should map after syntax"),
            open_end
        );
        assert_eq!(
            projection
                .source_to_visual(content_end, ProjectionBias::Before)
                .expect("content end should map"),
            VisualOffset::new(14)
        );
        assert_eq!(
            projection
                .visual_to_source(VisualOffset::new(14), ProjectionBias::After)
                .expect("close delimiter should map after syntax"),
            ByteOffset::new(18)
        );
    }

    #[test]
    fn unmatched_and_escaped_delimiters_remain_visible() {
        let projection = projection(r"\*literal * unmatched **ok**");
        let hidden = projection
            .runs()
            .iter()
            .filter(|run| run.kind() == VisualRunKind::HiddenSyntax)
            .count();
        assert_eq!(hidden, 2);
        assert_eq!(
            projection.visual_len().get(),
            u64::try_from(r"\*literal * unmatched ok".len()).expect("test length should fit")
        );
    }

    #[test]
    fn projection_scans_piece_tree_chunks_without_materializing_snapshot() {
        let parts = ["before ", "**", "羽🙂", "**", " after"];
        let mut buffer = TextBuffer::with_backend("", StorageBackend::PieceTree);
        for part in parts {
            let at = buffer.snapshot().len_bytes();
            let transaction = yu_text::Transaction::new(
                buffer.revision(),
                [yu_text::Edit::new(TextRange::empty(at), part)],
            );
            buffer.apply(&transaction).expect("append should apply");
        }
        let snapshot = buffer.snapshot();
        let range = TextRange::new(ByteOffset::ZERO, snapshot.len_bytes())
            .expect("source range should be ordered");
        let _projection = Projection::inline(&snapshot, range).expect("projection should build");
        assert_eq!(
            retained_snapshot_stats(std::slice::from_ref(&snapshot)).materialized_buffers(),
            0
        );
    }

    #[test]
    fn projection_consumes_parser_owned_inline_tokens() {
        let source = "before **羽🙂** after";
        let buffer = TextBuffer::with_backend(source, StorageBackend::PieceTree);
        let snapshot = buffer.snapshot();
        let range = TextRange::new(ByteOffset::ZERO, snapshot.len_bytes())
            .expect("source range should be ordered");
        let inline =
            yu_markdown::parse_inline(&snapshot, range).expect("inline parse should build");
        let from_cst = Projection::from_inline(&inline).expect("projection should build");
        let direct = Projection::inline(&snapshot, range).expect("direct projection should build");

        assert_eq!(from_cst.runs(), direct.runs());
        assert_eq!(from_cst.revision(), inline.revision());
        assert_eq!(from_cst.source_range(), inline.source_range());
        assert_eq!(
            retained_snapshot_stats(std::slice::from_ref(&snapshot)).materialized_buffers(),
            0
        );
    }

    #[test]
    fn projection_maps_through_prefix_edit_without_reparsing() {
        let source = "prefix **羽🙂** suffix";
        let mut buffer = TextBuffer::with_backend(source, StorageBackend::PieceTree);
        let snapshot = buffer.snapshot();
        let start = source.find("**").expect("strong delimiter should exist");
        let end = start + "**羽🙂**".len();
        let range = TextRange::new(ByteOffset::new(start as u64), ByteOffset::new(end as u64))
            .expect("projection range should be ordered");
        let projection = Projection::inline(&snapshot, range).expect("projection should build");
        let transaction = Transaction::new(
            buffer.revision(),
            [Edit::new(TextRange::empty(ByteOffset::ZERO), "前")],
        );
        let applied = buffer
            .apply(&transaction)
            .expect("prefix edit should apply");

        let mapped = projection
            .map_through(applied.change_set(), applied.result_snapshot())
            .expect("unchanged projection should map")
            .expect("prefix edit must not invalidate the range");
        let shifted = TextRange::new(
            ByteOffset::new((start + "前".len()) as u64),
            ByteOffset::new((end + "前".len()) as u64),
        )
        .expect("shifted range should be ordered");
        assert_eq!(mapped.source_range(), shifted);
        assert_eq!(mapped.visual_len(), projection.visual_len());
        assert_eq!(mapped.revision(), applied.result_snapshot().revision());
        assert_eq!(
            mapped
                .visual_to_source(VisualOffset::new(0), ProjectionBias::After)
                .expect("mapped visual boundary should resolve"),
            ByteOffset::new((start + "前".len() + 2) as u64)
        );
    }

    #[test]
    fn projection_is_invalidated_by_inside_or_boundary_edits() {
        let source = "prefix **羽🙂** suffix";
        let mut buffer = TextBuffer::new(source);
        let snapshot = buffer.snapshot();
        let start = source.find("**").expect("strong delimiter should exist");
        let end = start + "**羽🙂**".len();
        let range = TextRange::new(ByteOffset::new(start as u64), ByteOffset::new(end as u64))
            .expect("projection range should be ordered");
        let projection = Projection::inline(&snapshot, range).expect("projection should build");

        let inside = Transaction::new(
            buffer.revision(),
            [Edit::new(
                TextRange::empty(ByteOffset::new((start + 2) as u64)),
                "x",
            )],
        );
        let applied = buffer.apply(&inside).expect("inside edit should apply");
        assert!(
            projection
                .map_through(applied.change_set(), applied.result_snapshot())
                .expect("revision-bound map should succeed")
                .is_none()
        );

        let shifted_range = TextRange::new(
            ByteOffset::new(start as u64),
            ByteOffset::new((end + 1) as u64),
        )
        .expect("shifted projection range should be ordered");
        let boundary_projection = Projection::inline(applied.result_snapshot(), shifted_range)
            .expect("shifted projection should build");
        let boundary = Transaction::new(
            buffer.revision(),
            [Edit::new(
                TextRange::empty(ByteOffset::new((end + 1) as u64)),
                "x",
            )],
        );
        let applied = buffer.apply(&boundary).expect("boundary edit should apply");
        assert!(
            boundary_projection
                .map_through(applied.change_set(), applied.result_snapshot())
                .expect("revision-bound map should succeed")
                .is_none()
        );
    }

    #[test]
    fn identity_projection_maps_all_character_boundaries() {
        let source = "羽🙂\ntext";
        let projection = projection(source);
        assert_eq!(projection.runs().len(), 1);
        assert_eq!(projection.runs()[0].kind(), VisualRunKind::Visible);
        for (offset, _) in source
            .char_indices()
            .chain(std::iter::once((source.len(), ' ')))
        {
            let source_offset = ByteOffset::new(offset as u64);
            let visual = projection
                .source_to_visual(source_offset, ProjectionBias::After)
                .expect("source boundary should map");
            assert_eq!(
                projection
                    .visual_to_source(visual, ProjectionBias::After)
                    .expect("visual boundary should map"),
                source_offset
            );
        }
    }

    #[test]
    fn subrange_projection_starts_scanning_at_the_requested_boundary() {
        let source = "outside **hidden** **inside** suffix";
        let buffer = TextBuffer::new(source);
        let snapshot = buffer.snapshot();
        let start = source.find("**inside**").expect("inside span should exist");
        let end = start + "**inside**".len();
        let range = TextRange::new(ByteOffset::new(start as u64), ByteOffset::new(end as u64))
            .expect("subrange should be ordered");
        let projection = Projection::inline(&snapshot, range).expect("projection should build");
        assert_eq!(
            projection
                .runs()
                .iter()
                .filter(|run| run.kind() == VisualRunKind::HiddenSyntax)
                .count(),
            2
        );
        assert_eq!(projection.source_range(), range);
    }

    #[test]
    fn empty_projection_maps_its_single_source_boundary() {
        let buffer = TextBuffer::new("text");
        let snapshot = buffer.snapshot();
        let at = ByteOffset::new(2);
        let projection =
            Projection::inline(&snapshot, TextRange::empty(at)).expect("empty range is valid");
        assert_eq!(
            projection
                .source_to_visual(at, ProjectionBias::After)
                .expect("empty source boundary should map"),
            VisualOffset::ZERO
        );
        assert_eq!(
            projection
                .visual_to_source(VisualOffset::ZERO, ProjectionBias::After)
                .expect("empty visual boundary should map"),
            at
        );
    }
}
