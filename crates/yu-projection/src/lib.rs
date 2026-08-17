#![forbid(unsafe_code)]

//! Source-to-visual projection primitives.
//!
//! This phase intentionally implements only a small, lossless inline
//! projection. Matched Markdown emphasis, strong-emphasis, and code-span
//! delimiters from `yu-markdown::InlineDocument` become zero-width visual
//! runs; parser-owned line endings become explicit line-break runs, while all
//! other source bytes continue to point into the canonical TextSnapshot.

use std::error::Error;
use std::fmt;
use std::sync::Arc;
use yu_core::{Affinity, ByteOffset, Revision, TextAnchor, TextRange};
use yu_markdown::{
    Block, BlockKind, InlineDocument, InlineNodeKind, InlineParseError, InlineSpan, InlineSpanKind,
    ReferenceDefinitionIndex, TableBlock, parse_inline, parse_inline_with_definitions,
    parse_table_in_snapshot,
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
    /// A parser-owned soft or hard source line ending. The line ending has
    /// visual width in the byte projection so layout can create the next
    /// visual line without rescanning source text.
    LineBreak { hard: bool },
    /// Source syntax remains in the canonical source but occupies no visual bytes.
    HiddenSyntax,
    /// Transient IME preedit text projected over a canonical source range.
    Composition,
}

/// Semantic style carried by a visible visual run.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum VisualRunStyle {
    Plain,
    Emphasis,
    Strong,
    Code,
}

/// A source-backed visual run.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct VisualRun {
    source: TextRange,
    visual: VisualRange,
    kind: VisualRunKind,
    style: VisualRunStyle,
}

/// Source-backed metadata for one Markdown image in a projection.
///
/// The projection does not copy the destination or alt text. Native/resource
/// layers resolve these ranges against the same `TextSnapshot` and may keep
/// their own decoded/cache representation outside the editor model.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ImageSource {
    source: TextRange,
    label: TextRange,
    destination: Option<TextRange>,
    reference: Option<TextRange>,
}

impl ImageSource {
    #[must_use]
    pub const fn source(self) -> TextRange {
        self.source
    }

    #[must_use]
    pub const fn label(self) -> TextRange {
        self.label
    }

    /// Returns the inline destination range, when the image uses `![](...)`.
    #[must_use]
    pub const fn destination(self) -> Option<TextRange> {
        self.destination
    }

    /// Returns the reference label range, when the image uses a reference
    /// form such as `![logo][asset]` or `![logo][]`.
    #[must_use]
    pub const fn reference(self) -> Option<TextRange> {
        self.reference
    }

    #[must_use]
    pub const fn is_reference(self) -> bool {
        self.reference.is_some()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct LineBreakSpec {
    source: TextRange,
    hard: bool,
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

    #[must_use]
    pub const fn style(self) -> VisualRunStyle {
        self.style
    }

    #[must_use]
    pub const fn is_line_break(self) -> bool {
        matches!(self.kind, VisualRunKind::LineBreak { .. })
    }

    #[must_use]
    pub const fn is_hard_line_break(self) -> bool {
        matches!(self.kind, VisualRunKind::LineBreak { hard: true })
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
    NotFencedCodeBlock {
        kind: BlockKind,
    },
    InvalidCodeBlock {
        range: TextRange,
    },
    InvalidTaskListBlock {
        range: TextRange,
    },
    CompositionRangeOutsideProjection {
        range: TextRange,
        projection: TextRange,
    },
    CompositionSelectionOutOfBounds {
        range: TextRange,
        text_len: ByteOffset,
    },
    CompositionSelectionNotUtf8Boundary {
        offset: ByteOffset,
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
            Self::NotFencedCodeBlock { kind } => {
                write!(formatter, "block kind {kind:?} is not a fenced code block")
            }
            Self::InvalidCodeBlock { range } => {
                write!(formatter, "invalid fenced code block range {range:?}")
            }
            Self::InvalidTaskListBlock { range } => {
                write!(formatter, "invalid task-list block range {range:?}")
            }
            Self::CompositionRangeOutsideProjection { range, projection } => write!(
                formatter,
                "composition range {range:?} is outside projection {projection:?}"
            ),
            Self::CompositionSelectionOutOfBounds { range, text_len } => write!(
                formatter,
                "composition selection {range:?} exceeds preedit length {text_len:?}"
            ),
            Self::CompositionSelectionNotUtf8Boundary { offset } => {
                write!(
                    formatter,
                    "composition selection {offset:?} is not a UTF-8 boundary"
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
            | Self::NotFencedCodeBlock { .. }
            | Self::InvalidCodeBlock { .. }
            | Self::InvalidTaskListBlock { .. }
            | Self::CompositionRangeOutsideProjection { .. }
            | Self::CompositionSelectionOutOfBounds { .. }
            | Self::CompositionSelectionNotUtf8Boundary { .. }
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
    images: Vec<ImageSource>,
    visual_len: VisualOffset,
    composition: Option<CompositionState>,
}

#[derive(Clone, Debug)]
struct CompositionState {
    replacement_range: TextRange,
    text: Arc<str>,
    selection_bytes: TextRange,
    visual: VisualRange,
}

impl Projection {
    /// Builds a minimal Markdown inline projection.
    ///
    /// Matched emphasis, strong-emphasis, and backtick delimiters are hidden.
    /// Parser-owned line endings become explicit line-break runs; hard-break
    /// marker bytes remain zero-width hidden syntax. The parser is deliberately
    /// conservative: unmatched or escaped delimiters remain visible, and
    /// source bytes are never rewritten.
    pub fn inline(source: &TextSnapshot, source_range: TextRange) -> Result<Self, ProjectionError> {
        let inline = parse_inline(source, source_range)?;
        Self::from_inline(&inline)
    }

    /// Builds an inline projection with the current document's definition
    /// index, enabling resolved shortcut references such as `[project]`.
    pub fn inline_with_definitions(
        source: &TextSnapshot,
        source_range: TextRange,
        definitions: &ReferenceDefinitionIndex,
    ) -> Result<Self, ProjectionError> {
        let inline = parse_inline_with_definitions(source, source_range, Some(definitions))?;
        Self::from_inline(&inline)
    }

    /// Builds a zero-width projection for source-only blocks such as link
    /// definitions. The source remains canonical and addressable, but it does
    /// not contribute visual bytes.
    pub fn hidden(source: &TextSnapshot, source_range: TextRange) -> Result<Self, ProjectionError> {
        Self::from_source_parts(
            source,
            source_range,
            &[source_range],
            &[],
            &[],
            VisualRunStyle::Plain,
        )
    }

    /// Builds a projection from the parser-owned lossless inline token stream.
    ///
    /// Keeping this constructor separate makes the source of delimiter ranges
    /// explicit: the projection never rescans or owns a second inline syntax
    /// representation.
    pub fn from_inline(inline: &InlineDocument) -> Result<Self, ProjectionError> {
        Self::from_inline_with_hidden(inline, &[])
    }

    /// Builds a projection from parser-owned inline tokens and additional
    /// source ranges supplied by a block-level parser (for example `[ ]` in a
    /// task-list item).
    pub fn from_inline_with_hidden(
        inline: &InlineDocument,
        extra_hidden: &[TextRange],
    ) -> Result<Self, ProjectionError> {
        let source = inline.source();
        let source_range = inline.source_range();
        let mut hidden = inline
            .spans()
            .iter()
            .flat_map(|span| [span.opening(), span.closing()])
            .collect::<Vec<_>>();
        hidden.extend_from_slice(extra_hidden);
        let line_breaks = inline
            .nodes()
            .iter()
            .filter_map(|node| match node.kind() {
                InlineNodeKind::LineBreak { hard } => Some(LineBreakSpec {
                    source: node.range(),
                    hard,
                }),
                InlineNodeKind::Text
                | InlineNodeKind::Escaped
                | InlineNodeKind::Delimiter { .. }
                | InlineNodeKind::Punctuation { .. } => None,
            })
            .collect::<Vec<_>>();
        Self::from_source_parts(
            source,
            source_range,
            &hidden,
            &line_breaks,
            inline.spans(),
            VisualRunStyle::Plain,
        )
    }

    fn from_source_parts(
        source: &TextSnapshot,
        source_range: TextRange,
        hidden: &[TextRange],
        line_breaks: &[LineBreakSpec],
        spans: &[InlineSpan],
        default_style: VisualRunStyle,
    ) -> Result<Self, ProjectionError> {
        source.utf16_offset(source_range.start())?;
        source.utf16_offset(source_range.end())?;
        let runs = build_runs(
            source,
            source_range,
            hidden,
            line_breaks,
            spans,
            default_style,
        )?;
        let images = spans
            .iter()
            .filter_map(|span| match span.kind() {
                InlineSpanKind::Image | InlineSpanKind::ReferenceImage => Some(ImageSource {
                    source: span.source_range(),
                    label: span.content(),
                    destination: span.destination(),
                    reference: span.reference(),
                }),
                InlineSpanKind::Emphasis
                | InlineSpanKind::Strong
                | InlineSpanKind::CodeSpan
                | InlineSpanKind::Link
                | InlineSpanKind::ReferenceLink
                | InlineSpanKind::Autolink => None,
            })
            .collect::<Vec<_>>();
        let visual_len = runs
            .last()
            .map_or(VisualOffset::ZERO, |run| run.visual.end());
        Ok(Self {
            source: source.clone(),
            source_range,
            runs,
            images,
            visual_len,
            composition: None,
        })
    }

    /// Projects transient IME preedit text over a canonical source range.
    ///
    /// The source snapshot and its Revision remain unchanged. Markdown runs
    /// outside the replacement range retain their parser-owned source ranges;
    /// the preedit is a plain composition run and is never parsed as Markdown.
    pub fn with_composition(
        &self,
        replacement_range: TextRange,
        text: impl Into<Arc<str>>,
        selection_bytes: TextRange,
    ) -> Result<Self, ProjectionError> {
        if replacement_range.start() < self.source_range.start()
            || replacement_range.end() > self.source_range.end()
        {
            return Err(ProjectionError::CompositionRangeOutsideProjection {
                range: replacement_range,
                projection: self.source_range,
            });
        }
        self.source.utf16_offset(replacement_range.start())?;
        self.source.utf16_offset(replacement_range.end())?;
        let text = text.into();
        validate_composition_selection(text.as_ref(), selection_bytes)?;
        let visual_start =
            self.source_to_visual(replacement_range.start(), ProjectionBias::Before)?;
        let visual_end = self.source_to_visual(replacement_range.end(), ProjectionBias::After)?;
        let old_visual_len = visual_end
            .get()
            .checked_sub(visual_start.get())
            .ok_or(ProjectionError::OffsetOverflow)?;
        let new_visual_len =
            u64::try_from(text.len()).map_err(|_| ProjectionError::OffsetOverflow)?;
        let visual_end = visual_start
            .checked_add(new_visual_len)
            .ok_or(ProjectionError::OffsetOverflow)?;
        let visual_delta = i128::from(new_visual_len) - i128::from(old_visual_len);

        let mut runs = Vec::with_capacity(self.runs.len().saturating_add(1));
        for run in &self.runs {
            append_composition_run_part(&mut runs, *run, replacement_range, visual_delta)?;
        }
        if !text.is_empty() {
            let composition_run = VisualRun {
                source: replacement_range,
                visual: VisualRange::new(visual_start, visual_end)
                    .ok_or(ProjectionError::OffsetOverflow)?,
                kind: VisualRunKind::Composition,
                style: VisualRunStyle::Plain,
            };
            let insertion = runs
                .iter()
                .position(|run| {
                    run.kind() != VisualRunKind::Composition
                        && run.source().start() >= replacement_range.end()
                })
                .unwrap_or(runs.len());
            runs.insert(insertion, composition_run);
        }

        Ok(Self {
            source: self.source.clone(),
            source_range: self.source_range,
            runs,
            images: self.images.clone(),
            visual_len: shift_visual(self.visual_len, visual_delta)?,
            composition: Some(CompositionState {
                replacement_range,
                text,
                selection_bytes,
                visual: VisualRange::new(visual_start, visual_end)
                    .ok_or(ProjectionError::OffsetOverflow)?,
            }),
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
        if self.composition.is_some() {
            return Ok(None);
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
                style: run.style,
            });
        }
        snapshot.utf16_offset(source_range.start())?;
        snapshot.utf16_offset(source_range.end())?;
        Ok(Some(Self {
            source: snapshot.clone(),
            source_range,
            runs,
            images: self
                .images
                .iter()
                .map(|image| {
                    Ok(ImageSource {
                        source: map_range(image.source, changes)?,
                        label: map_range(image.label, changes)?,
                        destination: image
                            .destination
                            .map(|range| map_range(range, changes))
                            .transpose()?,
                        reference: image
                            .reference
                            .map(|range| map_range(range, changes))
                            .transpose()?,
                    })
                })
                .collect::<Result<Vec<_>, ProjectionError>>()?,
            visual_len: self.visual_len,
            composition: None,
        }))
    }

    #[must_use]
    pub fn revision(&self) -> yu_core::Revision {
        self.source.revision()
    }

    /// Returns the immutable source snapshot used by this projection.
    #[must_use]
    pub fn source(&self) -> &TextSnapshot {
        &self.source
    }

    #[must_use]
    pub const fn source_range(&self) -> TextRange {
        self.source_range
    }

    #[must_use]
    pub fn runs(&self) -> &[VisualRun] {
        &self.runs
    }

    /// Returns source-backed image metadata in parser span order.
    #[must_use]
    pub fn images(&self) -> &[ImageSource] {
        &self.images
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
        bias: ProjectionBias,
    ) -> Result<VisualOffset, ProjectionError> {
        self.validate_source(source)?;
        if let Some(composition) = &self.composition {
            let range = composition.replacement_range;
            if range.is_empty() {
                if source < range.start() {
                    return self.source_to_visual_runs(source, bias);
                }
                if source == range.start() {
                    return Ok(match bias {
                        ProjectionBias::Before => self.source_to_visual_runs(source, bias)?,
                        ProjectionBias::After => composition.visual.end(),
                    });
                }
                return self.source_to_visual_runs(source, bias);
            }
            if source < range.start() {
                return self.source_to_visual_runs(source, bias);
            }
            if source == range.start() {
                return Ok(composition.visual.start());
            }
            if source < range.end() {
                return Ok(match bias {
                    ProjectionBias::Before => composition.visual.start(),
                    ProjectionBias::After => composition.visual.end(),
                });
            }
            if source == range.end() {
                return Ok(composition.visual.end());
            }
            if source > range.end() {
                return self.source_to_visual_runs(source, bias);
            }
        }
        self.source_to_visual_runs(source, bias)
    }

    fn source_to_visual_runs(
        &self,
        source: ByteOffset,
        _bias: ProjectionBias,
    ) -> Result<VisualOffset, ProjectionError> {
        if self.runs.is_empty() {
            return Ok(VisualOffset::ZERO);
        }
        for run in &self.runs {
            if run.kind() == VisualRunKind::Composition {
                continue;
            }
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
        if let Some(composition) = &self.composition {
            let range = composition.visual;
            if visual >= range.start() && visual <= range.end() {
                if visual == range.start() {
                    return Ok(composition.replacement_range.start());
                }
                if visual == range.end() {
                    return Ok(composition.replacement_range.end());
                }
                return Ok(match bias {
                    ProjectionBias::Before => composition.replacement_range.start(),
                    ProjectionBias::After => composition.replacement_range.end(),
                });
            }
        }
        self.visual_to_source_runs(visual, bias)
    }

    fn visual_to_source_runs(
        &self,
        visual: VisualOffset,
        bias: ProjectionBias,
    ) -> Result<ByteOffset, ProjectionError> {
        if self.runs.is_empty() {
            return Ok(self.source_range.start());
        }

        for (index, run) in self.runs.iter().enumerate() {
            if run.kind() == VisualRunKind::Composition {
                continue;
            }
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

            if run.kind != VisualRunKind::HiddenSyntax
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

    /// Returns the transient preedit text, if this projection has one.
    #[must_use]
    pub fn composition_text(&self) -> Option<&str> {
        self.composition
            .as_ref()
            .map(|composition| composition.text.as_ref())
    }

    /// Returns the canonical range replaced by the transient preedit.
    #[must_use]
    pub fn composition_range(&self) -> Option<TextRange> {
        self.composition
            .as_ref()
            .map(|composition| composition.replacement_range)
    }

    /// Returns the preedit selection in temporary-text byte coordinates.
    #[must_use]
    pub fn composition_selection_bytes(&self) -> Option<TextRange> {
        self.composition
            .as_ref()
            .map(|composition| composition.selection_bytes)
    }

    /// Returns the preedit selection in the projected visual byte stream.
    pub fn composition_selection_visual(&self) -> Option<VisualRange> {
        let composition = self.composition.as_ref()?;
        let start = composition
            .visual
            .start()
            .checked_add(composition.selection_bytes.start().get())?;
        let end = composition
            .visual
            .start()
            .checked_add(composition.selection_bytes.end().get())?;
        VisualRange::new(start, end)
    }

    /// Returns a synthetic range suitable for passing the preedit text to a
    /// shaping provider. It is local to the temporary text, not canonical source.
    pub fn composition_shape_source_range(&self) -> Option<TextRange> {
        let text_len = u64::try_from(self.composition_text()?.len()).ok()?;
        TextRange::new(ByteOffset::ZERO, ByteOffset::new(text_len))
    }

    /// Returns the source coordinate space consumed by a shaping provider for
    /// one run. Composition text uses a temporary zero-based range because its
    /// byte length need not equal the replaced canonical range.
    pub fn shape_source_range_for_run(&self, run: VisualRun) -> TextRange {
        if run.kind() == VisualRunKind::Composition {
            self.composition_shape_source_range()
                .expect("composition run always has composition text")
        } else {
            run.source()
        }
    }

    /// Maps a byte slice in a run's shaping coordinate space back to canonical
    /// source. Every composition slice maps to the replacement range because
    /// preedit bytes are not canonical source bytes.
    pub fn source_range_for_run_slice(
        &self,
        run: VisualRun,
        local_start: u64,
        local_end: u64,
    ) -> Result<TextRange, ProjectionError> {
        if local_start > local_end {
            return Err(ProjectionError::OffsetOverflow);
        }
        if run.kind() == VisualRunKind::Composition {
            let text_len = u64::try_from(
                self.composition_text()
                    .expect("composition run always has composition text")
                    .len(),
            )
            .map_err(|_| ProjectionError::OffsetOverflow)?;
            if local_end > text_len {
                return Err(ProjectionError::OffsetOverflow);
            }
            return Ok(self
                .composition_range()
                .expect("composition run always has a replacement range"));
        }
        let start = run
            .source()
            .start()
            .checked_add(local_start)
            .ok_or(ProjectionError::OffsetOverflow)?;
        let end = run
            .source()
            .start()
            .checked_add(local_end)
            .ok_or(ProjectionError::OffsetOverflow)?;
        TextRange::new(start, end).ok_or(ProjectionError::OffsetOverflow)
    }

    /// Maps a byte slice in a run's shaping coordinate space to visual bytes.
    pub fn visual_range_for_run_slice(
        &self,
        run: VisualRun,
        local_start: u64,
        local_end: u64,
    ) -> Result<VisualRange, ProjectionError> {
        let start = run
            .visual()
            .start()
            .checked_add(local_start)
            .ok_or(ProjectionError::OffsetOverflow)?;
        let end = run
            .visual()
            .start()
            .checked_add(local_end)
            .ok_or(ProjectionError::OffsetOverflow)?;
        VisualRange::new(start, end).ok_or(ProjectionError::OffsetOverflow)
    }

    /// Reads the text consumed by a layout/shaping pass for one visual run.
    /// Composition runs read their transient text; all other runs read the
    /// canonical snapshot range.
    pub fn text_for_run(&self, run: VisualRun) -> Result<String, ProjectionError> {
        if run.kind() == VisualRunKind::Composition {
            return Ok(self
                .composition_text()
                .expect("composition run always has composition text")
                .to_owned());
        }
        String::from_utf8(read_range(&self.source, run.source())?)
            .map_err(|_| ProjectionError::OffsetOverflow)
    }

    /// Reads a local byte slice from the temporary text used by a run.
    pub fn text_for_run_slice(
        &self,
        run: VisualRun,
        local_start: u64,
        local_end: u64,
    ) -> Result<String, ProjectionError> {
        if local_start > local_end {
            return Err(ProjectionError::OffsetOverflow);
        }
        if run.kind() == VisualRunKind::Composition {
            let text = self
                .composition_text()
                .expect("composition run always has composition text");
            let start =
                usize::try_from(local_start).map_err(|_| ProjectionError::OffsetOverflow)?;
            let end = usize::try_from(local_end).map_err(|_| ProjectionError::OffsetOverflow)?;
            if end > text.len() || !text.is_char_boundary(start) || !text.is_char_boundary(end) {
                return Err(ProjectionError::OffsetOverflow);
            }
            return Ok(text[start..end].to_owned());
        }
        let range = self.source_range_for_run_slice(run, local_start, local_end)?;
        String::from_utf8(read_range(&self.source, range)?)
            .map_err(|_| ProjectionError::OffsetOverflow)
    }
}

fn validate_composition_selection(text: &str, selection: TextRange) -> Result<(), ProjectionError> {
    let text_len = ByteOffset::try_from(text.len()).map_err(|_| ProjectionError::OffsetOverflow)?;
    if selection.end() > text_len {
        return Err(ProjectionError::CompositionSelectionOutOfBounds {
            range: selection,
            text_len,
        });
    }
    for offset in [selection.start(), selection.end()] {
        let offset = usize::try_from(offset).map_err(|_| ProjectionError::OffsetOverflow)?;
        if !text.is_char_boundary(offset) {
            return Err(ProjectionError::CompositionSelectionNotUtf8Boundary {
                offset: ByteOffset::try_from(offset)
                    .map_err(|_| ProjectionError::OffsetOverflow)?,
            });
        }
    }
    Ok(())
}

fn append_composition_run_part(
    runs: &mut Vec<VisualRun>,
    run: VisualRun,
    replacement: TextRange,
    visual_delta: i128,
) -> Result<(), ProjectionError> {
    if run.kind() == VisualRunKind::Composition {
        return Ok(());
    }
    let source_start = run.source().start();
    let source_end = run.source().end();
    if source_end <= replacement.start() {
        runs.push(run);
        return Ok(());
    }
    if source_start >= replacement.end() && !replacement.is_empty() {
        runs.push(shift_run(run, visual_delta)?);
        return Ok(());
    }
    if replacement.is_empty() && source_start >= replacement.start() {
        runs.push(shift_run(run, visual_delta)?);
        return Ok(());
    }
    if source_start < replacement.start() && run.kind() == VisualRunKind::Visible {
        runs.push(clip_visible_run(
            run,
            source_start,
            replacement.start().min(source_end),
            0,
        )?);
    }
    if source_end > replacement.end() && run.kind() == VisualRunKind::Visible {
        runs.push(clip_visible_run(
            run,
            replacement.end().max(source_start),
            source_end,
            visual_delta,
        )?);
    }
    Ok(())
}

fn clip_visible_run(
    run: VisualRun,
    source_start: ByteOffset,
    source_end: ByteOffset,
    visual_delta: i128,
) -> Result<VisualRun, ProjectionError> {
    let start_delta = source_start
        .get()
        .checked_sub(run.source().start().get())
        .ok_or(ProjectionError::OffsetOverflow)?;
    let end_delta = source_end
        .get()
        .checked_sub(run.source().start().get())
        .ok_or(ProjectionError::OffsetOverflow)?;
    let visual_start = shift_visual(
        run.visual()
            .start()
            .checked_add(start_delta)
            .ok_or(ProjectionError::OffsetOverflow)?,
        visual_delta,
    )?;
    let visual_end = shift_visual(
        run.visual()
            .start()
            .checked_add(end_delta)
            .ok_or(ProjectionError::OffsetOverflow)?,
        visual_delta,
    )?;
    Ok(VisualRun {
        source: TextRange::new(source_start, source_end).ok_or(ProjectionError::OffsetOverflow)?,
        visual: VisualRange::new(visual_start, visual_end)
            .ok_or(ProjectionError::OffsetOverflow)?,
        kind: run.kind(),
        style: run.style(),
    })
}

fn shift_run(run: VisualRun, visual_delta: i128) -> Result<VisualRun, ProjectionError> {
    Ok(VisualRun {
        source: run.source(),
        visual: VisualRange::new(
            shift_visual(run.visual().start(), visual_delta)?,
            shift_visual(run.visual().end(), visual_delta)?,
        )
        .ok_or(ProjectionError::OffsetOverflow)?,
        kind: run.kind(),
        style: run.style(),
    })
}

fn shift_visual(value: VisualOffset, delta: i128) -> Result<VisualOffset, ProjectionError> {
    if delta >= 0 {
        let delta = u64::try_from(delta).map_err(|_| ProjectionError::OffsetOverflow)?;
        value
            .checked_add(delta)
            .ok_or(ProjectionError::OffsetOverflow)
    } else {
        let delta = u64::try_from(-delta).map_err(|_| ProjectionError::OffsetOverflow)?;
        value
            .get()
            .checked_sub(delta)
            .map(VisualOffset::new)
            .ok_or(ProjectionError::OffsetOverflow)
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

fn map_table_cell(
    range: yu_markdown::TableCellRange,
    changes: &ChangeSet,
) -> Result<yu_markdown::TableCellRange, ProjectionError> {
    let mapped = map_range(byte_range(range.start(), range.end())?, changes)?;
    Ok(yu_markdown::TableCellRange::new(
        usize::try_from(mapped.start()).map_err(|_| ProjectionError::OffsetOverflow)?,
        usize::try_from(mapped.end()).map_err(|_| ProjectionError::OffsetOverflow)?,
    ))
}

fn map_table_row(
    range: yu_markdown::TableRowRange,
    changes: &ChangeSet,
) -> Result<TextRange, ProjectionError> {
    map_range(byte_range(range.start(), range.end())?, changes)
}

fn build_runs(
    source: &TextSnapshot,
    source_range: TextRange,
    hidden: &[TextRange],
    line_breaks: &[LineBreakSpec],
    spans: &[InlineSpan],
    default_style: VisualRunStyle,
) -> Result<Vec<VisualRun>, ProjectionError> {
    let mut runs = Vec::new();
    let mut source_cursor = source_range.start();
    let mut visual_cursor = VisualOffset::ZERO;
    let mut events = hidden
        .iter()
        .copied()
        .map(ProjectionEvent::Hidden)
        .collect::<Vec<_>>();
    for line_break in line_breaks {
        if hidden.iter().any(|range| {
            range.start() <= line_break.source.start() && line_break.source.end() <= range.end()
        }) {
            continue;
        }
        let (prefix, ending) = line_break_parts(source, *line_break)?;
        if let Some(prefix) = prefix {
            events.push(ProjectionEvent::Hidden(prefix));
        }
        events.push(ProjectionEvent::LineBreak {
            source: ending,
            hard: line_break.hard,
        });
    }
    events.sort_by_key(|event| {
        let range = event.source();
        (range.start(), range.end())
    });
    for event in events {
        let event_range = event.source();
        if event_range.start() < source_range.start() || event_range.end() > source_range.end() {
            return Err(ProjectionError::SourceOutsideRange {
                offset: event_range.start(),
                range: source_range,
            });
        }
        if event_range.start() < source_cursor {
            if event_range.end() <= source_cursor {
                continue;
            }
            return Err(ProjectionError::OffsetOverflow);
        }
        if event_range.start() > source_cursor {
            visual_cursor = append_visible_runs(
                &mut runs,
                source_cursor,
                event_range.start(),
                visual_cursor,
                spans,
                default_style,
            )?;
        }
        match event {
            ProjectionEvent::Hidden(range) => {
                runs.push(VisualRun {
                    source: range,
                    visual: VisualRange::empty(visual_cursor),
                    kind: VisualRunKind::HiddenSyntax,
                    style: VisualRunStyle::Plain,
                });
                source_cursor = range.end();
            }
            ProjectionEvent::LineBreak { source, hard } => {
                let visual_end = visual_cursor
                    .checked_add(source.len())
                    .ok_or(ProjectionError::OffsetOverflow)?;
                runs.push(VisualRun {
                    source,
                    visual: VisualRange::new(visual_cursor, visual_end)
                        .ok_or(ProjectionError::OffsetOverflow)?,
                    kind: VisualRunKind::LineBreak { hard },
                    style: VisualRunStyle::Plain,
                });
                visual_cursor = visual_end;
                source_cursor = event_range.end();
            }
        }
    }
    if source_cursor < source_range.end() {
        let _ = append_visible_runs(
            &mut runs,
            source_cursor,
            source_range.end(),
            visual_cursor,
            spans,
            default_style,
        )?;
    }
    Ok(runs)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ProjectionEvent {
    Hidden(TextRange),
    LineBreak { source: TextRange, hard: bool },
}

impl ProjectionEvent {
    fn source(self) -> TextRange {
        match self {
            Self::Hidden(source) | Self::LineBreak { source, .. } => source,
        }
    }
}

fn line_break_parts(
    source: &TextSnapshot,
    line_break: LineBreakSpec,
) -> Result<(Option<TextRange>, TextRange), ProjectionError> {
    let end = line_break.source.end();
    let tail_start = ByteOffset::new(
        end.get()
            .checked_sub(2)
            .filter(|candidate| *candidate >= line_break.source.start().get())
            .or_else(|| end.get().checked_sub(1))
            .ok_or(ProjectionError::OffsetOverflow)?,
    );
    let tail = TextRange::new(tail_start, end).ok_or(ProjectionError::OffsetOverflow)?;
    let bytes = read_range(source, tail)?;
    let ending_len = if bytes.as_slice() == b"\r\n" {
        2_u64
    } else {
        1_u64
    };
    let ending_start = ByteOffset::new(
        end.get()
            .checked_sub(ending_len)
            .ok_or(ProjectionError::OffsetOverflow)?,
    );
    if ending_start < line_break.source.start() {
        return Err(ProjectionError::OffsetOverflow);
    }
    let ending = TextRange::new(ending_start, end).ok_or(ProjectionError::OffsetOverflow)?;
    let prefix = TextRange::new(line_break.source.start(), ending_start)
        .ok_or(ProjectionError::OffsetOverflow)?;
    Ok(((!prefix.is_empty()).then_some(prefix), ending))
}

fn append_visible_runs(
    runs: &mut Vec<VisualRun>,
    start: ByteOffset,
    end: ByteOffset,
    mut visual_cursor: VisualOffset,
    spans: &[InlineSpan],
    default_style: VisualRunStyle,
) -> Result<VisualOffset, ProjectionError> {
    let mut boundaries = vec![start, end];
    for span in spans {
        let content = span.content();
        if content.start() > start && content.start() < end {
            boundaries.push(content.start());
        }
        if content.end() > start && content.end() < end {
            boundaries.push(content.end());
        }
    }
    boundaries.sort_unstable();
    boundaries.dedup();
    for pair in boundaries.windows(2) {
        let visible = TextRange::new(pair[0], pair[1]).ok_or(ProjectionError::OffsetOverflow)?;
        if visible.is_empty() {
            continue;
        }
        let visual_end = visual_cursor
            .checked_add(visible.len())
            .ok_or(ProjectionError::OffsetOverflow)?;
        runs.push(VisualRun {
            source: visible,
            visual: VisualRange::new(visual_cursor, visual_end)
                .ok_or(ProjectionError::OffsetOverflow)?,
            kind: VisualRunKind::Visible,
            style: style_for(visible, spans, default_style),
        });
        visual_cursor = visual_end;
    }
    Ok(visual_cursor)
}

fn style_for(
    range: TextRange,
    spans: &[InlineSpan],
    default_style: VisualRunStyle,
) -> VisualRunStyle {
    spans
        .iter()
        .filter(|span| {
            span.content().start() <= range.start() && range.end() <= span.content().end()
        })
        .min_by_key(|span| {
            (
                span.content().len(),
                match span.kind() {
                    InlineSpanKind::CodeSpan => 0_u8,
                    InlineSpanKind::Strong => 1,
                    InlineSpanKind::Emphasis => 2,
                    InlineSpanKind::Link
                    | InlineSpanKind::Image
                    | InlineSpanKind::ReferenceLink
                    | InlineSpanKind::ReferenceImage
                    | InlineSpanKind::Autolink => 3,
                },
            )
        })
        .map_or(default_style, |span| match span.kind() {
            InlineSpanKind::Emphasis => VisualRunStyle::Emphasis,
            InlineSpanKind::Strong => VisualRunStyle::Strong,
            InlineSpanKind::CodeSpan => VisualRunStyle::Code,
            InlineSpanKind::Link
            | InlineSpanKind::Image
            | InlineSpanKind::ReferenceLink
            | InlineSpanKind::ReferenceImage
            | InlineSpanKind::Autolink => default_style,
        })
}

/// The kind of visual block projection produced by the editor pipeline.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum BlockProjectionKind {
    Inline,
    Heading,
    BlockQuote,
    List,
    Table,
    FencedCode,
    ReferenceDefinition,
    TaskList,
}

/// A source-backed GFM table projection.
///
/// The visual stream is intentionally still the ordinary inline projection in
/// this stage.  `TableBlock` carries absolute source ranges for the header,
/// delimiter and body cells so a later retained table layout can replace the
/// pipes with a grid without creating a second document model.
#[derive(Clone, Debug)]
pub struct TableProjection {
    visual: Projection,
    table: TableBlock,
}

impl TableProjection {
    fn from_block(
        source: &TextSnapshot,
        block: Block,
        definitions: Option<&ReferenceDefinitionIndex>,
    ) -> Result<Option<Self>, ProjectionError> {
        let Some(table) = parse_table_in_snapshot(source, block.range()) else {
            return Ok(None);
        };
        let visual = match definitions {
            Some(definitions) => {
                Projection::inline_with_definitions(source, block.range(), definitions)?
            }
            None => Projection::inline(source, block.range())?,
        };
        Ok(Some(Self { visual, table }))
    }

    #[must_use]
    pub fn visual(&self) -> &Projection {
        &self.visual
    }

    #[must_use]
    pub fn table(&self) -> &TableBlock {
        &self.table
    }

    #[must_use]
    pub fn revision(&self) -> Revision {
        self.visual.revision()
    }

    #[must_use]
    pub fn source_range(&self) -> TextRange {
        self.visual.source_range()
    }

    pub fn with_composition(
        &self,
        replacement_range: TextRange,
        text: impl Into<Arc<str>>,
        selection_bytes: TextRange,
    ) -> Result<Self, ProjectionError> {
        Ok(Self {
            visual: self
                .visual
                .with_composition(replacement_range, text, selection_bytes)?,
            table: self.table.clone(),
        })
    }

    fn map_through(
        &self,
        changes: &ChangeSet,
        snapshot: &TextSnapshot,
    ) -> Result<Option<Self>, ProjectionError> {
        let Some(visual) = self.visual.map_through(changes, snapshot)? else {
            return Ok(None);
        };
        let source_range = map_table_cell(self.table.source_range(), changes)?;
        let header = self
            .table
            .header()
            .iter()
            .copied()
            .map(|range| map_table_cell(range, changes))
            .collect::<Result<Vec<_>, _>>()?;
        let delimiter = self
            .table
            .delimiter()
            .iter()
            .copied()
            .map(|range| map_table_cell(range, changes))
            .collect::<Result<Vec<_>, _>>()?;
        let rows = self
            .table
            .rows()
            .iter()
            .map(|row| {
                row.iter()
                    .copied()
                    .map(|range| map_table_cell(range, changes))
                    .collect::<Result<Vec<_>, _>>()
            })
            .collect::<Result<Vec<_>, _>>()?;
        let row_ranges = self
            .table
            .row_ranges()
            .iter()
            .copied()
            .map(|range| {
                let mapped = map_table_row(range, changes)?;
                Ok(yu_markdown::TableRowRange::new(
                    usize::try_from(mapped.start()).map_err(|_| ProjectionError::OffsetOverflow)?,
                    usize::try_from(mapped.end()).map_err(|_| ProjectionError::OffsetOverflow)?,
                ))
            })
            .collect::<Result<Vec<_>, ProjectionError>>()?;
        let table = TableBlock::from_mapped_ranges(
            source_range,
            header,
            delimiter,
            self.table.alignments().to_vec(),
            rows,
            row_ranges,
        );
        Ok(Some(Self { visual, table }))
    }
}

/// A source-backed fenced-code projection.
///
/// Fence lines are hidden from the visual stream, while the body is kept as a
/// code-styled visible run. Markdown inline delimiters inside the body are
/// never parsed or paired by this projection.
#[derive(Clone, Debug)]
pub struct CodeProjection {
    visual: Projection,
    opening_fence: TextRange,
    info_string: TextRange,
    content: TextRange,
    closing_fence: Option<TextRange>,
    marker: char,
    closed: bool,
}

impl CodeProjection {
    pub fn from_block(source: &TextSnapshot, block: Block) -> Result<Self, ProjectionError> {
        let BlockKind::FencedCodeBlock { marker, closed } = block.kind() else {
            return Err(ProjectionError::NotFencedCodeBlock { kind: block.kind() });
        };
        source.utf16_offset(block.range().start())?;
        source.utf16_offset(block.range().end())?;
        let lines = line_ranges(source, block.range())?;
        let opening_fence = lines
            .first()
            .copied()
            .ok_or(ProjectionError::InvalidCodeBlock {
                range: block.range(),
            })?;
        let closing_fence = if closed {
            lines.last().copied().filter(|line| *line != opening_fence)
        } else {
            None
        };
        let content_start = opening_fence.end();
        let content_end = closing_fence.map_or(block.range().end(), TextRange::start);
        let content = TextRange::new(content_start, content_end).ok_or(
            ProjectionError::InvalidCodeBlock {
                range: block.range(),
            },
        )?;
        let info_string = code_info_range(source, opening_fence, marker)?;
        let mut hidden = vec![opening_fence];
        if let Some(closing) = closing_fence {
            hidden.push(closing);
        }
        let visual = Projection::from_source_parts(
            source,
            block.range(),
            &hidden,
            &[],
            &[],
            VisualRunStyle::Code,
        )?;
        Ok(Self {
            visual,
            opening_fence,
            info_string,
            content,
            closing_fence,
            marker,
            closed,
        })
    }

    #[must_use]
    pub fn visual(&self) -> &Projection {
        &self.visual
    }

    #[must_use]
    pub fn revision(&self) -> Revision {
        self.visual.revision()
    }

    #[must_use]
    pub fn source_range(&self) -> TextRange {
        self.visual.source_range()
    }

    #[must_use]
    pub const fn opening_fence(&self) -> TextRange {
        self.opening_fence
    }

    #[must_use]
    pub const fn info_string(&self) -> TextRange {
        self.info_string
    }

    #[must_use]
    pub const fn content(&self) -> TextRange {
        self.content
    }

    #[must_use]
    pub const fn closing_fence(&self) -> Option<TextRange> {
        self.closing_fence
    }

    #[must_use]
    pub const fn marker(&self) -> char {
        self.marker
    }

    #[must_use]
    pub const fn closed(&self) -> bool {
        self.closed
    }

    pub fn map_through(
        &self,
        changes: &ChangeSet,
        snapshot: &TextSnapshot,
    ) -> Result<Option<Self>, ProjectionError> {
        let Some(visual) = self.visual.map_through(changes, snapshot)? else {
            return Ok(None);
        };
        let opening_fence = map_range(self.opening_fence, changes)?;
        let info_string = map_range(self.info_string, changes)?;
        let content = map_range(self.content, changes)?;
        let closing_fence = self
            .closing_fence
            .map(|range| map_range(range, changes))
            .transpose()?;
        Ok(Some(Self {
            visual,
            opening_fence,
            info_string,
            content,
            closing_fence,
            marker: self.marker,
            closed: self.closed,
        }))
    }

    /// Projects transient IME text over the code block without changing the
    /// canonical source or the fenced-code metadata.
    pub fn with_composition(
        &self,
        replacement_range: TextRange,
        text: impl Into<Arc<str>>,
        selection_bytes: TextRange,
    ) -> Result<Self, ProjectionError> {
        Ok(Self {
            visual: self
                .visual
                .with_composition(replacement_range, text, selection_bytes)?,
            opening_fence: self.opening_fence,
            info_string: self.info_string,
            content: self.content,
            closing_fence: self.closing_fence,
            marker: self.marker,
            closed: self.closed,
        })
    }
}

/// A block projection selected from the Markdown block sequence.
#[derive(Clone, Debug)]
pub enum BlockProjection {
    Inline(Projection),
    Heading(Projection),
    BlockQuote(Projection),
    List(Projection),
    Table(TableProjection),
    FencedCode(CodeProjection),
    ReferenceDefinition(Projection),
    TaskList(Projection),
}

impl BlockProjection {
    pub fn from_block(source: &TextSnapshot, block: Block) -> Result<Self, ProjectionError> {
        match block.kind() {
            BlockKind::ReferenceDefinition => {
                Projection::hidden(source, block.range()).map(Self::ReferenceDefinition)
            }
            BlockKind::TaskListItem { .. } => Self::task_list(source, block, None),
            BlockKind::Paragraph => {
                if let Some(table) = TableProjection::from_block(source, block, None)? {
                    Ok(Self::Table(table))
                } else {
                    Self::from_block_without_definitions(source, block)
                }
            }
            _ => Self::from_block_without_definitions(source, block),
        }
    }

    /// Builds a block projection using a revision-bound reference definition
    /// index. Definition blocks stay hidden; ordinary inline blocks resolve
    /// shortcut references against the same source revision.
    pub fn from_block_with_definitions(
        source: &TextSnapshot,
        block: Block,
        definitions: &ReferenceDefinitionIndex,
    ) -> Result<Self, ProjectionError> {
        match block.kind() {
            BlockKind::ReferenceDefinition => {
                Projection::hidden(source, block.range()).map(Self::ReferenceDefinition)
            }
            BlockKind::TaskListItem { .. } => Self::task_list(source, block, Some(definitions)),
            BlockKind::FencedCodeBlock { .. } => {
                CodeProjection::from_block(source, block).map(Self::FencedCode)
            }
            BlockKind::AtxHeading { .. } => {
                Self::structural_inline(source, block, definitions, BlockProjectionKind::Heading)
            }
            BlockKind::BlockQuote { .. } => {
                Self::structural_inline(source, block, definitions, BlockProjectionKind::BlockQuote)
            }
            BlockKind::ListItem { .. } => {
                Self::structural_inline(source, block, definitions, BlockProjectionKind::List)
            }
            BlockKind::Paragraph => {
                if let Some(table) = TableProjection::from_block(source, block, Some(definitions))?
                {
                    Ok(Self::Table(table))
                } else {
                    Projection::inline_with_definitions(source, block.range(), definitions)
                        .map(Self::Inline)
                }
            }
            _ => Projection::inline_with_definitions(source, block.range(), definitions)
                .map(Self::Inline),
        }
    }

    /// Projects transient IME text over this block's visual projection. The
    /// block metadata remains source-backed; only the visual projection is
    /// replaced for the duration of the composition.
    pub fn with_composition(
        &self,
        replacement_range: TextRange,
        text: impl Into<Arc<str>>,
        selection_bytes: TextRange,
    ) -> Result<Self, ProjectionError> {
        match self {
            Self::Inline(projection) => projection
                .with_composition(replacement_range, text, selection_bytes)
                .map(Self::Inline),
            Self::Heading(projection) => projection
                .with_composition(replacement_range, text, selection_bytes)
                .map(Self::Heading),
            Self::BlockQuote(projection) => projection
                .with_composition(replacement_range, text, selection_bytes)
                .map(Self::BlockQuote),
            Self::List(projection) => projection
                .with_composition(replacement_range, text, selection_bytes)
                .map(Self::List),
            Self::Table(projection) => projection
                .with_composition(replacement_range, text, selection_bytes)
                .map(Self::Table),
            Self::FencedCode(projection) => projection
                .with_composition(replacement_range, text, selection_bytes)
                .map(Self::FencedCode),
            Self::ReferenceDefinition(projection) => projection
                .with_composition(replacement_range, text, selection_bytes)
                .map(Self::ReferenceDefinition),
            Self::TaskList(projection) => projection
                .with_composition(replacement_range, text, selection_bytes)
                .map(Self::TaskList),
        }
    }

    fn task_list(
        source: &TextSnapshot,
        block: Block,
        definitions: Option<&ReferenceDefinitionIndex>,
    ) -> Result<Self, ProjectionError> {
        let marker = yu_markdown::task_marker(source, block).ok_or(
            ProjectionError::InvalidTaskListBlock {
                range: block.range(),
            },
        )?;
        let inline = match definitions {
            Some(definitions) => {
                parse_inline_with_definitions(source, block.range(), Some(definitions))?
            }
            None => parse_inline(source, block.range())?,
        };
        Projection::from_inline_with_hidden(&inline, &[marker.range()]).map(Self::TaskList)
    }

    fn structural_inline(
        source: &TextSnapshot,
        block: Block,
        definitions: &ReferenceDefinitionIndex,
        kind: BlockProjectionKind,
    ) -> Result<Self, ProjectionError> {
        let inline = parse_inline_with_definitions(source, block.range(), Some(definitions))?;
        let projection = Projection::from_inline_with_hidden(
            &inline,
            &yu_markdown::block_syntax_hidden_ranges(source, block),
        )?;
        Ok(match kind {
            BlockProjectionKind::Heading => Self::Heading(projection),
            BlockProjectionKind::BlockQuote => Self::BlockQuote(projection),
            BlockProjectionKind::List => Self::List(projection),
            _ => Self::Inline(projection),
        })
    }

    fn from_block_without_definitions(
        source: &TextSnapshot,
        block: Block,
    ) -> Result<Self, ProjectionError> {
        match block.kind() {
            BlockKind::FencedCodeBlock { .. } => {
                CodeProjection::from_block(source, block).map(Self::FencedCode)
            }
            BlockKind::AtxHeading { .. } => {
                let inline = parse_inline(source, block.range())?;
                let projection = Projection::from_inline_with_hidden(
                    &inline,
                    &yu_markdown::block_syntax_hidden_ranges(source, block),
                )?;
                Ok(Self::Heading(projection))
            }
            BlockKind::BlockQuote { .. } => {
                let inline = parse_inline(source, block.range())?;
                let projection = Projection::from_inline_with_hidden(
                    &inline,
                    &yu_markdown::block_syntax_hidden_ranges(source, block),
                )?;
                Ok(Self::BlockQuote(projection))
            }
            BlockKind::ListItem { .. } => {
                let inline = parse_inline(source, block.range())?;
                Projection::from_inline(&inline).map(Self::List)
            }
            BlockKind::Paragraph => Projection::inline(source, block.range()).map(Self::Inline),
            _ => Projection::inline(source, block.range()).map(Self::Inline),
        }
    }

    #[must_use]
    pub const fn kind(&self) -> BlockProjectionKind {
        match self {
            Self::Inline(_) => BlockProjectionKind::Inline,
            Self::Heading(_) => BlockProjectionKind::Heading,
            Self::BlockQuote(_) => BlockProjectionKind::BlockQuote,
            Self::List(_) => BlockProjectionKind::List,
            Self::Table(_) => BlockProjectionKind::Table,
            Self::FencedCode(_) => BlockProjectionKind::FencedCode,
            Self::ReferenceDefinition(_) => BlockProjectionKind::ReferenceDefinition,
            Self::TaskList(_) => BlockProjectionKind::TaskList,
        }
    }

    #[must_use]
    pub fn visual(&self) -> &Projection {
        match self {
            Self::Inline(projection) => projection,
            Self::Heading(projection) => projection,
            Self::BlockQuote(projection) => projection,
            Self::List(projection) => projection,
            Self::Table(projection) => projection.visual(),
            Self::FencedCode(projection) => projection.visual(),
            Self::ReferenceDefinition(projection) => projection,
            Self::TaskList(projection) => projection,
        }
    }

    #[must_use]
    pub fn revision(&self) -> Revision {
        self.visual().revision()
    }

    #[must_use]
    pub fn source_range(&self) -> TextRange {
        self.visual().source_range()
    }

    /// Carries a block projection through an edit outside its source range.
    pub fn map_through(
        &self,
        changes: &ChangeSet,
        snapshot: &TextSnapshot,
    ) -> Result<Option<Self>, ProjectionError> {
        match self {
            Self::Inline(projection) => {
                Ok(projection.map_through(changes, snapshot)?.map(Self::Inline))
            }
            Self::Heading(projection) => Ok(projection
                .map_through(changes, snapshot)?
                .map(Self::Heading)),
            Self::BlockQuote(projection) => Ok(projection
                .map_through(changes, snapshot)?
                .map(Self::BlockQuote)),
            Self::List(projection) => {
                Ok(projection.map_through(changes, snapshot)?.map(Self::List))
            }
            Self::Table(projection) => {
                Ok(projection.map_through(changes, snapshot)?.map(Self::Table))
            }
            Self::FencedCode(projection) => Ok(projection
                .map_through(changes, snapshot)?
                .map(Self::FencedCode)),
            Self::ReferenceDefinition(projection) => Ok(projection
                .map_through(changes, snapshot)?
                .map(Self::ReferenceDefinition)),
            Self::TaskList(projection) => Ok(projection
                .map_through(changes, snapshot)?
                .map(Self::TaskList)),
        }
    }
}

fn line_ranges(source: &TextSnapshot, range: TextRange) -> Result<Vec<TextRange>, ProjectionError> {
    let start = usize::try_from(range.start()).map_err(|_| ProjectionError::OffsetOverflow)?;
    let end = usize::try_from(range.end()).map_err(|_| ProjectionError::OffsetOverflow)?;
    let mut cursor = source.chunk_cursor(range.start())?;
    let mut line_start = start;
    let mut lines = Vec::new();
    for chunk in &mut cursor {
        let chunk_start =
            usize::try_from(chunk.start()).map_err(|_| ProjectionError::OffsetOverflow)?;
        let chunk_end = chunk_start
            .checked_add(chunk.text().len())
            .ok_or(ProjectionError::OffsetOverflow)?;
        let local_start = start.max(chunk_start).saturating_sub(chunk_start);
        let local_end = end.min(chunk_end).saturating_sub(chunk_start);
        for (index, byte) in chunk.text().as_bytes()[local_start..local_end]
            .iter()
            .enumerate()
        {
            if *byte != b'\n' {
                continue;
            }
            let absolute = chunk_start
                .checked_add(local_start)
                .and_then(|offset| offset.checked_add(index + 1))
                .ok_or(ProjectionError::OffsetOverflow)?;
            lines.push(byte_range(line_start, absolute)?);
            line_start = absolute;
        }
    }
    if line_start < end {
        lines.push(byte_range(line_start, end)?);
    }
    Ok(lines)
}

fn code_info_range(
    source: &TextSnapshot,
    opening: TextRange,
    marker: char,
) -> Result<TextRange, ProjectionError> {
    let bytes = read_range(source, opening)?;
    let mut line_end = bytes.len();
    while line_end > 0 && matches!(bytes[line_end - 1], b'\n' | b'\r') {
        line_end -= 1;
    }
    let marker = u8::try_from(marker as u32).map_err(|_| ProjectionError::OffsetOverflow)?;
    let mut info_start = 0_usize;
    while info_start < line_end && bytes[info_start] == b' ' {
        info_start += 1;
    }
    while info_start < line_end && bytes[info_start] == marker {
        info_start += 1;
    }
    while info_start < line_end && matches!(bytes[info_start], b' ' | b'\t') {
        info_start += 1;
    }
    let mut info_end = line_end;
    while info_end > info_start && matches!(bytes[info_end - 1], b' ' | b'\t') {
        info_end -= 1;
    }
    let start = usize::try_from(opening.start())
        .map_err(|_| ProjectionError::OffsetOverflow)?
        .checked_add(info_start)
        .ok_or(ProjectionError::OffsetOverflow)?;
    let end = usize::try_from(opening.start())
        .map_err(|_| ProjectionError::OffsetOverflow)?
        .checked_add(info_end)
        .ok_or(ProjectionError::OffsetOverflow)?;
    byte_range(start, end)
}

fn read_range(source: &TextSnapshot, range: TextRange) -> Result<Vec<u8>, ProjectionError> {
    let start = usize::try_from(range.start()).map_err(|_| ProjectionError::OffsetOverflow)?;
    let end = usize::try_from(range.end()).map_err(|_| ProjectionError::OffsetOverflow)?;
    let mut bytes = Vec::with_capacity(end.saturating_sub(start));
    let mut cursor = source.chunk_cursor(range.start())?;
    for chunk in &mut cursor {
        let chunk_start =
            usize::try_from(chunk.start()).map_err(|_| ProjectionError::OffsetOverflow)?;
        let chunk_end = chunk_start
            .checked_add(chunk.text().len())
            .ok_or(ProjectionError::OffsetOverflow)?;
        if chunk_start >= end {
            break;
        }
        let local_start = start.max(chunk_start).saturating_sub(chunk_start);
        let local_end = end.min(chunk_end).saturating_sub(chunk_start);
        if local_start < local_end {
            bytes.extend_from_slice(&chunk.text().as_bytes()[local_start..local_end]);
        }
    }
    Ok(bytes)
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
    fn composition_projects_preedit_without_mutating_source_coordinates() {
        let source = "hello world";
        let projection = projection(source);
        let replacement =
            TextRange::new(ByteOffset::new(6), ByteOffset::new(11)).expect("replacement range");
        let selection =
            TextRange::new(ByteOffset::new(3), ByteOffset::new(3)).expect("selection range");
        let composed = projection
            .with_composition(replacement, "日本", selection)
            .expect("composition projection");

        assert_eq!(composed.revision(), projection.revision());
        assert_eq!(composed.source().as_str(), source);
        assert_eq!(composed.composition_text(), Some("日本"));
        assert_eq!(composed.composition_range(), Some(replacement));
        assert_eq!(composed.visual_len().get(), 11 - 5 + "日本".len() as u64);
        assert_eq!(
            composed
                .text_for_run(
                    *composed
                        .runs()
                        .iter()
                        .find(|run| run.kind() == VisualRunKind::Composition)
                        .expect("composition run")
                )
                .expect("preedit text"),
            "日本"
        );
        assert_eq!(
            composed
                .source_to_visual(ByteOffset::new(6), ProjectionBias::Before)
                .expect("source boundary"),
            VisualOffset::new(6)
        );
        assert_eq!(
            composed
                .source_to_visual(ByteOffset::new(6), ProjectionBias::After)
                .expect("source boundary"),
            VisualOffset::new(6)
        );
        assert_eq!(
            composed
                .visual_to_source(VisualOffset::new(7), ProjectionBias::Before)
                .expect("visual boundary"),
            ByteOffset::new(6)
        );
        assert_eq!(
            composed.composition_selection_visual(),
            Some(VisualRange::empty(VisualOffset::new(9)))
        );
    }

    #[test]
    fn composition_rejects_non_boundary_selection() {
        let projection = projection("hello");
        let replacement = TextRange::empty(ByteOffset::new(5));
        let selection =
            TextRange::new(ByteOffset::new(1), ByteOffset::new(2)).expect("selection range");
        assert!(matches!(
            projection.with_composition(replacement, "羽", selection),
            Err(ProjectionError::CompositionSelectionNotUtf8Boundary { .. })
        ));
    }

    #[test]
    fn composition_shifts_suffix_visual_ranges_without_changing_source_ranges() {
        let projection = projection("hello world again");
        let replacement =
            TextRange::new(ByteOffset::new(6), ByteOffset::new(11)).expect("replacement range");
        let selection = TextRange::empty(ByteOffset::ZERO);
        let composed = projection
            .with_composition(replacement, "x", selection)
            .expect("composition projection");

        assert_eq!(composed.visual_len(), VisualOffset::new(13));
        assert_eq!(
            composed
                .source_to_visual(ByteOffset::new(12), ProjectionBias::After)
                .expect("suffix source mapping"),
            VisualOffset::new(8)
        );
        assert_eq!(
            composed
                .visual_to_source(VisualOffset::new(8), ProjectionBias::After)
                .expect("suffix visual mapping"),
            ByteOffset::new(12)
        );
        assert!(composed.runs().iter().any(|run| {
            run.kind() == VisualRunKind::Composition
                && run.visual()
                    == VisualRange::new(VisualOffset::new(6), VisualOffset::new(7))
                        .expect("composition visual range")
        }));
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
    fn projection_hides_link_syntax_but_keeps_label_visible() {
        let projection = projection("[Yu](https://example.com)");
        let hidden = projection
            .runs()
            .iter()
            .filter(|run| run.kind() == VisualRunKind::HiddenSyntax)
            .count();
        assert_eq!(hidden, 2);
        assert_eq!(projection.visual_len().get(), 2);
        assert_eq!(
            projection
                .visual_to_source(VisualOffset::new(0), ProjectionBias::After)
                .expect("label start should map after hidden syntax"),
            ByteOffset::new(1)
        );
    }

    #[test]
    fn line_breaks_inside_hidden_link_tail_do_not_escape_as_visual_breaks() {
        let projection = projection("[label](url\nnext)");
        assert_eq!(
            projection
                .runs()
                .iter()
                .filter(|run| run.is_line_break())
                .count(),
            0
        );
        assert_eq!(projection.visual_len(), VisualOffset::new(5));
        assert_eq!(
            projection
                .runs()
                .iter()
                .filter(|run| run.kind() == VisualRunKind::HiddenSyntax)
                .count(),
            2
        );
    }

    #[test]
    fn projection_hides_reference_and_autolink_angles_but_keeps_content() {
        let source = "[Yu][project] <https://example.com> <dev@example.com>";
        let projection = projection(source);
        let hidden = projection
            .runs()
            .iter()
            .filter(|run| run.kind() == VisualRunKind::HiddenSyntax)
            .count();
        assert_eq!(hidden, 6);
        assert_eq!(projection.visual_len(), VisualOffset::new(38));
        assert_eq!(
            projection
                .source_to_visual(ByteOffset::new(1), ProjectionBias::After)
                .expect("reference label should map"),
            VisualOffset::new(0)
        );
        assert_eq!(
            projection
                .visual_to_source(VisualOffset::new(2), ProjectionBias::After)
                .expect("after reference label should map to the next source"),
            ByteOffset::new(13)
        );
    }

    #[test]
    fn definition_aware_projection_hides_shortcut_syntax() {
        let source = "[id]: /docs\n\n[id] ![id]\n";
        let buffer = TextBuffer::new(source);
        let snapshot = buffer.snapshot();
        let markdown = yu_markdown::parse(&snapshot);
        let paragraph = markdown.blocks().get(2).expect("paragraph should exist");
        let projection = Projection::inline_with_definitions(
            &snapshot,
            paragraph.range(),
            markdown.reference_definitions(),
        )
        .expect("definition-aware projection should build");

        assert_eq!(projection.visual_len(), VisualOffset::new(6));
        assert_eq!(
            projection
                .runs()
                .iter()
                .filter(|run| run.kind() == VisualRunKind::HiddenSyntax)
                .count(),
            4
        );

        let definition = markdown.blocks().get(0).expect("definition should exist");
        let hidden = BlockProjection::from_block_with_definitions(
            &snapshot,
            definition,
            markdown.reference_definitions(),
        )
        .expect("definition block projection should build");
        assert_eq!(hidden.kind(), BlockProjectionKind::ReferenceDefinition);
        assert_eq!(hidden.visual().visual_len(), VisualOffset::ZERO);
        assert_eq!(hidden.visual().source_range(), definition.range());
    }

    #[test]
    fn task_list_projection_hides_marker_but_keeps_item_text() {
        let source = "- [ ] task **text**\n";
        let snapshot = TextBuffer::new(source).snapshot();
        let markdown = yu_markdown::parse(&snapshot);
        let block = markdown.blocks().get(0).expect("task block should exist");
        assert!(matches!(
            block.kind(),
            BlockKind::TaskListItem {
                state: yu_markdown::TaskState::Todo,
                ..
            }
        ));
        let projection = BlockProjection::from_block_with_definitions(
            &snapshot,
            block,
            markdown.reference_definitions(),
        )
        .expect("task projection should build");
        assert_eq!(projection.kind(), BlockProjectionKind::TaskList);
        assert_eq!(
            projection.visual().visual_len().get(),
            source.len() as u64 - 3 - 4
        );
        assert!(projection.visual().runs().iter().any(|run| {
            run.kind() == VisualRunKind::HiddenSyntax
                && run.source()
                    == yu_markdown::task_marker(&snapshot, block)
                        .expect("marker")
                        .range()
        }));
        assert!(projection.visual().runs().iter().any(|run| {
            run.kind() == VisualRunKind::Visible && run.style() == VisualRunStyle::Strong
        }));
    }

    #[test]
    fn structural_block_projection_hides_heading_and_quote_prefixes() {
        let source = "  ## **标题**\n\n> 引用\n> **继续**\n\n- item\n";
        let snapshot = TextBuffer::new(source).snapshot();
        let markdown = yu_markdown::parse(&snapshot);

        let heading = markdown.blocks().get(0).expect("heading block");
        let heading_projection = BlockProjection::from_block_with_definitions(
            &snapshot,
            heading,
            markdown.reference_definitions(),
        )
        .expect("heading projection should build");
        assert_eq!(heading_projection.kind(), BlockProjectionKind::Heading);
        assert_eq!(
            heading_projection
                .visual()
                .runs()
                .iter()
                .filter(|run| run.kind() == VisualRunKind::HiddenSyntax)
                .count(),
            3
        );
        let heading_text = heading_projection
            .visual()
            .runs()
            .iter()
            .filter(|run| run.kind() != VisualRunKind::HiddenSyntax)
            .map(|run| heading_projection.visual().text_for_run(*run))
            .collect::<Result<String, _>>()
            .expect("heading visual text");
        assert_eq!(heading_text, "标题\n");

        let quote = markdown.blocks().get(2).expect("quote block");
        let quote_projection = BlockProjection::from_block_with_definitions(
            &snapshot,
            quote,
            markdown.reference_definitions(),
        )
        .expect("quote projection should build");
        assert_eq!(quote_projection.kind(), BlockProjectionKind::BlockQuote);
        let quote_text = quote_projection
            .visual()
            .runs()
            .iter()
            .filter(|run| run.kind() != VisualRunKind::HiddenSyntax)
            .map(|run| quote_projection.visual().text_for_run(*run))
            .collect::<Result<String, _>>()
            .expect("quote visual text");
        assert_eq!(quote_text, "引用\n继续\n");

        let list = markdown.blocks().get(4).expect("list block");
        let list_projection = BlockProjection::from_block_with_definitions(
            &snapshot,
            list,
            markdown.reference_definitions(),
        )
        .expect("list projection should build");
        assert_eq!(list_projection.kind(), BlockProjectionKind::List);
        assert!(list_projection.visual().runs().iter().any(|run| {
            run.kind() == VisualRunKind::Visible && run.source().start() == list.range().start()
        }));
    }

    #[test]
    fn table_projection_keeps_absolute_source_cell_ranges() {
        let source = "intro\n\n| A | B |\n| --- | :---: |\n| 1 | 2 |\n";
        let snapshot = TextBuffer::new(source).snapshot();
        let markdown = yu_markdown::parse(&snapshot);
        let table_block = markdown.blocks().get(2).expect("table paragraph");
        let projection = BlockProjection::from_block_with_definitions(
            &snapshot,
            table_block,
            markdown.reference_definitions(),
        )
        .expect("table projection should build");
        assert_eq!(projection.kind(), BlockProjectionKind::Table);
        let BlockProjection::Table(table) = &projection else {
            panic!("expected table projection");
        };
        assert_eq!(table.table().column_count(), 2);
        assert_eq!(table.table().body_row_count(), 1);
        assert_eq!(
            table.table().source_range().start(),
            table_block.range().start().get() as usize
        );
        assert_eq!(
            table.table().header()[0].start(),
            source.find("A").expect("header")
        );
        assert_eq!(
            table.table().rows()[0][1].end(),
            source.rfind('2').expect("body") + 1
        );
        let projected = projection
            .visual()
            .runs()
            .iter()
            .filter(|run| run.kind() != VisualRunKind::HiddenSyntax)
            .map(|run| projection.visual().text_for_run(*run))
            .collect::<Result<String, _>>()
            .expect("table visual text");
        assert!(projected.contains("| A | B |"));

        let mut buffer = TextBuffer::new(source);
        let edit = yu_text::Transaction::new(
            buffer.revision(),
            [yu_text::Edit::new(
                TextRange::empty(ByteOffset::ZERO),
                "前缀\n",
            )],
        );
        let applied = buffer.apply(&edit).expect("prefix edit should apply");
        let mapped = projection
            .map_through(applied.change_set(), applied.result_snapshot())
            .expect("table mapping should succeed")
            .expect("outside edit should retain table projection");
        let BlockProjection::Table(mapped_table) = mapped else {
            panic!("mapped projection should remain a table");
        };
        assert_eq!(
            mapped_table.table().header()[0].start(),
            table.table().header()[0].start() + 7
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
    fn projection_exposes_source_backed_image_metadata_and_maps_it() {
        let source = "![logo](assets/yu.png) and ![mark][asset]\n[asset]: icons/yu.png\n";
        let mut buffer = TextBuffer::with_backend(source, StorageBackend::PieceTree);
        let snapshot = buffer.snapshot();
        let range = TextRange::new(ByteOffset::ZERO, snapshot.len_bytes())
            .expect("source range should be ordered");
        let inline = Projection::inline(&snapshot, range).expect("projection should build");
        assert_eq!(inline.images().len(), 2);
        let image = inline.images()[0];
        let destination = image.destination().expect("destination");
        let destination_start = usize::try_from(destination.start().get()).expect("start");
        let destination_end = usize::try_from(destination.end().get()).expect("end");
        assert_eq!(&source[destination_start..destination_end], "assets/yu.png");
        assert_eq!(
            image.label(),
            TextRange::new(ByteOffset::new(2), ByteOffset::new(6)).expect("label")
        );
        assert!(!image.is_reference());
        assert!(inline.images()[1].is_reference());

        let markdown = yu_markdown::parse(&snapshot);
        let paragraph = markdown.blocks().get(0).expect("paragraph block").range();
        let paragraph_projection = Projection::inline_with_definitions(
            &snapshot,
            paragraph,
            markdown.reference_definitions(),
        )
        .expect("definition-aware projection should build");
        assert_eq!(paragraph_projection.images().len(), 2);
        assert!(paragraph_projection.images()[1].is_reference());
        let transaction = Transaction::new(
            buffer.revision(),
            [Edit::new(TextRange::empty(snapshot.len_bytes()), "x")],
        );
        let applied = buffer
            .apply(&transaction)
            .expect("prefix edit should apply");
        let mapped = paragraph_projection
            .map_through(applied.change_set(), applied.result_snapshot())
            .expect("map should succeed")
            .expect("outside edit should preserve projection");
        assert_eq!(mapped.images().len(), 2);
        assert_eq!(
            mapped.images()[0].source(),
            paragraph_projection.images()[0].source()
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
        assert_eq!(projection.runs().len(), 3);
        assert_eq!(projection.runs()[0].kind(), VisualRunKind::Visible);
        assert_eq!(
            projection.runs()[1].kind(),
            VisualRunKind::LineBreak { hard: false }
        );
        assert_eq!(projection.runs()[2].kind(), VisualRunKind::Visible);
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
    fn projection_materializes_soft_and_hard_line_break_runs() {
        let source = "a\nb  \nc\\\r\nd";
        let projection = projection(source);
        let line_breaks = projection
            .runs()
            .iter()
            .filter(|run| run.is_line_break())
            .copied()
            .collect::<Vec<_>>();

        assert_eq!(line_breaks.len(), 3);
        assert_eq!(
            line_breaks[0].kind(),
            VisualRunKind::LineBreak { hard: false }
        );
        assert_eq!(
            line_breaks[0].source(),
            TextRange::new(ByteOffset::new(1), ByteOffset::new(2)).expect("range")
        );
        assert_eq!(
            line_breaks[1].kind(),
            VisualRunKind::LineBreak { hard: true }
        );
        assert_eq!(
            line_breaks[1].source(),
            TextRange::new(ByteOffset::new(5), ByteOffset::new(6)).expect("range")
        );
        assert_eq!(
            line_breaks[2].kind(),
            VisualRunKind::LineBreak { hard: true }
        );
        assert_eq!(
            line_breaks[2].source(),
            TextRange::new(ByteOffset::new(8), ByteOffset::new(10)).expect("range")
        );
        assert_eq!(
            projection
                .runs()
                .iter()
                .filter(|run| run.kind() == VisualRunKind::HiddenSyntax)
                .map(|run| run.source())
                .collect::<Vec<_>>(),
            vec![
                TextRange::new(ByteOffset::new(3), ByteOffset::new(5)).expect("range"),
                TextRange::new(ByteOffset::new(7), ByteOffset::new(8)).expect("range"),
            ]
        );
        assert_eq!(projection.visual_len(), VisualOffset::new(8));
        assert_eq!(
            projection
                .source_to_visual(ByteOffset::new(3), ProjectionBias::After)
                .expect("hard-break marker should map"),
            VisualOffset::new(3)
        );
        assert_eq!(
            projection
                .source_to_visual(ByteOffset::new(6), ProjectionBias::After)
                .expect("after hard break should map"),
            VisualOffset::new(4)
        );
        assert_eq!(
            projection
                .visual_to_source(VisualOffset::new(3), ProjectionBias::Before)
                .expect("before hidden hard-break syntax should map"),
            ByteOffset::new(3)
        );
        assert_eq!(
            projection
                .visual_to_source(VisualOffset::new(3), ProjectionBias::After)
                .expect("after hidden hard-break syntax should map"),
            ByteOffset::new(5)
        );
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

    #[test]
    fn projection_consumes_semantic_spans_and_carries_visible_styles() {
        let projection = projection("**strong** _emphasis_ `code` plain");
        let visible_styles = projection
            .runs()
            .iter()
            .filter(|run| run.kind() == VisualRunKind::Visible)
            .map(|run| run.style())
            .collect::<Vec<_>>();

        assert_eq!(
            visible_styles,
            vec![
                VisualRunStyle::Strong,
                VisualRunStyle::Plain,
                VisualRunStyle::Emphasis,
                VisualRunStyle::Plain,
                VisualRunStyle::Code,
                VisualRunStyle::Plain,
            ]
        );
        assert_eq!(
            projection
                .runs()
                .iter()
                .filter(|run| run.kind() == VisualRunKind::HiddenSyntax)
                .count(),
            6
        );
    }

    #[test]
    fn fenced_code_projection_hides_fences_and_keeps_body_literal() {
        let source_text = "```rust\n**code**\nvalue\n```\n";
        let buffer = TextBuffer::with_backend(source_text, StorageBackend::PieceTree);
        let snapshot = buffer.snapshot();
        let markdown = yu_markdown::parse(&snapshot);
        let block = markdown.blocks().get(0).expect("code block should exist");
        let code = CodeProjection::from_block(&snapshot, block).expect("code projection builds");

        assert_eq!(code.marker(), '`');
        assert!(code.closed());
        assert_eq!(code.source_range(), block.range());
        assert_eq!(
            &snapshot.as_str()[usize::try_from(code.info_string().start())
                .expect("info start should fit")
                ..usize::try_from(code.info_string().end()).expect("info end should fit")],
            "rust"
        );
        assert_eq!(
            &snapshot.as_str()[usize::try_from(code.content().start())
                .expect("content start should fit")
                ..usize::try_from(code.content().end()).expect("content end should fit")],
            "**code**\nvalue\n"
        );
        assert_eq!(code.visual().visual_len().get(), code.content().len());
        assert_eq!(
            code.visual()
                .runs()
                .iter()
                .filter(|run| run.kind() == VisualRunKind::HiddenSyntax)
                .count(),
            2
        );
        assert!(code.visual().runs().iter().any(|run| {
            run.kind() == VisualRunKind::Visible && run.style() == VisualRunStyle::Code
        }));
        assert_eq!(
            code.visual()
                .source_to_visual(code.content().start(), ProjectionBias::After)
                .expect("content start should map after opening fence"),
            VisualOffset::ZERO
        );
    }

    #[test]
    fn unclosed_fenced_code_projection_owns_body_until_eof() {
        let source_text = "~~~python\nprint('羽')\n";
        let buffer = TextBuffer::new(source_text);
        let snapshot = buffer.snapshot();
        let markdown = yu_markdown::parse(&snapshot);
        let block = markdown.blocks().get(0).expect("code block should exist");
        let code = CodeProjection::from_block(&snapshot, block).expect("code projection builds");

        assert!(!code.closed());
        assert_eq!(code.closing_fence(), None);
        assert_eq!(code.content().end(), block.range().end());
        assert_eq!(code.visual().visual_len().get(), code.content().len());
    }
}
