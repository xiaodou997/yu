use yu_core::{Revision, TextRange};
use yu_layout::{LayoutConfig, LayoutError, LayoutSnapshot, ShapingProvider};
use yu_markdown::{Block, BlockKind, MarkdownDocument};
use yu_projection::BlockProjection;
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
    backend: LayoutBackend,
}

impl LayoutKey {
    fn new(block: Block, config: LayoutConfig, backend: LayoutBackend) -> Self {
        Self {
            range: block.range(),
            kind: block.kind(),
            max_width: config.max_width().to_bits(),
            line_height: config.line_height().to_bits(),
            backend,
        }
    }

    fn remapped(self, range: TextRange) -> Self {
        Self { range, ..self }
    }
}

#[derive(Debug)]
struct LayoutEntry {
    key: LayoutKey,
    layout: LayoutSnapshot,
}

impl LayoutCache {
    /// Returns a cached block layout or builds one from the supplied projection.
    pub fn get_or_build_block(
        &mut self,
        snapshot: &TextSnapshot,
        block: Block,
        config: LayoutConfig,
        projection: &BlockProjection,
    ) -> Result<&LayoutSnapshot, LayoutError> {
        let key = LayoutKey::new(block, config, LayoutBackend::Metrics);
        self.prepare(snapshot);
        if let Some(index) = self.entries.iter().position(|entry| entry.key == key) {
            self.stats.hits = self.stats.hits.saturating_add(1);
            return Ok(&self.entries[index].layout);
        }

        let layout = LayoutSnapshot::from_block_projection(projection, config)?;
        Ok(self.insert(key, layout))
    }

    /// Returns a cached block layout built from shaped glyph runs.
    pub fn get_or_build_block_with_shaper<S: ShapingProvider>(
        &mut self,
        snapshot: &TextSnapshot,
        block: Block,
        config: LayoutConfig,
        projection: &BlockProjection,
        shaper: &S,
    ) -> Result<&LayoutSnapshot, LayoutError> {
        let key = LayoutKey::new(block, config, LayoutBackend::Shaped);
        self.prepare(snapshot);
        if let Some(index) = self.entries.iter().position(|entry| entry.key == key) {
            self.stats.hits = self.stats.hits.saturating_add(1);
            return Ok(&self.entries[index].layout);
        }

        let layout = LayoutSnapshot::from_block_projection_with_shaper(projection, config, shaper)?;
        Ok(self.insert(key, layout))
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

    fn insert(&mut self, key: LayoutKey, layout: LayoutSnapshot) -> &LayoutSnapshot {
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
