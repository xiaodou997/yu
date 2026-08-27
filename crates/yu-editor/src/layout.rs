use yu_core::{Revision, ShapingProvider, TextRange};
use yu_layout::{LayoutConfig, LayoutError};

use crate::blockview::BlockView;
use crate::visual::VisualText;
use crate::widget::ImageSize;
use yu_markdown::{Block, BlockDecorations, BlockKind, MarkdownDocument};
use yu_text::{ChangeSet, TextSnapshot};

/// Cumulative counters for one editor's revision-bound layout cache.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct LayoutCacheStats {
    entries: usize,
    builds: u64,
    hits: u64,
    remapped: u64,
    invalidated: u64,
}

/// The source of advances used by one cached layout.
///
/// Metrics and shaped layouts deliberately occupy different cache keys. This
/// prevents a fast fallback measurement from being returned after a caller
/// switches to a glyph-level shaping provider.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum LayoutBackend {
    #[default]
    Metrics,
    Shaped,
}

impl LayoutCacheStats {
    #[must_use]
    pub const fn entries(self) -> usize {
        self.entries
    }

    #[must_use]
    pub const fn builds(self) -> u64 {
        self.builds
    }

    #[must_use]
    pub const fn hits(self) -> u64 {
        self.hits
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

/// Revision-bound layout snapshots keyed by block identity, layout config and
/// advance backend.
#[derive(Debug, Default)]
pub struct LayoutCache {
    entries: Vec<LayoutEntry>,
    stats: LayoutCacheStats,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct LayoutKey {
    range: TextRange,
    kind: BlockKind,
    max_width: u32,
    line_height: u32,
    default_advance: u32,
    backend: LayoutBackend,
}

impl LayoutKey {
    fn new(block: Block, config: LayoutConfig, backend: LayoutBackend) -> Self {
        Self {
            range: block.range(),
            kind: block.kind(),
            max_width: config.max_width().to_bits(),
            line_height: config.line_height().to_bits(),
            default_advance: config.default_advance().to_bits(),
            backend,
        }
    }

    fn remapped(self, range: TextRange) -> Self {
        Self { range, ..self }
    }
}

/// 排一个块要的那一份产出：装饰、它投影出来的视觉文本、已经解码到位的图片。
///
/// 三样凑成一个参数而不是三个：它们必须来自**同一次**产出（同 range 同
/// Revision），分开传就多三次拿错的机会——而拿错的表现是「画面少了几个
/// 字」，`BlockLayoutInput` 要到排版时才拒绝。
#[derive(Clone, Copy)]
pub struct BlockLayoutSource<'a> {
    visual: &'a VisualText,
    decorations: &'a BlockDecorations,
    sizes: &'a [ImageSize],
}

impl<'a> BlockLayoutSource<'a> {
    #[must_use]
    pub const fn new(
        visual: &'a VisualText,
        decorations: &'a BlockDecorations,
        sizes: &'a [ImageSize],
    ) -> Self {
        Self {
            visual,
            decorations,
            sizes,
        }
    }
}

#[derive(Debug)]
struct LayoutEntry {
    key: LayoutKey,
    layout: BlockView,
}

impl LayoutCache {
    /// 缓存里有就取，没有就按这份装饰排一份。
    ///
    /// `sizes` 是已经解码到位的图片。缓存**不按它建键**：图片就绪与否不是
    /// 块的身份，而且那样会让不关心图片的调用方（命中测试）每次都另建一份。
    /// 命中之后由 [`BlockView::needs_widget_rebuild`] 判要不要重排——那是
    /// 不变量 D7 的「资源就绪后触发受影响 block 重新 layout」。
    pub fn get_or_build_block(
        &mut self,
        snapshot: &TextSnapshot,
        block: Block,
        config: LayoutConfig,
        source: BlockLayoutSource<'_>,
    ) -> Result<&BlockView, LayoutError> {
        let key = LayoutKey::new(block, config, LayoutBackend::Metrics);
        self.prepare(snapshot);
        if let Some(index) = self.reusable(key, source.sizes) {
            self.stats.hits = self.stats.hits.saturating_add(1);
            return Ok(&self.entries[index].layout);
        }

        let layout = BlockView::build_with_images(
            source.visual,
            source.decorations,
            config,
            &yu_layout::MonospaceMetrics::new(config.default_advance()),
            source.sizes,
        )?;
        Ok(self.insert(key, layout))
    }

    /// 同上，但用调用方给的 shaping 后端。
    pub fn get_or_build_block_with_shaper<S: ShapingProvider>(
        &mut self,
        snapshot: &TextSnapshot,
        block: Block,
        config: LayoutConfig,
        source: BlockLayoutSource<'_>,
        shaper: &S,
    ) -> Result<&BlockView, LayoutError> {
        let key = LayoutKey::new(block, config, LayoutBackend::Shaped);
        self.prepare(snapshot);
        if let Some(index) = self.reusable(key, source.sizes) {
            self.stats.hits = self.stats.hits.saturating_add(1);
            return Ok(&self.entries[index].layout);
        }

        let layout = BlockView::build_shaped_with_images(
            source.visual,
            source.decorations,
            config,
            shaper,
            source.sizes,
        )?;
        Ok(self.insert(key, layout))
    }

    /// 这个键上有没有一份还能用的布局。
    ///
    /// 还欠着 placeholder、而这一批尺寸里正好有它要的那一张时不能用：那份
    /// 布局把图片排成了一个四行宽的空盒子，直接返回就是「图片解码完了画面
    /// 不变」——不报错，只是永远看不到图。
    fn reusable(&mut self, key: LayoutKey, sizes: &[ImageSize]) -> Option<usize> {
        let index = self.entries.iter().position(|entry| entry.key == key)?;
        if self.entries[index].layout.needs_widget_rebuild(sizes) {
            self.entries.remove(index);
            self.stats.invalidated = self.stats.invalidated.saturating_add(1);
            return None;
        }
        Some(index)
    }

    fn prepare(&mut self, snapshot: &TextSnapshot) {
        let current_revision = snapshot.revision();
        let stale = self
            .entries
            .iter()
            .filter(|entry| entry.layout.revision() != current_revision)
            .count();
        if stale > 0 {
            self.stats.invalidated = self.stats.invalidated.saturating_add(stale as u64);
            self.entries
                .retain(|entry| entry.layout.revision() == current_revision);
        }
    }

    fn insert(&mut self, key: LayoutKey, layout: BlockView) -> &BlockView {
        self.entries.push(LayoutEntry { key, layout });
        self.stats.builds = self.stats.builds.saturating_add(1);
        let index = self.entries.len().saturating_sub(1);
        &self.entries[index].layout
    }

    /// Maps layouts through a successful source edit, retaining only layouts
    /// whose projection range was untouched.
    pub fn map_through(
        &mut self,
        changes: &ChangeSet,
        snapshot: &TextSnapshot,
    ) -> Result<(), LayoutError> {
        let mut mapped = Vec::with_capacity(self.entries.len());
        let mut remapped = 0_u64;
        let mut invalidated = 0_u64;
        for entry in &self.entries {
            match entry.layout.map_through(changes, snapshot)? {
                Some(layout) => {
                    mapped.push(LayoutEntry {
                        key: entry.key.remapped(layout.source_range()),
                        layout,
                    });
                    remapped = remapped.saturating_add(1);
                }
                None => {
                    invalidated = invalidated.saturating_add(1);
                }
            }
        }
        self.entries = mapped;
        self.stats.remapped = self.stats.remapped.saturating_add(remapped);
        self.stats.invalidated = self.stats.invalidated.saturating_add(invalidated);
        Ok(())
    }

    /// Retains only entries whose block range and kind still exist.
    pub fn retain_blocks(&mut self, markdown: &MarkdownDocument) {
        let before = self.entries.len();
        self.entries.retain(|entry| {
            markdown
                .blocks()
                .iter()
                .any(|block| block.range() == entry.key.range && block.kind() == entry.key.kind)
        });
        let dropped = before.saturating_sub(self.entries.len());
        self.stats.invalidated = self.stats.invalidated.saturating_add(dropped as u64);
    }

    /// Drops every cached layout, for example when a document is reset.
    pub fn clear(&mut self) {
        let dropped = self.entries.len();
        self.entries.clear();
        self.stats.invalidated = self.stats.invalidated.saturating_add(dropped as u64);
    }

    #[must_use]
    pub fn stats(&self) -> LayoutCacheStats {
        LayoutCacheStats {
            entries: self.entries.len(),
            ..self.stats
        }
    }

    #[must_use]
    pub fn revision(&self) -> Option<Revision> {
        self.entries.first().map(|entry| entry.layout.revision())
    }
}
