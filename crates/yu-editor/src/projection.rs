use yu_core::{Revision, TextRange};
use yu_markdown::{Block, BlockKind, MarkdownDocument};
use yu_projection::{BlockProjection, Projection, ProjectionError};
use yu_text::{ChangeSet, TextSnapshot};

/// Cumulative counters for one editor's source-backed projection cache.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ProjectionCacheStats {
    entries: usize,
    builds: u64,
    hits: u64,
    remapped: u64,
    invalidated: u64,
}

impl ProjectionCacheStats {
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

/// Revision-bound projections owned by one editor document.
///
/// Entries are reusable only when their source revision matches the current
/// snapshot. A successful edit remaps entries whose ranges are strictly
/// outside the changed ranges and drops entries touched by an edit or boundary
/// insertion. This is intentionally conservative about Markdown delimiter
/// context.
#[derive(Debug, Default)]
pub struct ProjectionCache {
    entries: Vec<ProjectionEntry>,
    stats: ProjectionCacheStats,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ProjectionKey {
    Range(TextRange),
    Block { range: TextRange, kind: BlockKind },
}

impl ProjectionKey {
    fn remapped(self, range: TextRange) -> Self {
        match self {
            Self::Range(_) => Self::Range(range),
            Self::Block { kind, .. } => Self::Block { range, kind },
        }
    }
}

#[derive(Debug)]
struct ProjectionEntry {
    key: ProjectionKey,
    projection: BlockProjection,
}

impl ProjectionCache {
    /// Returns a cached projection for this revision/range or builds one.
    pub fn get_or_build(
        &mut self,
        snapshot: &TextSnapshot,
        range: TextRange,
    ) -> Result<&Projection, ProjectionError> {
        let index = self.get_or_build_key(snapshot, ProjectionKey::Range(range), || {
            Projection::inline(snapshot, range).map(BlockProjection::Inline)
        })?;
        match &self.entries[index].projection {
            BlockProjection::Inline(projection) => Ok(projection),
            BlockProjection::FencedCode(_) => unreachable!("range projections are inline"),
        }
    }

    /// Returns a projection keyed by one Markdown block's range and kind.
    pub fn get_or_build_block(
        &mut self,
        snapshot: &TextSnapshot,
        block: Block,
    ) -> Result<&BlockProjection, ProjectionError> {
        let index = self.get_or_build_key(
            snapshot,
            ProjectionKey::Block {
                range: block.range(),
                kind: block.kind(),
            },
            || BlockProjection::from_block(snapshot, block),
        )?;
        Ok(&self.entries[index].projection)
    }

    fn get_or_build_key<F>(
        &mut self,
        snapshot: &TextSnapshot,
        key: ProjectionKey,
        build: F,
    ) -> Result<usize, ProjectionError>
    where
        F: FnOnce() -> Result<BlockProjection, ProjectionError>,
    {
        let current_revision = snapshot.revision();
        let stale = self
            .entries
            .iter()
            .filter(|entry| entry.projection.revision() != current_revision)
            .count();
        if stale > 0 {
            self.stats.invalidated = self.stats.invalidated.saturating_add(stale as u64);
            self.entries
                .retain(|entry| entry.projection.revision() == current_revision);
        }

        if let Some(index) = self.entries.iter().position(|entry| entry.key == key) {
            self.stats.hits = self.stats.hits.saturating_add(1);
            return Ok(index);
        }

        let projection = build()?;
        self.entries.push(ProjectionEntry { key, projection });
        self.stats.builds = self.stats.builds.saturating_add(1);
        Ok(self.entries.len().saturating_sub(1))
    }

    /// Applies a successful source change to every cached projection.
    pub fn map_through(
        &mut self,
        changes: &ChangeSet,
        snapshot: &TextSnapshot,
    ) -> Result<(), ProjectionError> {
        let mut mapped = Vec::with_capacity(self.entries.len());
        let mut remapped = 0_u64;
        let mut invalidated = 0_u64;
        for entry in &self.entries {
            match entry.projection.map_through(changes, snapshot)? {
                Some(projection) => {
                    mapped.push(ProjectionEntry {
                        key: entry.key.remapped(projection.source_range()),
                        projection,
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

    /// Retains block-keyed entries that still match a parsed Markdown block.
    pub fn retain_blocks(&mut self, markdown: &MarkdownDocument) {
        let before = self.entries.len();
        self.entries.retain(|entry| match entry.key {
            ProjectionKey::Range(_) => true,
            ProjectionKey::Block { range, kind } => markdown
                .blocks()
                .iter()
                .any(|block| block.range() == range && block.kind() == kind),
        });
        let dropped = before.saturating_sub(self.entries.len());
        self.stats.invalidated = self.stats.invalidated.saturating_add(dropped as u64);
    }

    /// Drops every cached projection, for example when a document is reset.
    pub fn clear(&mut self) {
        let dropped = self.entries.len();
        self.entries.clear();
        self.stats.invalidated = self.stats.invalidated.saturating_add(dropped as u64);
    }

    #[must_use]
    pub fn stats(&self) -> ProjectionCacheStats {
        ProjectionCacheStats {
            entries: self.entries.len(),
            ..self.stats
        }
    }

    #[must_use]
    pub fn revision(&self) -> Option<Revision> {
        self.entries
            .first()
            .map(|entry| entry.projection.revision())
    }
}
