//! Generated TOC text and navigation in visual UTF-8 coordinates.
use std::ops::Range;
use std::sync::Arc;

use yu_core::{Revision, TextRange};
use yu_markdown::MarkdownDocument;

use crate::{DecorationError, VisualText};

#[derive(Clone, Debug, Default)]
pub(crate) struct TocCache {
    revision: Option<Revision>,
    value: Arc<std::sync::OnceLock<Result<Arc<TocProjection>, DecorationError>>>,
}

impl TocCache {
    pub(crate) fn bind(&mut self, revision: Revision) {
        if self.revision != Some(revision) {
            self.revision = Some(revision);
            self.value = Arc::default();
        }
    }

    pub(crate) fn get(
        &mut self,
        markdown: &MarkdownDocument,
    ) -> Result<Arc<TocProjection>, DecorationError> {
        self.bind(markdown.revision());
        self.value
            .get_or_init(|| TocProjection::build(markdown).map(Arc::new))
            .clone()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TocLink {
    /// Relative to the beginning of generated text, not canonical source.
    pub visual: Range<usize>,
    pub target: TextRange,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TocProjection {
    pub revision: Revision,
    pub text: Arc<str>,
    pub links: Arc<[TocLink]>,
}

impl TocProjection {
    pub fn target_for_hit(
        &self,
        marker: TextRange,
        block: &crate::BlockView,
        hit: crate::BlockHit,
    ) -> Option<TextRange> {
        if block.visual().revision() != self.revision || hit.content_source() != Some(marker) {
            return None;
        }
        let base = block
            .visual()
            .canonical_source_to_visual(marker.start())
            .get();
        let offset = hit.content_visual()?.get().checked_sub(base)?;
        self.target_at(usize::try_from(offset).ok()?)
    }

    pub fn build(document: &MarkdownDocument) -> Result<Self, DecorationError> {
        let mut decorations_cache = crate::DecorationCache::default();
        let mut text = String::new();
        let mut links = Vec::new();
        let mut depths: Vec<usize> = Vec::new();
        for heading in document.table_of_contents().headings() {
            let block = document
                .semantic_blocks()
                .get(heading.block)
                .expect("indexed heading");
            let decorations = decorations_cache.decorate_navigation(document, block)?;
            let visual =
                VisualText::new(document.source(), heading.label, decorations.set().clone())?;
            let label = crate::panel::fold_lines(visual.text());
            let depth = heading.parent.map_or(0, |parent| depths[parent] + 1);
            depths.push(depth);
            if !text.is_empty() {
                text.push('\n');
            }
            for _ in 0..depth {
                text.push_str("  ");
            }
            let start = text.len();
            text.push_str(if label.is_empty() {
                "未命名标题"
            } else {
                &label
            });
            links.push(TocLink {
                visual: start..text.len(),
                target: heading.label,
            });
        }
        if text.is_empty() {
            text.push_str("暂无标题");
        }
        Ok(Self {
            revision: document.revision(),
            text: text.into(),
            links: links.into(),
        })
    }

    /// Painted text identity is independent of the nearest caret edge.
    pub fn target_at(&self, visual_byte: usize) -> Option<TextRange> {
        let index = self
            .links
            .partition_point(|link| link.visual.end <= visual_byte);
        self.links
            .get(index)
            .filter(|link| link.visual.contains(&visual_byte))
            .map(|link| link.target)
    }
}
