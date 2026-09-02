//! 面板上那两列文字：大纲的树、搜索结果的行。
//!
//! # 为什么它在 Rust，不在壳里
//!
//! 这三样东西——平表→树与跨刷新的身份链、「拿源码减掉被藏起来的区间」、
//! 「一处命中显示成哪一行字」——**没有一行需要 AppKit 或 Win32**。它们原来
//! 住在 `platform/macos` 的 `OutlinePanel.swift` / `PanelLabel.swift` /
//! `SearchPanel.swift` 里，只是因为消费它们的控件在那儿。
//!
//! 第二端要一份自己的壳（`NSOutlineView` → `TreeView`），照着写第二遍的
//! 表现不是崩，是**两端同一条标题显示得不一样**、**展开状态在一端活得下来
//! 在另一端活不下来**——都不报错。这与刀 c 把 `FrameKey` 与
//! `RasterizingShaper` 提上来是同一条规矩：唯一实现要落在两端之下，不是
//! 「Rust 一份 + 每个壳各一份」。
//!
//! 挪完之后壳里剩下的是「把一棵树喂给一个树控件」，那本来就该各写各的。
//!
//! # 判据的分工
//!
//! 「**藏对了没有**」不在这里证——那是 `yu-decoration/src/hidden.rs` 的线性
//! 参照实现与 `yu-markdown` 那 45 条压住的事。这一层可能错的是别的：减法漏
//! 一段或多一段、身份链把两条同名标题混成一条、上下文越出块（那会让回报
//! 隐藏区间的那一步拒绝，于是那一行**悄悄**带回语法标记）。用例照这个分工写。

use std::collections::HashMap;

use yu_core::{LineIndex, Revision, TextRange};
use yu_text::{TextPositionError, TextSnapshot};

use crate::document::{EditorDocument, EditorDocumentError};
use crate::outline::{OutlineItem, OutlineSnapshot};
use crate::visual::{VisualTextError, read_visible};

/// 身份链里的分隔符：U+001F UNIT SEPARATOR。
///
/// 标题正文里出现它的可能性不是零，但它是控制字符，落进 Markdown 源码里本来
/// 就不显示；用 `/` 或 `\u{0}` 反而更容易撞上真实文本。
const IDENTITY_SEPARATOR: char = '\u{1F}';

/// 面板这一层可能出的错。
///
/// 都是「这一版拿不到」，不是「这一版是错的」：调用方保留上一版比中止好。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PanelError {
    /// 装饰产出失败，或者块下标越界。
    Document(EditorDocumentError),
    /// 偏移落在源码之外，或不在字符边界上。
    Position(TextPositionError),
    /// 源码读不出来。
    Visual(VisualTextError),
    /// 命中不在任何块里。
    OrphanMatch(TextRange),
}

impl std::fmt::Display for PanelError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Document(error) => write!(formatter, "panel decorations unavailable: {error}"),
            Self::Position(error) => write!(formatter, "panel offset unavailable: {error}"),
            Self::Visual(error) => write!(formatter, "panel source unavailable: {error}"),
            Self::OrphanMatch(range) => {
                write!(formatter, "search hit {range:?} belongs to no block")
            }
        }
    }
}

impl std::error::Error for PanelError {}

impl From<EditorDocumentError> for PanelError {
    fn from(error: EditorDocumentError) -> Self {
        Self::Document(error)
    }
}

impl From<TextPositionError> for PanelError {
    fn from(error: TextPositionError) -> Self {
        Self::Position(error)
    }
}

impl From<VisualTextError> for PanelError {
    fn from(error: VisualTextError) -> Self {
        Self::Visual(error)
    }
}

/// 大纲面板上的一行。
///
/// `child_count` 是**直接**孩子的条数。这份表按文档顺序排，而层级规则
/// （就近挂靠）使文档顺序恰好是前序遍历、一条标题的后代恰好连续，所以
/// 「前序 + 直接孩子数」足以无歧义地还原整棵树——壳里因此没有一次查表、
/// 也没有「父亲查不到怎么办」那一支。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OutlineRow {
    item: OutlineItem,
    label: String,
    identity: String,
    child_count: usize,
}

impl OutlineRow {
    #[must_use]
    pub const fn item(&self) -> OutlineItem {
        self.item
    }

    /// 面板上显示的那一行文字：正文区间减掉被藏起来的几段，再折成一行。
    #[must_use]
    pub fn label(&self) -> &str {
        &self.label
    }

    /// 跨刷新的身份：从根到自己的 label 链，同一个父亲下的同名兄弟按出现
    /// 次序区分。
    ///
    /// **不用 `index`，也不用 `block`**：在文档最前面插一条标题会把后面每
    /// 一条的两个数一起推后，展开状态与选中行会整体错位——不报错，只是
    /// 展开的变成了别人。
    #[must_use]
    pub fn identity(&self) -> &str {
        &self.identity
    }

    #[must_use]
    pub const fn child_count(&self) -> usize {
        self.child_count
    }
}

/// 一版大纲的完整面板形状。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OutlineTree {
    revision: Revision,
    rows: Vec<OutlineRow>,
}

impl OutlineTree {
    /// 按这一版文档算出整棵树。
    ///
    /// # Errors
    ///
    /// 装饰产出失败，或者偏移换算失败。
    pub fn build(document: &mut EditorDocument) -> Result<Self, PanelError> {
        let outline = OutlineSnapshot::from_document(document);
        let snapshot = document.snapshot();

        let mut rows: Vec<OutlineRow> = Vec::with_capacity(outline.len());
        // key 是「父亲的身份 + label」，值是这个 label 在该父亲下出现过几次。
        let mut occurrences: HashMap<String, usize> = HashMap::new();
        for item in outline.items() {
            let label = fold_lines(&visible_source(
                document,
                &snapshot,
                item.block(),
                item.label_range(),
            )?);
            let parent_identity = item
                .parent()
                .and_then(|parent| rows.get(parent))
                .map_or("", OutlineRow::identity);
            let mut base = String::with_capacity(parent_identity.len() + label.len() + 2);
            base.push_str(parent_identity);
            base.push(IDENTITY_SEPARATOR);
            base.push_str(&label);
            let seen = occurrences.entry(base.clone()).or_insert(0);
            let identity = format!("{base}{IDENTITY_SEPARATOR}{seen}");
            *seen += 1;
            if let Some(parent) = item.parent().and_then(|parent| rows.get_mut(parent)) {
                parent.child_count += 1;
            }
            rows.push(OutlineRow {
                item: *item,
                label,
                identity,
                child_count: 0,
            });
        }

        Ok(Self {
            revision: outline.revision(),
            rows,
        })
    }

    #[must_use]
    pub const fn revision(&self) -> Revision {
        self.revision
    }

    /// 按文档顺序（也就是前序）排的全部行。
    #[must_use]
    pub fn rows(&self) -> &[OutlineRow] {
        &self.rows
    }
}

/// 搜索结果面板上的一行。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SearchRow {
    hit: TextRange,
    block: usize,
    block_range: TextRange,
    context: TextRange,
    label: String,
}

impl SearchRow {
    /// 这一行指向的那一处命中。点它就是把选区设成这一段。
    #[must_use]
    pub const fn hit(&self) -> TextRange {
        self.hit
    }

    #[must_use]
    pub const fn block(&self) -> usize {
        self.block
    }

    #[must_use]
    pub const fn block_range(&self) -> TextRange {
        self.block_range
    }

    /// 显示出来的那一段源码在哪：命中所在的那一行 ∩ 它所在的块。
    #[must_use]
    pub const fn context(&self) -> TextRange {
        self.context
    }

    /// 面板上显示的那一行文字：上下文减掉被藏起来的几段，再收掉首尾空白。
    #[must_use]
    pub fn label(&self) -> &str {
        &self.label
    }
}

/// 当前查询在这一版源码上的结果列表。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SearchResults {
    revision: Revision,
    rows: Vec<SearchRow>,
}

impl SearchResults {
    /// 按这一版文档与当前查询算出整列结果。没有搜索时是空的。
    ///
    /// # Errors
    ///
    /// 命中不在任何块里，装饰产出失败，或者偏移换算失败。
    pub fn build(document: &mut EditorDocument) -> Result<Self, PanelError> {
        let snapshot = document.snapshot();
        let revision = snapshot.revision();
        let hits: Vec<TextRange> = document
            .search()
            .map_or_else(Vec::new, |state| state.matches().to_vec());

        let mut rows = Vec::with_capacity(hits.len());
        for hit in hits {
            let block = document
                .block_index_for_source(hit.start())
                .ok_or(PanelError::OrphanMatch(hit))?;
            let block_range = document
                .markdown()
                .blocks()
                .get(block)
                .map(yu_markdown::Block::range)
                .ok_or(PanelError::OrphanMatch(hit))?;
            let context = context_range(&snapshot, hit, block_range)?;
            let raw = visible_source(document, &snapshot, block, context)?;
            rows.push(SearchRow {
                hit,
                block,
                block_range,
                context,
                label: fold_lines(raw.trim()),
            });
        }

        Ok(Self { revision, rows })
    }

    #[must_use]
    pub const fn revision(&self) -> Revision {
        self.revision
    }

    /// 按文档顺序排的全部行，一处命中一行。
    #[must_use]
    pub fn rows(&self) -> &[SearchRow] {
        &self.rows
    }
}

/// 一处命中显示成哪一段源码：**命中所在的那一行 ∩ 它所在的块**。
///
/// 裁进块里不是可选的：拿隐藏区间那一步只接受落在一个块里的请求（跨块的
/// 入口会逼那一层去回答「块边界在哪」，那是上一层的事）。不裁的后果不是
/// 画错，是**静默地不剥**——那一行悄悄带回语法标记。
///
/// 今天这个交集取不出东西来：块的边界也是按 LF 划的，所以一行必然落在一个
/// 块里。留着它的理由只剩一条，而它足够——**「块的边界还没合并」是一条已
/// 登记的欠账**（见 overview 的「块结构合并：调查结论」），那道闸门一旦打开，
/// 块会下降到容器里，「一行落在一个块里」就不再成立。
///
/// > 原来还有第二条理由：平台侧用的是 `NSString.lineRange(for:)`，它认 `\r`
/// > 与 `U+2028/2029`，而块扫描器只认 LF。**那条理由这一刀消失了**，因为
/// > 消失的是那第二个「一行」的定义本身（[`yu_text::TextSnapshot::line_range`]
/// > 现在是唯一实现）。少一条理由，但少的是不该存在的那一条。
fn context_range(
    snapshot: &yu_text::TextSnapshot,
    hit: TextRange,
    block_range: TextRange,
) -> Result<TextRange, PanelError> {
    let line: LineIndex = snapshot.line_index(hit.start())?;
    let line_range = snapshot.line_range(line)?;
    let start = line_range.start().max(block_range.start());
    let end = line_range.end().min(block_range.end());
    Ok(TextRange::new(start, end).unwrap_or_else(|| TextRange::empty(start)))
}

/// 一个块里的一段源码，**减掉被装饰藏起来的那几段**。
///
/// 这一步不自己写：`## **粗** 标题` 显示成 `粗 标题` 与编辑器里那个块显示成
/// 什么，本来就是同一个问题，唯一实现在 [`crate::visual`]（不变量 D4）。平台
/// 侧原来那一份是被 C ABI 逼出来的——区间从一次 FFI 调用来，减法在壳里做，
/// 于是同一个答案有了第二份代码。挪回来之后它连同「区间自相矛盾就整段原样
/// 返回」那一支一起没了：那种输入在这一侧表达不出来。
///
/// **要的是规范装饰，不带光标露出**：面板上的标签不能因为光标恰好停在那个
/// 标题里就突然长出 `**`。`block_decorations` 走的正是无露出的那条缓存路径。
fn visible_source(
    document: &mut EditorDocument,
    snapshot: &TextSnapshot,
    block: usize,
    range: TextRange,
) -> Result<String, PanelError> {
    let decorations = document.block_decorations(block)?;
    Ok(read_visible(snapshot, range, decorations.set())?)
}

/// 折成一行：面板上一行就是一行。
///
/// 唯一会撞上多行的是 **Setext 标题**（`多行\n标题\n===` 的正文是
/// `"多行\n标题"`）与块边界还没合并时的上下文。折行只动空白：按 Unicode 的
/// 换行切开，各段收掉首尾空白，空段丢掉，用一个空格接起来。
fn fold_lines(raw: &str) -> String {
    if !raw.contains(is_line_break) {
        return raw.to_owned();
    }
    raw.split(is_line_break)
        .map(str::trim)
        .filter(|piece| !piece.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

/// Unicode 的换行字符，与 Swift `Character.isNewline` 同一组。
const fn is_line_break(character: char) -> bool {
    matches!(
        character,
        '\n' | '\r' | '\u{0B}' | '\u{0C}' | '\u{85}' | '\u{2028}' | '\u{2029}'
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use yu_core::ByteOffset;
    use yu_text::TextBuffer;

    fn range(start: u64, end: u64) -> TextRange {
        TextRange::new(ByteOffset::new(start), ByteOffset::new(end)).expect("ordered")
    }

    #[test]
    fn folding_a_single_line_changes_nothing() {
        assert_eq!(fold_lines("  留着首尾空白  "), "  留着首尾空白  ");
    }

    #[test]
    fn folding_joins_lines_with_one_space() {
        assert_eq!(fold_lines("多行\n标题"), "多行 标题");
        assert_eq!(fold_lines("多行 \n\n 标题\n"), "多行 标题");
    }

    /// `\u{2028}` 也是换行：面板上一行就是一行，别的行分隔符不许漏网。
    #[test]
    fn folding_covers_the_other_unicode_line_breaks() {
        assert_eq!(fold_lines("a\u{2028}b\u{85}c"), "a b c");
    }

    /// 上下文是「命中所在的那一行」，含它末尾那个 LF——收尾的 trim 会去掉它。
    #[test]
    fn context_is_the_line_that_holds_the_hit() {
        let snapshot = TextBuffer::new("第一行\n第二行\n第三行\n").snapshot();
        let hit = range(10, 13);
        let block = range(0, 30);
        assert_eq!(
            context_range(&snapshot, hit, block).expect("in bounds"),
            range(10, 20)
        );
    }

    /// **块比行窄的时候必须裁。**
    ///
    /// 语料造不出这个情形——块的边界今天也是按 LF 划的，所以一行必然落在一个
    /// 块里，删掉那个交集全部用例照样绿。理由写在 [`context_range`] 上：
    /// 「块的边界还没合并」那道闸门一旦打开就不再成立。不裁的后果不是画错，
    /// 是**静默地不剥**：请求跨出块，那一行悄悄带回语法标记。所以这里手造
    /// 一条：一行完整，块只覆盖它的后半截。
    #[test]
    fn context_is_clipped_into_the_block() {
        let snapshot = TextBuffer::new("abcdefghij").snapshot();
        assert_eq!(
            context_range(&snapshot, range(4, 6), range(3, 7)).expect("in bounds"),
            range(3, 7)
        );
    }

    /// 块与行完全不相交时给一个空区间，不给一个逆序的区间。
    #[test]
    fn a_block_disjoint_from_the_line_yields_an_empty_context() {
        let snapshot = TextBuffer::new("abc\ndef\n").snapshot();
        let context = context_range(&snapshot, range(0, 1), range(4, 8)).expect("in bounds");
        assert!(context.is_empty());
    }
}
