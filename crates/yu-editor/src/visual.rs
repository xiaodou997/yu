//! 装饰应用之后的视觉字节流，以及它与源码之间的双向映射。
//!
//! # 这里不是第二个映射实现
//!
//! 不变量 D4 说 `DecorationSet` 是投影映射链的**唯一**实现。这个文件不重新
//! 算一遍隐藏了多少字节，它做的是 `DecorationSet` 按定义做不了的三件事：
//!
//! 1. **换原点。** 装饰集合的视觉偏移是**整篇文档**的（它的 `source_len`
//!    就是文档长度），而 `BlockLayout` 排的是**一个块**，视觉文本从 0 开始。
//!    两者差一个常量：这段 source 起点的视觉偏移。
//! 2. **拿出文本。** 装饰集合不持有源码（它是一组区间加一个 Revision），
//!    所以「视觉文本长什么样」只能由持有 `TextSnapshot` 的这一层拼。
//! 3. **叠 composition。** IME 的 preedit 往视觉文本里**插入**一段不在
//!    source 里的文字。第 5.1 节的四种 `Decoration` 都表达不了插入，不变量
//!    H1 也说它是 transient overlay、不进 canonical source——所以它不是一批
//!    装饰，而是薄薄一层平移，叠在规范映射上面。
//!
//! # 边界校验留在这一层
//!
//! `DecorationSet` 不持有源码，回答不了「这个偏移是不是字符边界」，它的文档
//! 明说了这一点。这里持有 `TextSnapshot`，所以校验归这里
//! （`docs/specs/coordinates.md`：不得静默取整）。

use std::sync::Arc;
use std::{error::Error, fmt};

use yu_core::{Affinity, ByteOffset, Revision, TextAnchor, TextRange, VisualOffset, VisualRange};
use yu_decoration::{Bias, DecorationSet};
use yu_text::{ChangeSet, TextSnapshot};

/// 视觉字节流出错的几种方式。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum VisualTextError {
    /// 源码偏移落在这段投影之外。
    SourceOutsideRange {
        offset: ByteOffset,
        range: TextRange,
    },
    /// 源码偏移落在一个多字节字符中间。
    SourceNotCharBoundary {
        offset: ByteOffset,
    },
    /// 视觉偏移越过了视觉文本的末尾。
    VisualOutOfBounds {
        offset: VisualOffset,
        len: VisualOffset,
    },
    /// preedit 要替换的 canonical 区间不在这段投影里。
    CompositionOutsideRange {
        range: TextRange,
        visual: TextRange,
    },
    /// preedit 内部的选中越出了 preedit 文本。
    CompositionSelectionOutOfBounds {
        range: TextRange,
        text_len: ByteOffset,
    },
    /// preedit 内部的选中落在字符中间。
    CompositionSelectionNotUtf8Boundary {
        offset: ByteOffset,
    },
    /// 已经有一层 preedit 了。第二层要从规范态重建，不能叠上去。
    CompositionAlreadyActive,
    /// 读源码失败。跨层只带走说明文字，理由同 `LayoutError::Upstream`。
    Source(String),
    OffsetOverflow,
}

impl fmt::Display for VisualTextError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::SourceOutsideRange { offset, range } => {
                write!(formatter, "源码偏移 {offset:?} 落在投影区间 {range:?} 之外")
            }
            Self::SourceNotCharBoundary { offset } => {
                write!(formatter, "源码偏移 {offset:?} 不在字符边界上")
            }
            Self::VisualOutOfBounds { offset, len } => {
                write!(formatter, "视觉偏移 {offset:?} 越过了视觉长度 {len:?}")
            }
            Self::CompositionOutsideRange { range, visual } => write!(
                formatter,
                "preedit 替换区间 {range:?} 不在投影区间 {visual:?} 里"
            ),
            Self::CompositionSelectionOutOfBounds { range, text_len } => write!(
                formatter,
                "preedit 选中 {range:?} 越过了 preedit 长度 {text_len:?}"
            ),
            Self::CompositionSelectionNotUtf8Boundary { offset } => {
                write!(formatter, "preedit 选中的 {offset:?} 不在字符边界上")
            }
            Self::CompositionAlreadyActive => {
                formatter.write_str("这份视觉文本上已经叠了一层 preedit")
            }
            Self::Source(message) => formatter.write_str(message),
            Self::OffsetOverflow => formatter.write_str("视觉偏移溢出"),
        }
    }
}

impl Error for VisualTextError {}

/// 一次进行中的 IME preedit，落在某一段投影里。
#[derive(Clone, Debug, PartialEq, Eq)]
struct ActiveComposition {
    /// 被替换掉的 canonical 源码区间。
    replacement: TextRange,
    text: Arc<str>,
    /// preedit 文本内部的选中，相对 `text` 的字节偏移。
    selection_bytes: TextRange,
    /// preedit 在**局部**视觉文本里占的区间。
    visual: VisualRange,
}

/// 一段源码投影之后的视觉字节流。
///
/// 「一段」通常是一个 Markdown 块，也可以是整篇文档——两种用法共用这一份
/// 实现，映射的算术只写了一遍。
#[derive(Clone)]
pub struct VisualText {
    source: TextSnapshot,
    range: TextRange,
    set: DecorationSet,
    /// `range.start()` 的**全局**视觉偏移。局部偏移都是全局减去它。
    base: VisualOffset,
    text: String,
    composition: Option<ActiveComposition>,
}

impl fmt::Debug for VisualText {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("VisualText")
            .field("revision", &self.revision())
            .field("range", &self.range)
            .field("text", &self.text)
            .field("composition", &self.composition)
            .finish()
    }
}

impl VisualText {
    /// 按一份装饰集合投影 `range` 这一段源码。
    ///
    /// `set` 的视觉坐标是整篇文档的；`range` 之外的装饰会影响 `base`，
    /// 但不进视觉文本。
    ///
    /// # Errors
    ///
    /// 源码读不出来，或视觉长度溢出。
    pub fn new(
        source: &TextSnapshot,
        range: TextRange,
        set: DecorationSet,
    ) -> Result<Self, VisualTextError> {
        let base = set.source_to_visual(range.start());
        let text = read_visible(source, range, &set)?;
        Ok(Self {
            source: source.clone(),
            range,
            set,
            base,
            text,
            composition: None,
        })
    }

    /// 把一段 preedit 叠在这份投影上。
    ///
    /// 源码快照与 Revision 都不变（不变量 H1）：preedit 只改视觉文本。
    ///
    /// # Errors
    ///
    /// 替换区间不在这段投影里、落在字符中间，或 preedit 内部的选中越界。
    pub fn with_composition(
        &self,
        replacement: TextRange,
        text: impl Into<Arc<str>>,
        selection_bytes: TextRange,
    ) -> Result<Self, VisualTextError> {
        if self.composition.is_some() {
            // 叠第二层 preedit 会拿 canonical 的偏移去切**已经叠过**的文本
            // ——切出来的位置是错的，而且不报错。调用方应当从规范态重建。
            return Err(VisualTextError::CompositionAlreadyActive);
        }
        if replacement.start() < self.range.start() || replacement.end() > self.range.end() {
            return Err(VisualTextError::CompositionOutsideRange {
                range: replacement,
                visual: self.range,
            });
        }
        self.validate_source(replacement.start())?;
        self.validate_source(replacement.end())?;
        let preedit = text.into();
        validate_selection(preedit.as_ref(), selection_bytes)?;

        let start = self.canonical_visual(replacement.start());
        let end = self.canonical_visual(replacement.end());
        let (from, to) = (
            usize::try_from(start.get()).map_err(|_| VisualTextError::OffsetOverflow)?,
            usize::try_from(end.get()).map_err(|_| VisualTextError::OffsetOverflow)?,
        );
        let (head, tail) = (
            self.text
                .get(..from)
                .ok_or(VisualTextError::OffsetOverflow)?,
            self.text.get(to..).ok_or(VisualTextError::OffsetOverflow)?,
        );
        let mut composed = String::with_capacity(head.len() + preedit.len() + tail.len());
        composed.push_str(head);
        composed.push_str(preedit.as_ref());
        composed.push_str(tail);
        let visual_end = start
            .checked_add(preedit.len() as u64)
            .ok_or(VisualTextError::OffsetOverflow)?;

        Ok(Self {
            source: self.source.clone(),
            range: self.range,
            set: self.set.clone(),
            base: self.base,
            text: composed,
            composition: Some(ActiveComposition {
                replacement,
                text: preedit,
                selection_bytes,
                visual: VisualRange::new(start, visual_end)
                    .ok_or(VisualTextError::OffsetOverflow)?,
            }),
        })
    }

    #[must_use]
    pub fn revision(&self) -> Revision {
        self.source.revision()
    }

    /// 这份投影所依据的源码快照。
    #[must_use]
    pub const fn source(&self) -> &TextSnapshot {
        &self.source
    }

    #[must_use]
    pub const fn source_range(&self) -> TextRange {
        self.range
    }

    /// 规范态的装饰集合。composition 不在里面——它不是装饰。
    #[must_use]
    pub const fn decorations(&self) -> &DecorationSet {
        &self.set
    }

    /// 投影之后的视觉文本。preedit 已经叠进去了。
    #[must_use]
    pub fn text(&self) -> &str {
        &self.text
    }

    #[must_use]
    pub fn visual_len(&self) -> VisualOffset {
        VisualOffset::new(self.text.len() as u64)
    }

    /// 源码边界 → 视觉边界。
    ///
    /// 隐藏区间的视觉宽度是零，它的「前」与「后」是同一个视觉偏移，所以
    /// 规范态这个方向不需要 bias。`bias` 只在 preedit 里起作用：那一段
    /// 视觉文本不在 source 里，落进去的源码偏移得选一端。
    ///
    /// # Errors
    ///
    /// 偏移落在投影之外或字符中间。
    pub fn source_to_visual(
        &self,
        source: ByteOffset,
        bias: Bias,
    ) -> Result<VisualOffset, VisualTextError> {
        self.validate_source(source)?;
        let Some(composition) = &self.composition else {
            return Ok(self.canonical_visual(source));
        };
        let range = composition.replacement;
        if range.is_empty() {
            if source == range.start() && bias == Bias::After {
                return Ok(composition.visual.end());
            }
            if source >= range.start() {
                return self.shifted(source, composition);
            }
            return Ok(self.canonical_visual(source));
        }
        if source < range.start() {
            return Ok(self.canonical_visual(source));
        }
        if source == range.start() {
            return Ok(composition.visual.start());
        }
        if source < range.end() {
            return Ok(match bias {
                Bias::Before => composition.visual.start(),
                Bias::After => composition.visual.end(),
            });
        }
        if source == range.end() {
            return Ok(composition.visual.end());
        }
        self.shifted(source, composition)
    }

    /// 视觉边界 → 源码边界。
    ///
    /// # Errors
    ///
    /// 偏移越过了视觉文本的末尾。
    pub fn visual_to_source(
        &self,
        visual: VisualOffset,
        bias: Bias,
    ) -> Result<ByteOffset, VisualTextError> {
        if visual > self.visual_len() {
            return Err(VisualTextError::VisualOutOfBounds {
                offset: visual,
                len: self.visual_len(),
            });
        }
        let Some(composition) = &self.composition else {
            return Ok(self.canonical_visual_to_source(visual, bias));
        };
        let span = composition.visual;
        if visual >= span.start() && visual <= span.end() {
            // preedit 内部的每一个视觉边界都指回同一段 canonical 替换范围
            // ——那段文字根本不在 source 里，只能报它两端之一。
            if visual == span.start() {
                return Ok(composition.replacement.start());
            }
            if visual == span.end() {
                return Ok(composition.replacement.end());
            }
            return Ok(match bias {
                Bias::Before => composition.replacement.start(),
                Bias::After => composition.replacement.end(),
            });
        }
        if visual < span.start() {
            return Ok(self.canonical_visual_to_source(visual, bias));
        }
        let canonical = offset_by(visual, -self.composition_delta(composition))?;
        Ok(self.canonical_visual_to_source(canonical, bias))
    }

    /// 不含 preedit 的那一份 source → visual 映射。
    ///
    /// 装配样式段要用它：段落是 canonical 源码上的区间，得先在 canonical
    /// 视觉空间里排好，再整体让 preedit 挪一次。混用两个空间会让 preedit
    /// 旁边那一段文字排错字型——不报错，只是画得不对。
    #[must_use]
    pub fn canonical_source_to_visual(&self, source: ByteOffset) -> VisualOffset {
        self.canonical_visual(source)
    }

    /// preedit 在视觉文本里占的区间。
    #[must_use]
    pub fn composition_visual(&self) -> Option<VisualRange> {
        self.composition
            .as_ref()
            .map(|composition| composition.visual)
    }

    /// 进行中的 preedit 文本。
    #[must_use]
    pub fn composition_text(&self) -> Option<&str> {
        self.composition
            .as_ref()
            .map(|composition| composition.text.as_ref())
    }

    /// preedit 替换掉的 canonical 源码区间。
    #[must_use]
    pub fn composition_range(&self) -> Option<TextRange> {
        self.composition
            .as_ref()
            .map(|composition| composition.replacement)
    }

    /// preedit 内部的选中，相对 preedit 文本。
    #[must_use]
    pub fn composition_selection_bytes(&self) -> Option<TextRange> {
        self.composition
            .as_ref()
            .map(|composition| composition.selection_bytes)
    }

    /// preedit 内部的选中，换算成视觉偏移。
    #[must_use]
    pub fn composition_selection_visual(&self) -> Option<VisualRange> {
        let composition = self.composition.as_ref()?;
        let start = composition
            .visual
            .start()
            .checked_add(composition.selection_bytes.start().get())?;
        let end = composition
            .visual
            .start()
            .checked_add(composition.selection_bytes.end().get())?;
        VisualRange::new(start, end)
    }

    /// 一次源码编辑之后这份投影还在不在，以及它整体挪了多少字节。
    ///
    /// `None` 表示编辑碰到了这段区间或它的边界，调用方必须重新产装饰。
    /// 判据与 v1 一致：**相接也算碰到**——紧贴块首插入的字符会改变块的语法
    /// 归属，沿用旧装饰的后果是「多打一个 `#` 但标题级别没变」。
    ///
    /// 编辑落在区间之外时，区间里每一个偏移都在每一处改动的同一侧，所以
    /// 平移量是个常量。返回这个常量而不是一份迁移好的投影：装饰、行级装饰、
    /// 语义标注要一起挪，让它们各自问一遍锚点就有几十份同样的算术。
    ///
    /// # Errors
    ///
    /// Revision 对不上，或锚点迁移失败。
    pub fn shift_through(&self, changes: &ChangeSet) -> Result<Option<i64>, VisualTextError> {
        if self.revision() != changes.before() {
            return Err(VisualTextError::Source(
                "视觉文本与变更集的 Revision 对不上".to_owned(),
            ));
        }
        // preedit 是 transient 的，编辑一落地它就该重建。
        if self.composition.is_some() {
            return Ok(None);
        }
        shift_for(self.range, changes)
    }

    fn shifted(
        &self,
        source: ByteOffset,
        composition: &ActiveComposition,
    ) -> Result<VisualOffset, VisualTextError> {
        offset_by(
            self.canonical_visual(source),
            self.composition_delta(composition),
        )
    }

    /// preedit 让替换点之后的视觉偏移整体挪了多少。
    ///
    /// **可以是负数**：把三个字替换成一个字符的 preedit 时，后面的文字往前
    /// 挪。用无符号数算这一步会在那种情况下饱和到 0——不 panic、不报错，
    /// 只是 preedit 之后的每一个光标位置都差几个字节。
    fn composition_delta(&self, composition: &ActiveComposition) -> i128 {
        let old_start = self.canonical_visual(composition.replacement.start());
        let old_end = self.canonical_visual(composition.replacement.end());
        let old_len = i128::from(old_end.get()) - i128::from(old_start.get());
        let new_len = i128::from(composition.visual.end().get())
            - i128::from(composition.visual.start().get());
        new_len - old_len
    }

    fn canonical_visual(&self, source: ByteOffset) -> VisualOffset {
        let clamped = source.max(self.range.start()).min(self.range.end());
        VisualOffset::new(
            self.set
                .source_to_visual(clamped)
                .get()
                .saturating_sub(self.base.get()),
        )
    }

    fn canonical_visual_to_source(&self, visual: VisualOffset, bias: Bias) -> ByteOffset {
        let global = VisualOffset::new(visual.get().saturating_add(self.base.get()));
        self.set
            .visual_to_source(global, bias)
            .max(self.range.start())
            .min(self.range.end())
    }

    fn validate_source(&self, source: ByteOffset) -> Result<(), VisualTextError> {
        if source < self.range.start() || source > self.range.end() {
            return Err(VisualTextError::SourceOutsideRange {
                offset: source,
                range: self.range,
            });
        }
        self.source
            .utf16_offset(source)
            .map_err(|_| VisualTextError::SourceNotCharBoundary { offset: source })?;
        Ok(())
    }
}

/// 一个视觉偏移加上一个可正可负的平移量。
fn offset_by(offset: VisualOffset, delta: i128) -> Result<VisualOffset, VisualTextError> {
    u64::try_from(i128::from(offset.get()) + delta)
        .map(VisualOffset::new)
        .map_err(|_| VisualTextError::OffsetOverflow)
}

/// 一次编辑碰没碰到这段区间。**相接也算碰到。**
fn touches(old: TextRange, range: TextRange) -> bool {
    old.start() <= range.end() && old.end() >= range.start()
}

/// 一次编辑之后 `range` 还在不在，以及它整体挪了多少字节。
///
/// `None` 表示编辑碰到了这段区间或它的边界。判据与 v1 一致：**相接也算
/// 碰到**——紧贴块首插入的字符会改变块的语法归属。
///
/// # Errors
///
/// 锚点迁移失败，或平移量溢出。
pub fn shift_for(range: TextRange, changes: &ChangeSet) -> Result<Option<i64>, VisualTextError> {
    if changes
        .changes()
        .iter()
        .any(|change| touches(change.old_range(), range))
    {
        return Ok(None);
    }
    let moved = changes
        .map_anchor(TextAnchor::new(
            changes.before(),
            range.start(),
            Affinity::Before,
        ))
        .map_err(|error| VisualTextError::Source(error.to_string()))?
        .offset();
    let delta = i64::try_from(moved.get())
        .ok()
        .zip(i64::try_from(range.start().get()).ok())
        .and_then(|(new, old)| new.checked_sub(old))
        .ok_or(VisualTextError::OffsetOverflow)?;
    Ok(Some(delta))
}

fn validate_selection(text: &str, selection: TextRange) -> Result<(), VisualTextError> {
    let text_len = ByteOffset::new(text.len() as u64);
    if selection.end() > text_len {
        return Err(VisualTextError::CompositionSelectionOutOfBounds {
            range: selection,
            text_len,
        });
    }
    for offset in [selection.start(), selection.end()] {
        let index = usize::try_from(offset).map_err(|_| VisualTextError::OffsetOverflow)?;
        if !text.is_char_boundary(index) {
            return Err(VisualTextError::CompositionSelectionNotUtf8Boundary { offset });
        }
    }
    Ok(())
}

/// `range` 里没被隐藏的那些字节，按源码顺序拼起来。
///
/// 隐藏区间**从装饰集合要**（[`DecorationSet::hidden_spans`]），不自己遍历
/// 一遍装饰去数。那份数据正是 `source_to_visual` 那棵树的原料，所以「哪些
/// 字节被藏了」在这条路上仍然只有一个答案（不变量 D4）。自己再数一遍会得到
/// 第二个实现——哪怕今天结果一样，它会在下一次改动时分叉，而分叉的表现是
/// 画面比光标少几个字。
pub(crate) fn read_visible(
    source: &TextSnapshot,
    range: TextRange,
    set: &DecorationSet,
) -> Result<String, VisualTextError> {
    let mut text = String::new();
    let mut cursor = range.start();
    for &(from, to) in set.hidden_spans() {
        if to <= cursor {
            continue;
        }
        if from >= range.end() {
            break;
        }
        if from > cursor
            && let Some(visible) = TextRange::new(cursor, from.min(range.end()))
        {
            push_source(source, visible, &mut text)?;
        }
        cursor = cursor.max(to);
        if cursor >= range.end() {
            return Ok(text);
        }
    }
    if let Some(visible) = TextRange::new(cursor, range.end()) {
        push_source(source, visible, &mut text)?;
    }
    Ok(text)
}

/// 把一段源码追加到 `text`。
///
/// 按 chunk 走而不是 `as_str()[a..b]`：源码住在 rope 里，整篇物化一遍只为
/// 取一个块是 O(文档长度)（不变量 E4 也不让 rope 的索引跑出 `yu-text`）。
fn push_source(
    source: &TextSnapshot,
    range: TextRange,
    text: &mut String,
) -> Result<(), VisualTextError> {
    let start = usize::try_from(range.start()).map_err(|_| VisualTextError::OffsetOverflow)?;
    let end = usize::try_from(range.end()).map_err(|_| VisualTextError::OffsetOverflow)?;
    text.reserve(end.saturating_sub(start));
    let chunks = source
        .chunk_cursor(range.start())
        .map_err(|error| VisualTextError::Source(error.to_string()))?;
    for chunk in chunks {
        let chunk_start =
            usize::try_from(chunk.start()).map_err(|_| VisualTextError::OffsetOverflow)?;
        if chunk_start >= end {
            break;
        }
        let chunk_end = chunk_start.saturating_add(chunk.text().len());
        let local_start = start.max(chunk_start).saturating_sub(chunk_start);
        let local_end = end.min(chunk_end).saturating_sub(chunk_start);
        if local_start < local_end {
            text.push_str(&chunk.text()[local_start..local_end]);
        }
    }
    Ok(())
}
