#![forbid(unsafe_code)]

//! Lossless Markdown syntax experiments.
//!
//! Phase 1 provides deliberately small, chunk-aware block and inline token
//! scanners. They preserve every source byte through ranges, but are not yet
//! CommonMark semantic parsers.

use std::error::Error;
use std::fmt;
use std::iter::Peekable;

use yu_core::{Affinity, ByteOffset, Revision, TextAnchor, TextRange};
use yu_syntax::{Tree, TreeFragment};
use yu_text::{
    AnchorMapError, ChangeSet, ChunkCursor, SnapshotRetentionStats, TextSnapshot,
    retained_snapshot_stats,
};

mod block_sequence;
mod classify;
mod extension;
mod inline;
mod reference;
mod table;
mod task;

pub use block_sequence::{
    Block, BlockCompactionPolicy, BlockKind, BlockSequence, BlockState, BlockStorageStats,
    RetainedBlockStats, TaskState,
};
use block_sequence::{BlockRecord, ResolvedBlockRecord, SourceHash, retained_block_stats};
use classify::BlockShape;
pub use extension::{
    BlockContext, BlockDecorations, BlockOrnament, BlockWidget, CheckboxSpan, DelimitedSpan,
    Extension, ExtensionError, ExtensionOutput, ExtensionSet, ImageSpan, MarkerOrnament,
    SyntaxNode, reveals,
};
pub use inline::{
    InlineDelimiter, InlineDocument, InlineNode, InlineNodeKind, InlineParseError,
    InlinePunctuation, InlineSpan, InlineSpanKind, parse_inline, parse_inline_with_definitions,
};
pub use reference::{ReferenceDefinition, ReferenceDefinitionIndex};
pub use table::{
    TableAlignment, TableBlock, TableCellAddress, TableCellRange, TableRowRange, parse_table,
    parse_table_in_snapshot,
};
pub use task::TaskMarker;

/// Parser-owned source ranges and semantics for the first-line marker of a
/// list item. `marker_range` covers only `-`, `*`, `+`, `1.` or `1)`, while
/// `prefix_range` also includes leading indentation and following whitespace.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ListMarker {
    ordered: bool,
    marker: char,
    start: u32,
    indent: u8,
    marker_range: TextRange,
    prefix_range: TextRange,
}

impl ListMarker {
    #[must_use]
    pub const fn ordered(self) -> bool {
        self.ordered
    }

    #[must_use]
    pub const fn marker(self) -> char {
        self.marker
    }

    #[must_use]
    pub const fn start(self) -> u32 {
        self.start
    }

    /// Returns the exact number of parser-accepted leading ASCII spaces.
    #[must_use]
    pub const fn indent(self) -> u8 {
        self.indent
    }

    #[must_use]
    pub const fn marker_range(self) -> TextRange {
        self.marker_range
    }

    #[must_use]
    pub const fn prefix_range(self) -> TextRange {
        self.prefix_range
    }
}

/// A lossless block view of one immutable text revision.
///
/// # 一个 Revision 一棵树，跟着这份文档走
///
/// 语法树此前住在 `yu-editor::DecorationCache` 里，那里的模块文档给过两条
/// 理由：`MarkdownDocument` 是每次重建的值类型，装不下增量解析要的「上一版
/// 的树」；而只要块序列的调用方会被迫付解析的钱。
///
/// **第一条经不起复查**：[`parse_incremental`] 手上就有上一版
/// `MarkdownDocument`，上一棵树跟着它走即可——与从缓存里取 `TreeFragment`
/// 是同一件事，只是取的地方换了。第二条仍然成立，代价量过了（1 MiB 的单字符
/// 编辑，块序列增量 250 µs、语法树增量 331 µs），而在交互路径上这笔钱早就
/// 付了：编辑之后要出画面，出画面就要装饰，装饰就要树。
///
/// 搬过来是为了下一刀：块的 kind 要改由树给，而分类发生在解析块的时候。
/// 树在另一个 crate 的另一个缓存里的话，那件事做不成。
#[derive(Clone, Debug)]
pub struct MarkdownDocument {
    revision: Revision,
    source_len: ByteOffset,
    source: TextSnapshot,
    blocks: BlockSequence,
    references: ReferenceDefinitionIndex,
    /// 语法树。`None` **只有一种成因**：源码超过 4 GiB，`yu-syntax` 明确
    /// 拒绝（`ParseError::SourceTooLarge`，位置是 32 位的）。那种文档今天
    /// 也一样什么都渲染不出来——装饰这一步就失败了——所以这里不为它另建一
    /// 条降级路径，只把「没有树」如实说出来。
    ///
    /// 做成 `Option` 而不是让 [`parse`] 返回 `Result`，是因为后者会传染到
    /// 九十多个调用方，换来的只是把一个 4 GiB 的边角情形提前几微秒报出来。
    tree: Option<Tree>,
    /// 这一版解析实际重新扫描过的源码字节数（不变量 J1 的可断言量）。
    reparsed_bytes: u32,
}

impl MarkdownDocument {
    /// 这一版的语法树，根节点是 `Document`。
    ///
    /// 整篇只解析一次，各家共用——每个 extension 自己再解析一遍会让「同一份
    /// 源码在两个 extension 眼里不一样」成为可能（不变量 D6）。
    #[must_use]
    pub const fn tree(&self) -> Option<&Tree> {
        self.tree.as_ref()
    }

    /// 这一版解析实际重新扫描过的源码字节数。
    ///
    /// 不变量 J1「编辑只重解析受影响范围」的**可断言量**。选它而不是耗时，
    /// 是因为它对同样的输入永远给同样的答案。
    #[must_use]
    pub const fn reparsed_bytes(&self) -> u32 {
        self.reparsed_bytes
    }

    #[must_use]
    pub fn revision(&self) -> Revision {
        self.revision
    }

    /// 这一版的源码快照。
    #[must_use]
    pub const fn source(&self) -> &TextSnapshot {
        &self.source
    }

    #[must_use]
    pub fn source_len(&self) -> ByteOffset {
        self.source_len
    }

    #[must_use]
    pub fn blocks(&self) -> &BlockSequence {
        &self.blocks
    }

    /// Returns the source-backed link definitions for this document revision.
    #[must_use]
    pub fn reference_definitions(&self) -> &ReferenceDefinitionIndex {
        &self.references
    }

    #[must_use]
    pub fn block_storage_stats(&self) -> BlockStorageStats {
        self.blocks.storage_stats()
    }

    #[must_use]
    pub fn needs_block_compaction(&self, policy: BlockCompactionPolicy) -> bool {
        policy.should_compact(self.block_storage_stats())
    }

    /// Packs all active block records into one allocation.
    ///
    /// This is intentionally explicit because its cost is linear in the number
    /// of blocks. Product code should call it from an idle/background task.
    pub fn compact_blocks(&mut self) -> bool {
        let stats = self.block_storage_stats();
        if stats.blocks() == 0 || (stats.segments() == 1 && stats.reclaimable_records() == 0) {
            return false;
        }
        self.blocks = self.blocks.compacted();
        true
    }

    pub fn compact_blocks_if_needed(&mut self, policy: BlockCompactionPolicy) -> bool {
        if !self.needs_block_compaction(policy) {
            return false;
        }
        self.compact_blocks()
    }

    /// Confirms that ordered block ranges cover the source exactly once.
    #[must_use]
    pub fn has_lossless_coverage(&self) -> bool {
        let mut expected_start = ByteOffset::ZERO;
        for block in &self.blocks {
            if block.range.start() != expected_start {
                return false;
            }
            expected_start = block.range.end();
        }
        expected_start == self.source_len
    }
}

impl PartialEq for MarkdownDocument {
    /// **不比树，也不比重扫字节数。** 树是同一份源码的另一种读法——源码与块
    /// 相等而树不等，那是解析器坏了，不是两份文档不同；拿它进等价判断只会让
    /// 「增量等于全量」这条差分自己证自己。重扫字节数更明显：它是**怎么算出
    /// 来的**，不是**算出了什么**。
    fn eq(&self, other: &Self) -> bool {
        self.revision == other.revision
            && self.source_len == other.source_len
            && self.blocks == other.blocks
            && self.references == other.references
    }
}

impl Eq for MarkdownDocument {}

/// De-duplicated storage retained by a set of immutable Markdown revisions.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MarkdownRetentionStats {
    documents: usize,
    document_bytes: usize,
    text: SnapshotRetentionStats,
    blocks: RetainedBlockStats,
}

impl MarkdownRetentionStats {
    #[must_use]
    pub const fn documents(self) -> usize {
        self.documents
    }

    #[must_use]
    pub const fn document_bytes(self) -> usize {
        self.document_bytes
    }

    #[must_use]
    pub const fn text(self) -> SnapshotRetentionStats {
        self.text
    }

    #[must_use]
    pub const fn blocks(self) -> RetainedBlockStats {
        self.blocks
    }

    #[must_use]
    pub const fn estimated_bytes(self) -> usize {
        self.document_bytes
            .saturating_add(self.text.estimated_bytes())
            .saturating_add(self.blocks.estimated_bytes())
    }
}

#[must_use]
pub fn retained_markdown_stats(documents: &[MarkdownDocument]) -> MarkdownRetentionStats {
    let snapshots = documents
        .iter()
        .map(|document| document.source.clone())
        .collect::<Vec<_>>();
    MarkdownRetentionStats {
        documents: documents.len(),
        document_bytes: documents
            .len()
            .saturating_mul(std::mem::size_of::<MarkdownDocument>()),
        text: retained_snapshot_stats(&snapshots),
        blocks: retained_block_stats(documents.iter().map(|document| &document.blocks)),
    }
}

/// The observable result of one conservative incremental block parse.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IncrementalParse {
    document: MarkdownDocument,
    reparsed_range: TextRange,
    reused_prefix_blocks: usize,
    reused_suffix_blocks: usize,
}

impl IncrementalParse {
    #[must_use]
    pub fn document(&self) -> &MarkdownDocument {
        &self.document
    }

    #[must_use]
    pub fn into_document(self) -> MarkdownDocument {
        self.document
    }

    #[must_use]
    pub fn reparsed_range(&self) -> TextRange {
        self.reparsed_range
    }

    #[must_use]
    pub fn reused_prefix_blocks(&self) -> usize {
        self.reused_prefix_blocks
    }

    #[must_use]
    pub fn reused_suffix_blocks(&self) -> usize {
        self.reused_suffix_blocks
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum IncrementalParseError {
    PreviousRevision {
        document: Revision,
        change_set: Revision,
    },
    SnapshotRevision {
        snapshot: Revision,
        change_set: Revision,
    },
    AnchorMap(AnchorMapError),
}

impl fmt::Display for IncrementalParseError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::PreviousRevision {
                document,
                change_set,
            } => write!(
                formatter,
                "previous document revision {document:?} does not match change set {change_set:?}"
            ),
            Self::SnapshotRevision {
                snapshot,
                change_set,
            } => write!(
                formatter,
                "snapshot revision {snapshot:?} does not match change set {change_set:?}"
            ),
            Self::AnchorMap(error) => write!(formatter, "cannot map reparse boundary: {error}"),
        }
    }
}

impl Error for IncrementalParseError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::AnchorMap(error) => Some(error),
            Self::PreviousRevision { .. } | Self::SnapshotRevision { .. } => None,
        }
    }
}

impl From<AnchorMapError> for IncrementalParseError {
    fn from(error: AnchorMapError) -> Self {
        Self::AnchorMap(error)
    }
}

#[derive(Clone, Copy, Debug)]
struct Line {
    start: usize,
    end: usize,
    analysis: LineAnalysis,
    source_hash: SourceHash,
}

impl Line {
    fn is_blank(self) -> bool {
        self.analysis.blank
    }

    fn opening_fence(self) -> Option<Fence> {
        let analysis = self.analysis;
        if !analysis.indent_valid || !matches!(analysis.prefix, Some('`' | '~')) {
            return None;
        }
        Some(Fence {
            marker: analysis.prefix.expect("validated fence must have a marker"),
            count: analysis.prefix_count,
        })
        .filter(|fence| fence.count >= 3)
    }

    fn is_closing_fence(self, opening: Fence) -> bool {
        let analysis = self.analysis;
        analysis.indent_valid
            && analysis.prefix == Some(opening.marker)
            && analysis.prefix_count >= opening.count
            && analysis.tail_whitespace
    }

    fn atx_heading_level(self) -> Option<u8> {
        let analysis = self.analysis;
        if !analysis.indent_valid
            || analysis.prefix != Some('#')
            || !(1..=6).contains(&analysis.prefix_count)
            || !matches!(analysis.after_prefix, None | Some(' ' | '\t'))
        {
            return None;
        }
        u8::try_from(analysis.prefix_count).ok()
    }

    fn block_marker(self) -> Option<LineMarker> {
        if !self.analysis.indent_valid {
            return None;
        }
        let depth = u8::try_from(self.analysis.leading_spaces / 2).ok()?;
        match self.analysis.prefix {
            Some('>') => Some(LineMarker::BlockQuote { depth: 1 }),
            Some(marker @ ('-' | '+' | '*'))
                if self.analysis.prefix_count == 1
                    && self.analysis.after_prefix.is_none_or(is_markdown_space) =>
            {
                Some(LineMarker::List {
                    ordered: false,
                    depth,
                    marker,
                    start: 1,
                })
            }
            Some('0'..='9')
                if self.analysis.ordered_digits > 0
                    && self.analysis.ordered_digits <= 9
                    && matches!(self.analysis.after_prefix, Some('.' | ')'))
                    && self.analysis.marker_following.is_none_or(is_markdown_space) =>
            {
                Some(LineMarker::List {
                    ordered: true,
                    depth,
                    marker: self.analysis.after_prefix.unwrap_or('.'),
                    start: self.analysis.ordered_value.unwrap_or(1),
                })
            }
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug)]
struct LineAnalysis {
    blank: bool,
    indent_valid: bool,
    syntax_done: bool,
    leading_spaces: usize,
    prefix: Option<char>,
    prefix_count: usize,
    after_prefix: Option<char>,
    marker_following: Option<char>,
    ordered_value: Option<u32>,
    ordered_digits: usize,
    tail_whitespace: bool,
}

impl Default for LineAnalysis {
    fn default() -> Self {
        Self {
            blank: true,
            indent_valid: true,
            syntax_done: false,
            leading_spaces: 0,
            prefix: None,
            prefix_count: 0,
            after_prefix: None,
            marker_following: None,
            ordered_value: None,
            ordered_digits: 0,
            tail_whitespace: true,
        }
    }
}

impl LineAnalysis {
    fn wants_input(self) -> bool {
        self.blank || !self.syntax_done
    }

    fn push(&mut self, character: char) {
        self.blank &= character.is_whitespace();
        if !self.indent_valid || self.syntax_done {
            return;
        }

        let Some(prefix) = self.prefix else {
            if character == ' ' {
                self.leading_spaces += 1;
                if self.leading_spaces > 3 {
                    self.indent_valid = false;
                }
            } else {
                self.prefix = Some(character);
                self.prefix_count = 1;
                if !matches!(character, '#' | '`' | '~' | '>' | '-' | '+' | '*')
                    && !character.is_ascii_digit()
                {
                    self.syntax_done = true;
                }
                if character.is_ascii_digit() {
                    self.ordered_value = character.to_digit(10);
                    self.ordered_digits = 1;
                }
            }
            return;
        };

        if self.after_prefix.is_none() && character == prefix && matches!(prefix, '#' | '`' | '~') {
            self.prefix_count += 1;
            return;
        }
        if self.after_prefix.is_none()
            && prefix.is_ascii_digit()
            && character.is_ascii_digit()
            && self.ordered_digits < 9
        {
            self.ordered_value = self.ordered_value.and_then(|value| {
                value
                    .checked_mul(10)
                    .and_then(|value| value.checked_add(character.to_digit(10)?))
            });
            self.ordered_digits += 1;
            return;
        }
        if self.after_prefix.is_none() {
            self.after_prefix = Some(character);
        } else if prefix.is_ascii_digit() && self.marker_following.is_none() {
            self.marker_following = Some(character);
        }
        self.tail_whitespace &= character.is_whitespace();
        let ordered_delimiter_pending = prefix.is_ascii_digit()
            && matches!(self.after_prefix, Some('.' | ')'))
            && self.marker_following.is_none();
        if prefix == '#' || (!self.tail_whitespace && !ordered_delimiter_pending) {
            self.syntax_done = true;
        }
    }
}

#[derive(Clone, Copy, Debug)]
struct Fence {
    marker: char,
    count: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum LineMarker {
    BlockQuote {
        depth: u8,
    },
    List {
        ordered: bool,
        depth: u8,
        marker: char,
        start: u32,
    },
}

fn is_markdown_space(character: char) -> bool {
    matches!(character, ' ' | '\t')
}

/// Scans an immutable Snapshot without materializing a contiguous source copy.
#[must_use]
pub fn parse(snapshot: &TextSnapshot) -> MarkdownDocument {
    // 树先解析：块的身份由它给（`crate::classify`）。
    let (tree, reparsed_bytes) = parse_tree(snapshot, &[]);
    let blocks =
        BlockSequence::from_records(BlockParser::new(snapshot, tree.as_ref(), 0).collect());
    MarkdownDocument {
        revision: snapshot.revision(),
        source_len: snapshot.len_bytes(),
        source: snapshot.clone(),
        references: ReferenceDefinitionIndex::from_blocks(snapshot, &blocks),
        blocks,
        tree,
        reparsed_bytes,
    }
}

/// 解析语法树，超长时给 `None`。
///
/// 传空 fragment 切片等价于全量解析（`yu_syntax::parse_with_fragments` 的
/// 契约），所以全量与增量两条路共用这一个函数——两条路各写一遍，迟早有一条
/// 忘了统计重扫字节数，而那个数是不变量 J1 的可断言量。
fn parse_tree(snapshot: &TextSnapshot, fragments: &[TreeFragment]) -> (Option<Tree>, u32) {
    match yu_syntax::parse_with_fragments(snapshot, fragments) {
        Ok(parsed) => (Some(parsed.tree().clone()), parsed.reparsed_bytes()),
        Err(_) => (None, 0),
    }
}

/// Reparses from a conservative boundary until source, state, and block shape
/// converge with an unaffected old block, then shares the remaining suffix.
pub fn parse_incremental(
    previous: &MarkdownDocument,
    snapshot: &TextSnapshot,
    changes: &ChangeSet,
) -> Result<IncrementalParse, IncrementalParseError> {
    if previous.revision != changes.before() {
        return Err(IncrementalParseError::PreviousRevision {
            document: previous.revision,
            change_set: changes.before(),
        });
    }
    if snapshot.revision() != changes.after() {
        return Err(IncrementalParseError::SnapshotRevision {
            snapshot: snapshot.revision(),
            change_set: changes.after(),
        });
    }

    if changes.changes().is_empty() {
        let document = MarkdownDocument {
            revision: snapshot.revision(),
            source_len: snapshot.len_bytes(),
            source: snapshot.clone(),
            blocks: previous.blocks.clone(),
            references: ReferenceDefinitionIndex::from_blocks(snapshot, &previous.blocks),
            // 一个字节都没改，树整棵搬过来，一次都不重扫。
            tree: previous.tree.clone(),
            reparsed_bytes: 0,
        };
        return Ok(IncrementalParse {
            reparsed_range: TextRange::empty(snapshot.len_bytes()),
            reused_prefix_blocks: document.blocks.len(),
            reused_suffix_blocks: 0,
            document,
        });
    }

    let earliest = changes
        .changes()
        .iter()
        .map(|change| change.old_range().start())
        .min()
        .expect("non-empty changes must have an earliest offset");
    let affected = previous.blocks.first_ending_after(earliest);
    let reparse_index = affected.saturating_sub(1);
    let old_start = previous
        .blocks
        .get(reparse_index)
        .map_or(ByteOffset::ZERO, |block| block.range().start());
    let mapped = changes.map_anchor(TextAnchor::new(
        previous.revision,
        old_start,
        Affinity::Before,
    ))?;
    let new_start = mapped.offset();
    let new_start_usize = usize::try_from(new_start)
        .expect("a mapped document offset must fit the platform address space");

    let latest_changed_end = changes
        .changes()
        .iter()
        .map(|change| change.old_range().end())
        .max()
        .expect("non-empty changes must have a latest offset");
    let mut candidate_index = previous
        .blocks
        .first_starting_at_or_after(latest_changed_end)
        .max(reparse_index);

    // 树的增量复用来源是**上一版文档自己的树**。此前它在
    // `yu-editor::DecorationCache` 里，那一层要自己攒一批 `TreeFragment`
    // 并跨编辑接力（「连着编辑好几次都没人要过树」）；树跟着文档走之后
    // 那件事不存在了——每次编辑都解析，上一棵树永远正好在基准 Revision 上。
    //
    // 它排在块扫描**之前**：块的身份由树给，扫块时树必须已经在新版上。
    let fragments = previous
        .tree
        .as_ref()
        .map(|tree| TreeFragment::apply_change_set(&TreeFragment::from_tree(tree), changes))
        .unwrap_or_default();
    let (tree, reparsed_bytes) = parse_tree(snapshot, &fragments);

    let mut parser = BlockParser::new(snapshot, tree.as_ref(), new_start_usize);
    let mut middle = Vec::new();
    let mut scanned_end = new_start;
    let mut reused_suffix_start = previous.blocks.len();
    let mut suffix_delta = 0_i128;

    for new_record in &mut parser {
        scanned_end = new_record.block.range.end();

        let mut candidate = None;
        while candidate_index < previous.blocks.len() {
            let old_record = previous
                .blocks
                .resolved_record(candidate_index)
                .expect("candidate index must identify an old block");
            let mapped_range =
                map_unchanged_range(previous.revision, old_record.block.range, changes)?;
            if mapped_range.start() < new_record.block.range.start() {
                candidate_index += 1;
                continue;
            }
            candidate = Some((old_record, mapped_range));
            break;
        }

        if let Some((old_record, mapped_range)) = candidate
            && records_converge(
                old_record,
                mapped_range,
                &new_record,
                &previous.source,
                snapshot,
            )
        {
            reused_suffix_start = candidate_index;
            suffix_delta = i128::from(new_record.block.range.start().get())
                - i128::from(old_record.block.range.start().get());
            break;
        }

        middle.push(new_record);
    }

    let blocks = BlockSequence::assemble(
        (&previous.blocks, 0..reparse_index),
        middle,
        (
            &previous.blocks,
            reused_suffix_start..previous.blocks.len(),
            suffix_delta,
        ),
    );
    let document = MarkdownDocument {
        revision: snapshot.revision(),
        source_len: snapshot.len_bytes(),
        source: snapshot.clone(),
        references: ReferenceDefinitionIndex::from_blocks(snapshot, &blocks),
        blocks,
        tree,
        reparsed_bytes,
    };
    let reparsed_range = TextRange::new(new_start, scanned_end)
        .expect("the scanner cannot finish before its reparse boundary");

    Ok(IncrementalParse {
        document,
        reparsed_range,
        reused_prefix_blocks: reparse_index,
        reused_suffix_blocks: previous.blocks.len() - reused_suffix_start,
    })
}

/// 按行走的块扫描器。
///
/// 它定**边界**：哪几行属于同一个块、块从哪个字节到哪个字节、铺满整篇源码
/// （`has_lossless_coverage`）、以及增量复用要的 `BlockState` 与源码哈希。
///
/// 它**不定块是什么**——`BlockKind` 由 [`crate::classify`] 问语法树要。行首
/// 那几个判断（`opening_fence` / `atx_heading_level` / `block_marker` /
/// `is_reference_definition`）留在这里只为一件事：**这一行要不要另起一个
/// 块**。它们的答案不再直接变成块的身份。
struct BlockParser<'a> {
    snapshot: &'a TextSnapshot,
    tree: Option<&'a Tree>,
    lines: Peekable<LineCursor<'a>>,
}

impl<'a> BlockParser<'a> {
    fn new(snapshot: &'a TextSnapshot, tree: Option<&'a Tree>, start: usize) -> Self {
        Self {
            snapshot,
            tree,
            lines: LineCursor::new(snapshot, start).peekable(),
        }
    }

    fn is_reference_definition(&self, line: Line) -> bool {
        let Some(range) = TextRange::new(
            ByteOffset::try_from(line.start).expect("line start must fit u64"),
            ByteOffset::try_from(line.end).expect("line end must fit u64"),
        ) else {
            return false;
        };
        reference::is_reference_definition_line(self.snapshot, range)
    }

    /// 一个块：边界在这里定，身份问树。
    fn record(
        &self,
        shape: BlockShape,
        start: usize,
        end: usize,
        end_state: BlockState,
        source_hash: SourceHash,
    ) -> BlockRecord {
        let range = block_range(start, end);
        let kind = classify::classify(self.tree, self.snapshot, range, shape);
        BlockRecord {
            block: Block { kind, range },
            start_state: BlockState::Normal,
            end_state,
            source_hash,
        }
    }

    /// 空行块。它不问树：空行在树里没有节点，而「这一行只有空白」是一句
    /// 词法事实，不是 Markdown 语法的分类。
    fn blank_record(&self, line: Line) -> BlockRecord {
        BlockRecord {
            block: Block {
                kind: BlockKind::BlankLine,
                range: block_range(line.start, line.end),
            },
            start_state: BlockState::Normal,
            end_state: BlockState::Normal,
            source_hash: line.source_hash,
        }
    }
}

impl Iterator for BlockParser<'_> {
    type Item = BlockRecord;

    fn next(&mut self) -> Option<Self::Item> {
        let line = self.lines.next()?;
        if line.is_blank() {
            return Some(self.blank_record(line));
        }

        if let Some(fence) = line.opening_fence() {
            let block_start = line.start;
            let mut closed = false;
            let mut end = line.end;
            let mut source_hash = line.source_hash;
            for candidate in self.lines.by_ref() {
                end = candidate.end;
                source_hash = concatenate_hash(
                    source_hash,
                    candidate.source_hash,
                    candidate.end - candidate.start,
                );
                if candidate.is_closing_fence(fence) {
                    closed = true;
                    break;
                }
            }
            let end_state = if closed {
                BlockState::Normal
            } else {
                BlockState::Fenced {
                    marker: fence.marker,
                    minimum: fence.count,
                }
            };
            return Some(self.record(
                BlockShape::Fence {
                    marker: fence.marker,
                    closed,
                },
                block_start,
                end,
                end_state,
                source_hash,
            ));
        }

        // ATX 标题与引用定义各占一行，各自另起一个块。这两条判断只管边界：
        // 「是不是标题」「是不是一条定义」由树回答，`BlockShape::Plain` 把
        // 这个问题整个交出去。
        if line.atx_heading_level().is_some() || self.is_reference_definition(line) {
            return Some(self.record(
                BlockShape::Plain,
                line.start,
                line.end,
                BlockState::Normal,
                line.source_hash,
            ));
        }

        if let Some(marker) = line.block_marker() {
            return Some(self.parse_container(line, marker));
        }

        let block_start = line.start;
        let mut end = line.end;
        let mut source_hash = line.source_hash;
        while let Some(candidate) = self.lines.peek().copied() {
            if candidate.is_blank()
                || candidate.opening_fence().is_some()
                || candidate.atx_heading_level().is_some()
                || candidate.block_marker().is_some()
                || self.is_reference_definition(candidate)
            {
                break;
            }
            let line = self
                .lines
                .next()
                .expect("a peeked paragraph line must remain available");
            end = line.end;
            source_hash = concatenate_hash(source_hash, line.source_hash, line.end - line.start);
        }
        Some(self.record(
            BlockShape::Plain,
            block_start,
            end,
            BlockState::Normal,
            source_hash,
        ))
    }
}

impl BlockParser<'_> {
    fn parse_container(&mut self, first: Line, marker: LineMarker) -> BlockRecord {
        let block_start = first.start;
        let mut end = first.end;
        let mut source_hash = first.source_hash;

        while let Some(candidate) = self.lines.peek().copied() {
            if candidate.is_blank()
                || candidate.opening_fence().is_some()
                || candidate.atx_heading_level().is_some()
            {
                break;
            }

            match (marker, candidate.block_marker()) {
                (
                    LineMarker::BlockQuote { depth },
                    Some(LineMarker::BlockQuote { depth: next }),
                ) if depth == next => {
                    self.consume_line(&mut end, &mut source_hash);
                }
                (LineMarker::BlockQuote { .. }, Some(_)) | (LineMarker::List { .. }, Some(_)) => {
                    break;
                }
                (_, None) => {
                    // A non-marked line is a lazy continuation of the current
                    // container. Keeping it in the same source range avoids
                    // inventing a second canonical text representation.
                    self.consume_line(&mut end, &mut source_hash);
                }
            }
        }

        // `- [x]` 是不是一个任务项由树回答（`classify`）。这里只把行扫描器
        // 读到的那部分负载带过去：序号、标记字符、缩进层数。
        let shape = match marker {
            LineMarker::BlockQuote { depth } => BlockShape::Quote { depth },
            LineMarker::List {
                ordered,
                depth,
                marker,
                start,
            } => BlockShape::List {
                ordered,
                depth,
                marker,
                start,
            },
        };
        self.record(shape, block_start, end, BlockState::Normal, source_hash)
    }

    fn consume_line(&mut self, end: &mut usize, source_hash: &mut SourceHash) {
        let line = self
            .lines
            .next()
            .expect("a peeked container continuation must remain available");
        *end = line.end;
        *source_hash = concatenate_hash(*source_hash, line.source_hash, line.end - line.start);
    }
}

/// Returns the parser-owned task marker range for a task-list block.
#[must_use]
pub fn task_marker(source: &TextSnapshot, block: Block) -> Option<TaskMarker> {
    let BlockKind::TaskListItem { ordered, .. } = block.kind() else {
        return None;
    };
    task::parse_task_marker(source, block.range(), ordered)
}

/// Returns the parser-owned first-line list marker and structural prefix.
///
/// Consumers use the exact prefix range for source hiding and retain the
/// marker range as the identity of a semantic bullet or ordinal. Parsing is
/// chunk-aware and validates the source against the stable `BlockKind`
/// metadata instead of asking projection/layout code to rescan Markdown.
#[must_use]
pub fn list_marker(source: &TextSnapshot, block: Block) -> Option<ListMarker> {
    let (ordered, expected_marker, expected_start) = match block.kind() {
        BlockKind::ListItem {
            ordered,
            marker,
            start,
            ..
        }
        | BlockKind::TaskListItem {
            ordered,
            marker,
            start,
            ..
        } => (ordered, marker, start),
        _ => return None,
    };
    let prefix_start = block.range().start();
    let mut cursor = SourceByteCursor::new(source, block.range())?;
    let mut next = cursor.next();
    let mut leading_spaces = 0_usize;
    while let Some((_, b' ')) = next {
        leading_spaces = leading_spaces.saturating_add(1);
        if leading_spaces > 3 {
            return None;
        }
        next = cursor.next();
    }

    let marker_start = next?.0;
    let (marker_end, parsed_marker, parsed_start, mut following) = if ordered {
        let mut value = 0_u32;
        let mut digits = 0_usize;
        let mut current = next?;
        while current.1.is_ascii_digit() {
            value = value
                .checked_mul(10)?
                .checked_add(u32::from(current.1 - b'0'))?;
            digits = digits.saturating_add(1);
            if digits > 9 {
                return None;
            }
            current = cursor.next()?;
        }
        if !matches!(current.1, b'.' | b')') {
            return None;
        }
        (
            current.0.checked_add(1)?,
            char::from(current.1),
            value,
            cursor.next(),
        )
    } else {
        let (position, byte) = next?;
        if !matches!(byte, b'-' | b'+' | b'*') {
            return None;
        }
        (position.checked_add(1)?, char::from(byte), 1, cursor.next())
    };
    if parsed_marker != expected_marker || parsed_start != expected_start {
        return None;
    }
    if following.is_some_and(|(_, byte)| !matches!(byte, b' ' | b'\t' | b'\r' | b'\n')) {
        return None;
    }
    let mut prefix_end = marker_end;
    while let Some((position, byte)) = following {
        if !matches!(byte, b' ' | b'\t') {
            break;
        }
        prefix_end = position.checked_add(1)?;
        following = cursor.next();
    }
    Some(ListMarker {
        ordered,
        marker: parsed_marker,
        start: parsed_start,
        indent: u8::try_from(leading_spaces).ok()?,
        marker_range: TextRange::new(
            ByteOffset::try_from(marker_start).ok()?,
            ByteOffset::try_from(marker_end).ok()?,
        )?,
        prefix_range: TextRange::new(prefix_start, ByteOffset::try_from(prefix_end).ok()?)?,
    })
}

/// 标题的正文区间：不含结构标记的那一段。
///
/// ATX 去掉行首的 `#` 前缀（连同它后面的空格），Setext 去掉**下划线那一
/// 行**。两种拼法在 [`BlockKind::Heading`] 里是同一个变体，所以「正文在哪」
/// 这个问题也只该有一个答案——导出与可访问性此前各写了一份，两份都只认
/// ATX，于是 Setext 标题在大纲里会带着一行 `===`。
///
/// 不是标题的块返回它自己的整段 range。
#[must_use]
pub fn heading_content_range(source: &TextSnapshot, block: Block) -> TextRange {
    let BlockKind::Heading { level } = block.kind() else {
        return block.range();
    };
    let lines = block_line_ranges(source, block.range());
    let Some(first) = lines.first().copied() else {
        return block.range();
    };
    if let Some(prefix) = heading_prefix_range(source, first, level) {
        return TextRange::new(prefix.end(), line_content_end(source, first))
            .unwrap_or_else(|| block.range());
    }
    // Setext：正文是下划线那一行之前的全部，可能不止一行。
    let content_end = lines
        .iter()
        .rev()
        .nth(1)
        .map_or_else(|| first.start(), |line| line_content_end(source, *line));
    TextRange::new(first.start(), content_end).unwrap_or_else(|| block.range())
}

/// 一行去掉行尾换行符之后的终点。
fn line_content_end(source: &TextSnapshot, line: TextRange) -> ByteOffset {
    let Some(cursor) = SourceByteCursor::new(source, line) else {
        return line.end();
    };
    let mut end = line.start();
    for (position, byte) in cursor {
        if !matches!(byte, b'\r' | b'\n') {
            end = ByteOffset::try_from(position.saturating_add(1)).unwrap_or(end);
        }
    }
    end
}

/// Returns parser-owned block syntax ranges that the visual projection may
/// hide without re-parsing Markdown in a consumer.
///
/// The ranges deliberately cover only structural prefixes. Ordinary list
/// prefixes are hidden because projection/layout retain a source-backed
/// semantic marker; task-list prefixes remain visible because their checkbox
/// is an additional control rather than a replacement for the bullet. ATX
/// heading and blockquote prefixes are block-level style/indentation rather
/// than editable text.
#[must_use]
pub fn block_syntax_hidden_ranges(source: &TextSnapshot, block: Block) -> Vec<TextRange> {
    let lines = block_line_ranges(source, block.range());
    match block.kind() {
        // Setext 标题在这里是一段**空**的隐藏区间：它的结构标记是下面那一
        // 整行，不是行首的一段前缀，`heading_prefix_range` 认不出来也不该
        // 认。下划线由装饰层藏（`extension/heading.rs`）。
        BlockKind::Heading { level } => lines
            .first()
            .and_then(|line| heading_prefix_range(source, *line, level))
            .into_iter()
            .collect(),
        BlockKind::BlockQuote { .. } => lines
            .iter()
            .filter_map(|line| blockquote_prefix_range(source, *line))
            .collect(),
        BlockKind::BlankLine
        | BlockKind::ReferenceDefinition
        | BlockKind::Paragraph
        | BlockKind::FencedCodeBlock { .. }
        | BlockKind::TaskListItem { .. } => Vec::new(),
        BlockKind::ListItem { .. } => list_marker(source, block)
            .map(ListMarker::prefix_range)
            .into_iter()
            .collect(),
    }
}

pub(crate) fn block_line_ranges(source: &TextSnapshot, range: TextRange) -> Vec<TextRange> {
    let Ok(start) = usize::try_from(range.start()) else {
        return Vec::new();
    };
    let Ok(end) = usize::try_from(range.end()) else {
        return Vec::new();
    };
    let Ok(mut chunks) = source.chunk_cursor(range.start()) else {
        return Vec::new();
    };
    let mut line_start = start;
    let mut lines = Vec::new();
    for chunk in &mut chunks {
        let Ok(chunk_start) = usize::try_from(chunk.start()) else {
            return Vec::new();
        };
        let chunk_end = chunk_start.saturating_add(chunk.text().len());
        let local_start = start.max(chunk_start).saturating_sub(chunk_start);
        let local_end = end.min(chunk_end).saturating_sub(chunk_start);
        for (index, byte) in chunk.text().as_bytes()[local_start..local_end]
            .iter()
            .enumerate()
        {
            if *byte != b'\n' {
                continue;
            }
            let absolute = chunk_start
                .saturating_add(local_start)
                .saturating_add(index + 1);
            let Ok(line_end) = ByteOffset::try_from(absolute) else {
                return Vec::new();
            };
            let Ok(line_start_offset) = ByteOffset::try_from(line_start) else {
                return Vec::new();
            };
            let Some(line) = TextRange::new(line_start_offset, line_end) else {
                return Vec::new();
            };
            lines.push(line);
            line_start = absolute;
        }
    }
    if line_start < end {
        let Ok(line_start_offset) = ByteOffset::try_from(line_start) else {
            return Vec::new();
        };
        let Ok(line_end) = ByteOffset::try_from(end) else {
            return Vec::new();
        };
        if let Some(line) = TextRange::new(line_start_offset, line_end) {
            lines.push(line);
        }
    }
    lines
}

fn heading_prefix_range(source: &TextSnapshot, line: TextRange, level: u8) -> Option<TextRange> {
    let start = usize::try_from(line.start()).ok()?;
    let line_end = usize::try_from(line.end()).ok()?;
    let mut cursor = SourceByteCursor::new(source, line)?;
    let mut next = cursor.next();
    let mut leading_spaces = 0_usize;
    while let Some((_, b' ')) = next {
        leading_spaces = leading_spaces.saturating_add(1);
        if leading_spaces > 3 {
            return None;
        }
        next = cursor.next();
    }
    let mut hashes = 0_u8;
    while hashes < level {
        let (_, byte) = next?;
        if byte != b'#' {
            return None;
        }
        hashes = hashes.saturating_add(1);
        next = cursor.next();
    }
    let prefix_end = match next {
        None => line_end,
        Some((position, b'\n' | b'\r')) => position,
        Some((position, b' ' | b'\t')) => {
            let mut end = position.saturating_add(1);
            for (next_position, byte) in cursor {
                if matches!(byte, b' ' | b'\t') {
                    end = next_position.saturating_add(1);
                } else {
                    end = next_position;
                    break;
                }
            }
            end
        }
        Some(_) => return None,
    };
    let start = ByteOffset::try_from(start).ok()?;
    let end = ByteOffset::try_from(prefix_end.min(line_end)).ok()?;
    (end > start).then(|| TextRange::new(start, end)).flatten()
}

fn blockquote_prefix_range(source: &TextSnapshot, line: TextRange) -> Option<TextRange> {
    let start = usize::try_from(line.start()).ok()?;
    let line_end = usize::try_from(line.end()).ok()?;
    let mut cursor = SourceByteCursor::new(source, line)?;
    let mut next = cursor.next();
    let mut leading_spaces = 0_usize;
    while let Some((_, b' ')) = next {
        leading_spaces = leading_spaces.saturating_add(1);
        if leading_spaces > 3 {
            return None;
        }
        next = cursor.next();
    }
    let Some((_, b'>')) = next else {
        return None;
    };
    next = cursor.next();
    let prefix_end = match next {
        None => line_end,
        Some((position, b'\n' | b'\r')) => position,
        Some((position, b' ' | b'\t')) => {
            let mut end = position.saturating_add(1);
            for (next_position, byte) in cursor {
                if matches!(byte, b' ' | b'\t') {
                    end = next_position.saturating_add(1);
                } else {
                    end = next_position;
                    break;
                }
            }
            end
        }
        Some(_) => return None,
    };
    let start = ByteOffset::try_from(start).ok()?;
    let end = ByteOffset::try_from(prefix_end.min(line_end)).ok()?;
    (end > start).then(|| TextRange::new(start, end)).flatten()
}

struct SourceByteCursor<'a> {
    chunks: ChunkCursor<'a>,
    requested_start: usize,
    end: usize,
    current: Option<&'a str>,
    current_start: usize,
    current_index: usize,
}

impl<'a> SourceByteCursor<'a> {
    fn new(source: &'a TextSnapshot, range: TextRange) -> Option<Self> {
        Some(Self {
            chunks: source.chunk_cursor(range.start()).ok()?,
            requested_start: usize::try_from(range.start()).ok()?,
            end: usize::try_from(range.end()).ok()?,
            current: None,
            current_start: 0,
            current_index: 0,
        })
    }
}

impl Iterator for SourceByteCursor<'_> {
    type Item = (usize, u8);

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            if let Some(current) = self.current {
                if self.current_index < self.current_start + current.len()
                    && self.current_index < self.end
                {
                    let local = self.current_index - self.current_start;
                    let position = self.current_index;
                    let byte = current.as_bytes()[local];
                    self.current_index += 1;
                    return Some((position, byte));
                }
                self.current = None;
            }

            let chunk = self.chunks.next()?;
            self.current_start = usize::try_from(chunk.start()).ok()?;
            self.current_index = self.current_start.max(self.requested_start);
            self.current = Some(chunk.text());
            if self.current_index < self.end {
                continue;
            }
            return None;
        }
    }
}

fn block_range(start: usize, end: usize) -> TextRange {
    let start = ByteOffset::try_from(start).unwrap_or(ByteOffset::new(u64::MAX));
    let end = ByteOffset::try_from(end).unwrap_or(ByteOffset::new(u64::MAX));
    TextRange::new(start, end).unwrap_or_else(|| TextRange::empty(start))
}

struct LineCursor<'a> {
    chunks: ChunkCursor<'a>,
    source_len: usize,
    requested_start: usize,
    current_text: Option<&'a str>,
    current_start: usize,
    current_local: usize,
    line_start: usize,
    pending_cr: bool,
    analysis: LineAnalysis,
    source_hash: SourceHash,
    finished: bool,
}

impl<'a> LineCursor<'a> {
    fn new(snapshot: &'a TextSnapshot, start: usize) -> Self {
        let source_len = usize::try_from(snapshot.len_bytes())
            .expect("Snapshot length must fit the platform address space");
        let start_offset = ByteOffset::try_from(start).expect("line start must fit u64");
        let chunks = snapshot
            .chunk_cursor(start_offset)
            .expect("block boundary must be a valid UTF-8 offset");
        Self {
            chunks,
            source_len,
            requested_start: start,
            current_text: None,
            current_start: 0,
            current_local: 0,
            line_start: start,
            pending_cr: false,
            analysis: LineAnalysis::default(),
            source_hash: SourceHash(0),
            finished: start >= source_len,
        }
    }
}

impl Iterator for LineCursor<'_> {
    type Item = Line;

    fn next(&mut self) -> Option<Self::Item> {
        if self.finished {
            return None;
        }

        loop {
            if self.current_text.is_none() {
                let Some(chunk) = self.chunks.next() else {
                    self.finished = true;
                    return (self.line_start < self.source_len).then_some(Line {
                        start: self.line_start,
                        end: self.source_len,
                        analysis: self.analysis,
                        source_hash: self.source_hash,
                    });
                };
                self.current_start =
                    usize::try_from(chunk.start()).expect("chunk offset must fit usize");
                self.current_local = self
                    .requested_start
                    .saturating_sub(self.current_start)
                    .min(chunk.text().len());
                self.current_text = Some(chunk.text());
            }

            let text = self.current_text.expect("current chunk was initialized");
            while self.current_local < text.len() {
                if !self.analysis.wants_input() {
                    let Some(newline) = text.as_bytes()[self.current_local..]
                        .iter()
                        .position(|value| *value == b'\n')
                    else {
                        self.source_hash =
                            extend_hash(self.source_hash, &text.as_bytes()[self.current_local..]);
                        self.current_local = text.len();
                        break;
                    };
                    let consumed_end = self.current_local + newline + 1;
                    self.source_hash = extend_hash(
                        self.source_hash,
                        &text.as_bytes()[self.current_local..consumed_end],
                    );
                    let absolute = self.current_start + self.current_local + newline;
                    let line = Line {
                        start: self.line_start,
                        end: absolute + 1,
                        analysis: self.analysis,
                        source_hash: self.source_hash,
                    };
                    self.line_start = absolute + 1;
                    self.analysis = LineAnalysis::default();
                    self.source_hash = SourceHash(0);
                    self.pending_cr = false;
                    self.current_local = consumed_end;
                    return Some(line);
                }

                let character_start = self.current_local;
                let first = text.as_bytes()[character_start];
                let character = if first.is_ascii() {
                    char::from(first)
                } else {
                    text[self.current_local..]
                        .chars()
                        .next()
                        .expect("non-empty UTF-8 tail must contain a character")
                };
                let absolute = self.current_start + self.current_local;
                self.current_local += character.len_utf8();
                self.source_hash = extend_hash(
                    self.source_hash,
                    &text.as_bytes()[character_start..self.current_local],
                );
                if character == '\n' {
                    let line = Line {
                        start: self.line_start,
                        end: absolute + 1,
                        analysis: self.analysis,
                        source_hash: self.source_hash,
                    };
                    self.line_start = absolute + 1;
                    self.analysis = LineAnalysis::default();
                    self.source_hash = SourceHash(0);
                    self.pending_cr = false;
                    return Some(line);
                }

                if self.pending_cr {
                    self.analysis.push('\r');
                    self.pending_cr = false;
                }
                if character == '\r' {
                    self.pending_cr = true;
                } else {
                    self.analysis.push(character);
                }
            }
            self.current_text = None;
        }
    }
}

const HASH_BASE: u64 = 0x0000_0100_0000_01b3;

fn extend_hash(mut hash: SourceHash, bytes: &[u8]) -> SourceHash {
    for byte in bytes {
        hash.0 = hash
            .0
            .wrapping_mul(HASH_BASE)
            .wrapping_add(u64::from(*byte) + 1);
    }
    hash
}

fn concatenate_hash(left: SourceHash, right: SourceHash, right_len: usize) -> SourceHash {
    SourceHash(
        left.0
            .wrapping_mul(wrapping_power(HASH_BASE, right_len))
            .wrapping_add(right.0),
    )
}

fn wrapping_power(mut base: u64, mut exponent: usize) -> u64 {
    let mut result = 1_u64;
    while exponent > 0 {
        if exponent & 1 == 1 {
            result = result.wrapping_mul(base);
        }
        base = base.wrapping_mul(base);
        exponent >>= 1;
    }
    result
}

fn map_unchanged_range(
    revision: Revision,
    range: TextRange,
    changes: &ChangeSet,
) -> Result<TextRange, AnchorMapError> {
    let start = changes
        .map_anchor(TextAnchor::new(revision, range.start(), Affinity::After))?
        .offset();
    let end = changes
        .map_anchor(TextAnchor::new(revision, range.end(), Affinity::Before))?
        .offset();
    Ok(TextRange::new(start, end).expect("an unaffected mapped block must remain ordered"))
}

fn records_converge(
    old: ResolvedBlockRecord,
    mapped_old_range: TextRange,
    new: &BlockRecord,
    old_source: &TextSnapshot,
    new_source: &TextSnapshot,
) -> bool {
    mapped_old_range == new.block.range
        && old.block.kind == new.block.kind
        && old.start_state == new.start_state
        && old.end_state == new.end_state
        && old.source_hash == new.source_hash
        && ranges_equal(old_source, old.block.range, new_source, new.block.range)
}

fn ranges_equal(
    left_source: &TextSnapshot,
    left_range: TextRange,
    right_source: &TextSnapshot,
    right_range: TextRange,
) -> bool {
    left_range.len() == right_range.len()
        && RangeSlices::new(left_source, left_range)
            .flat_map(|slice| slice.iter().copied())
            .eq(RangeSlices::new(right_source, right_range).flat_map(|slice| slice.iter().copied()))
}

struct RangeSlices<'a> {
    chunks: ChunkCursor<'a>,
    start: usize,
    end: usize,
}

impl<'a> RangeSlices<'a> {
    fn new(snapshot: &'a TextSnapshot, range: TextRange) -> Self {
        Self {
            chunks: snapshot
                .chunk_cursor(range.start())
                .expect("block ranges must start at valid UTF-8 boundaries"),
            start: usize::try_from(range.start()).expect("block offset must fit usize"),
            end: usize::try_from(range.end()).expect("block offset must fit usize"),
        }
    }
}

impl<'a> Iterator for RangeSlices<'a> {
    type Item = &'a [u8];

    fn next(&mut self) -> Option<Self::Item> {
        for chunk in self.chunks.by_ref() {
            let chunk_start = usize::try_from(chunk.start()).expect("chunk offset must fit usize");
            if chunk_start >= self.end {
                return None;
            }
            let chunk_end = chunk_start + chunk.text().len();
            let start = self.start.max(chunk_start) - chunk_start;
            let end = self.end.min(chunk_end) - chunk_start;
            if start < end {
                return Some(&chunk.text().as_bytes()[start..end]);
            }
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use yu_text::{Edit, TextBuffer, Transaction, retained_snapshot_stats};

    #[test]
    fn scanner_covers_source_without_gaps() {
        let source = "# 羽\n\nparagraph\ncontinued\n\n```rust\nfn main() {}\n```\n";
        let buffer = TextBuffer::new(source);
        let document = parse(&buffer.snapshot());

        assert!(document.has_lossless_coverage());
        assert_eq!(document.source_len().get(), source.len() as u64);
        assert_eq!(document.blocks().len(), 5);
        assert_eq!(kind_at(&document, 0), BlockKind::Heading { level: 1 });
        assert_eq!(kind_at(&document, 1), BlockKind::BlankLine);
        assert_eq!(kind_at(&document, 2), BlockKind::Paragraph);
        assert_eq!(
            kind_at(&document, 4),
            BlockKind::FencedCodeBlock {
                marker: '`',
                closed: true
            }
        );
    }

    #[test]
    fn block_sequence_resolves_source_caret_boundaries_without_linear_scan() {
        let source = "# title\n\nparagraph\n";
        let buffer = TextBuffer::new(source);
        let document = parse(&buffer.snapshot());
        let blocks = document.blocks();

        assert_eq!(blocks.block_index_for_offset(ByteOffset::new(0)), Some(0));
        assert_eq!(blocks.block_index_for_offset(ByteOffset::new(8)), Some(1));
        assert_eq!(blocks.block_index_for_offset(ByteOffset::new(9)), Some(2));
        assert_eq!(
            blocks.block_index_for_offset(ByteOffset::new(source.len() as u64)),
            Some(2)
        );
        assert_eq!(
            blocks.block_index_for_offset(ByteOffset::new(source.len() as u64 + 1)),
            None
        );
        assert_eq!(
            blocks.block_index_range_for_source_range(
                TextRange::new(ByteOffset::new(8), ByteOffset::new(10))
                    .expect("test range should be valid")
            ),
            Some(1..3)
        );
        assert_eq!(
            blocks.block_index_range_for_source_range(
                TextRange::new(ByteOffset::new(9), ByteOffset::new(18))
                    .expect("test range should be valid")
            ),
            Some(2..3)
        );
    }

    #[test]
    fn scanner_preserves_phase_one_line_classification_rules() {
        let cases = [
            ("   # title\n", BlockKind::Heading { level: 1 }),
            ("    # title\n", BlockKind::Paragraph),
            ("####### title\n", BlockKind::Paragraph),
            ("\u{00a0}\n", BlockKind::BlankLine),
            (
                "```\r\nbody\r\n```\r\n",
                BlockKind::FencedCodeBlock {
                    marker: '`',
                    closed: true,
                },
            ),
            (
                "```\nbody\n``` trailing\n",
                BlockKind::FencedCodeBlock {
                    marker: '`',
                    closed: false,
                },
            ),
        ];

        for (source, expected) in cases {
            let document = parse(&TextBuffer::new(source).snapshot());
            assert_eq!(document.blocks().len(), 1, "source {source:?}");
            assert_eq!(kind_at(&document, 0), expected, "source {source:?}");
            assert!(document.has_lossless_coverage());
        }
    }

    #[test]
    fn scanner_classifies_blockquotes_and_list_items_without_losing_ranges() {
        let source = "> quoted\n> continued\n\n- one\n  continuation\n  - nested\n- two\n\n1. first\n2) second\n";
        let buffer = TextBuffer::new(source);
        let snapshot = buffer.snapshot();
        let document = parse(&snapshot);

        assert!(document.has_lossless_coverage());
        assert_eq!(document.blocks().len(), 8);
        assert_eq!(kind_at(&document, 0), BlockKind::BlockQuote { depth: 1 });
        assert_eq!(kind_at(&document, 1), BlockKind::BlankLine);
        assert_eq!(
            kind_at(&document, 2),
            BlockKind::ListItem {
                ordered: false,
                depth: 0,
                marker: '-',
                start: 1,
            }
        );
        assert_eq!(
            kind_at(&document, 3),
            BlockKind::ListItem {
                ordered: false,
                depth: 1,
                marker: '-',
                start: 1,
            }
        );
        assert_eq!(
            kind_at(&document, 4),
            BlockKind::ListItem {
                ordered: false,
                depth: 0,
                marker: '-',
                start: 1,
            }
        );
        assert_eq!(kind_at(&document, 5), BlockKind::BlankLine);
        assert_eq!(
            kind_at(&document, 6),
            BlockKind::ListItem {
                ordered: true,
                depth: 0,
                marker: '.',
                start: 1,
            }
        );
        assert_eq!(
            kind_at(&document, 7),
            BlockKind::ListItem {
                ordered: true,
                depth: 0,
                marker: ')',
                start: 2,
            }
        );

        let reconstructed: String = document
            .blocks()
            .iter()
            .map(|block| {
                let start = usize::try_from(block.range().start()).expect("offset fits usize");
                let end = usize::try_from(block.range().end()).expect("offset fits usize");
                &snapshot.as_str()[start..end]
            })
            .collect();
        assert_eq!(reconstructed, source);
    }

    #[test]
    fn block_syntax_hidden_ranges_are_parser_owned_and_line_local() {
        let source = "  ## 标题\n\n> 引用\n  延续\n\n- 普通\n\n- [ ] 任务\n";
        let buffer = TextBuffer::new(source);
        let snapshot = buffer.snapshot();
        let document = parse(&snapshot);

        let heading = document.blocks().get(0).expect("heading block");
        let heading_ranges = block_syntax_hidden_ranges(&snapshot, heading);
        assert_eq!(heading_ranges.len(), 1);
        assert_eq!(
            &snapshot.as_str()
                [heading_ranges[0].start().get() as usize..heading_ranges[0].end().get() as usize],
            "  ## "
        );

        let quote = document.blocks().get(2).expect("quote block");
        let quote_ranges = block_syntax_hidden_ranges(&snapshot, quote);
        assert_eq!(quote_ranges.len(), 1);
        assert_eq!(
            &snapshot.as_str()
                [quote_ranges[0].start().get() as usize..quote_ranges[0].end().get() as usize],
            "> "
        );

        let list = document.blocks().get(4).expect("list block");
        let list_ranges = block_syntax_hidden_ranges(&snapshot, list);
        assert_eq!(list_ranges.len(), 1);
        assert_eq!(
            &snapshot.as_str()
                [list_ranges[0].start().get() as usize..list_ranges[0].end().get() as usize],
            "- "
        );

        let task = document.blocks().get(6).expect("task block");
        assert!(block_syntax_hidden_ranges(&snapshot, task).is_empty());
    }

    #[test]
    fn list_markers_keep_token_and_prefix_ranges_distinct() {
        // 三个列表项之间必须空一行：序号不是 1 的有序列表、以及没有内容的
        // `+`，在 CommonMark 里都**打断不了段落**，挨着写的话它们是上一段的
        // 延续而不是列表项（见 `crate::classify`）。
        let source = "  - item\n\n12) ordered\n\n+\n";
        let snapshot = TextBuffer::new(source).snapshot();
        let document = parse(&snapshot);

        let unordered = list_marker(&snapshot, document.blocks().get(0).expect("unordered list"))
            .expect("unordered marker");
        assert!(!unordered.ordered());
        assert_eq!(unordered.marker(), '-');
        assert_eq!(unordered.start(), 1);
        assert_eq!(unordered.indent(), 2);
        assert_eq!(
            &source[unordered.marker_range().start().get() as usize
                ..unordered.marker_range().end().get() as usize],
            "-"
        );
        assert_eq!(
            &source[unordered.prefix_range().start().get() as usize
                ..unordered.prefix_range().end().get() as usize],
            "  - "
        );

        let ordered = list_marker(&snapshot, document.blocks().get(2).expect("ordered list"))
            .expect("ordered marker");
        assert!(ordered.ordered());
        assert_eq!(ordered.marker(), ')');
        assert_eq!(ordered.start(), 12);
        assert_eq!(ordered.indent(), 0);
        assert_eq!(
            &source[ordered.marker_range().start().get() as usize
                ..ordered.marker_range().end().get() as usize],
            "12)"
        );
        assert_eq!(
            &source[ordered.prefix_range().start().get() as usize
                ..ordered.prefix_range().end().get() as usize],
            "12) "
        );

        let empty = list_marker(&snapshot, document.blocks().get(4).expect("empty list"))
            .expect("empty marker");
        assert_eq!(empty.marker_range(), empty.prefix_range());
    }

    #[test]
    fn scanner_classifies_task_list_items_and_exposes_marker_ranges() {
        let source = "- [ ] todo\n1. [x] done\n- [X] done\n- [x]attached\n";
        let snapshot = TextBuffer::new(source).snapshot();
        let document = parse(&snapshot);

        assert_eq!(document.blocks().len(), 4);
        assert_eq!(
            kind_at(&document, 0),
            BlockKind::TaskListItem {
                ordered: false,
                depth: 0,
                marker: '-',
                start: 1,
                state: TaskState::Todo,
            }
        );
        assert_eq!(
            kind_at(&document, 1),
            BlockKind::TaskListItem {
                ordered: true,
                depth: 0,
                marker: '.',
                start: 1,
                state: TaskState::Done,
            }
        );
        assert_eq!(
            kind_at(&document, 2),
            BlockKind::TaskListItem {
                ordered: false,
                depth: 0,
                marker: '-',
                start: 1,
                state: TaskState::Done,
            }
        );
        assert_eq!(
            kind_at(&document, 3),
            BlockKind::ListItem {
                ordered: false,
                depth: 0,
                marker: '-',
                start: 1,
            }
        );

        let marker = task_marker(
            &snapshot,
            document.blocks().get(0).expect("task block should exist"),
        )
        .expect("task marker should be source-backed");
        assert_eq!(marker.state(), TaskState::Todo);
        assert_eq!(
            &snapshot.as_str()[usize::try_from(marker.range().start()).expect("offset")
                ..usize::try_from(marker.range().end()).expect("offset")],
            "[ ]"
        );
        assert!(document.has_lossless_coverage());
    }

    /// Setext 标题两边曾经不一致：树认得，行扫描器把它当成一个普通段落。
    /// 块的身份由树给之后，两种拼法落在同一个变体上。
    #[test]
    fn setext_and_atx_headings_are_the_same_kind() {
        for (source, level) in [
            ("标题\n===\n", 1),
            ("标题\n---\n", 2),
            ("多行\n标题\n===\n", 1),
            ("# 标题\n", 1),
            ("## 标题\n", 2),
        ] {
            let document = parse(&TextBuffer::new(source).snapshot());
            assert_eq!(document.blocks().len(), 1, "source {source:?}");
            assert_eq!(
                kind_at(&document, 0),
                BlockKind::Heading { level },
                "source {source:?}"
            );
            assert!(document.has_lossless_coverage());
        }
    }

    /// 树的叶子节点横跨两个块时，两个块都不是它。
    ///
    /// `foo\n-` 是一个二级 Setext 标题，而 `-` 那一行在行扫描器眼里像一个
    /// 列表标记，于是它另起了一块。两块都认领这个标题的话，画面上会出现两
    /// 个放大的行，第二个只有一个 `-`。
    #[test]
    fn a_block_that_is_only_a_fragment_of_a_leaf_node_is_a_paragraph() {
        for source in ["foo\n-\n", "foo\n- \n"] {
            let document = parse(&TextBuffer::new(source).snapshot());
            assert_eq!(document.blocks().len(), 2, "source {source:?}");
            assert_eq!(kind_at(&document, 0), BlockKind::Paragraph, "{source:?}");
            assert_eq!(kind_at(&document, 1), BlockKind::Paragraph, "{source:?}");
            assert!(document.has_lossless_coverage());
        }

        // 容器节点横跨是正常的：块就是容器里的一组行。缩进的任务项因此仍然
        // 认得出自己的复选框——它的 `Task` 子节点挂在**内层**的 `ListItem`
        // 上，而外层那个横跨了两个块。
        let document = parse(&TextBuffer::new("- a\n  - [x] b\n").snapshot());
        assert_eq!(
            kind_at(&document, 1),
            BlockKind::TaskListItem {
                ordered: false,
                depth: 1,
                marker: '-',
                start: 1,
                state: TaskState::Done,
            }
        );
    }

    /// 块横跨好几个树块时，树说不出它是什么，按普通段落画。
    ///
    /// `- a\n<div>\nx` 在行扫描器眼里是一个块（`<div>` 是列表项的惰性延续），
    /// 在树里是 `ListItem` 加一个 `HTMLBlock`。退回行扫描器的形状会给整块画
    /// 一个列表标记，而它的后半段根本不是列表。
    #[test]
    fn a_block_spanning_several_tree_blocks_is_a_paragraph() {
        for source in ["- a\n<div>\nx\n", "> a\n<div>\nx\n"] {
            let document = parse(&TextBuffer::new(source).snapshot());
            assert_eq!(document.blocks().len(), 1, "source {source:?}");
            assert_eq!(kind_at(&document, 0), BlockKind::Paragraph, "{source:?}");
        }
    }

    /// 标题行与引用定义行各自另起一个块。
    ///
    /// 这两条边界判断是行扫描器的事——树给的是身份，不是边界。去掉它们的话
    /// `# 标题\n段落` 会变成**一个**块，而那个块横跨两个树块，于是标题不再是
    /// 标题：块的身份没错，是边界把它吃掉了。
    #[test]
    fn a_heading_or_definition_line_starts_its_own_block() {
        let heading = parse(&TextBuffer::new("# 标题\n段落\n").snapshot());
        assert_eq!(heading.blocks().len(), 2);
        assert_eq!(kind_at(&heading, 0), BlockKind::Heading { level: 1 });
        assert_eq!(kind_at(&heading, 1), BlockKind::Paragraph);

        let definition = parse(&TextBuffer::new("[a]: /x\n段落\n").snapshot());
        assert_eq!(definition.blocks().len(), 2);
        assert_eq!(kind_at(&definition, 0), BlockKind::ReferenceDefinition);
        assert_eq!(kind_at(&definition, 1), BlockKind::Paragraph);
        assert_eq!(definition.reference_definitions().definitions().len(), 1);
    }

    /// 打断不了段落的标记行不是它自己那种块。
    ///
    /// CommonMark 说序号不是 1 的有序列表不能打断段落，行扫描器不知道这条
    /// 规则——它看见 `2.` 就另起一块。边界仍然归它，身份归树。
    #[test]
    fn markers_that_cannot_interrupt_a_paragraph_are_paragraphs() {
        let document = parse(&TextBuffer::new("foo\n2. bar\n").snapshot());
        assert_eq!(document.blocks().len(), 2);
        assert_eq!(kind_at(&document, 1), BlockKind::Paragraph);
    }

    /// 引用定义同理：`foo` 后面那一行是段落的延续，不是一条定义。
    #[test]
    fn a_definition_line_continuing_a_paragraph_is_not_a_definition() {
        let snapshot = TextBuffer::new("foo\n[a]: /x\n").snapshot();
        let document = parse(&snapshot);
        assert_eq!(document.blocks().len(), 2);
        assert_eq!(kind_at(&document, 1), BlockKind::Paragraph);
        assert!(
            document.reference_definitions().definitions().is_empty(),
            "段落的延续不该进引用表"
        );
    }

    #[test]
    fn heading_content_range_covers_both_spellings() {
        for (source, expected) in [
            ("## 标题\n", "标题"),
            ("  ###   多空格\n", "多空格"),
            ("标题\n===\n", "标题"),
            ("多行\n标题\n---\n", "多行\n标题"),
            ("段落\n", "段落\n"),
        ] {
            let snapshot = TextBuffer::new(source).snapshot();
            let document = parse(&snapshot);
            let block = document.blocks().get(0).expect("至少有一个块");
            let content = heading_content_range(&snapshot, block);
            assert_eq!(
                &snapshot.as_str()[content.start().get() as usize..content.end().get() as usize],
                expected,
                "source {source:?}"
            );
        }
    }

    #[test]
    fn scanner_does_not_treat_attached_markers_as_list_items() {
        let source = "-attached\n1.attached\n*attached\n";
        let document = parse(&TextBuffer::new(source).snapshot());

        assert_eq!(document.blocks().len(), 1);
        assert_eq!(kind_at(&document, 0), BlockKind::Paragraph);
        assert!(document.has_lossless_coverage());
    }

    #[test]
    fn scanner_extracts_source_backed_reference_definitions() {
        let source = "[Project Link]: <https://example.com> \"title\"\n[other]: /docs\n\n[Project Link]\n![other]\n";
        let snapshot = TextBuffer::new(source).snapshot();
        let document = parse(&snapshot);

        assert!(document.has_lossless_coverage());
        assert_eq!(kind_at(&document, 0), BlockKind::ReferenceDefinition);
        assert_eq!(kind_at(&document, 1), BlockKind::ReferenceDefinition);
        assert_eq!(kind_at(&document, 2), BlockKind::BlankLine);
        assert_eq!(kind_at(&document, 3), BlockKind::Paragraph);
        assert_eq!(document.reference_definitions().definitions().len(), 2);

        let paragraph = document.blocks().get(3).expect("paragraph should exist");
        let label_start = source[paragraph.range().start().get() as usize..]
            .find("Project Link")
            .expect("shortcut label should exist") as u64
            + paragraph.range().start().get();
        let label = TextRange::new(
            ByteOffset::new(label_start),
            ByteOffset::new(label_start + "Project Link".len() as u64),
        )
        .expect("label range should be ordered");
        let definition = document
            .reference_definitions()
            .lookup(&snapshot, label)
            .expect("definition should resolve case-insensitively");
        assert_eq!(
            &source[definition.destination().start().get() as usize
                ..definition.destination().end().get() as usize],
            "https://example.com"
        );
    }

    #[test]
    fn four_space_indented_definition_remains_literal_paragraph_text() {
        let document = parse(&TextBuffer::new("    [id]: /docs\n").snapshot());
        assert_eq!(document.blocks().len(), 1);
        assert_eq!(kind_at(&document, 0), BlockKind::Paragraph);
        assert!(document.reference_definitions().definitions().is_empty());
    }

    #[test]
    fn scanner_reads_syntax_across_piece_boundaries_without_materializing() {
        let parts = [
            "#", " 羽", "\r", "\n", "\r\n", "```", "rust\n", "body", "\n`", "``\n",
        ];
        let mut buffer = TextBuffer::new("");
        for part in parts {
            let at = buffer.snapshot().len_bytes();
            let transaction =
                Transaction::new(buffer.revision(), [Edit::new(TextRange::empty(at), part)]);
            buffer
                .apply(&transaction)
                .expect("append transaction should apply");
        }
        let snapshot = buffer.snapshot();
        assert_eq!(
            retained_snapshot_stats(std::slice::from_ref(&snapshot)).materialized_buffers(),
            0
        );

        let document = parse(&snapshot);

        assert!(document.has_lossless_coverage());
        assert_eq!(document.blocks().len(), 3);
        assert_eq!(kind_at(&document, 0), BlockKind::Heading { level: 1 });
        assert_eq!(kind_at(&document, 1), BlockKind::BlankLine);
        assert_eq!(
            kind_at(&document, 2),
            BlockKind::FencedCodeBlock {
                marker: '`',
                closed: true
            }
        );
        assert_eq!(
            retained_snapshot_stats(&[snapshot]).materialized_buffers(),
            0
        );
    }

    #[test]
    fn reference_definition_scan_stays_chunk_aware() {
        let parts = [
            "prefix\n\n[",
            "project",
            "]: <",
            "https://example.com",
            ">\n\n[project]\n",
        ];
        let mut buffer = TextBuffer::new("");
        for part in parts {
            let at = buffer.snapshot().len_bytes();
            let transaction =
                Transaction::new(buffer.revision(), [Edit::new(TextRange::empty(at), part)]);
            buffer
                .apply(&transaction)
                .expect("append transaction should apply");
        }
        let snapshot = buffer.snapshot();
        let document = parse(&snapshot);

        assert_eq!(document.reference_definitions().definitions().len(), 1);
        assert_eq!(
            retained_snapshot_stats(std::slice::from_ref(&snapshot)).materialized_buffers(),
            0
        );
    }

    #[test]
    fn unclosed_fence_owns_the_remaining_source() {
        let buffer = TextBuffer::new("before\n\n~~~\ninside\n");
        let document = parse(&buffer.snapshot());

        assert!(document.has_lossless_coverage());
        assert_eq!(
            kind_at(&document, 2),
            BlockKind::FencedCodeBlock {
                marker: '~',
                closed: false
            }
        );
    }

    #[test]
    fn empty_source_has_lossless_coverage() {
        let buffer = TextBuffer::new("");
        let document = parse(&buffer.snapshot());

        assert!(document.blocks().is_empty());
        assert!(document.has_lossless_coverage());
    }

    #[test]
    fn crlf_bytes_are_preserved_in_ranges() {
        let source = "# title\r\n\r\ntext\r\n";
        let buffer = TextBuffer::new(source);
        let snapshot = buffer.snapshot();
        let document = parse(&snapshot);
        let reconstructed: String = document
            .blocks()
            .iter()
            .map(|block| {
                let start =
                    usize::try_from(block.range().start()).expect("test offset should fit usize");
                let end =
                    usize::try_from(block.range().end()).expect("test offset should fit usize");
                &snapshot.as_str()[start..end]
            })
            .collect();

        assert_eq!(reconstructed, source);
    }

    #[test]
    fn empty_change_set_reuses_the_entire_document() {
        let mut buffer = TextBuffer::new("# title\n\nbody\n");
        let previous = parse(&buffer.snapshot());
        let transaction = Transaction::new(buffer.revision(), std::iter::empty::<Edit>());
        let applied = buffer
            .apply(&transaction)
            .expect("empty transaction should still advance the revision");

        let incremental =
            parse_incremental(&previous, applied.result_snapshot(), applied.change_set())
                .expect("matching revisions should parse incrementally");

        assert_eq!(incremental.document(), &parse(applied.result_snapshot()));
        assert_eq!(incremental.reused_prefix_blocks(), previous.blocks().len());
        assert!(incremental.reparsed_range().is_empty());
    }

    #[test]
    fn incremental_parse_rebuilds_reference_definition_index() {
        let source = "[id]: /docs\n\n[id]\n";
        let mut buffer = TextBuffer::new(source);
        let previous = parse(&buffer.snapshot());
        let label_start = source.find("id").expect("definition label should exist");
        let transaction = Transaction::new(
            buffer.revision(),
            [Edit::new(
                TextRange::new(
                    ByteOffset::new(label_start as u64),
                    ByteOffset::new((label_start + 2) as u64),
                )
                .expect("label range should be ordered"),
                "new",
            )],
        );
        let applied = buffer
            .apply(&transaction)
            .expect("definition edit should apply");
        let incremental =
            parse_incremental(&previous, applied.result_snapshot(), applied.change_set())
                .expect("definition edit should parse incrementally");
        let full = parse(applied.result_snapshot());

        assert_eq!(incremental.document(), &full);
        assert_ne!(
            previous.reference_definitions().fingerprint(),
            full.reference_definitions().fingerprint()
        );
        assert_eq!(full.reference_definitions().definitions().len(), 1);
    }

    #[test]
    fn incremental_parse_reclassifies_task_state_like_full_parse() {
        let source = "- [ ] todo\n\n- [x] done\n";
        let mut buffer = TextBuffer::new(source);
        let previous = parse(&buffer.snapshot());
        let state_offset = source.find("[ ]").expect("todo marker should exist") + 1;
        let transaction = Transaction::new(
            buffer.revision(),
            [Edit::new(
                TextRange::new(
                    ByteOffset::new(state_offset as u64),
                    ByteOffset::new((state_offset + 1) as u64),
                )
                .expect("task state range should be ordered"),
                "x",
            )],
        );
        let applied = buffer
            .apply(&transaction)
            .expect("task state edit should apply");
        let incremental =
            parse_incremental(&previous, applied.result_snapshot(), applied.change_set())
                .expect("task state edit should parse incrementally");
        let full = parse(applied.result_snapshot());

        assert_eq!(incremental.document(), &full);
        assert!(matches!(
            kind_at(&full, 0),
            BlockKind::TaskListItem {
                state: TaskState::Done,
                ..
            }
        ));
        assert!(matches!(
            kind_at(&full, 2),
            BlockKind::TaskListItem {
                state: TaskState::Done,
                ..
            }
        ));
        assert!(
            incremental
                .reparsed_range()
                .contains(ByteOffset::new(state_offset as u64))
        );
    }

    #[test]
    fn empty_incremental_parse_rebinds_definition_index_revision() {
        let mut buffer = TextBuffer::new("[id]: /docs\n\n[id]\n");
        let previous = parse(&buffer.snapshot());
        let transaction = Transaction::new(buffer.revision(), std::iter::empty::<Edit>());
        let applied = buffer
            .apply(&transaction)
            .expect("empty transaction should apply");
        let incremental =
            parse_incremental(&previous, applied.result_snapshot(), applied.change_set())
                .expect("empty edit should parse incrementally");

        assert_eq!(
            incremental.document().reference_definitions().revision(),
            applied.result_snapshot().revision()
        );
        assert_eq!(
            incremental
                .document()
                .reference_definitions()
                .definitions()
                .len(),
            1
        );
    }

    #[test]
    fn incremental_parse_rejects_revision_mismatches() {
        let mut buffer = TextBuffer::new("body\n");
        let old_snapshot = buffer.snapshot();
        let previous = parse(&old_snapshot);
        let transaction = Transaction::new(
            buffer.revision(),
            [Edit::new(TextRange::empty(ByteOffset::ZERO), "# ")],
        );
        let applied = buffer
            .apply(&transaction)
            .expect("valid transaction should apply");

        assert!(matches!(
            parse_incremental(&previous, &old_snapshot, applied.change_set()),
            Err(IncrementalParseError::SnapshotRevision { .. })
        ));
        let wrong_previous = parse(applied.result_snapshot());
        assert!(matches!(
            parse_incremental(
                &wrong_previous,
                applied.result_snapshot(),
                applied.change_set()
            ),
            Err(IncrementalParseError::PreviousRevision { .. })
        ));
    }

    fn kind_at(document: &MarkdownDocument, index: usize) -> BlockKind {
        document
            .blocks()
            .get(index)
            .expect("test block must exist")
            .kind()
    }
}
