#![forbid(unsafe_code)]

//! Source-to-visual projection primitives.
//!
//! This phase intentionally implements only a small, lossless inline
//! projection. Matched Markdown emphasis, strong-emphasis, and code-span
//! delimiters become zero-width visual runs; all other source bytes remain
//! visible and continue to point into the canonical TextSnapshot.

use std::error::Error;
use std::fmt;
use yu_core::{ByteOffset, TextRange};
use yu_text::{ChunkCursor, TextPositionError, TextSnapshot};

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
            Self::SourceOutsideRange { .. }
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
        source.utf16_offset(source_range.start())?;
        source.utf16_offset(source_range.end())?;
        let hidden = find_hidden_ranges(source, source_range)?;
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

const CODE_MARKER: u8 = 0x60;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Delimiter {
    marker: u8,
    len: usize,
    range: TextRange,
}

fn find_hidden_ranges(
    source: &TextSnapshot,
    source_range: TextRange,
) -> Result<Vec<TextRange>, ProjectionError> {
    let delimiters = scan_delimiters(source, source_range)?;
    let code_pairs = pair_delimiters(&delimiters, CODE_MARKER);
    let mut hidden = code_pairs
        .iter()
        .flat_map(|pair| [pair.start(), pair.end()])
        .collect::<Vec<_>>();

    let inline_delimiters = delimiters
        .iter()
        .copied()
        .filter(|delimiter| delimiter.marker != CODE_MARKER)
        .filter(|delimiter| {
            !code_pairs.iter().any(|pair| {
                pair.start().end() < delimiter.range.start()
                    && delimiter.range.end() < pair.end().start()
            })
        })
        .collect::<Vec<_>>();
    for pair in pair_delimiters(&inline_delimiters, b'*') {
        hidden.extend([pair.start(), pair.end()]);
    }
    for pair in pair_delimiters(&inline_delimiters, b'_') {
        hidden.extend([pair.start(), pair.end()]);
    }
    hidden.sort_by_key(|range| (range.start(), range.end()));
    Ok(hidden)
}

fn scan_delimiters(
    source: &TextSnapshot,
    source_range: TextRange,
) -> Result<Vec<Delimiter>, ProjectionError> {
    let cursor = ByteCursor::new(source, source_range)?;
    let mut cursor = cursor.peekable();
    let mut delimiters = Vec::new();
    while let Some((start, byte)) = cursor.next() {
        if byte == b'\\' {
            let _ = cursor.next();
            continue;
        }
        if !matches!(byte, b'*' | b'_' | CODE_MARKER) {
            continue;
        }
        let mut end = start
            .checked_add(1)
            .ok_or(ProjectionError::OffsetOverflow)?;
        while cursor.peek().is_some_and(|(_, next)| *next == byte) {
            let (next_start, _) = cursor.next().expect("peeked delimiter must be available");
            end = next_start
                .checked_add(1)
                .ok_or(ProjectionError::OffsetOverflow)?;
        }
        let range = byte_range(start, end)?;
        delimiters.push(Delimiter {
            marker: byte,
            len: end - start,
            range,
        });
    }
    Ok(delimiters)
}

fn pair_delimiters(delimiters: &[Delimiter], marker: u8) -> Vec<TextRangePair> {
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

struct ByteCursor<'a> {
    chunks: ChunkCursor<'a>,
    requested_start: usize,
    end: usize,
    current: Option<&'a [u8]>,
    current_start: usize,
    current_index: usize,
}

impl<'a> ByteCursor<'a> {
    fn new(source: &'a TextSnapshot, range: TextRange) -> Result<Self, ProjectionError> {
        let start = usize::try_from(range.start()).map_err(|_| ProjectionError::OffsetOverflow)?;
        let end = usize::try_from(range.end()).map_err(|_| ProjectionError::OffsetOverflow)?;
        Ok(Self {
            chunks: source.chunk_cursor(range.start())?,
            requested_start: start,
            end,
            current: None,
            current_start: start,
            current_index: start,
        })
    }
}

impl Iterator for ByteCursor<'_> {
    type Item = (usize, u8);

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            if let Some(current) = self.current {
                if self.current_index < self.current_start + current.len()
                    && self.current_index < self.end
                {
                    let local = self.current_index - self.current_start;
                    let value = current[local];
                    let position = self.current_index;
                    self.current_index += 1;
                    return Some((position, value));
                }
                self.current = None;
            }

            let chunk = self.chunks.next()?;
            self.current_start = usize::try_from(chunk.start()).ok()?;
            self.current_index = self.current_start.max(self.requested_start);
            self.current = Some(chunk.text().as_bytes());
            if self.current_index < self.end {
                continue;
            }
            return None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use yu_text::{StorageBackend, TextBuffer, retained_snapshot_stats};

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
