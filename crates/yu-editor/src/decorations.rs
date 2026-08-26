//! 一份文档的语法树与逐块装饰，按 Revision 缓存。
//!
//! # 语法树归这里
//!
//! `ExtensionSet::decorate` 要一棵**整篇文档**的树，并且明说了不自己解析：
//! 每块解析一次是 O(块数 × 文档长度)。那棵树得有人按 Revision 持有，可选的
//! 位置有两个：
//!
//! - `MarkdownDocument`。它是每次解析重建的**值类型**，装不下「上一版的树」
//!   ——而增量解析（`parse_with_fragments`）恰恰需要那个。而且 `yu-export`
//!   这类只要块序列的调用方也会被迫付解析的钱。
//! - `EditorDocument` 的独立缓存，也就是这里。要装饰的人才付钱，旧树也有地方
//!   待着。
//!
//! 选了后者。**增量解析还没接上**：这一版每换一个 Revision 就整篇重解析一次，
//! 位置留好了而已（不变量 J1 说的「编辑只重解析受影响范围」在 `yu-syntax`
//! 里已经实现，缺的是这里把 fragment 传下去）。
//!
//! # 装饰按块缓存，不按文档
//!
//! 一次编辑通常只碰一个块。块级缓存让其余的块连装饰都不用重产——它们的区间
//! 整体平移一个常量就行（[`DecorationCache::shift_through`]）。

use std::error::Error;
use std::fmt;

use yu_core::{Revision, TextRange};
use yu_decoration::DecorationSet;
use yu_markdown::{
    Block, BlockDecorations, BlockKind, ExtensionError, ExtensionSet, MarkdownDocument,
};
use yu_syntax::{ParseError, Tree};
use yu_text::{ChangeSet, TextSnapshot};

use crate::visual::shift_for;

/// 产装饰这条路上出错的两种方式。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DecorationError {
    /// 解析整篇文档失败。
    Parse(ParseError),
    /// extension 集合产出失败。
    Extension(ExtensionError),
}

impl fmt::Display for DecorationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Parse(error) => write!(formatter, "解析失败：{error}"),
            Self::Extension(error) => error.fmt(formatter),
        }
    }
}

impl Error for DecorationError {}

impl From<ParseError> for DecorationError {
    fn from(error: ParseError) -> Self {
        Self::Parse(error)
    }
}

impl From<ExtensionError> for DecorationError {
    fn from(error: ExtensionError) -> Self {
        Self::Extension(error)
    }
}

/// 一个编辑器的装饰缓存计数。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DecorationCacheStats {
    entries: usize,
    builds: u64,
    hits: u64,
    remapped: u64,
    invalidated: u64,
    parses: u64,
}

impl DecorationCacheStats {
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

    /// 整篇文档解析了几次。增量解析接上之后这个数字不会变——变的是每次
    /// 重扫了多少字节，那个量住在 `yu-syntax`。
    #[must_use]
    pub const fn parses(self) -> u64 {
        self.parses
    }
}

struct Entry {
    range: TextRange,
    kind: BlockKind,
    decorations: BlockDecorations,
}

/// 与 Revision 绑定的语法树 + 逐块装饰。
pub struct DecorationCache {
    extensions: ExtensionSet,
    tree: Option<(Revision, Tree)>,
    entries: Vec<Entry>,
    stats: DecorationCacheStats,
}

impl Default for DecorationCache {
    fn default() -> Self {
        Self {
            extensions: ExtensionSet::markdown(),
            tree: None,
            entries: Vec::new(),
            stats: DecorationCacheStats::default(),
        }
    }
}

impl fmt::Debug for DecorationCache {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("DecorationCache")
            .field("revision", &self.revision())
            .field("entries", &self.entries.len())
            .finish()
    }
}

impl DecorationCache {
    /// 这一版源码的语法树，第一次问的时候解析。
    ///
    /// # Errors
    ///
    /// 源码超过 `u32::MAX` 字节。
    pub fn tree(&mut self, snapshot: &TextSnapshot) -> Result<&Tree, DecorationError> {
        let revision = snapshot.revision();
        if !self
            .tree
            .as_ref()
            .is_some_and(|(cached, _)| *cached == revision)
        {
            let parsed = yu_syntax::parse(snapshot)?;
            self.stats.parses = self.stats.parses.saturating_add(1);
            self.tree = Some((revision, parsed.into_tree()));
        }
        Ok(&self.tree.as_ref().expect("刚刚填过").1)
    }

    /// 一个块的规范装饰（无光标露出），第一次问的时候产出。
    ///
    /// # Errors
    ///
    /// 解析或装饰产出失败。
    pub fn get_or_build_block(
        &mut self,
        snapshot: &TextSnapshot,
        block: Block,
    ) -> Result<&BlockDecorations, DecorationError> {
        self.retire_stale(snapshot.revision());
        if let Some(index) = self
            .entries
            .iter()
            .position(|entry| entry.range == block.range() && entry.kind == block.kind())
        {
            self.stats.hits = self.stats.hits.saturating_add(1);
            return Ok(&self.entries[index].decorations);
        }
        let decorations = self.decorate(snapshot, block, None)?;
        self.entries.push(Entry {
            range: block.range(),
            kind: block.kind(),
            decorations,
        });
        self.stats.builds = self.stats.builds.saturating_add(1);
        let index = self.entries.len().saturating_sub(1);
        Ok(&self.entries[index].decorations)
    }

    /// 产一份**不进缓存**的装饰。
    ///
    /// 光标露出语法（`active`）不推进 Revision，进缓存会让别的块也看见一份
    /// 只对焦点块成立的产出。
    ///
    /// # Errors
    ///
    /// 解析或装饰产出失败。
    pub fn decorate(
        &mut self,
        snapshot: &TextSnapshot,
        block: Block,
        active: Option<TextRange>,
    ) -> Result<BlockDecorations, DecorationError> {
        let extensions = std::mem::replace(&mut self.extensions, ExtensionSet::empty());
        let result = self
            .tree(snapshot)
            .and_then(|tree| Ok(extensions.decorate(snapshot, tree, block, active)?));
        self.extensions = extensions;
        result
    }

    /// 整篇文档的装饰集合：每个块的产出合并成一份。
    ///
    /// 这是 v2 里「一份文档一份 `DecorationSet`」的那个东西。原生镜像与
    /// IME 用它把源码坐标换成视觉坐标。
    ///
    /// # Errors
    ///
    /// 解析或装饰产出失败，或各块的集合合不到一起。
    pub fn document_set(
        &mut self,
        snapshot: &TextSnapshot,
        blocks: &[Block],
        active: Option<TextRange>,
    ) -> Result<DecorationSet, DecorationError> {
        let mut sets = Vec::with_capacity(blocks.len());
        for block in blocks.iter().copied() {
            // 露出只对光标所在的那个块成立；其余块走规范缓存。
            let reveal = active.filter(|active| overlaps(block.range(), *active));
            if reveal.is_some() {
                sets.push(self.decorate(snapshot, block, reveal)?);
            } else {
                sets.push(self.get_or_build_block(snapshot, block)?.clone());
            }
        }
        let merged = DecorationSet::merge(
            snapshot.revision(),
            snapshot.len_bytes(),
            sets.iter().map(BlockDecorations::set),
        )
        .map_err(|error| DecorationError::Extension(ExtensionError::Merge(error)))?;
        Ok(merged)
    }

    /// 一次成功的编辑之后，把没被碰到的块整体平移过去。
    ///
    /// 碰到的块直接丢掉：它的语法可能整个变了，沿用旧装饰的后果是「多打了
    /// 一个 `#` 但标题级别没变」——不报错，只是画得不对。
    pub fn shift_through(&mut self, changes: &ChangeSet, snapshot: &TextSnapshot) {
        let revision = snapshot.revision();
        let len = snapshot.len_bytes();
        let mut kept = Vec::with_capacity(self.entries.len());
        let mut remapped = 0_u64;
        let mut invalidated = 0_u64;
        for entry in &self.entries {
            let shifted = shift_for(entry.range, changes)
                .ok()
                .flatten()
                .and_then(|delta| {
                    let decorations = entry.decorations.shifted(delta, revision, len).ok()?;
                    let range = decorations.range();
                    Some(Entry {
                        range,
                        kind: entry.kind,
                        decorations,
                    })
                });
            match shifted {
                Some(entry) => {
                    kept.push(entry);
                    remapped = remapped.saturating_add(1);
                }
                None => invalidated = invalidated.saturating_add(1),
            }
        }
        self.entries = kept;
        self.stats.remapped = self.stats.remapped.saturating_add(remapped);
        self.stats.invalidated = self.stats.invalidated.saturating_add(invalidated);
    }

    /// 只留下仍然对得上一个已解析块的条目。
    pub fn retain_blocks(&mut self, markdown: &MarkdownDocument) {
        let before = self.entries.len();
        self.entries.retain(|entry| {
            markdown
                .blocks()
                .iter()
                .any(|block| block.range() == entry.range && block.kind() == entry.kind)
        });
        let dropped = before.saturating_sub(self.entries.len());
        self.stats.invalidated = self.stats.invalidated.saturating_add(dropped as u64);
    }

    /// 丢掉全部装饰与语法树。
    pub fn clear(&mut self) {
        let dropped = self.entries.len();
        self.entries.clear();
        self.tree = None;
        self.stats.invalidated = self.stats.invalidated.saturating_add(dropped as u64);
    }

    #[must_use]
    pub fn stats(&self) -> DecorationCacheStats {
        DecorationCacheStats {
            entries: self.entries.len(),
            ..self.stats
        }
    }

    #[must_use]
    pub fn revision(&self) -> Option<Revision> {
        self.entries
            .first()
            .map(|entry| entry.decorations.revision())
    }

    fn retire_stale(&mut self, revision: Revision) {
        let stale = self
            .entries
            .iter()
            .filter(|entry| entry.decorations.revision() != revision)
            .count();
        if stale > 0 {
            self.stats.invalidated = self.stats.invalidated.saturating_add(stale as u64);
            self.entries
                .retain(|entry| entry.decorations.revision() == revision);
        }
    }
}

/// 两段区间有没有交叠。空的 `active`（一个光标位置）落在块的两端也算。
fn overlaps(block: TextRange, active: TextRange) -> bool {
    active.start() <= block.end() && block.start() <= active.end()
}

/// 这份装饰把块里的多少个字节藏起来了。
///
/// 「光标碰到语法就露出来」的判据是**露出来的那份藏得更少**。数隐藏区间的
/// 长度会把重叠的算两遍（`- [x]` 就有三条互相重叠的），所以从映射里取：
/// 块的长度减去它投影之后的视觉长度。
#[must_use]
pub(crate) fn hidden_bytes(decorations: &BlockDecorations) -> u64 {
    let set = decorations.set();
    let range = decorations.range();
    let visible = set
        .source_to_visual(range.end())
        .get()
        .saturating_sub(set.source_to_visual(range.start()).get());
    range.len().saturating_sub(visible)
}
