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
//! 选了后者，**增量解析走的就是这条路**：一次编辑之后
//! [`DecorationCache::shift_through`] 把 `ChangeSet` 应用到上一棵树的
//! `TreeFragment` 上，下一次要树时把它们交给 `parse_with_fragments`——没被这次
//! 编辑碰到的块整段搬过来，不重新扫描。不变量 J1（「编辑只重解析受影响范围」）
//! 在这一层的可断言量是 [`DecorationCacheStats::reparsed_bytes`]。
//!
//! # 复用的来源有两个，而且互斥
//!
//! 树正好在这次编辑的基准 Revision 上时，fragment 从那棵树现取；树落后于编辑
//! （连着编辑好几次都没人要过树）时，拿上一批 fragment 接着往下平移。
//!
//! **这两种情形不会同时成立**，所以先看哪个都一样。理由是解析成功之后
//! [`DecorationCache::tree`] 会把 fragment 清掉：想有一批标着 Revision R 的
//! fragment，就得有人在 R 上调过 `advance_fragments`，而它标的是编辑**之后**
//! 那一版；想让树也在 R 上，就得有人在 R 上解析过，而那一次解析清掉了
//! fragment。一前一后都不成立。
//!
//! 两个来源都对不上基准 Revision 时**不猜**——丢掉 fragment，下一次整篇重解析。
//! 复用一段上下文不符的旧子树不会 panic，只会让某个块的类型悄悄错掉（理由写在
//! `yu_syntax::fragment` 的开头）。多扫一遍是慢，猜错是错。
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
use yu_syntax::{ParseError, Tree, TreeFragment};
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
    reparsed_bytes: u64,
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

    /// 问了几次树，其中有几次真的调用了解析器。
    ///
    /// 增量解析接上之后这个数字**不会变**：每换一个 Revision 仍然要解析一次。
    /// 变的是每次真正读了多少字节，那件事看 [`Self::reparsed_bytes`]。
    #[must_use]
    pub const fn parses(self) -> u64 {
        self.parses
    }

    /// 这些解析加起来重新扫描了多少字节源码。
    ///
    /// 不变量 J1 在编辑器这一层的可断言量。选字节数而不是耗时，是因为它对同
    /// 样的输入永远给同样的答案；耗时随机器和负载浮动，拿它当门禁只会得到一
    /// 条时不时变红的检查，然后被调松到失去意义。
    #[must_use]
    pub const fn reparsed_bytes(self) -> u64 {
        self.reparsed_bytes
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
    /// 下一次解析的复用来源，以及它们描述的是哪一版文档。
    fragments: Option<(Revision, Vec<TreeFragment>)>,
    entries: Vec<Entry>,
    stats: DecorationCacheStats,
}

impl Default for DecorationCache {
    fn default() -> Self {
        Self {
            extensions: ExtensionSet::markdown(),
            tree: None,
            fragments: None,
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
    /// 手上有对得上这一版的 fragment 就走增量：没被编辑碰到的块整段搬过来。
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
            let fragments = match &self.fragments {
                Some((cached, fragments)) if *cached == revision => fragments.as_slice(),
                _ => &[],
            };
            let parsed = yu_syntax::parse_with_fragments(snapshot, fragments)?;
            self.stats.parses = self.stats.parses.saturating_add(1);
            self.stats.reparsed_bytes = self
                .stats
                .reparsed_bytes
                .saturating_add(u64::from(parsed.reparsed_bytes()));
            self.tree = Some((revision, parsed.into_tree()));
            // fragment 的活到此为止：树现在就在这一版上，下一次编辑从树上现取。
            // **这一行就是模块开头那条「两个来源互斥」的来源**，顺带让上一棵树
            // 不再被引用着。
            self.fragments = None;
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
        self.advance_fragments(changes);
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

    /// 把这次编辑应用到复用来源上，让下一次解析只重扫受影响的范围。
    ///
    /// 来源有两个而且互斥（理由在模块开头），所以这里先看哪个都一样；两个都
    /// 对不上编辑的基准 Revision 时丢掉，下一次整篇重解析。
    fn advance_fragments(&mut self, changes: &ChangeSet) {
        let base = changes.before();
        let chained = self.fragments.take();
        let source = match &self.tree {
            Some((revision, tree)) if *revision == base => Some(TreeFragment::from_tree(tree)),
            _ => chained.and_then(|(revision, fragments)| (revision == base).then_some(fragments)),
        };
        self.fragments = source.map(|fragments| {
            (
                changes.after(),
                TreeFragment::apply_change_set(&fragments, changes),
            )
        });
    }

    /// 丢掉全部装饰与语法树。
    pub fn clear(&mut self) {
        let dropped = self.entries.len();
        self.entries.clear();
        // 与树一起清。Revision 是每个 `TextBuffer` 自己从头数的，不是全局唯一
        // 的——换一份文档之后同一个编号指的是另一段字节，光靠 Revision 判据挡
        // 不住，得在这里丢掉。
        self.tree = None;
        self.fragments = None;
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

#[cfg(test)]
mod tests {
    use yu_core::ByteOffset;
    use yu_syntax::Tree;
    use yu_text::{Edit, TextBuffer, Transaction};

    use super::{DecorationCache, TextRange, TextSnapshot};

    /// 容器、围栏、缩进代码块、引用定义——增量复用最容易悄悄出错的那些构造。
    const SOURCE: &str = r#"# 标题

一段文字，带 **强调** 和 `代码`。

> 引用里的段落
> 第二行

1.  列表项

        缩进代码块

```rust
fn main() {}
```

[id]: /docs

结尾段落 [id]。
"#;

    fn at(offset: usize) -> TextRange {
        TextRange::empty(ByteOffset::new(offset as u64))
    }

    fn full_parse(snapshot: &TextSnapshot) -> Tree {
        yu_syntax::parse(snapshot)
            .expect("测试文档不会超长")
            .into_tree()
    }

    /// 在 `buffer` 上做一次编辑，并把它交给缓存。返回新快照。
    fn edit(
        cache: &mut DecorationCache,
        buffer: &mut TextBuffer,
        range: TextRange,
        insert: &str,
    ) -> TextSnapshot {
        let transaction = Transaction::new(buffer.revision(), [Edit::new(range, insert)]);
        let applied = buffer.apply(&transaction).expect("测试编辑应当合法");
        cache.shift_through(applied.change_set(), applied.result_snapshot());
        applied.result_snapshot().clone()
    }

    fn char_boundaries(source: &str) -> Vec<usize> {
        source
            .char_indices()
            .map(|(index, _)| index)
            .chain(std::iter::once(source.len()))
            .collect()
    }

    /// 一份有 `blocks` 个块的文档，每块 `## 标题` + 一个段落。
    fn many_blocks(blocks: usize) -> String {
        let mut source = String::new();
        for index in 0..blocks {
            source.push_str(&format!(
                "## Section {index}\n\nParagraph {index} with *emphasis* and `code`.\n\n"
            ));
        }
        source
    }

    /// **C3 在这一层的守护**：缓存增量产出的树，必须与直接全量解析同一份快照
    /// 相等。
    ///
    /// `yu-syntax` 的差分测试已经压过 `parse_with_fragments` 本身；这里压的是
    /// **接线**——`ChangeSet` 有没有按正确的方向转成 `FragmentChange`、
    /// fragment 有没有配错 Revision。接错了不会报错，只会让某个块的类型悄悄
    /// 错掉。
    #[test]
    fn the_incremental_tree_matches_a_full_parse_after_every_edit() {
        let boundaries = char_boundaries(SOURCE);
        let stride = boundaries.len().div_ceil(24).max(1);
        for offset in boundaries.into_iter().step_by(stride) {
            for insert in ["x", "\n", "`", ">", "#", "    "] {
                let mut buffer = TextBuffer::new(SOURCE.to_owned());
                let mut cache = DecorationCache::default();
                // 先把树建起来，这样下一步才有 fragment 可复用。
                cache.tree(&buffer.snapshot()).expect("初次解析");

                let snapshot = edit(&mut cache, &mut buffer, at(offset), insert);
                let incremental = cache.tree(&snapshot).expect("增量解析").clone();
                assert_eq!(
                    incremental,
                    full_parse(&snapshot),
                    "在 {offset} 处插入 {insert:?} 之后增量与全量不一致\n\
                     增量 {}\n全量 {}",
                    incremental.to_sexp(),
                    full_parse(&snapshot).to_sexp()
                );
            }
        }
    }

    /// 同一份缓存连着编辑，每一步都必须仍然与全量一致。
    ///
    /// 上一条每次都从干净的缓存出发，压不住「fragment 用过一次之后状态坏了」。
    #[test]
    fn a_sequence_of_edits_on_one_cache_stays_equivalent() {
        let mut buffer = TextBuffer::new(SOURCE.to_owned());
        let mut cache = DecorationCache::default();
        cache.tree(&buffer.snapshot()).expect("初次解析");

        // 依次：在围栏里打字、把一段变成引用、拆开一个列表项、删掉一个字符。
        // 锚点每一步都在**当前**文本里重新找：拿原始语料的偏移去切改过的文本，
        // 上一步一挪就会落在字符中间。
        let steps: [(&str, &str, &str); 4] = [
            ("```rust\n", "fn", "let x = 1;\n"),
            ("# 标题", "#", ">"),
            ("1.  列表项", "列表", "\n\n"),
            ("**强调**", "强调", ""),
        ];

        for (anchor, needle, insert) in steps {
            let text = buffer.snapshot().as_str().to_owned();
            let base = text.find(anchor).expect("锚点还在");
            let start = text[base..].find(needle).expect("目标串还在") + base;
            let range = TextRange::new(
                ByteOffset::new(start as u64),
                ByteOffset::new((start + needle.len()) as u64),
            )
            .expect("有序偏移构成合法 range");
            let snapshot = edit(&mut cache, &mut buffer, range, insert);
            let incremental = cache.tree(&snapshot).expect("增量解析").clone();
            assert_eq!(
                incremental,
                full_parse(&snapshot),
                "把 {needle:?} 换成 {insert:?} 之后增量与全量不一致\n增量 {}",
                incremental.to_sexp()
            );
        }
    }

    /// **J1 的可断言量**：一次单字符编辑重扫的字节数与文档大小无关。
    ///
    /// 只断言「小于某个上界」是不够的——那样把 `BUDGET` 定得足够大，退化成
    /// 全量重扫也能过。所以同时断言它**不随文档增长**：文档大 8 倍，重扫的
    /// 字节数不许跟着涨。
    #[test]
    fn one_edit_rescans_a_bounded_number_of_bytes() {
        /// 实测一次单字符编辑重扫约 66 字节（差不多就是被改的那个块）。
        /// 上限留到 256 是给块大小的余量：判据是它必须小到让「复用失效」一定
        /// 越界——最小的那份文档全量重扫就有三千多字节。
        const BUDGET: u64 = 256;

        let measure = |blocks: usize| -> (u64, u64) {
            let source = many_blocks(blocks);
            let mut buffer = TextBuffer::new(source.clone());
            let mut cache = DecorationCache::default();
            cache.tree(&buffer.snapshot()).expect("初次解析");
            let full = cache.stats().reparsed_bytes();

            let middle = char_boundaries(&source)[blocks / 2 * 20];
            let snapshot = edit(&mut cache, &mut buffer, at(middle), "X");
            cache.tree(&snapshot).expect("增量解析");
            (full, cache.stats().reparsed_bytes() - full)
        };

        let (small_full, small) = measure(64);
        let (_, large) = measure(512);

        assert!(
            small <= BUDGET && large <= BUDGET,
            "单字符编辑重扫了 {small} / {large} 字节，超出上界 {BUDGET}"
        );
        assert!(
            large <= small * 2,
            "文档大了 8 倍，重扫字节数从 {small} 涨到 {large}——复用没生效"
        );
        assert!(
            small * 8 <= small_full,
            "增量重扫 {small} 字节，全量才 {small_full} 字节——这份语料太小，\
             证明不了复用生效"
        );
    }

    /// 中间没人要过树时，编辑接着往下链，复用仍然成立。
    ///
    /// 真实链路上每次编辑后都会渲染一次，树总是新的；批量编辑（查找替换、
    /// 连续粘贴）走的才是这条。它单独压，因为 `yu-syntax` 那边的测试每步都
    /// 重新 `from_tree`，从不链。
    #[test]
    fn edits_without_an_intervening_parse_keep_reusing() {
        const BUDGET: u64 = 512;

        let source = many_blocks(256);
        let boundaries = char_boundaries(&source);
        let mut buffer = TextBuffer::new(source.clone());
        let mut cache = DecorationCache::default();
        cache.tree(&buffer.snapshot()).expect("初次解析");
        let full = cache.stats().reparsed_bytes();

        // 四次编辑，中间一次都不问树。从后往前改，免得偏移互相影响。
        let mut snapshot = buffer.snapshot();
        for step in [200_usize, 150, 100, 50] {
            snapshot = edit(&mut cache, &mut buffer, at(boundaries[step * 20]), "X");
        }

        let incremental = cache.tree(&snapshot).expect("增量解析").clone();
        assert_eq!(
            incremental,
            full_parse(&snapshot),
            "链下来的 fragment 产出的树与全量不一致"
        );
        let rescanned = cache.stats().reparsed_bytes() - full;
        assert!(
            rescanned <= BUDGET,
            "四次编辑之后重扫了 {rescanned} 字节，超出上界 {BUDGET}（全量是 {full}）"
        );
        assert_eq!(cache.stats().parses(), 2, "中间不该有人触发解析");
    }

    /// **fragment 对不上要问的那一版时，不许拿来用。**
    ///
    /// `tree()` 是公开方法，收谁的快照都行；产品链路上每次都传当前快照，但
    /// 契约不是「调用方会守规矩」。连编辑两次而中途不解析，树停在第 0 版、
    /// fragment 标着第 2 版，这时去问**第 1 版**——两个来源都对不上，只能整篇
    /// 重解析。拿第 2 版的 fragment 去解析第 1 版，复用会落在错位的字节上：
    /// 树悄悄不对，不 panic。
    #[test]
    fn fragments_are_not_used_for_a_revision_they_do_not_describe() {
        let source = many_blocks(64);
        let boundaries = char_boundaries(&source);
        let mut buffer = TextBuffer::new(source.clone());
        let mut cache = DecorationCache::default();
        cache.tree(&buffer.snapshot()).expect("初次解析");

        // 两次编辑中途都不问树：树停在第 0 版，fragment 一路链到第 2 版。
        let first = edit(&mut cache, &mut buffer, at(boundaries[40 * 20]), "X");
        edit(&mut cache, &mut buffer, at(boundaries[10 * 20]), "Y");

        let tree = cache.tree(&first).expect("解析中间那一版").clone();
        assert_eq!(
            tree,
            full_parse(&first),
            "拿第 2 版的 fragment 解析第 1 版，树错了"
        );
    }

    /// 手上没有对得上基准 Revision 的来源时，老老实实全量重解析。
    ///
    /// 这是「不猜」的那一支：缓存刚建好还没解析过任何东西，就先收到一次编辑。
    #[test]
    fn an_edit_without_a_reusable_source_falls_back_to_a_full_parse() {
        let source = many_blocks(64);
        let mut buffer = TextBuffer::new(source);
        let mut cache = DecorationCache::default();

        let snapshot = edit(&mut cache, &mut buffer, at(0), "X");
        let incremental = cache.tree(&snapshot).expect("解析").clone();
        assert_eq!(incremental, full_parse(&snapshot));

        let rescanned = cache.stats().reparsed_bytes();
        assert!(
            rescanned > 1_000,
            "没有复用来源时应当整篇重扫，实际只读了 {rescanned} 字节"
        );
    }
}
