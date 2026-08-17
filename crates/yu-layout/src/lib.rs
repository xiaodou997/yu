#![forbid(unsafe_code)]

//! Source-backed block layout contracts.
//!
//! This crate deliberately stops before font shaping and GPU painting. It
//! turns a projection into visual lines and grapheme clusters using a
//! replaceable advance provider, then exposes deterministic caret and hit-test
//! mappings for the future platform/layout layers.

use std::error::Error;
use std::fmt;
use std::ops::Range;

use unicode_segmentation::UnicodeSegmentation;
use yu_core::{Affinity, ByteOffset, TextAnchor, TextRange};
use yu_projection::{
    BlockProjection, Projection, ProjectionBias, ProjectionError, VisualOffset, VisualRange,
    VisualRunKind, VisualRunStyle,
};
use yu_text::{ChangeSet, TextSnapshot};

mod shaping;
mod table;

pub use shaping::{
    FontFaceId, Glyph, GlyphId, GlyphRun, Script, ShapedText, ShapingProvider, TextDirection,
};
pub use table::{TableCellLayout, TableLayoutHit, TableLayoutSnapshot};

/// Layout dimensions and wrapping policy independent of any font backend.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LayoutConfig {
    max_width: f32,
    line_height: f32,
    default_advance: f32,
}

impl LayoutConfig {
    #[must_use]
    pub const fn new(max_width: f32, line_height: f32) -> Self {
        Self {
            max_width,
            line_height,
            default_advance: 1.0,
        }
    }

    /// Returns a copy using the fallback advance for metrics-only layout.
    ///
    /// Shaped layout providers can replace this value per glyph run; the
    /// fallback is used by the deterministic metrics backend and by the FFI
    /// viewport contract before a native shaper is attached.
    #[must_use]
    pub const fn with_default_advance(mut self, default_advance: f32) -> Self {
        self.default_advance = default_advance;
        self
    }

    #[must_use]
    pub const fn max_width(self) -> f32 {
        self.max_width
    }

    #[must_use]
    pub const fn line_height(self) -> f32 {
        self.line_height
    }

    #[must_use]
    pub const fn default_advance(self) -> f32 {
        self.default_advance
    }

    fn validate(self) -> Result<(), LayoutError> {
        if !self.max_width.is_finite() || self.max_width <= 0.0 {
            return Err(LayoutError::InvalidConfig(
                "max_width must be finite and positive",
            ));
        }
        if !self.line_height.is_finite() || self.line_height <= 0.0 {
            return Err(LayoutError::InvalidConfig(
                "line_height must be finite and positive",
            ));
        }
        if !self.default_advance.is_finite() || self.default_advance <= 0.0 {
            return Err(LayoutError::InvalidConfig(
                "default_advance must be finite and positive",
            ));
        }
        Ok(())
    }
}

impl Default for LayoutConfig {
    fn default() -> Self {
        Self::new(80.0, 1.0)
    }
}

/// Supplies an advance for one Unicode grapheme cluster.
///
/// `yu-font::FontMetrics` is the current contract adapter; a native shaping
/// backend can provide the same trait without changing the layout tree or
/// hit-test contracts.
pub trait ClusterMetrics {
    fn advance(&self, cluster: &str, style: VisualRunStyle) -> f32;
}

/// Deterministic metrics used before a font/shaping backend exists.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MonospaceMetrics {
    advance: f32,
}

impl MonospaceMetrics {
    #[must_use]
    pub const fn new(advance: f32) -> Self {
        Self { advance }
    }

    #[must_use]
    pub const fn advance(self) -> f32 {
        self.advance
    }
}

impl Default for MonospaceMetrics {
    fn default() -> Self {
        Self::new(1.0)
    }
}

impl ClusterMetrics for MonospaceMetrics {
    fn advance(&self, _cluster: &str, _style: VisualRunStyle) -> f32 {
        self.advance
    }
}

/// Errors raised while constructing or querying a layout snapshot.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LayoutError {
    Projection(ProjectionError),
    InvalidConfig(&'static str),
    InvalidMetrics(u32),
    Shaping(String),
    InvalidPoint,
    InvalidImageBounds,
    InvalidTable(&'static str),
    OffsetOverflow,
}

/// Errors raised by the viewport height index.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HeightIndexError {
    InvalidHeight(u32),
    OutOfBounds { index: usize, len: usize },
}

impl fmt::Display for HeightIndexError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidHeight(height) => {
                write!(formatter, "invalid line height {}", f32::from_bits(*height))
            }
            Self::OutOfBounds { index, len } => {
                write!(formatter, "height index {index} is outside {len} entries")
            }
        }
    }
}

impl Error for HeightIndexError {}

/// A Fenwick-tree index over variable visual line heights.
///
/// Prefix queries and point updates are O(log n); `find_line` locates the line
/// containing a viewport y coordinate without laying out off-screen blocks.
#[derive(Clone, Debug, PartialEq)]
pub struct HeightIndex {
    values: Vec<f32>,
    tree: Vec<f32>,
}

impl Default for HeightIndex {
    fn default() -> Self {
        Self {
            values: Vec::new(),
            tree: vec![0.0],
        }
    }
}

impl HeightIndex {
    pub fn new(heights: impl IntoIterator<Item = f32>) -> Result<Self, HeightIndexError> {
        let values = heights.into_iter().collect::<Vec<_>>();
        for height in &values {
            validate_height(*height)?;
        }
        let mut index = Self {
            tree: vec![0.0; values.len().saturating_add(1)],
            values,
        };
        let values = index.values.clone();
        for (position, height) in values.into_iter().enumerate() {
            index.add(position, height);
        }
        Ok(index)
    }

    pub fn uniform(count: usize, height: f32) -> Result<Self, HeightIndexError> {
        Self::new(std::iter::repeat_n(height, count))
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.values.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.values.is_empty()
    }

    #[must_use]
    pub fn height(&self, index: usize) -> Option<f32> {
        self.values.get(index).copied()
    }

    #[must_use]
    pub fn total_height(&self) -> f32 {
        self.prefix_height(self.values.len())
    }

    #[must_use]
    pub fn prefix_height(&self, end: usize) -> f32 {
        let mut position = end.min(self.values.len());
        let mut total = 0.0;
        while position > 0 {
            total += self.tree[position];
            position &= position - 1;
        }
        total
    }

    pub fn set(&mut self, index: usize, height: f32) -> Result<(), HeightIndexError> {
        let Some(previous) = self.values.get_mut(index) else {
            return Err(HeightIndexError::OutOfBounds {
                index,
                len: self.values.len(),
            });
        };
        validate_height(height)?;
        let delta = height - *previous;
        *previous = height;
        self.add(index, delta);
        Ok(())
    }

    pub fn push(&mut self, height: f32) -> Result<(), HeightIndexError> {
        validate_height(height)?;
        let index = self.values.len();
        let position = index.saturating_add(1);
        let low_bit = position & position.wrapping_neg();
        let existing =
            self.prefix_height(index) - self.prefix_height(position.saturating_sub(low_bit));
        self.values.push(height);
        self.tree.push(existing + height);
        Ok(())
    }

    #[must_use]
    pub fn find_line(&self, y: f32) -> Option<usize> {
        if self.values.is_empty() || !y.is_finite() {
            return None;
        }
        if y <= 0.0 {
            return Some(0);
        }
        let total = self.total_height();
        if y >= total {
            return Some(self.values.len().saturating_sub(1));
        }

        let mut position = 0_usize;
        let mut accumulated = 0.0_f32;
        let mut step = 1_usize;
        while step < self.values.len() {
            step <<= 1;
        }
        while step > 0 {
            let candidate = position.saturating_add(step);
            if candidate <= self.values.len() && accumulated + self.tree[candidate] <= y {
                accumulated += self.tree[candidate];
                position = candidate;
            }
            step >>= 1;
        }
        Some(position.min(self.values.len().saturating_sub(1)))
    }

    fn add(&mut self, index: usize, delta: f32) {
        let mut position = index.saturating_add(1);
        while position < self.tree.len() {
            self.tree[position] += delta;
            position = position.saturating_add(position & position.wrapping_neg());
        }
    }
}

fn validate_height(height: f32) -> Result<(), HeightIndexError> {
    if height.is_finite() && height >= 0.0 {
        Ok(())
    } else {
        Err(HeightIndexError::InvalidHeight(height.to_bits()))
    }
}

impl fmt::Display for LayoutError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Projection(error) => error.fmt(formatter),
            Self::InvalidConfig(message) => formatter.write_str(message),
            Self::InvalidMetrics(advance) => write!(
                formatter,
                "invalid cluster advance {}",
                f32::from_bits(*advance)
            ),
            Self::Shaping(message) => write!(formatter, "shaping failed: {message}"),
            Self::InvalidPoint => {
                formatter.write_str("layout point must contain finite coordinates")
            }
            Self::InvalidImageBounds => {
                formatter.write_str("image bounds must be finite and have positive height")
            }
            Self::InvalidTable(message) => write!(formatter, "invalid table layout: {message}"),
            Self::OffsetOverflow => formatter.write_str("layout offset overflow"),
        }
    }
}

impl Error for LayoutError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Projection(error) => Some(error),
            Self::InvalidConfig(_)
            | Self::InvalidMetrics(_)
            | Self::Shaping(_)
            | Self::InvalidPoint
            | Self::InvalidImageBounds
            | Self::InvalidTable(_)
            | Self::OffsetOverflow => None,
        }
    }
}

impl From<ProjectionError> for LayoutError {
    fn from(error: ProjectionError) -> Self {
        Self::Projection(error)
    }
}

/// A point in the block's local layout coordinate system.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LayoutPoint {
    x: f32,
    y: f32,
}

/// A finite, non-negative image rectangle in block-local layout coordinates.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LayoutRect {
    x: f32,
    y: f32,
    width: f32,
    height: f32,
}

impl LayoutRect {
    pub fn new(x: f32, y: f32, width: f32, height: f32) -> Result<Self, LayoutError> {
        if !x.is_finite()
            || !y.is_finite()
            || !width.is_finite()
            || !height.is_finite()
            || x < 0.0
            || y < 0.0
            || width < 0.0
            || height <= 0.0
            || !(x + width).is_finite()
            || !(y + height).is_finite()
        {
            return Err(LayoutError::InvalidImageBounds);
        }
        Ok(Self {
            x,
            y,
            width,
            height,
        })
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

impl LayoutPoint {
    #[must_use]
    pub const fn new(x: f32, y: f32) -> Self {
        Self { x, y }
    }

    #[must_use]
    pub const fn x(self) -> f32 {
        self.x
    }

    #[must_use]
    pub const fn y(self) -> f32 {
        self.y
    }

    fn validate(self) -> Result<(), LayoutError> {
        if self.x.is_finite() && self.y.is_finite() {
            Ok(())
        } else {
            Err(LayoutError::InvalidPoint)
        }
    }
}

/// One grapheme-backed visual cluster in a line.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct VisualCluster {
    source: TextRange,
    visual: VisualRange,
    line: usize,
    x: f32,
    width: f32,
    style: VisualRunStyle,
    line_break: bool,
}

/// One shaped glyph retained as a draw placement for the scene layer.
///
/// The placement is still source-backed: it identifies the source and visual
/// cluster that produced the glyph, while `x`/`y` are layout coordinates. `y`
/// is a baseline coordinate, so a scene can pass it directly to an atlas
/// primitive without inventing a second text coordinate system.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GlyphPlacement {
    face: FontFaceId,
    glyph: GlyphId,
    source: TextRange,
    visual: VisualRange,
    line: usize,
    x: f32,
    y: f32,
    style: VisualRunStyle,
}

/// Source-backed geometry for one Markdown image. The destination/resource
/// identity is intentionally resolved by the workspace layer; layout only
/// determines where the image occupies the projected alt-label span.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ImagePlacement {
    source: TextRange,
    label: TextRange,
    visual: VisualRange,
    line: usize,
    bounds: LayoutRect,
}

/// Intrinsic pixel dimensions supplied by a decoded image publication. The
/// layout layer uses the dimensions only to choose a finite aspect-preserving
/// rectangle; pixels and resource identities remain outside this crate.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ImageIntrinsicSize {
    width: u32,
    height: u32,
}

impl ImageIntrinsicSize {
    pub fn new(width: u32, height: u32) -> Result<Self, LayoutError> {
        if width == 0 || height == 0 {
            return Err(LayoutError::InvalidImageBounds);
        }
        Ok(Self { width, height })
    }

    #[must_use]
    pub const fn width(self) -> u32 {
        self.width
    }

    #[must_use]
    pub const fn height(self) -> u32 {
        self.height
    }
}

impl ImagePlacement {
    #[must_use]
    pub const fn source(self) -> TextRange {
        self.source
    }

    #[must_use]
    pub const fn label(self) -> TextRange {
        self.label
    }

    #[must_use]
    pub const fn visual(self) -> VisualRange {
        self.visual
    }

    #[must_use]
    pub const fn line(self) -> usize {
        self.line
    }

    #[must_use]
    pub const fn bounds(self) -> LayoutRect {
        self.bounds
    }
}

impl GlyphPlacement {
    #[must_use]
    pub const fn face(self) -> FontFaceId {
        self.face
    }

    #[must_use]
    pub const fn glyph(self) -> GlyphId {
        self.glyph
    }

    #[must_use]
    pub const fn source(self) -> TextRange {
        self.source
    }

    #[must_use]
    pub const fn visual(self) -> VisualRange {
        self.visual
    }

    #[must_use]
    pub const fn line(self) -> usize {
        self.line
    }

    #[must_use]
    pub const fn x(self) -> f32 {
        self.x
    }

    /// Returns the baseline y coordinate for this glyph.
    #[must_use]
    pub const fn y(self) -> f32 {
        self.y
    }

    #[must_use]
    pub const fn style(self) -> VisualRunStyle {
        self.style
    }
}

impl VisualCluster {
    #[must_use]
    pub const fn source(self) -> TextRange {
        self.source
    }

    #[must_use]
    pub const fn visual(self) -> VisualRange {
        self.visual
    }

    #[must_use]
    pub const fn line(self) -> usize {
        self.line
    }

    #[must_use]
    pub const fn x(self) -> f32 {
        self.x
    }

    #[must_use]
    pub const fn width(self) -> f32 {
        self.width
    }

    #[must_use]
    pub const fn style(self) -> VisualRunStyle {
        self.style
    }

    #[must_use]
    pub const fn is_line_break(self) -> bool {
        self.line_break
    }
}

/// One laid-out visual line and its cluster index range.
#[derive(Clone, Debug, PartialEq)]
pub struct VisualLine {
    index: usize,
    source: TextRange,
    visual: VisualRange,
    y: f32,
    width: f32,
    clusters: Range<usize>,
}

struct LineDraft {
    index: usize,
    source_start: ByteOffset,
    source_end: ByteOffset,
    visual_start: VisualOffset,
    visual_end: VisualOffset,
    width: f32,
    cluster_start: usize,
}

impl VisualLine {
    #[must_use]
    pub const fn index(&self) -> usize {
        self.index
    }

    #[must_use]
    pub const fn source(&self) -> TextRange {
        self.source
    }

    #[must_use]
    pub const fn visual(&self) -> VisualRange {
        self.visual
    }

    #[must_use]
    pub const fn y(&self) -> f32 {
        self.y
    }

    #[must_use]
    pub const fn width(&self) -> f32 {
        self.width
    }

    #[must_use]
    pub fn cluster_range(&self) -> Range<usize> {
        self.clusters.clone()
    }
}

/// A source/visual caret resolved into layout coordinates.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LayoutCaret {
    source: ByteOffset,
    visual: VisualOffset,
    line: usize,
    point: LayoutPoint,
    bias: ProjectionBias,
}

impl LayoutCaret {
    #[must_use]
    pub const fn source(self) -> ByteOffset {
        self.source
    }

    #[must_use]
    pub const fn visual(self) -> VisualOffset {
        self.visual
    }

    #[must_use]
    pub const fn line(self) -> usize {
        self.line
    }

    #[must_use]
    pub const fn point(self) -> LayoutPoint {
        self.point
    }

    #[must_use]
    pub const fn bias(self) -> ProjectionBias {
        self.bias
    }
}

/// A hit-test result that can be written back to source selection.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LayoutHit {
    source: ByteOffset,
    visual: VisualOffset,
    line: usize,
    point: LayoutPoint,
    bias: ProjectionBias,
    image: Option<TextRange>,
}

impl LayoutHit {
    #[must_use]
    pub const fn source(self) -> ByteOffset {
        self.source
    }

    #[must_use]
    pub const fn visual(self) -> VisualOffset {
        self.visual
    }

    #[must_use]
    pub const fn line(self) -> usize {
        self.line
    }

    #[must_use]
    pub const fn point(self) -> LayoutPoint {
        self.point
    }

    #[must_use]
    pub const fn bias(self) -> ProjectionBias {
        self.bias
    }

    /// Returns the complete source range when this point landed on an image
    /// placement. Callers can use it to select/activate the image as one
    /// source-backed object instead of placing a caret inside its alt label.
    #[must_use]
    pub const fn image(self) -> Option<TextRange> {
        self.image
    }
}

/// A revision-bound, block-local layout snapshot.
#[derive(Clone, Debug)]
pub struct LayoutSnapshot {
    projection: Projection,
    config: LayoutConfig,
    lines: Vec<VisualLine>,
    clusters: Vec<VisualCluster>,
    glyphs: Vec<GlyphPlacement>,
    images: Vec<ImagePlacement>,
    table: Option<TableLayoutSnapshot>,
}

impl LayoutSnapshot {
    /// Builds layout with deterministic one-unit-per-grapheme metrics.
    pub fn from_projection(
        projection: &Projection,
        config: LayoutConfig,
    ) -> Result<Self, LayoutError> {
        let metrics = MonospaceMetrics::new(config.default_advance());
        Self::from_projection_with_metrics(projection, config, &metrics)
    }

    /// Builds layout from either an inline or fenced-code block projection.
    pub fn from_block_projection(
        projection: &BlockProjection,
        config: LayoutConfig,
    ) -> Result<Self, LayoutError> {
        let metrics = MonospaceMetrics::new(config.default_advance());
        Self::from_block_projection_with_metrics(projection, config, &metrics)
    }

    /// Builds layout from a block projection with caller-provided metrics.
    pub fn from_block_projection_with_metrics<M: ClusterMetrics>(
        projection: &BlockProjection,
        config: LayoutConfig,
        metrics: &M,
    ) -> Result<Self, LayoutError> {
        let mut layout = Self::from_projection_with_metrics(projection.visual(), config, metrics)?;
        if let BlockProjection::Table(table) = projection {
            let table_layout = TableLayoutSnapshot::from_projection(table, config, metrics)?;
            layout.apply_table_geometry(&table_layout)?;
            layout.table = Some(table_layout);
        }
        Ok(layout)
    }

    /// Builds a block layout from shaped glyph runs.
    ///
    /// The shaper owns font fallback and glyph selection while the layout
    /// snapshot remains source-backed. Glyph cluster ranges are mapped back
    /// through the projection, so ligatures and combining marks remain
    /// addressable by source selection and hit testing.
    pub fn from_block_projection_with_shaper<S: ShapingProvider>(
        projection: &BlockProjection,
        config: LayoutConfig,
        shaper: &S,
    ) -> Result<Self, LayoutError> {
        let mut layout = Self::from_projection_with_shaper(projection.visual(), config, shaper)?;
        if let BlockProjection::Table(table) = projection {
            let table_layout =
                TableLayoutSnapshot::from_projection_with_shaper(table, config, shaper)?;
            layout.apply_table_geometry(&table_layout)?;
            layout.table = Some(table_layout);
        }
        Ok(layout)
    }

    /// Builds layout with a caller-provided grapheme advance provider.
    pub fn from_projection_with_metrics<M: ClusterMetrics>(
        projection: &Projection,
        config: LayoutConfig,
        metrics: &M,
    ) -> Result<Self, LayoutError> {
        config.validate()?;
        let mut layout = Self {
            projection: projection.clone(),
            config,
            lines: Vec::new(),
            clusters: Vec::new(),
            glyphs: Vec::new(),
            images: Vec::new(),
            table: None,
        };
        layout.build(metrics)?;
        layout.build_image_placements()?;
        Ok(layout)
    }

    /// Builds layout from a source-backed shaping provider.
    pub fn from_projection_with_shaper<S: ShapingProvider>(
        projection: &Projection,
        config: LayoutConfig,
        shaper: &S,
    ) -> Result<Self, LayoutError> {
        config.validate()?;
        let mut layout = Self {
            projection: projection.clone(),
            config,
            lines: Vec::new(),
            clusters: Vec::new(),
            glyphs: Vec::new(),
            images: Vec::new(),
            table: None,
        };
        layout.build_shaped(shaper)?;
        layout.build_image_placements()?;
        Ok(layout)
    }

    #[must_use]
    pub fn revision(&self) -> yu_core::Revision {
        self.projection.revision()
    }

    #[must_use]
    pub fn source_range(&self) -> TextRange {
        self.projection.source_range()
    }

    #[must_use]
    pub fn visual_len(&self) -> VisualOffset {
        self.projection.visual_len()
    }

    #[must_use]
    pub const fn config(&self) -> LayoutConfig {
        self.config
    }

    #[must_use]
    pub fn lines(&self) -> &[VisualLine] {
        &self.lines
    }

    #[must_use]
    pub fn clusters(&self) -> &[VisualCluster] {
        &self.clusters
    }

    /// Returns shaped glyph placements in painter order.
    ///
    /// Metrics-only layouts intentionally return an empty slice. The scene
    /// layer can therefore distinguish deterministic layout from a layout
    /// that has real glyph identities available for atlas lookup.
    #[must_use]
    pub fn glyphs(&self) -> &[GlyphPlacement] {
        &self.glyphs
    }

    /// Returns source-backed image geometry in parser span order.
    #[must_use]
    pub fn images(&self) -> &[ImagePlacement] {
        &self.images
    }

    /// Returns table cell geometry when this block is a GFM table. The
    /// delimiter row is source-backed but intentionally absent from visible
    /// cell geometry.
    #[must_use]
    pub fn table(&self) -> Option<&TableLayoutSnapshot> {
        self.table.as_ref()
    }

    /// Applies decoded image dimensions without changing source or visual
    /// mappings. Width is capped to the remaining line width and height keeps
    /// the decoded aspect ratio, so hit-testing and the later scene overlay
    /// share the same bounds once a publication becomes ready.
    pub fn apply_image_intrinsic_sizes(
        &mut self,
        measurements: &[(TextRange, ImageIntrinsicSize)],
    ) -> Result<(), LayoutError> {
        for placement in &mut self.images {
            let Some((_, size)) = measurements
                .iter()
                .find(|(source, _)| *source == placement.source)
            else {
                continue;
            };
            let bounds = placement.bounds;
            let intrinsic_width = size.width as f32;
            let intrinsic_height = size.height as f32;
            let available_width = (self.config.max_width - bounds.x).max(1.0);
            let scale = (available_width / intrinsic_width).min(1.0);
            let width = (intrinsic_width * scale).max(1.0);
            let height = (intrinsic_height * scale).max(self.config.line_height);
            placement.bounds = LayoutRect::new(bounds.x, bounds.y, width, height)?;
        }
        Ok(())
    }

    /// Returns the block height after image placement measurement. Ordinary
    /// text keeps its line-height total; a ready image may extend the block
    /// beyond that line box and therefore must participate in the viewport
    /// height index.
    #[must_use]
    pub fn block_height(&self) -> f32 {
        let line_height = self.lines.len() as f32 * self.config.line_height;
        self.images.iter().fold(line_height, |height, image| {
            height.max(image.bounds.y() + image.bounds.height())
        })
    }

    #[must_use]
    pub fn projection(&self) -> &Projection {
        &self.projection
    }

    /// Builds a height index for this snapshot's lines.
    pub fn height_index(&self) -> Result<HeightIndex, HeightIndexError> {
        HeightIndex::uniform(self.lines.len(), self.config.line_height)
    }

    /// Carries an unchanged layout through an edit strictly outside its
    /// source range. Visual coordinates remain stable while source ranges are
    /// remapped to the new revision.
    pub fn map_through(
        &self,
        changes: &ChangeSet,
        snapshot: &TextSnapshot,
    ) -> Result<Option<Self>, LayoutError> {
        let Some(projection) = self.projection.map_through(changes, snapshot)? else {
            return Ok(None);
        };
        let lines = self
            .lines
            .iter()
            .map(|line| {
                Ok(VisualLine {
                    index: line.index,
                    source: map_source_range(line.source, changes)?,
                    visual: line.visual,
                    y: line.y,
                    width: line.width,
                    clusters: line.clusters.clone(),
                })
            })
            .collect::<Result<Vec<_>, LayoutError>>()?;
        let clusters = self
            .clusters
            .iter()
            .map(|cluster| {
                Ok(VisualCluster {
                    source: map_source_range(cluster.source, changes)?,
                    visual: cluster.visual,
                    line: cluster.line,
                    x: cluster.x,
                    width: cluster.width,
                    style: cluster.style,
                    line_break: cluster.line_break,
                })
            })
            .collect::<Result<Vec<_>, LayoutError>>()?;
        let glyphs = self
            .glyphs
            .iter()
            .map(|glyph| {
                Ok(GlyphPlacement {
                    face: glyph.face,
                    glyph: glyph.glyph,
                    source: map_source_range(glyph.source, changes)?,
                    visual: glyph.visual,
                    line: glyph.line,
                    x: glyph.x,
                    y: glyph.y,
                    style: glyph.style,
                })
            })
            .collect::<Result<Vec<_>, LayoutError>>()?;
        let images = self
            .images
            .iter()
            .map(|image| {
                Ok(ImagePlacement {
                    source: map_source_range(image.source, changes)?,
                    label: map_source_range(image.label, changes)?,
                    visual: image.visual,
                    line: image.line,
                    bounds: image.bounds,
                })
            })
            .collect::<Result<Vec<_>, LayoutError>>()?;
        let table = self
            .table
            .as_ref()
            .map(|table| table.map_through(changes, snapshot))
            .transpose()?;
        Ok(Some(Self {
            projection,
            config: self.config,
            lines,
            clusters,
            glyphs,
            images,
            table,
        }))
    }

    /// Resolves a source boundary to a line, visual boundary and point.
    pub fn caret_for_source(
        &self,
        source: ByteOffset,
        bias: ProjectionBias,
    ) -> Result<LayoutCaret, LayoutError> {
        let visual = self.projection.source_to_visual(source, bias)?;
        self.caret_for_visual(visual, bias)
    }

    /// Resolves a projected visual boundary to layout coordinates. This is
    /// especially important for transient composition text: multiple visual
    /// preedit boundaries intentionally map to one canonical replacement
    /// range, so source-first lookup cannot recover an interior preedit caret.
    pub fn caret_for_visual(
        &self,
        visual: VisualOffset,
        bias: ProjectionBias,
    ) -> Result<LayoutCaret, LayoutError> {
        let source = match self
            .table
            .as_ref()
            .and_then(|table| table.source_for_visual_hit(&self.projection, visual, bias))
        {
            Some(source) => source,
            None => self.projection.visual_to_source(visual, bias)?,
        };
        let (line, point) = self.point_for_visual(visual, bias)?;
        Ok(LayoutCaret {
            source,
            visual,
            line,
            point,
            bias,
        })
    }

    /// Hit-tests a local point and returns a source boundary.
    pub fn hit_test(&self, point: LayoutPoint) -> Result<LayoutHit, LayoutError> {
        point.validate()?;
        if let Some(image) = self.images.iter().find(|image| {
            let bounds = image.bounds;
            point.x >= bounds.x()
                && point.x <= bounds.x() + bounds.width()
                && point.y >= bounds.y()
                && point.y <= bounds.y() + bounds.height()
        }) {
            let bounds = image.bounds;
            let midpoint = bounds.x() + bounds.width() * 0.5;
            let before = point.x < midpoint;
            let visual = if before {
                image.visual.start()
            } else {
                image.visual.end()
            };
            let source = if before {
                image.source.start()
            } else {
                image.source.end()
            };
            return Ok(LayoutHit {
                source,
                visual,
                line: image.line,
                point: LayoutPoint::new(
                    if before {
                        bounds.x()
                    } else {
                        bounds.x() + bounds.width()
                    },
                    bounds.y(),
                ),
                bias: if before {
                    ProjectionBias::Before
                } else {
                    ProjectionBias::After
                },
                image: Some(image.source),
            });
        }
        let line = self.line_for_y(point.y);
        let line_data = &self.lines[line];
        let mut visual = line_data.visual.start();
        let mut bias = ProjectionBias::Before;
        let mut x = line_data.width;

        if point.x <= 0.0 {
            x = 0.0;
        } else {
            for cluster_index in line_data.clusters.clone() {
                let cluster = self.clusters[cluster_index];
                if cluster.line_break {
                    continue;
                }
                if point.x < cluster.x + cluster.width / 2.0 {
                    visual = cluster.visual.start();
                    x = cluster.x;
                    bias = ProjectionBias::Before;
                    break;
                }
                visual = cluster.visual.end();
                x = cluster.x + cluster.width;
                bias = ProjectionBias::After;
            }
            if point.x >= line_data.width() {
                visual = self.line_content_visual_end(line_data);
                x = line_data.width();
                bias = ProjectionBias::Before;
            }
        }

        let source = match self
            .table
            .as_ref()
            .and_then(|table| table.source_for_visual_hit(&self.projection, visual, bias))
        {
            Some(source) => source,
            None => self.projection.visual_to_source(visual, bias)?,
        };
        Ok(LayoutHit {
            source,
            visual,
            line,
            point: LayoutPoint::new(x, line_data.y()),
            bias,
            image: None,
        })
    }

    fn build_image_placements(&mut self) -> Result<(), LayoutError> {
        let mut placements = Vec::with_capacity(self.projection.images().len());
        for image in self.projection.images().iter().copied() {
            let visual_start = self
                .projection
                .source_to_visual(image.label().start(), ProjectionBias::Before)?;
            let visual_end = self
                .projection
                .source_to_visual(image.label().end(), ProjectionBias::After)?;
            let visual =
                VisualRange::new(visual_start, visual_end).ok_or(LayoutError::OffsetOverflow)?;
            let line_index = self.line_for_visual(visual.start(), ProjectionBias::Before);
            let line = &self.lines[line_index];
            let mut left = f32::INFINITY;
            let mut right = 0.0_f32;
            let mut found_cluster = false;
            for cluster_index in line.clusters.clone() {
                let cluster = self.clusters[cluster_index];
                if cluster.is_line_break() {
                    continue;
                }
                let overlaps = if visual.is_empty() {
                    cluster.visual().start() == visual.start()
                } else {
                    cluster.visual().start() < visual.end()
                        && visual.start() < cluster.visual().end()
                };
                if overlaps {
                    found_cluster = true;
                    left = left.min(cluster.x());
                    right = right.max(cluster.x() + cluster.width());
                }
            }
            if !found_cluster {
                let point = self
                    .point_for_visual(visual.start(), ProjectionBias::Before)?
                    .1;
                left = point.x();
                right = point.x();
            }
            let minimum_width = self.config.line_height * 4.0;
            let remaining = (self.config.max_width - left).max(self.config.line_height);
            let width = (right - left).max(minimum_width).min(remaining);
            let bounds = LayoutRect::new(left.max(0.0), line.y(), width, self.config.line_height)?;
            placements.push(ImagePlacement {
                source: image.source(),
                label: image.label(),
                visual,
                line: line_index,
                bounds,
            });
        }
        self.images = placements;
        Ok(())
    }

    fn build<M: ClusterMetrics>(&mut self, metrics: &M) -> Result<(), LayoutError> {
        let source_range = self.projection.source_range();
        let runs = self.projection.runs().to_vec();
        let mut line_source_start = source_range.start();
        let mut line_source_end = line_source_start;
        let mut line_visual_start = VisualOffset::ZERO;
        let mut line_width = 0.0_f32;
        let mut line_cluster_start = 0_usize;
        let mut line_index = 0_usize;
        let mut last_was_break = false;

        for run in runs {
            if let VisualRunKind::LineBreak { .. } = run.kind() {
                line_source_end = line_source_end.max(run.source().end());
                let visual_end = run.visual().end();
                self.clusters.push(VisualCluster {
                    source: run.source(),
                    visual: run.visual(),
                    line: line_index,
                    x: line_width,
                    width: 0.0,
                    style: run.style(),
                    line_break: true,
                });
                self.push_line(LineDraft {
                    index: line_index,
                    source_start: line_source_start,
                    source_end: line_source_end,
                    visual_start: line_visual_start,
                    visual_end,
                    width: line_width,
                    cluster_start: line_cluster_start,
                })?;
                line_index = line_index.saturating_add(1);
                line_cluster_start = self.clusters.len();
                line_source_start = run.source().end();
                line_source_end = run.source().end();
                line_visual_start = visual_end;
                line_width = 0.0;
                last_was_break = true;
                continue;
            }
            if run.kind() == VisualRunKind::HiddenSyntax {
                line_source_end = line_source_end.max(run.source().end());
                continue;
            }
            let text = self
                .projection
                .text_for_run(run)
                .map_err(LayoutError::Projection)?;
            for (local_start, cluster_text) in text.grapheme_indices(true) {
                let local_end = local_start
                    .checked_add(cluster_text.len())
                    .ok_or(LayoutError::OffsetOverflow)?;
                let local_start =
                    u64::try_from(local_start).map_err(|_| LayoutError::OffsetOverflow)?;
                let local_end =
                    u64::try_from(local_end).map_err(|_| LayoutError::OffsetOverflow)?;
                let cluster_source = self
                    .projection
                    .source_range_for_run_slice(run, local_start, local_end)
                    .map_err(LayoutError::Projection)?;
                let cluster_visual = self
                    .projection
                    .visual_range_for_run_slice(run, local_start, local_end)
                    .map_err(LayoutError::Projection)?;

                if cluster_text.contains('\n') {
                    line_source_end = line_source_end.max(cluster_source.end());
                    self.clusters.push(VisualCluster {
                        source: cluster_source,
                        visual: cluster_visual,
                        line: line_index,
                        x: line_width,
                        width: 0.0,
                        style: run.style(),
                        line_break: true,
                    });
                    self.push_line(LineDraft {
                        index: line_index,
                        source_start: line_source_start,
                        source_end: line_source_end,
                        visual_start: line_visual_start,
                        visual_end: cluster_visual.end(),
                        width: line_width,
                        cluster_start: line_cluster_start,
                    })?;
                    line_index = line_index.saturating_add(1);
                    line_cluster_start = self.clusters.len();
                    line_source_start = cluster_source.end();
                    line_source_end = cluster_source.end();
                    line_visual_start = cluster_visual.end();
                    line_width = 0.0;
                    last_was_break = true;
                    continue;
                }

                let advance = metrics.advance(cluster_text, run.style());
                if !advance.is_finite() || advance < 0.0 {
                    return Err(LayoutError::InvalidMetrics(advance.to_bits()));
                }
                if line_width > 0.0 && line_width + advance > self.config.max_width {
                    self.push_line(LineDraft {
                        index: line_index,
                        source_start: line_source_start,
                        source_end: line_source_end,
                        visual_start: line_visual_start,
                        visual_end: cluster_visual.start(),
                        width: line_width,
                        cluster_start: line_cluster_start,
                    })?;
                    line_index = line_index.saturating_add(1);
                    line_cluster_start = self.clusters.len();
                    line_source_start = cluster_source.start();
                    line_source_end = cluster_source.start();
                    line_visual_start = cluster_visual.start();
                    line_width = 0.0;
                }
                line_source_end = line_source_end.max(cluster_source.end());
                self.clusters.push(VisualCluster {
                    source: cluster_source,
                    visual: cluster_visual,
                    line: line_index,
                    x: line_width,
                    width: advance,
                    style: run.style(),
                    line_break: false,
                });
                line_width += advance;
                last_was_break = false;
            }
        }

        if self.lines.is_empty() || !last_was_break {
            self.push_line(LineDraft {
                index: line_index,
                source_start: line_source_start,
                source_end: line_source_end,
                visual_start: line_visual_start,
                visual_end: self.projection.visual_len(),
                width: line_width,
                cluster_start: line_cluster_start,
            })?;
        } else {
            self.push_line(LineDraft {
                index: line_index,
                source_start: line_source_start,
                source_end: line_source_end,
                visual_start: line_visual_start,
                visual_end: line_visual_start,
                width: 0.0,
                cluster_start: line_cluster_start,
            })?;
        }
        Ok(())
    }

    fn build_shaped<S: ShapingProvider>(&mut self, shaper: &S) -> Result<(), LayoutError> {
        let source_range = self.projection.source_range();
        let runs = self.projection.runs().to_vec();
        let mut line_source_start = source_range.start();
        let mut line_source_end = line_source_start;
        let mut line_visual_start = VisualOffset::ZERO;
        let mut line_width = 0.0_f32;
        let mut line_cluster_start = 0_usize;
        let mut line_index = 0_usize;
        let mut last_was_break = false;

        for run in runs {
            if let VisualRunKind::LineBreak { .. } = run.kind() {
                line_source_end = line_source_end.max(run.source().end());
                let visual_end = run.visual().end();
                self.clusters.push(VisualCluster {
                    source: run.source(),
                    visual: run.visual(),
                    line: line_index,
                    x: line_width,
                    width: 0.0,
                    style: run.style(),
                    line_break: true,
                });
                self.push_line(LineDraft {
                    index: line_index,
                    source_start: line_source_start,
                    source_end: line_source_end,
                    visual_start: line_visual_start,
                    visual_end,
                    width: line_width,
                    cluster_start: line_cluster_start,
                })?;
                line_index = line_index.saturating_add(1);
                line_cluster_start = self.clusters.len();
                line_source_start = run.source().end();
                line_source_end = run.source().end();
                line_visual_start = visual_end;
                line_width = 0.0;
                last_was_break = true;
                continue;
            }
            if run.kind() == VisualRunKind::HiddenSyntax {
                line_source_end = line_source_end.max(run.source().end());
                continue;
            }
            let text = self
                .projection
                .text_for_run(run)
                .map_err(LayoutError::Projection)?;
            let shape_source = self.projection.shape_source_range_for_run(run);
            let shaped = shaper
                .shape(&text, shape_source, run.style())
                .map_err(|error| LayoutError::Shaping(error.to_string()))?;
            if shaped.source() != shape_source {
                return Err(LayoutError::Shaping(
                    "shaper returned a source range different from the requested run".into(),
                ));
            }

            for glyph_run in shaped.runs() {
                if glyph_run.source().start() < shape_source.start()
                    || glyph_run.source().end() > shape_source.end()
                {
                    return Err(LayoutError::Shaping(
                        "glyph run source range is outside the requested visual run".into(),
                    ));
                }
                let mut previous_source_end = glyph_run.source().start();
                for glyph in glyph_run.glyphs() {
                    let glyph_source = glyph.source();
                    if glyph_source.start() < glyph_run.source().start()
                        || glyph_source.end() > glyph_run.source().end()
                        || glyph_source.start() < previous_source_end
                    {
                        return Err(LayoutError::Shaping(
                            "glyph source ranges must be ordered within their run".into(),
                        ));
                    }
                    previous_source_end = glyph_source.end();
                    let local_start = glyph_source
                        .start()
                        .get()
                        .checked_sub(shape_source.start().get())
                        .ok_or(LayoutError::OffsetOverflow)?;
                    let local_end = glyph_source
                        .end()
                        .get()
                        .checked_sub(shape_source.start().get())
                        .ok_or(LayoutError::OffsetOverflow)?;
                    let canonical_source = self
                        .projection
                        .source_range_for_run_slice(run, local_start, local_end)
                        .map_err(LayoutError::Projection)?;
                    let cluster_visual = self
                        .projection
                        .visual_range_for_run_slice(run, local_start, local_end)
                        .map_err(LayoutError::Projection)?;
                    let cluster_text = self
                        .projection
                        .text_for_run_slice(run, local_start, local_end)
                        .map_err(LayoutError::Projection)?;
                    let is_line_break = cluster_text.contains('\n');
                    let advance = if is_line_break { 0.0 } else { glyph.advance() };
                    if !advance.is_finite() || advance < 0.0 {
                        return Err(LayoutError::InvalidMetrics(advance.to_bits()));
                    }
                    if !glyph.x_offset().is_finite() || !glyph.y_offset().is_finite() {
                        return Err(LayoutError::Shaping("glyph offsets must be finite".into()));
                    }
                    if is_line_break {
                        line_source_end = line_source_end.max(canonical_source.end());
                        self.clusters.push(VisualCluster {
                            source: canonical_source,
                            visual: cluster_visual,
                            line: line_index,
                            x: line_width,
                            width: 0.0,
                            style: glyph_run.style(),
                            line_break: true,
                        });
                        self.push_line(LineDraft {
                            index: line_index,
                            source_start: line_source_start,
                            source_end: line_source_end,
                            visual_start: line_visual_start,
                            visual_end: cluster_visual.end(),
                            width: line_width,
                            cluster_start: line_cluster_start,
                        })?;
                        line_index = line_index.saturating_add(1);
                        line_cluster_start = self.clusters.len();
                        line_source_start = canonical_source.end();
                        line_source_end = canonical_source.end();
                        line_visual_start = cluster_visual.end();
                        line_width = 0.0;
                        last_was_break = true;
                        continue;
                    }

                    if line_width > 0.0 && line_width + advance > self.config.max_width {
                        self.push_line(LineDraft {
                            index: line_index,
                            source_start: line_source_start,
                            source_end: line_source_end,
                            visual_start: line_visual_start,
                            visual_end: cluster_visual.start(),
                            width: line_width,
                            cluster_start: line_cluster_start,
                        })?;
                        line_index = line_index.saturating_add(1);
                        line_cluster_start = self.clusters.len();
                        line_source_start = canonical_source.start();
                        line_source_end = canonical_source.start();
                        line_visual_start = cluster_visual.start();
                        line_width = 0.0;
                    }
                    line_source_end = line_source_end.max(canonical_source.end());
                    let glyph_x = line_width + glyph.x_offset();
                    let glyph_y = self.baseline_for_line(line_index)? + glyph.y_offset();
                    if !glyph_x.is_finite() || !glyph_y.is_finite() {
                        return Err(LayoutError::InvalidPoint);
                    }
                    self.glyphs.push(GlyphPlacement {
                        face: glyph_run.face(),
                        glyph: glyph.id(),
                        source: canonical_source,
                        visual: cluster_visual,
                        line: line_index,
                        x: glyph_x,
                        y: glyph_y,
                        style: glyph_run.style(),
                    });
                    self.clusters.push(VisualCluster {
                        source: canonical_source,
                        visual: cluster_visual,
                        line: line_index,
                        x: line_width,
                        width: advance,
                        style: glyph_run.style(),
                        line_break: false,
                    });
                    line_width += advance;
                    last_was_break = false;
                }
            }
        }

        if self.lines.is_empty() || !last_was_break {
            self.push_line(LineDraft {
                index: line_index,
                source_start: line_source_start,
                source_end: line_source_end,
                visual_start: line_visual_start,
                visual_end: self.projection.visual_len(),
                width: line_width,
                cluster_start: line_cluster_start,
            })?;
        } else {
            self.push_line(LineDraft {
                index: line_index,
                source_start: line_source_start,
                source_end: line_source_end,
                visual_start: line_visual_start,
                visual_end: line_visual_start,
                width: 0.0,
                cluster_start: line_cluster_start,
            })?;
        }
        Ok(())
    }

    /// Replaces the temporary linear text coordinates produced by the generic
    /// projection pass with the retained GFM table grid. The projection still
    /// owns every source/visual range; this method only changes layout x/y and
    /// line membership so scene glyphs, caret queries and hit-testing agree
    /// with the table cell geometry.
    fn apply_table_geometry(&mut self, table: &TableLayoutSnapshot) -> Result<(), LayoutError> {
        if table.revision() != self.revision() {
            return Err(LayoutError::InvalidTable(
                "table and text layout revisions differ",
            ));
        }
        let original_clusters = self.clusters.clone();
        let mut targets = vec![None; self.clusters.len()];
        for cell in table.cells().iter().copied() {
            let mut x = cell.content_x();
            for (index, cluster) in original_clusters.iter().copied().enumerate() {
                if cluster.is_line_break()
                    || !source_range_contains(cell.source(), cluster.source())
                {
                    continue;
                }
                if targets[index].is_some() {
                    return Err(LayoutError::InvalidTable(
                        "a visual cluster belongs to multiple table cells",
                    ));
                }
                targets[index] = Some((cell.row(), cluster.x(), x));
                self.clusters[index] = VisualCluster {
                    source: cluster.source,
                    visual: cluster.visual,
                    line: cell.row(),
                    x,
                    width: cluster.width,
                    style: cluster.style,
                    line_break: false,
                };
                x += cluster.width;
            }
        }
        if self
            .clusters
            .iter()
            .zip(targets.iter())
            .any(|(cluster, target)| !cluster.is_line_break() && target.is_none())
        {
            return Err(LayoutError::InvalidTable(
                "a table visual cluster has no source cell",
            ));
        }

        let mut used_clusters = vec![false; self.clusters.len()];
        for (glyph_index, original_glyph) in self.glyphs.clone().into_iter().enumerate() {
            let Some((index, (_, target))) = self
                .clusters
                .iter()
                .copied()
                .zip(targets.iter().copied())
                .enumerate()
                .find(|(index, (cluster, target))| {
                    !used_clusters[*index]
                        && target.is_some()
                        && cluster.source() == original_glyph.source
                        && cluster.visual() == original_glyph.visual
                })
            else {
                return Err(LayoutError::InvalidTable(
                    "a shaped table glyph has no visual cluster",
                ));
            };
            let (row, old_cluster_x, new_cluster_x) = target.expect("target checked above");
            used_clusters[index] = true;
            let old_baseline = self.baseline_for_line(original_glyph.line)?;
            let y_offset = original_glyph.y - old_baseline;
            let x_offset = original_glyph.x - old_cluster_x;
            if !x_offset.is_finite() || !y_offset.is_finite() {
                return Err(LayoutError::InvalidPoint);
            }
            let new_baseline = self.baseline_for_line(row)?;
            let glyph = &mut self.glyphs[glyph_index];
            glyph.x = new_cluster_x + x_offset;
            glyph.y = new_baseline + y_offset;
            glyph.line = row;
        }

        for image in &mut self.images {
            let Some(cell) = table
                .cells()
                .iter()
                .copied()
                .find(|cell| source_range_contains(cell.source(), image.source))
            else {
                continue;
            };
            let available = (cell.bounds().x() + cell.bounds().width() - cell.content_x()).max(1.0);
            let width = image.bounds.width().min(available).max(1.0);
            image.line = cell.row();
            image.bounds = LayoutRect::new(
                cell.content_x(),
                cell.bounds().y(),
                width,
                image.bounds.height().min(cell.bounds().height()),
            )?;
        }

        let mut lines = Vec::with_capacity(table.row_sources().len());
        let mut cluster_start = 0;
        for (row, source) in table.row_sources().iter().copied().enumerate() {
            let start = cluster_start;
            if cluster_start < self.clusters.len() && self.clusters[cluster_start].line() < row {
                return Err(LayoutError::InvalidTable(
                    "table cluster lines are not ordered",
                ));
            }
            while cluster_start < self.clusters.len() && self.clusters[cluster_start].line() == row
            {
                cluster_start += 1;
            }
            let mut row_cells = table
                .cells()
                .iter()
                .copied()
                .filter(|cell| cell.row() == row);
            let first = row_cells
                .clone()
                .next()
                .ok_or(LayoutError::InvalidTable("table row has no cells"))?;
            let last = row_cells
                .next_back()
                .ok_or(LayoutError::InvalidTable("table row has no cells"))?;
            let visual = VisualRange::new(first.visual().start(), last.visual().end())
                .ok_or(LayoutError::OffsetOverflow)?;
            lines.push(VisualLine {
                index: row,
                source,
                visual,
                y: first.bounds().y(),
                width: table.bounds().width(),
                clusters: start..cluster_start,
            });
        }
        if cluster_start != self.clusters.len() {
            return Err(LayoutError::InvalidTable(
                "table cluster lines exceed table rows",
            ));
        }
        self.lines = lines;
        Ok(())
    }

    fn push_line(&mut self, draft: LineDraft) -> Result<(), LayoutError> {
        let source = TextRange::new(draft.source_start, draft.source_end)
            .ok_or(LayoutError::OffsetOverflow)?;
        let visual = VisualRange::new(draft.visual_start, draft.visual_end)
            .ok_or(LayoutError::OffsetOverflow)?;
        self.lines.push(VisualLine {
            index: draft.index,
            source,
            visual,
            y: draft.index as f32 * self.config.line_height,
            width: draft.width,
            clusters: draft.cluster_start..self.clusters.len(),
        });
        Ok(())
    }

    fn baseline_for_line(&self, index: usize) -> Result<f32, LayoutError> {
        let line_y = index as f32 * self.config.line_height;
        let baseline = line_y + self.config.line_height;
        if baseline.is_finite() {
            Ok(baseline)
        } else {
            Err(LayoutError::InvalidPoint)
        }
    }

    fn line_for_y(&self, y: f32) -> usize {
        let raw = (y / self.config.line_height).floor();
        if raw.is_sign_negative() {
            0
        } else {
            (raw as usize).min(self.lines.len().saturating_sub(1))
        }
    }

    fn point_for_visual(
        &self,
        visual: VisualOffset,
        bias: ProjectionBias,
    ) -> Result<(usize, LayoutPoint), LayoutError> {
        let line_index = self.line_for_visual(visual, bias);
        let line = &self.lines[line_index];
        for cluster_index in line.clusters.clone() {
            let cluster = self.clusters[cluster_index];
            if visual < cluster.visual().start() {
                return Ok((line_index, LayoutPoint::new(cluster.x(), line.y())));
            }
            if visual == cluster.visual().start() {
                return Ok((line_index, LayoutPoint::new(cluster.x(), line.y())));
            }
            if visual < cluster.visual().end() {
                let x = match bias {
                    ProjectionBias::Before => cluster.x(),
                    ProjectionBias::After => cluster.x() + cluster.width(),
                };
                return Ok((line_index, LayoutPoint::new(x, line.y())));
            }
            if visual == cluster.visual().end() {
                return Ok((
                    line_index,
                    LayoutPoint::new(cluster.x() + cluster.width(), line.y()),
                ));
            }
        }
        Ok((line_index, LayoutPoint::new(line.width(), line.y())))
    }

    fn line_for_visual(&self, visual: VisualOffset, bias: ProjectionBias) -> usize {
        for (index, line) in self.lines.iter().enumerate() {
            if visual < line.visual().end()
                || (visual == line.visual().end()
                    && (bias == ProjectionBias::Before || index + 1 == self.lines.len()))
            {
                return index;
            }
        }
        self.lines.len().saturating_sub(1)
    }

    fn line_content_visual_end(&self, line: &VisualLine) -> VisualOffset {
        line.cluster_range()
            .rev()
            .map(|index| self.clusters[index])
            .find_map(|cluster| {
                if cluster.is_line_break() {
                    None
                } else {
                    Some(cluster.visual().end())
                }
            })
            .unwrap_or(line.visual().start())
    }
}

fn source_range_contains(outer: TextRange, inner: TextRange) -> bool {
    outer.start() <= inner.start() && inner.end() <= outer.end()
}

fn map_source_range(range: TextRange, changes: &ChangeSet) -> Result<TextRange, LayoutError> {
    let start = changes
        .map_anchor(TextAnchor::new(
            changes.before(),
            range.start(),
            Affinity::Before,
        ))
        .map_err(|error| LayoutError::Projection(ProjectionError::AnchorMap(error)))?
        .offset();
    let end = changes
        .map_anchor(TextAnchor::new(
            changes.before(),
            range.end(),
            Affinity::After,
        ))
        .map_err(|error| LayoutError::Projection(ProjectionError::AnchorMap(error)))?
        .offset();
    TextRange::new(start, end).ok_or(LayoutError::OffsetOverflow)
}

#[cfg(test)]
mod tests {
    use super::*;
    use unicode_segmentation::UnicodeSegmentation;
    use yu_core::{ByteOffset, TextRange};
    use yu_projection::Projection;
    use yu_text::TextBuffer;

    fn projection(source: &str) -> Projection {
        let snapshot = TextBuffer::new(source).snapshot();
        let range = TextRange::new(ByteOffset::ZERO, snapshot.len_bytes())
            .expect("source range should be ordered");
        Projection::inline(&snapshot, range).expect("projection should build")
    }

    #[test]
    fn image_placement_uses_alt_span_and_hit_test_returns_whole_source_range() {
        let source = "![alt](image.png)";
        let projection = projection(source);
        let layout = LayoutSnapshot::from_projection(&projection, LayoutConfig::new(80.0, 10.0))
            .expect("image layout");
        assert_eq!(layout.images().len(), 1);
        let image = layout.images()[0];
        assert_eq!(image.label().start().get(), 2);
        assert_eq!(image.label().end().get(), 5);
        assert!(image.bounds().width() >= 40.0);
        assert_eq!(image.bounds().y(), 0.0);

        let left_hit = layout
            .hit_test(LayoutPoint::new(
                image.bounds().x() + 1.0,
                image.bounds().y() + 5.0,
            ))
            .expect("left image hit");
        assert_eq!(left_hit.image(), Some(image.source()));
        assert_eq!(left_hit.source(), image.source().start());

        let right_hit = layout
            .hit_test(LayoutPoint::new(
                image.bounds().x() + image.bounds().width() - 1.0,
                image.bounds().y() + 5.0,
            ))
            .expect("right image hit");
        assert_eq!(right_hit.image(), Some(image.source()));
        assert_eq!(right_hit.source(), image.source().end());
    }

    #[test]
    fn intrinsic_image_size_preserves_aspect_ratio_and_hit_test_bounds() {
        let source = "![alt](image.png)";
        let projection = projection(source);
        let mut layout =
            LayoutSnapshot::from_projection(&projection, LayoutConfig::new(80.0, 10.0))
                .expect("image layout");
        let image_source = layout.images()[0].source();
        layout
            .apply_image_intrinsic_sizes(&[(
                image_source,
                ImageIntrinsicSize::new(200, 100).expect("dimensions"),
            )])
            .expect("intrinsic size");
        let image = layout.images()[0];
        assert_eq!(image.bounds().width(), 80.0);
        assert_eq!(image.bounds().height(), 40.0);
        assert_eq!(layout.block_height(), 40.0);
        let hit = layout
            .hit_test(LayoutPoint::new(79.0, 39.0))
            .expect("intrinsic image hit");
        assert_eq!(hit.image(), Some(image_source));
    }

    #[derive(Clone, Copy)]
    enum TestShape {
        FixedGrapheme(f32),
        Ligature(f32),
        InvalidAdvance,
        Error,
    }

    #[derive(Clone, Copy)]
    struct TestShaper {
        shape: TestShape,
    }

    impl ShapingProvider for TestShaper {
        type Error = &'static str;

        fn shape(
            &self,
            text: &str,
            source: TextRange,
            style: VisualRunStyle,
        ) -> Result<ShapedText, Self::Error> {
            if matches!(self.shape, TestShape::Error) {
                return Err("boom");
            }
            let glyphs = match self.shape {
                TestShape::FixedGrapheme(advance) => text
                    .grapheme_indices(true)
                    .map(|(start, cluster)| {
                        let end = start + cluster.len();
                        let source_start = source
                            .start()
                            .checked_add(u64::try_from(start).expect("test offset fits"))
                            .expect("source offset fits");
                        let source_end = source
                            .start()
                            .checked_add(u64::try_from(end).expect("test offset fits"))
                            .expect("source offset fits");
                        let source = TextRange::new(source_start, source_end)
                            .expect("glyph range should be ordered");
                        Glyph::new(GlyphId::from_raw(1), source, advance, 0.0, 0.0)
                    })
                    .collect(),
                TestShape::Ligature(advance) => {
                    vec![Glyph::new(GlyphId::from_raw(2), source, advance, 0.0, 0.0)]
                }
                TestShape::InvalidAdvance => {
                    vec![Glyph::new(GlyphId::from_raw(3), source, f32::NAN, 0.0, 0.0)]
                }
                TestShape::Error => unreachable!(),
            };
            Ok(ShapedText::new(
                source,
                vec![GlyphRun::new(
                    FontFaceId::from_raw(1),
                    source,
                    style,
                    TextDirection::Ltr,
                    Script::Unknown,
                    glyphs,
                )],
            ))
        }
    }

    #[test]
    fn layout_wraps_graphemes_without_splitting_unicode() {
        let projection = projection("羽🙂ab");
        let layout = LayoutSnapshot::from_projection(&projection, LayoutConfig::new(2.0, 1.5))
            .expect("layout should build");

        assert_eq!(layout.lines().len(), 2);
        assert_eq!(layout.clusters().len(), 4);
        assert_eq!(layout.clusters()[0].source().len(), "羽".len() as u64);
        assert_eq!(layout.clusters()[1].source().len(), "🙂".len() as u64);
        assert_eq!(layout.lines()[0].width(), 2.0);
        assert_eq!(layout.lines()[1].width(), 2.0);
    }

    #[test]
    fn layout_keeps_hidden_delimiters_in_source_line_but_not_width() {
        let source = "**羽🙂**";
        let layout = LayoutSnapshot::from_projection(&projection(source), LayoutConfig::default())
            .expect("layout should build");

        assert_eq!(layout.lines().len(), 1);
        assert_eq!(layout.lines()[0].source().len(), source.len() as u64);
        assert_eq!(layout.lines()[0].width(), 2.0);
        assert_eq!(layout.clusters().len(), 2);
    }

    #[test]
    fn source_visual_caret_and_hit_test_round_trip_at_cluster_boundaries() {
        let source = "**羽🙂**";
        let layout = LayoutSnapshot::from_projection(&projection(source), LayoutConfig::default())
            .expect("layout should build");
        let emoji_start = ByteOffset::new(usize_to_u64(source.find('🙂').expect("emoji exists")));
        let caret = layout
            .caret_for_source(emoji_start, ProjectionBias::After)
            .expect("caret should resolve");
        let hit = layout
            .hit_test(LayoutPoint::new(caret.point().x() + 0.1, caret.point().y()))
            .expect("hit-test should resolve");

        assert_eq!(caret.source(), emoji_start);
        assert_eq!(hit.source(), emoji_start);
        assert_eq!(hit.line(), 0);
    }

    #[test]
    fn newline_creates_a_following_empty_caret_line() {
        let layout = LayoutSnapshot::from_projection(&projection("a\n"), LayoutConfig::default())
            .expect("layout should build");
        assert_eq!(layout.lines().len(), 2);
        let caret = layout
            .caret_for_source(ByteOffset::new(2), ProjectionBias::After)
            .expect("end caret should resolve");
        assert_eq!(caret.line(), 1);
        assert_eq!(caret.point().y(), 1.0);
        let end_of_first_line = layout
            .hit_test(LayoutPoint::new(10.0, 0.0))
            .expect("line-end hit-test should resolve");
        assert_eq!(end_of_first_line.source(), ByteOffset::new(1));
    }

    #[test]
    fn layout_consumes_explicit_soft_and_hard_break_runs() {
        let source = "a  \nb\r\nc";
        let layout = LayoutSnapshot::from_projection(&projection(source), LayoutConfig::default())
            .expect("layout should build");

        assert_eq!(layout.lines().len(), 3);
        assert_eq!(layout.lines()[0].width(), 1.0);
        assert_eq!(layout.lines()[1].width(), 1.0);
        assert_eq!(layout.lines()[2].width(), 1.0);
        assert_eq!(
            layout
                .clusters()
                .iter()
                .filter(|cluster| cluster.is_line_break())
                .count(),
            2
        );
        assert_eq!(layout.lines()[0].source().end().get(), 4);
        assert_eq!(layout.lines()[0].visual().end().get(), 2);
        assert_eq!(layout.lines()[1].source().start().get(), 4);
        assert_eq!(layout.lines()[1].source().end().get(), 7);
        assert_eq!(layout.lines()[1].visual().start().get(), 2);
        assert_eq!(layout.lines()[1].visual().end().get(), 5);

        let after_first_break = layout
            .caret_for_source(ByteOffset::new(4), ProjectionBias::After)
            .expect("caret after hard break should resolve");
        assert_eq!(after_first_break.line(), 1);
        assert_eq!(after_first_break.point().x(), 0.0);
        let after_second_break = layout
            .caret_for_source(ByteOffset::new(7), ProjectionBias::After)
            .expect("caret after CRLF should resolve");
        assert_eq!(after_second_break.line(), 2);
        assert_eq!(after_second_break.point().x(), 0.0);
    }

    #[test]
    fn shaped_layout_consumes_explicit_line_break_runs_without_shaping_markers() {
        let layout = LayoutSnapshot::from_projection_with_shaper(
            &projection("a  \nb\r\nc"),
            LayoutConfig::default(),
            &TestShaper {
                shape: TestShape::FixedGrapheme(1.0),
            },
        )
        .expect("shaped layout should build");

        assert_eq!(layout.lines().len(), 3);
        assert_eq!(layout.glyphs().len(), 3);
        assert_eq!(layout.clusters().len(), 5);
        assert!(
            layout
                .clusters()
                .iter()
                .filter(|cluster| cluster.is_line_break())
                .all(|cluster| cluster.width() == 0.0)
        );
    }

    #[test]
    fn code_style_and_literal_delimiters_reach_layout() {
        let projection = projection("`**literal**`");
        let layout = LayoutSnapshot::from_projection(&projection, LayoutConfig::default())
            .expect("layout should build");
        assert!(
            layout
                .clusters()
                .iter()
                .all(|cluster| cluster.style() == VisualRunStyle::Code)
        );
        assert_eq!(layout.lines()[0].width(), 11.0);
    }

    #[test]
    fn caller_metrics_control_wrapping_without_changing_clusters() {
        let projection = projection("abcd");
        let metrics = MonospaceMetrics::new(2.0);
        let layout = LayoutSnapshot::from_projection_with_metrics(
            &projection,
            LayoutConfig::new(4.0, 1.0),
            &metrics,
        )
        .expect("custom metrics should build");

        assert_eq!(layout.lines().len(), 2);
        assert_eq!(layout.clusters().len(), 4);
        assert_eq!(layout.lines()[0].width(), 4.0);
        assert_eq!(
            layout.lines()[0].source(),
            TextRange::new(ByteOffset::ZERO, ByteOffset::new(2)).expect("line range")
        );
        assert_eq!(
            layout.lines()[1].source(),
            TextRange::new(ByteOffset::new(2), ByteOffset::new(4)).expect("line range")
        );
    }

    #[test]
    fn shaped_glyph_advances_control_wrapping_and_preserve_source_clusters() {
        let layout = LayoutSnapshot::from_projection_with_shaper(
            &projection("abcd"),
            LayoutConfig::new(4.0, 1.0),
            &TestShaper {
                shape: TestShape::FixedGrapheme(2.0),
            },
        )
        .expect("shaped layout should build");

        assert_eq!(layout.lines().len(), 2);
        assert_eq!(layout.lines()[0].width(), 4.0);
        assert_eq!(
            layout.lines()[0].source(),
            TextRange::new(ByteOffset::ZERO, ByteOffset::new(2)).expect("line range")
        );
        assert_eq!(
            layout.lines()[1].source(),
            TextRange::new(ByteOffset::new(2), ByteOffset::new(4)).expect("line range")
        );
        assert_eq!(layout.clusters().len(), 4);
        assert_eq!(layout.clusters()[0].source().len(), 1);
        assert_eq!(layout.clusters()[3].source().start().get(), 3);
    }

    #[test]
    fn shaped_layout_retains_baseline_glyph_placements() {
        let layout = LayoutSnapshot::from_projection_with_shaper(
            &projection("abcd"),
            LayoutConfig::new(4.0, 2.0),
            &TestShaper {
                shape: TestShape::FixedGrapheme(2.0),
            },
        )
        .expect("shaped layout should build");

        assert_eq!(layout.glyphs().len(), 4);
        assert_eq!(layout.glyphs()[0].face(), FontFaceId::from_raw(1));
        assert_eq!(layout.glyphs()[0].glyph(), GlyphId::from_raw(1));
        assert_eq!(layout.glyphs()[0].source().start().get(), 0);
        assert_eq!(layout.glyphs()[0].x(), 0.0);
        assert_eq!(layout.glyphs()[0].y(), 2.0);
        assert_eq!(layout.glyphs()[2].line(), 1);
        assert_eq!(layout.glyphs()[2].x(), 0.0);
        assert_eq!(layout.glyphs()[2].y(), 4.0);
    }

    #[test]
    fn shaped_ligature_cluster_is_not_split() {
        let layout = LayoutSnapshot::from_projection_with_shaper(
            &projection("fi"),
            LayoutConfig::new(0.5, 1.0),
            &TestShaper {
                shape: TestShape::Ligature(1.0),
            },
        )
        .expect("ligature layout should build");

        assert_eq!(layout.lines().len(), 1);
        assert_eq!(layout.lines()[0].width(), 1.0);
        assert_eq!(layout.clusters().len(), 1);
        assert_eq!(layout.clusters()[0].source().len(), 2);
    }

    #[test]
    fn shaping_errors_are_reported_by_layout() {
        let result = LayoutSnapshot::from_projection_with_shaper(
            &projection("text"),
            LayoutConfig::default(),
            &TestShaper {
                shape: TestShape::Error,
            },
        );
        assert!(matches!(
            result,
            Err(LayoutError::Shaping(message)) if message == "boom"
        ));
    }

    #[test]
    fn invalid_shaped_advance_is_rejected() {
        let result = LayoutSnapshot::from_projection_with_shaper(
            &projection("text"),
            LayoutConfig::default(),
            &TestShaper {
                shape: TestShape::InvalidAdvance,
            },
        );
        assert!(matches!(
            result,
            Err(LayoutError::InvalidMetrics(bits)) if f32::from_bits(bits).is_nan()
        ));
    }

    #[test]
    fn invalid_layout_config_is_rejected_before_scanning_source() {
        let result =
            LayoutSnapshot::from_projection(&projection("text"), LayoutConfig::new(0.0, 1.0));
        assert!(matches!(result, Err(LayoutError::InvalidConfig(_))));
    }

    #[test]
    fn metrics_layout_uses_configured_default_advance() {
        let layout = LayoutSnapshot::from_projection(
            &projection("ab"),
            LayoutConfig::new(10.0, 1.0).with_default_advance(2.5),
        )
        .expect("configured metrics should build");
        assert_eq!(layout.lines()[0].width(), 5.0);
    }

    #[test]
    fn invalid_default_advance_is_rejected() {
        let result = LayoutSnapshot::from_projection(
            &projection("text"),
            LayoutConfig::new(10.0, 1.0).with_default_advance(0.0),
        );
        assert!(matches!(result, Err(LayoutError::InvalidConfig(_))));
    }

    #[test]
    fn height_index_supports_prefix_updates_and_viewport_lookup() {
        let mut index = HeightIndex::new([10.0, 20.0, 15.0]).expect("heights should be valid");
        assert_eq!(index.total_height(), 45.0);
        assert_eq!(index.prefix_height(2), 30.0);
        assert_eq!(index.find_line(0.0), Some(0));
        assert_eq!(index.find_line(10.0), Some(1));
        assert_eq!(index.find_line(30.0), Some(2));
        index.set(1, 25.0).expect("point update should succeed");
        assert_eq!(index.total_height(), 50.0);
        index.push(5.0).expect("append should succeed");
        assert_eq!(index.len(), 4);
        assert_eq!(index.find_line(49.0), Some(2));
    }

    #[test]
    fn layout_map_through_prefix_edit_preserves_visual_coordinates() {
        let source_text = "prefix **羽**";
        let mut buffer = TextBuffer::new(source_text);
        let snapshot = buffer.snapshot();
        let start = source_text.find("**").expect("strong span exists");
        let range = TextRange::new(ByteOffset::new(usize_to_u64(start)), snapshot.len_bytes())
            .expect("projection range should be ordered");
        let projection = Projection::inline(&snapshot, range).expect("projection should build");
        let layout = LayoutSnapshot::from_projection(&projection, LayoutConfig::default())
            .expect("layout should build");
        let transaction = yu_text::Transaction::new(
            buffer.revision(),
            [yu_text::Edit::new(TextRange::empty(ByteOffset::ZERO), "前")],
        );
        let applied = buffer
            .apply(&transaction)
            .expect("prefix edit should apply");

        let mapped = layout
            .map_through(applied.change_set(), applied.result_snapshot())
            .expect("layout should map")
            .expect("outside edit should preserve layout");
        assert_eq!(mapped.visual_len(), layout.visual_len());
        assert_eq!(mapped.source_range().start().get(), range.start().get() + 3);
        assert_eq!(
            mapped.clusters()[0].source().start().get(),
            layout.clusters()[0].source().start().get() + 3
        );
        assert_eq!(mapped.revision(), applied.result_snapshot().revision());
    }

    #[test]
    fn shaped_glyph_sources_map_through_prefix_edit() {
        let source_text = "prefix ab";
        let mut buffer = TextBuffer::new(source_text);
        let snapshot = buffer.snapshot();
        let start = source_text.find("ab").expect("text exists");
        let range = TextRange::new(ByteOffset::new(usize_to_u64(start)), snapshot.len_bytes())
            .expect("projection range should be ordered");
        let projection = Projection::inline(&snapshot, range).expect("projection should build");
        let layout = LayoutSnapshot::from_projection_with_shaper(
            &projection,
            LayoutConfig::new(4.0, 1.0),
            &TestShaper {
                shape: TestShape::FixedGrapheme(1.0),
            },
        )
        .expect("layout should build");
        let transaction = yu_text::Transaction::new(
            buffer.revision(),
            [yu_text::Edit::new(TextRange::empty(ByteOffset::ZERO), "前")],
        );
        let applied = buffer
            .apply(&transaction)
            .expect("prefix edit should apply");

        let mapped = layout
            .map_through(applied.change_set(), applied.result_snapshot())
            .expect("layout should map")
            .expect("outside edit should preserve layout");
        assert_eq!(mapped.glyphs().len(), layout.glyphs().len());
        assert_eq!(mapped.glyphs()[0].x(), layout.glyphs()[0].x());
        assert_eq!(mapped.glyphs()[0].y(), layout.glyphs()[0].y());
        assert_eq!(mapped.glyphs()[0].source().start().get(), 10);
    }

    #[test]
    fn table_layout_hides_delimiter_row_and_hit_tests_source_cells() {
        let source = "prefix\n\n| A | B |\n| --- | :---: |\n| 1 | 2 |\n";
        let buffer = TextBuffer::new(source);
        let snapshot = buffer.snapshot();
        let markdown = yu_markdown::parse(&snapshot);
        let block = markdown.blocks().get(2).expect("table block");
        let projection = BlockProjection::from_block_with_definitions(
            &snapshot,
            block,
            markdown.reference_definitions(),
        )
        .expect("table projection");
        let layout = LayoutSnapshot::from_block_projection_with_metrics(
            &projection,
            LayoutConfig::new(20.0, 2.0),
            &MonospaceMetrics::new(1.0),
        )
        .expect("table layout");
        let table = layout.table().expect("table layout metadata");

        assert_eq!(table.cells().len(), 4);
        assert_eq!(table.column_widths(), &[3.0, 3.0]);
        assert_eq!(table.bounds().width(), 6.0);
        assert_eq!(table.bounds().height(), 4.0);
        let delimiter = table.delimiter_source().expect("delimiter source");
        assert_eq!(
            &source[usize::try_from(delimiter.start()).expect("delimiter start")
                ..usize::try_from(delimiter.end()).expect("delimiter end")],
            "| --- | :---: |\n"
        );

        let hit = table
            .hit_test(LayoutPoint::new(3.5, 2.5))
            .expect("hit-test should validate")
            .expect("point should land in body cell");
        assert_eq!(hit.row(), 1);
        assert_eq!(hit.column(), 1);
        assert_eq!(
            &source[usize::try_from(hit.source().start()).expect("hit source start")
                ..usize::try_from(hit.source().end()).expect("hit source end")],
            "2"
        );

        let visual = layout
            .projection()
            .runs()
            .iter()
            .filter(|run| run.kind() != yu_projection::VisualRunKind::HiddenSyntax)
            .map(|run| layout.projection().text_for_run(*run))
            .collect::<Result<String, _>>()
            .expect("table visual text");
        assert!(!visual.contains("---"));
    }

    #[test]
    fn shaped_table_glyphs_follow_source_cells_and_visual_rows() {
        let source = "| A | B |\n| --- | :---: |\n| 1 | 2 |\n";
        let buffer = TextBuffer::new(source);
        let snapshot = buffer.snapshot();
        let markdown = yu_markdown::parse(&snapshot);
        let block = markdown.blocks().get(0).expect("table block");
        let projection = BlockProjection::from_block_with_definitions(
            &snapshot,
            block,
            markdown.reference_definitions(),
        )
        .expect("table projection");
        let layout = LayoutSnapshot::from_block_projection_with_shaper(
            &projection,
            LayoutConfig::new(20.0, 2.0),
            &TestShaper {
                shape: TestShape::FixedGrapheme(1.0),
            },
        )
        .expect("shaped table layout");

        assert_eq!(layout.lines().len(), 2);
        assert_eq!(layout.lines()[0].width(), 6.0);
        assert_eq!(layout.lines()[1].width(), 6.0);
        assert_eq!(layout.glyphs().len(), 4);
        assert_eq!(
            layout
                .glyphs()
                .iter()
                .map(|glyph| (glyph.x(), glyph.y(), glyph.line()))
                .collect::<Vec<_>>(),
            vec![(1.0, 2.0, 0), (4.0, 2.0, 0), (1.0, 4.0, 1), (4.0, 4.0, 1)]
        );

        let first_cell = layout.table().expect("table metadata").cells()[0];
        assert_eq!(first_cell.visual().start().get(), 0);
        assert_eq!(first_cell.visual().end().get(), 1);
        assert_eq!(first_cell.content_x(), 1.0);
        let body_hit = layout
            .hit_test(LayoutPoint::new(4.1, 2.5))
            .expect("table hit-test");
        assert_eq!(body_hit.line(), 1);
        assert_eq!(
            body_hit.source(),
            ByteOffset::new(source.rfind('2').expect("body 2") as u64)
        );
    }

    #[test]
    fn table_hit_test_keeps_interior_cell_source_boundaries() {
        let source = "| AB | CD |\n| --- | --- |\n| 12 | 34 |\n";
        let buffer = TextBuffer::new(source);
        let snapshot = buffer.snapshot();
        let markdown = yu_markdown::parse(&snapshot);
        let block = markdown.blocks().get(0).expect("table block");
        let projection = BlockProjection::from_block_with_definitions(
            &snapshot,
            block,
            markdown.reference_definitions(),
        )
        .expect("table projection");
        let layout = LayoutSnapshot::from_block_projection_with_shaper(
            &projection,
            LayoutConfig::new(40.0, 2.0),
            &TestShaper {
                shape: TestShape::FixedGrapheme(1.0),
            },
        )
        .expect("shaped table layout");

        let interior_hit = layout
            .hit_test(LayoutPoint::new(2.1, 0.5))
            .expect("interior table hit-test");
        assert_eq!(
            interior_hit.source(),
            ByteOffset::new(source.find('B').expect("cell B") as u64)
        );
        assert_eq!(interior_hit.line(), 0);
    }

    #[test]
    fn table_layout_maps_cell_ranges_through_prefix_edits() {
        let source = "intro\n\n| A | B |\n| --- | --- |\n| 1 | 2 |\n";
        let mut buffer = TextBuffer::new(source);
        let snapshot = buffer.snapshot();
        let markdown = yu_markdown::parse(&snapshot);
        let block = markdown.blocks().get(2).expect("table block");
        let projection = BlockProjection::from_block_with_definitions(
            &snapshot,
            block,
            markdown.reference_definitions(),
        )
        .expect("table projection");
        let layout =
            LayoutSnapshot::from_block_projection(&projection, LayoutConfig::new(20.0, 1.0))
                .expect("table layout");
        let old_cell = layout.table().expect("table metadata").cells()[0].source();
        let transaction = yu_text::Transaction::new(
            buffer.revision(),
            [yu_text::Edit::new(TextRange::empty(ByteOffset::ZERO), "前")],
        );
        let applied = buffer.apply(&transaction).expect("prefix edit");
        let mapped = layout
            .map_through(applied.change_set(), applied.result_snapshot())
            .expect("layout should map")
            .expect("outside edit should preserve table layout");
        assert_eq!(
            mapped.table().expect("mapped table").cells()[0]
                .source()
                .start(),
            ByteOffset::new(old_cell.start().get() + 3)
        );
        assert_eq!(
            mapped.table().expect("mapped table").revision(),
            applied.result_snapshot().revision()
        );
    }

    fn usize_to_u64(value: usize) -> u64 {
        u64::try_from(value).expect("test offset should fit")
    }
}
