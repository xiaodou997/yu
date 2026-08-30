//! 逐块装饰，按 Revision 缓存。
//!
//! # 语法树不在这里
//!
//! 它跟着 `MarkdownDocument` 走。这里此前持有它，模块文档也写过两条理由；
//! 第十一刀复查之后把树搬走了，理由写在 `yu_markdown::MarkdownDocument` 的
//! 类型文档上。一句话：块的 kind 下一刀要改由树给，而分类发生在解析块的时候
//! ——树在另一个 crate 的另一个缓存里的话，那件事做不成。
//!
//! 搬走之后这一层少了两样东西：`TreeFragment` 的接力（每次编辑都解析，上一棵
//! 树永远正好在基准 Revision 上，没有「连着编辑好几次都没人要过树」这种情形
//! 了），以及 `DecorationCacheStats::reparsed_bytes`（不变量 J1 的可断言量
//! 换到了 `MarkdownDocument::reparsed_bytes`）。
//!
//! # 这一层还管什么
//!
//! 一个块的装饰只跟「这一版源码 + 这个块」有关，所以能按 Revision 缓存；
//! 光标露出语法（`active`）**不**进缓存——它不推进 Revision，进了会让别的块
//! 也看见一份只对焦点块成立的产出。
//!
use std::error::Error;
use std::fmt;

use yu_core::{Revision, TextRange};
use yu_decoration::DecorationSet;
use yu_markdown::{
    Block, BlockDecorations, BlockKind, ExtensionError, ExtensionSet, MarkdownDocument,
};
use yu_syntax::ParseError;
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

    /// 真的跑了几次 extension 产出装饰。缓存命中不算。
    ///
    /// J1 那条「重扫了多少字节」现在在
    /// `yu_markdown::MarkdownDocument::reparsed_bytes` 上——树跟着文档走了。
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

/// 逐块装饰，与 Revision 绑定。
///
/// **树不在这里了。** 它跟着 `MarkdownDocument` 走（那边的类型文档写了理由），
/// 这一层只管装饰的缓存与复用。
pub struct DecorationCache {
    extensions: ExtensionSet,
    entries: Vec<Entry>,
    /// 上一次产出装饰时那份引用表的指纹。
    ///
    /// 装饰依赖它：`[文字][标签]` 成不成立要查表（不变量 C6）。而条目是按
    /// **块**留的（range + kind 对得上就复用），一条定义被删掉时，用到它的
    /// 那个块一个字节都没变——不清掉的话，那个链接会一直画成链接，直到有人
    /// 碰它所在的那一块。反过来也一样：补上定义之后它还是一段普通文字。
    ///
    /// 指纹折的是各条定义的**标签与目标的内容哈希**，不是它们的位置，所以
    /// 一次只挪动定义的编辑不会白清一遍。
    ///
    /// `None` 是「还没产出过装饰」。写成 `Option` 而不是「空表的指纹」，是
    /// 为了不把那个常数在两个 crate 里各写一遍。
    references: Option<u64>,
    stats: DecorationCacheStats,
}

impl Default for DecorationCache {
    fn default() -> Self {
        Self {
            extensions: ExtensionSet::markdown(),
            entries: Vec::new(),
            references: None,
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
    /// 一个块的规范装饰（无光标露出），第一次问的时候产出。
    ///
    /// # Errors
    ///
    /// 解析或装饰产出失败。
    pub fn get_or_build_block(
        &mut self,
        markdown: &MarkdownDocument,
        block: Block,
    ) -> Result<&BlockDecorations, DecorationError> {
        self.retire_stale(markdown.revision());
        self.retire_stale_references(markdown);
        if let Some(index) = self
            .entries
            .iter()
            .position(|entry| entry.range == block.range() && entry.kind == block.kind())
        {
            self.stats.hits = self.stats.hits.saturating_add(1);
            return Ok(&self.entries[index].decorations);
        }
        let decorations = self.decorate(markdown, block, None)?;
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
        markdown: &MarkdownDocument,
        block: Block,
        active: Option<TextRange>,
    ) -> Result<BlockDecorations, DecorationError> {
        let tree = markdown
            .tree()
            .ok_or(DecorationError::Parse(ParseError::SourceTooLarge))?;
        self.stats.parses = self.stats.parses.saturating_add(1);
        Ok(self.extensions.decorate(
            markdown.source(),
            tree,
            markdown.reference_definitions(),
            block,
            active,
        )?)
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
        markdown: &MarkdownDocument,
        blocks: &[Block],
        active: Option<TextRange>,
    ) -> Result<DecorationSet, DecorationError> {
        let snapshot = markdown.source();
        let mut sets = Vec::with_capacity(blocks.len());
        for block in blocks.iter().copied() {
            // 露出只对光标所在的那个块成立；其余块走规范缓存。
            let reveal = active.filter(|active| overlaps(block.range(), *active));
            if reveal.is_some() {
                sets.push(self.decorate(markdown, block, reveal)?);
            } else {
                sets.push(self.get_or_build_block(markdown, block)?.clone());
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

    /// 引用表的内容变了就一条都不留。
    ///
    /// 那不是某一块的事：每一条候选引用的判断都可能翻面，而条目是按块留的
    /// （见 [`DecorationCache::references`]）。
    ///
    /// 两条路都要过这一道——编辑之后走 [`Self::retain_blocks`]，第一次产装饰
    /// 走 [`Self::get_or_build_block`]。只放在前者上的话，缓存里会先攒下一批
    /// 按旧表算的条目；只放在后者上的话，一次「只删了一条定义」的编辑要等到
    /// 有人来问装饰才生效，而那时 `retain_blocks` 已经把条目按块留下来了。
    fn retire_stale_references(&mut self, markdown: &MarkdownDocument) {
        let references = markdown.reference_definitions().fingerprint();
        if self.references == Some(references) {
            return;
        }
        self.references = Some(references);
        self.clear();
    }

    /// 只留下仍然对得上一个已解析块的条目。
    pub fn retain_blocks(&mut self, markdown: &MarkdownDocument) {
        self.retire_stale_references(markdown);
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

    /// 丢掉全部装饰。
    pub fn clear(&mut self) {
        let dropped = self.entries.len();
        self.entries.clear();
        // Revision 是每个 `TextBuffer` 自己从头数的，不是全局唯一的——换一份
        // 文档之后同一个编号指的是另一段字节，光靠 Revision 判据挡不住，
        // 得在这里丢掉。
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
