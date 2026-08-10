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
use yu_core::{ByteOffset, TextRange};
use yu_projection::{
    BlockProjection, Projection, ProjectionBias, ProjectionError, VisualOffset, VisualRange,
    VisualRunKind, VisualRunStyle,
};

/// Layout dimensions and wrapping policy independent of any font backend.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LayoutConfig {
    max_width: f32,
    line_height: f32,
}

impl LayoutConfig {
    #[must_use]
    pub const fn new(max_width: f32, line_height: f32) -> Self {
        Self {
            max_width,
            line_height,
        }
    }

    #[must_use]
    pub const fn max_width(self) -> f32 {
        self.max_width
    }

    #[must_use]
    pub const fn line_height(self) -> f32 {
        self.line_height
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
/// A future `yu-font` implementation can provide shaping-aware metrics without
/// changing the layout tree or hit-test contracts.
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
    InvalidPoint,
    OffsetOverflow,
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
            Self::InvalidPoint => {
                formatter.write_str("layout point must contain finite coordinates")
            }
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
            | Self::InvalidPoint
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
}

/// A revision-bound, block-local layout snapshot.
#[derive(Clone, Debug)]
pub struct LayoutSnapshot {
    projection: Projection,
    config: LayoutConfig,
    lines: Vec<VisualLine>,
    clusters: Vec<VisualCluster>,
}

impl LayoutSnapshot {
    /// Builds layout with deterministic one-unit-per-grapheme metrics.
    pub fn from_projection(
        projection: &Projection,
        config: LayoutConfig,
    ) -> Result<Self, LayoutError> {
        let metrics = MonospaceMetrics::default();
        Self::from_projection_with_metrics(projection, config, &metrics)
    }

    /// Builds layout from either an inline or fenced-code block projection.
    pub fn from_block_projection(
        projection: &BlockProjection,
        config: LayoutConfig,
    ) -> Result<Self, LayoutError> {
        Self::from_projection(projection.visual(), config)
    }

    /// Builds layout from a block projection with caller-provided metrics.
    pub fn from_block_projection_with_metrics<M: ClusterMetrics>(
        projection: &BlockProjection,
        config: LayoutConfig,
        metrics: &M,
    ) -> Result<Self, LayoutError> {
        Self::from_projection_with_metrics(projection.visual(), config, metrics)
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
        };
        layout.build(metrics)?;
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

    #[must_use]
    pub fn projection(&self) -> &Projection {
        &self.projection
    }

    /// Resolves a source boundary to a line, visual boundary and point.
    pub fn caret_for_source(
        &self,
        source: ByteOffset,
        bias: ProjectionBias,
    ) -> Result<LayoutCaret, LayoutError> {
        let visual = self.projection.source_to_visual(source, bias)?;
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

        let source = self.projection.visual_to_source(visual, bias)?;
        Ok(LayoutHit {
            source,
            visual,
            line,
            point: LayoutPoint::new(x, line_data.y()),
            bias,
        })
    }

    fn build<M: ClusterMetrics>(&mut self, metrics: &M) -> Result<(), LayoutError> {
        let source_range = self.projection.source_range();
        let source = self.projection.source().clone();
        let runs = self.projection.runs().to_vec();
        let mut line_source_start = source_range.start();
        let mut line_source_end = line_source_start;
        let mut line_visual_start = VisualOffset::ZERO;
        let mut line_width = 0.0_f32;
        let mut line_cluster_start = 0_usize;
        let mut line_index = 0_usize;
        let mut last_was_break = false;

        for run in runs {
            line_source_end = line_source_end.max(run.source().end());
            if run.kind() != VisualRunKind::Visible {
                continue;
            }
            let text = read_source_range(&source, run.source())?;
            for (local_start, cluster_text) in text.grapheme_indices(true) {
                let local_end = local_start
                    .checked_add(cluster_text.len())
                    .ok_or(LayoutError::OffsetOverflow)?;
                let source_start = add_offset(run.source().start(), local_start)?;
                let source_end = add_offset(run.source().start(), local_end)?;
                let visual_start = add_visual(run.visual().start(), local_start)?;
                let visual_end = add_visual(run.visual().start(), local_end)?;
                let cluster_source =
                    TextRange::new(source_start, source_end).ok_or(LayoutError::OffsetOverflow)?;
                let cluster_visual = VisualRange::new(visual_start, visual_end)
                    .ok_or(LayoutError::OffsetOverflow)?;
                line_source_end = line_source_end.max(source_end);

                if cluster_text.contains('\n') {
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
                        visual_end,
                        width: line_width,
                        cluster_start: line_cluster_start,
                    })?;
                    line_index = line_index.saturating_add(1);
                    line_cluster_start = self.clusters.len();
                    line_source_start = source_end;
                    line_source_end = source_end;
                    line_visual_start = visual_end;
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
                        visual_end: visual_start,
                        width: line_width,
                        cluster_start: line_cluster_start,
                    })?;
                    line_index = line_index.saturating_add(1);
                    line_cluster_start = self.clusters.len();
                    line_source_start = cluster_source.start();
                    line_source_end = cluster_source.start();
                    line_visual_start = visual_start;
                    line_width = 0.0;
                }
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

fn add_offset(start: ByteOffset, local: usize) -> Result<ByteOffset, LayoutError> {
    start
        .checked_add(u64::try_from(local).map_err(|_| LayoutError::OffsetOverflow)?)
        .ok_or(LayoutError::OffsetOverflow)
}

fn add_visual(start: VisualOffset, local: usize) -> Result<VisualOffset, LayoutError> {
    start
        .checked_add(u64::try_from(local).map_err(|_| LayoutError::OffsetOverflow)?)
        .ok_or(LayoutError::OffsetOverflow)
}

fn read_source_range(
    source: &yu_text::TextSnapshot,
    range: TextRange,
) -> Result<String, LayoutError> {
    let start = usize::try_from(range.start()).map_err(|_| LayoutError::OffsetOverflow)?;
    let end = usize::try_from(range.end()).map_err(|_| LayoutError::OffsetOverflow)?;
    let mut text = String::with_capacity(end.saturating_sub(start));
    let mut cursor = source
        .chunk_cursor(range.start())
        .map_err(|error| LayoutError::Projection(ProjectionError::SourcePosition(error)))?;
    for chunk in &mut cursor {
        let chunk_start =
            usize::try_from(chunk.start()).map_err(|_| LayoutError::OffsetOverflow)?;
        let chunk_end = chunk_start
            .checked_add(chunk.text().len())
            .ok_or(LayoutError::OffsetOverflow)?;
        if chunk_start >= end {
            break;
        }
        let local_start = start.max(chunk_start).saturating_sub(chunk_start);
        let local_end = end.min(chunk_end).saturating_sub(chunk_start);
        if local_start < local_end {
            text.push_str(&chunk.text()[local_start..local_end]);
        }
    }
    Ok(text)
}

#[cfg(test)]
mod tests {
    use super::*;
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
    }

    #[test]
    fn invalid_layout_config_is_rejected_before_scanning_source() {
        let result =
            LayoutSnapshot::from_projection(&projection("text"), LayoutConfig::new(0.0, 1.0));
        assert!(matches!(result, Err(LayoutError::InvalidConfig(_))));
    }

    fn usize_to_u64(value: usize) -> u64 {
        u64::try_from(value).expect("test offset should fit")
    }
}
