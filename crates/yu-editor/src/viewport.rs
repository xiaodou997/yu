use std::error::Error;
use std::fmt;

use yu_core::{Affinity, ByteOffset, Revision, TextAnchor, TextRange};
use yu_layout::{HeightIndex, HeightIndexError, LayoutConfig};
use yu_markdown::{BlockKind, MarkdownDocument};
use yu_text::{AnchorMapError, ChangeSet, TextSnapshot};

use crate::LayoutBackend;

/// Configuration shared by block layout measurement and viewport estimation.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ViewportConfig {
    layout: LayoutConfig,
    estimated_block_height: f32,
    overscan: f32,
}

impl ViewportConfig {
    #[must_use]
    pub const fn new(layout: LayoutConfig, estimated_block_height: f32, overscan: f32) -> Self {
        Self {
            layout,
            estimated_block_height,
            overscan,
        }
    }

    #[must_use]
    pub const fn layout(self) -> LayoutConfig {
        self.layout
    }

    #[must_use]
    pub const fn estimated_block_height(self) -> f32 {
        self.estimated_block_height
    }

    #[must_use]
    pub const fn overscan(self) -> f32 {
        self.overscan
    }

    fn validate(self) -> Result<(), ViewportError> {
        if !self.layout.max_width().is_finite() || self.layout.max_width() <= 0.0 {
            return Err(ViewportError::InvalidConfig(
                "layout max_width must be finite and positive",
            ));
        }
        if !self.layout.line_height().is_finite() || self.layout.line_height() <= 0.0 {
            return Err(ViewportError::InvalidConfig(
                "layout line_height must be finite and positive",
            ));
        }
        if !self.estimated_block_height.is_finite() || self.estimated_block_height <= 0.0 {
            return Err(ViewportError::InvalidConfig(
                "estimated_block_height must be finite and positive",
            ));
        }
        if !self.overscan.is_finite() || self.overscan < 0.0 {
            return Err(ViewportError::InvalidConfig(
                "overscan must be finite and non-negative",
            ));
        }
        Ok(())
    }
}

impl Default for ViewportConfig {
    fn default() -> Self {
        Self::new(LayoutConfig::default(), 1.0, 2.0)
    }
}

/// Scroll position and viewport height in the document's local y coordinate.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ViewportRect {
    scroll_y: f32,
    height: f32,
}

impl ViewportRect {
    #[must_use]
    pub const fn new(scroll_y: f32, height: f32) -> Self {
        Self { scroll_y, height }
    }

    #[must_use]
    pub const fn scroll_y(self) -> f32 {
        self.scroll_y
    }

    #[must_use]
    pub const fn height(self) -> f32 {
        self.height
    }

    pub(crate) fn validate(self) -> Result<(), ViewportError> {
        if !self.scroll_y.is_finite() || self.scroll_y < 0.0 {
            return Err(ViewportError::InvalidViewport(
                "scroll_y must be finite and non-negative",
            ));
        }
        if !self.height.is_finite() || self.height < 0.0 {
            return Err(ViewportError::InvalidViewport(
                "viewport height must be finite and non-negative",
            ));
        }
        Ok(())
    }
}

/// The document-space caret geometry used by the platform scroll protocol.
///
/// `y` is the top of the caret line relative to the document content, not the
/// current viewport. The source offset and revision make the geometry safe to
/// discard when a newer edit has already been published.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ViewportCaret {
    source: ByteOffset,
    block: usize,
    x: f32,
    y: f32,
    width: f32,
    height: f32,
}

impl ViewportCaret {
    pub(crate) const fn new(
        source: ByteOffset,
        block: usize,
        x: f32,
        y: f32,
        width: f32,
        height: f32,
    ) -> Self {
        Self {
            source,
            block,
            x,
            y,
            width,
            height,
        }
    }

    #[must_use]
    pub const fn source(self) -> ByteOffset {
        self.source
    }

    #[must_use]
    pub const fn block(self) -> usize {
        self.block
    }

    #[must_use]
    pub const fn x(self) -> f32 {
        self.x
    }

    #[must_use]
    pub const fn y(self) -> f32 {
        self.y
    }

    #[must_use]
    pub const fn width(self) -> f32 {
        self.width
    }

    #[must_use]
    pub const fn height(self) -> f32 {
        self.height
    }
}

/// A revision-bound request for the platform viewport to reveal the focus
/// caret. The target is absolute document scroll, so the platform does not
/// need to reconstruct block heights or line geometry.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CaretScrollRequest {
    revision: Revision,
    caret: ViewportCaret,
    current_scroll_y: f32,
    target_scroll_y: f32,
    margin: f32,
    needs_scroll: bool,
}

impl CaretScrollRequest {
    pub(crate) const fn new(
        revision: Revision,
        caret: ViewportCaret,
        current_scroll_y: f32,
        target_scroll_y: f32,
        margin: f32,
        needs_scroll: bool,
    ) -> Self {
        Self {
            revision,
            caret,
            current_scroll_y,
            target_scroll_y,
            margin,
            needs_scroll,
        }
    }

    #[must_use]
    pub const fn revision(self) -> Revision {
        self.revision
    }

    #[must_use]
    pub const fn caret(self) -> ViewportCaret {
        self.caret
    }

    #[must_use]
    pub const fn current_scroll_y(self) -> f32 {
        self.current_scroll_y
    }

    #[must_use]
    pub const fn target_scroll_y(self) -> f32 {
        self.target_scroll_y
    }

    #[must_use]
    pub fn delta_y(self) -> f32 {
        self.target_scroll_y - self.current_scroll_y
    }

    #[must_use]
    pub const fn margin(self) -> f32 {
        self.margin
    }

    #[must_use]
    pub const fn needs_scroll(self) -> bool {
        self.needs_scroll
    }
}

/// A half-open range of block indices selected for measurement/painting.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ViewportRange {
    start: usize,
    end: usize,
}

impl ViewportRange {
    #[must_use]
    pub const fn new(start: usize, end: usize) -> Option<Self> {
        if start <= end {
            Some(Self { start, end })
        } else {
            None
        }
    }

    #[must_use]
    pub const fn start(self) -> usize {
        self.start
    }

    #[must_use]
    pub const fn end(self) -> usize {
        self.end
    }

    #[must_use]
    pub const fn len(self) -> usize {
        self.end - self.start
    }

    #[must_use]
    pub const fn is_empty(self) -> bool {
        self.start == self.end
    }
}

/// A block selected by a viewport query, with its current estimated/measured y.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ViewportBlock {
    index: usize,
    source: TextRange,
    kind: BlockKind,
    y: f32,
    height: f32,
    measured: bool,
}

impl ViewportBlock {
    #[must_use]
    pub const fn index(self) -> usize {
        self.index
    }

    #[must_use]
    pub const fn source(self) -> TextRange {
        self.source
    }

    #[must_use]
    pub const fn kind(self) -> BlockKind {
        self.kind
    }

    #[must_use]
    pub const fn y(self) -> f32 {
        self.y
    }

    #[must_use]
    pub const fn height(self) -> f32 {
        self.height
    }

    #[must_use]
    pub const fn is_measured(self) -> bool {
        self.measured
    }
}

/// The immutable result of one cross-block viewport query.
#[derive(Clone, Debug, PartialEq)]
pub struct ViewportSnapshot {
    revision: Revision,
    range: ViewportRange,
    content_height: f32,
    blocks: Vec<ViewportBlock>,
}

impl ViewportSnapshot {
    #[must_use]
    pub const fn revision(&self) -> Revision {
        self.revision
    }

    #[must_use]
    pub const fn range(&self) -> ViewportRange {
        self.range
    }

    #[must_use]
    pub const fn content_height(&self) -> f32 {
        self.content_height
    }

    #[must_use]
    pub fn blocks(&self) -> &[ViewportBlock] {
        &self.blocks
    }
}

/// Cumulative lifecycle counters for a document's viewport state.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ViewportStats {
    entries: usize,
    measured: usize,
    remapped: u64,
    invalidated: u64,
}

impl ViewportStats {
    #[must_use]
    pub const fn entries(self) -> usize {
        self.entries
    }

    #[must_use]
    pub const fn measured(self) -> usize {
        self.measured
    }

    #[must_use]
    pub const fn remapped(self) -> u64 {
        self.remapped
    }

    #[must_use]
    pub const fn invalidated(self) -> u64 {
        self.invalidated
    }
}

/// Incremental cross-block height state used by a future renderer viewport.
///
/// Unmeasured blocks use `estimated_block_height`. Visible blocks are measured
/// by `EditorDocument` on demand and update the Fenwick index without laying
/// out the rest of the document.
#[derive(Debug, Default)]
pub struct ViewportLayout {
    config: ViewportConfig,
    backend: LayoutBackend,
    entries: Vec<ViewportEntry>,
    heights: HeightIndex,
    revision: Option<Revision>,
    remapped: u64,
    invalidated: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct ViewportKey {
    range: TextRange,
    kind: BlockKind,
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct ViewportEntry {
    key: ViewportKey,
    height: f32,
    measured: bool,
}

impl ViewportLayout {
    pub fn new(config: ViewportConfig) -> Result<Self, ViewportError> {
        config.validate()?;
        Ok(Self {
            config,
            backend: LayoutBackend::Metrics,
            entries: Vec::new(),
            heights: HeightIndex::default(),
            revision: None,
            remapped: 0,
            invalidated: 0,
        })
    }

    #[must_use]
    pub const fn config(&self) -> ViewportConfig {
        self.config
    }

    #[must_use]
    pub const fn backend(&self) -> LayoutBackend {
        self.backend
    }

    /// Switches the measurement backend and resets measured heights to the
    /// configured estimate. Metrics and shaped glyph runs can have different
    /// line wrapping, so retaining old measured heights would make viewport
    /// range selection stale.
    pub(crate) fn set_backend(&mut self, backend: LayoutBackend) -> Result<(), ViewportError> {
        if self.backend == backend {
            return Ok(());
        }
        self.backend = backend;
        for entry in &mut self.entries {
            entry.height = self.config.estimated_block_height();
            entry.measured = false;
        }
        self.rebuild_index()
    }

    #[must_use]
    pub const fn revision(&self) -> Option<Revision> {
        self.revision
    }

    #[must_use]
    pub fn height_index(&self) -> &HeightIndex {
        &self.heights
    }

    #[must_use]
    pub fn stats(&self) -> ViewportStats {
        ViewportStats {
            entries: self.entries.len(),
            measured: self.entries.iter().filter(|entry| entry.measured).count(),
            remapped: self.remapped,
            invalidated: self.invalidated,
        }
    }

    /// Synchronizes block keys while retaining measurements for unchanged keys.
    pub fn sync(&mut self, markdown: &MarkdownDocument) -> Result<(), ViewportError> {
        if self.revision == Some(markdown.revision())
            && self.entries.len() == markdown.blocks().len()
            && self
                .entries
                .iter()
                .zip(markdown.blocks())
                .all(|(entry, block)| {
                    entry.key.range == block.range() && entry.key.kind == block.kind()
                })
        {
            return Ok(());
        }
        let old_entries = std::mem::take(&mut self.entries);
        let mut used = vec![false; old_entries.len()];
        let mut entries = Vec::with_capacity(markdown.blocks().len());
        for block in markdown.blocks() {
            let key = ViewportKey {
                range: block.range(),
                kind: block.kind(),
            };
            if let Some((index, entry)) = old_entries
                .iter()
                .enumerate()
                .find(|(index, entry)| !used[*index] && entry.key == key)
            {
                used[index] = true;
                entries.push(*entry);
            } else {
                entries.push(ViewportEntry {
                    key,
                    height: self.config.estimated_block_height(),
                    measured: false,
                });
            }
        }
        self.invalidated = self
            .invalidated
            .saturating_add(used.iter().filter(|used| !**used).count() as u64);
        self.entries = entries;
        self.revision = Some(markdown.revision());
        self.rebuild_index()
    }

    /// Maps measured entries through a successful source edit, then validates
    /// them against the new Markdown block sequence.
    pub fn map_through(
        &mut self,
        changes: &ChangeSet,
        snapshot: &TextSnapshot,
        markdown: &MarkdownDocument,
    ) -> Result<(), ViewportError> {
        let had_state = self.revision.is_some() || !self.entries.is_empty();
        if let Some(revision) = self.revision
            && revision != changes.before()
        {
            return Err(ViewportError::RevisionMismatch {
                expected: changes.before(),
                actual: revision,
            });
        }
        if snapshot.revision() != changes.after() || markdown.revision() != changes.after() {
            return Err(ViewportError::RevisionMismatch {
                expected: changes.after(),
                actual: snapshot.revision(),
            });
        }

        let mut mapped = Vec::with_capacity(self.entries.len());
        for entry in &self.entries {
            if changes
                .changes()
                .iter()
                .any(|change| change_affects_range(*change, entry.key.range))
            {
                self.invalidated = self.invalidated.saturating_add(1);
                continue;
            }
            let range = map_range(entry.key.range, changes)?;
            mapped.push(ViewportEntry {
                key: ViewportKey {
                    range,
                    kind: entry.key.kind,
                },
                height: entry.height,
                measured: entry.measured,
            });
            self.remapped = self.remapped.saturating_add(1);
        }
        self.entries = mapped;
        self.revision = Some(changes.after());
        if !had_state {
            self.heights = HeightIndex::default();
            return Ok(());
        }
        self.rebuild_index()?;
        self.sync(markdown)
    }

    /// Clears all block estimates and measured heights.
    pub fn clear(&mut self) {
        self.invalidated = self.invalidated.saturating_add(self.entries.len() as u64);
        self.entries.clear();
        self.heights = HeightIndex::default();
        self.revision = None;
    }

    /// Returns the current range selected by a scroll position, including
    /// configured overscan. Calling this also synchronizes block count/keys.
    pub fn visible_range(
        &mut self,
        markdown: &MarkdownDocument,
        viewport: ViewportRect,
    ) -> Result<ViewportRange, ViewportError> {
        viewport.validate()?;
        self.sync(markdown)?;
        let count = self.entries.len();
        if count == 0 {
            return Ok(ViewportRange { start: 0, end: 0 });
        }
        let top = (viewport.scroll_y() - self.config.overscan()).max(0.0);
        let bottom_sum = viewport.scroll_y() + viewport.height() + self.config.overscan();
        let bottom = if bottom_sum.is_finite() {
            bottom_sum
        } else {
            f32::MAX
        };
        let start = self.heights.find_line(top).unwrap_or(0);
        let last = self.heights.find_line(bottom).unwrap_or(start);
        let end = last.saturating_add(1).min(count);
        Ok(ViewportRange { start, end })
    }

    /// Replaces an estimated height with the measured layout height.
    /// Returns whether either the height or measured state changed.
    pub fn set_block_height(&mut self, index: usize, height: f32) -> Result<bool, ViewportError> {
        if !height.is_finite() || height < 0.0 {
            return Err(ViewportError::HeightIndex(HeightIndexError::InvalidHeight(
                height.to_bits(),
            )));
        }
        let Some(entry) = self.entries.get_mut(index) else {
            return Err(ViewportError::HeightIndex(HeightIndexError::OutOfBounds {
                index,
                len: self.entries.len(),
            }));
        };
        let changed = entry.height != height || !entry.measured;
        self.heights.set(index, height)?;
        entry.height = height;
        entry.measured = true;
        Ok(changed)
    }

    /// Materializes metadata for a previously selected block range.
    pub fn snapshot(
        &self,
        markdown: &MarkdownDocument,
        range: ViewportRange,
    ) -> Result<ViewportSnapshot, ViewportError> {
        if self.revision != Some(markdown.revision()) {
            return Err(ViewportError::RevisionMismatch {
                expected: markdown.revision(),
                actual: self.revision.unwrap_or(Revision::INITIAL),
            });
        }
        if range.end() > self.entries.len() || range.end() > markdown.blocks().len() {
            return Err(ViewportError::HeightIndex(HeightIndexError::OutOfBounds {
                index: range.end(),
                len: self.entries.len().min(markdown.blocks().len()),
            }));
        }
        let blocks = (range.start()..range.end())
            .map(|index| {
                let block = markdown
                    .blocks()
                    .get(index)
                    .ok_or(ViewportError::HeightIndex(HeightIndexError::OutOfBounds {
                        index,
                        len: markdown.blocks().len(),
                    }))?;
                let entry = self.entries[index];
                Ok(ViewportBlock {
                    index,
                    source: block.range(),
                    kind: block.kind(),
                    y: self.heights.prefix_height(index),
                    height: entry.height,
                    measured: entry.measured,
                })
            })
            .collect::<Result<Vec<_>, ViewportError>>()?;
        Ok(ViewportSnapshot {
            revision: markdown.revision(),
            range,
            content_height: self.heights.total_height(),
            blocks,
        })
    }

    fn rebuild_index(&mut self) -> Result<(), ViewportError> {
        self.heights = HeightIndex::new(self.entries.iter().map(|entry| entry.height))?;
        Ok(())
    }
}

/// Errors raised by viewport estimation, mapping or block selection.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ViewportError {
    AnchorMap(AnchorMapError),
    HeightIndex(HeightIndexError),
    InvalidConfig(&'static str),
    InvalidRange,
    InvalidMargin,
    InvalidViewport(&'static str),
    RevisionMismatch {
        expected: Revision,
        actual: Revision,
    },
}

impl fmt::Display for ViewportError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::AnchorMap(error) => error.fmt(formatter),
            Self::HeightIndex(error) => error.fmt(formatter),
            Self::InvalidConfig(message) | Self::InvalidViewport(message) => {
                formatter.write_str(message)
            }
            Self::InvalidRange => formatter.write_str("viewport source range mapping overflowed"),
            Self::InvalidMargin => {
                formatter.write_str("caret reveal margin must be finite and non-negative")
            }
            Self::RevisionMismatch { expected, actual } => write!(
                formatter,
                "viewport revision mismatch: expected {expected:?}, got {actual:?}"
            ),
        }
    }
}

impl Error for ViewportError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::AnchorMap(error) => Some(error),
            Self::HeightIndex(error) => Some(error),
            Self::InvalidConfig(_)
            | Self::InvalidRange
            | Self::InvalidMargin
            | Self::InvalidViewport(_)
            | Self::RevisionMismatch { .. } => None,
        }
    }
}

impl From<AnchorMapError> for ViewportError {
    fn from(error: AnchorMapError) -> Self {
        Self::AnchorMap(error)
    }
}

impl From<HeightIndexError> for ViewportError {
    fn from(error: HeightIndexError) -> Self {
        Self::HeightIndex(error)
    }
}

fn change_affects_range(change: yu_text::TextChange, range: TextRange) -> bool {
    let old = change.old_range();
    old.start() <= range.end() && old.end() >= range.start()
}

fn map_range(range: TextRange, changes: &ChangeSet) -> Result<TextRange, ViewportError> {
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
    TextRange::new(start, end).ok_or(ViewportError::InvalidRange)
}
