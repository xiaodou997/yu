//! 文档搜索：一份查询在这一版源码上的全部匹配。
//!
//! # 为什么它不是装饰
//!
//! 上一刀留下的预判是「搜索与引用表（S6 第十三刀）是同一个形状，缓存要按
//! 查询失效」。**查下去不是。**
//!
//! - 三张装饰表里没有一张能表达「一段文字底下画一块颜色」：
//!   [`yu_core::TextAttrs`] 只有字型与字号倍率，`BlockOrnament` 与
//!   `BlockWidget` 说的是别的事。
//! - 而选区早就有答案，且**不在装饰里**：`yu-workspace` 从一个源码区间加一份
//!   `BlockLayout` 直接产出 `EditorDecorationPrimitive`，场景层画矩形，
//!   `DecorationCache` 与 `DecorationSet` 全程不参与。
//!
//! 所以搜索高亮走选区那条路，**`DecorationCache` 一个字节都不用清**。
//!
//! 两者到底哪里像、哪里不像：引用表改变的是**块的语义**（`[a][b]` 到底是不是
//! 链接、要不要藏定界符），所以必须清装饰；查询改变的只是**画在文字底下的
//! 矩形**，不改任何块的语义、不藏任何 source。两者都是「文档级状态 + 按块
//! 缓存」，但只有前者会让装饰翻面。
//!
//! 反过来说：哪一天搜索要改变文字本身（隐藏不匹配的行、折叠），那时它才变成
//! 装饰问题，那时才需要指纹。现在不做。
//!
//! # 「当前匹配」不存在这里
//!
//! 一个搜索状态最自然的写法是 `{ query, matches, current: usize }`。这里
//! **没有** `current`：它由选区推出来（[`SearchState::current`]）。
//!
//! 理由是这样就只有一份真相。存一个下标，它与选区就是两个可以对不上的答案：
//! 用户在文档里点一下、撤销一次编辑、或者别的路径动了选区，下标都不会跟着
//! 变，于是高亮的「当前」停在别处——不报错，只是指错了地方。而且「跳到下一个」
//! 本来就要走已有的选区入口（导航只能有一个实现），选区因此**必然**是被更新
//! 的那一份。
//!
//! 顺带白拿一件事：帧身份已经把选区算在内，所以「当前匹配换了一个」自动让
//! 帧失效，不用再加一项。
//! # Matching options
//!
//! Literal matching defaults to case-sensitive substring search for compatibility.
//! Caseless search uses Unicode default case folding with an explicit source-byte
//! boundary map: expansions such as ß → ss must never report half a source scalar.
//! Whole-word matching requires both source endpoints to be UAX #29 boundaries.
//! This is not dictionary-based CJK segmentation or accent/width normalization.

use caseless::Caseless;
use unicode_segmentation::UnicodeSegmentation;
use yu_core::{Revision, TextRange};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SearchOptions {
    pub match_case: bool,
    pub whole_words: bool,
}

impl Default for SearchOptions {
    fn default() -> Self {
        Self {
            match_case: true,
            whole_words: false,
        }
    }
}
use yu_text::TextSnapshot;

/// 一份查询在一版源码上的全部匹配。
///
/// 它是 `(TextSnapshot, query)` 的纯函数，跟着 Revision 失效。匹配不重叠：
/// 一次命中之后从它的末尾继续找，所以 `aa` 在 `aaa` 里有一个匹配，不是两个。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SearchState {
    revision: Revision,
    query: String,
    options: SearchOptions,
    matches: Vec<TextRange>,
}

impl SearchState {
    /// 扫一遍源码。空查询没有匹配。
    #[must_use]
    pub fn new(snapshot: &TextSnapshot, query: impl Into<String>) -> Self {
        Self::with_options(snapshot, query, SearchOptions::default())
    }

    #[must_use]
    pub fn with_options(
        snapshot: &TextSnapshot,
        query: impl Into<String>,
        options: SearchOptions,
    ) -> Self {
        let query = query.into();
        let matches = find_matches(snapshot.as_str(), &query, options);
        Self {
            revision: snapshot.revision(),
            query,
            options,
            matches,
        }
    }

    #[must_use]
    pub const fn options(&self) -> SearchOptions {
        self.options
    }

    #[must_use]
    pub const fn revision(&self) -> Revision {
        self.revision
    }

    #[must_use]
    pub fn query(&self) -> &str {
        &self.query
    }

    /// 全部匹配，按文档顺序，互不重叠。
    #[must_use]
    pub fn matches(&self) -> &[TextRange] {
        &self.matches
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.matches.is_empty()
    }

    /// 「当前匹配」：选区**恰好**落在哪一个匹配上。
    ///
    /// 恰好相等，不是相交——相交会让一次「全选」把每个匹配都变成当前。跳到
    /// 下一个匹配的做法是把选区设成那一段，所以相等这个条件由构造成立。
    #[must_use]
    pub fn current(&self, selection: TextRange) -> Option<usize> {
        if selection.is_empty() {
            return None;
        }
        self.matches
            .binary_search_by(|candidate| candidate.start().cmp(&selection.start()))
            .ok()
            .filter(|index| self.matches[*index] == selection)
    }
}

fn find_matches(source: &str, query: &str, options: SearchOptions) -> Vec<TextRange> {
    if query.is_empty() {
        return Vec::new();
    }
    let words = options.whole_words.then(|| {
        source
            .split_word_bound_indices()
            .map(|(offset, _)| offset)
            .chain(std::iter::once(source.len()))
            .collect::<Vec<_>>()
    });
    let mut folded = String::new();
    let mut boundaries = Vec::new();
    let wanted;
    let (text, needle) = if options.match_case {
        (source, query)
    } else {
        for (offset, scalar) in source.char_indices() {
            boundaries.push((folded.len(), offset));
            folded.extend(std::iter::once(scalar).default_case_fold());
        }
        boundaries.push((folded.len(), source.len()));
        wanted = query.chars().default_case_fold().collect::<String>();
        (folded.as_str(), wanted.as_str())
    };
    if needle.is_empty() {
        return Vec::new();
    }
    let source_offset = |offset| {
        if options.match_case {
            Some(offset)
        } else {
            boundaries
                .binary_search_by_key(&offset, |&(index, _)| index)
                .ok()
                .map(|index| boundaries[index].1)
        }
    };
    let mut matches = Vec::new();
    let mut cursor = 0;
    while let Some(relative) = text[cursor..].find(needle) {
        let start = cursor + relative;
        let end = start + needle.len();
        let mapped = source_offset(start)
            .zip(source_offset(end))
            .filter(|&(from, to)| {
                words.as_ref().is_none_or(|bounds| {
                    bounds.binary_search(&from).is_ok() && bounds.binary_search(&to).is_ok()
                })
            });
        if let Some((from, to)) = mapped {
            matches.push(
                TextRange::new(
                    yu_core::ByteOffset::new(from as u64),
                    yu_core::ByteOffset::new(to as u64),
                )
                .expect("ordered match"),
            );
            cursor = end;
        } else {
            // Rejected candidates may overlap a later valid match (sß / ss).
            // Advancing to `end` here would silently lose that valid match.
            cursor = start
                + text[start..]
                    .chars()
                    .next()
                    .expect("nonempty needle")
                    .len_utf8();
        }
    }
    matches
}

#[cfg(test)]
mod tests {
    use super::*;
    use yu_core::ByteOffset;
    use yu_text::TextBuffer;

    fn snapshot(source: &str) -> TextSnapshot {
        TextBuffer::new(source).snapshot()
    }

    fn range(start: u64, end: u64) -> TextRange {
        TextRange::new(ByteOffset::new(start), ByteOffset::new(end)).expect("ordered")
    }

    #[test]
    fn unicode_folding_preserves_source_boundaries_and_expansions() {
        let options = SearchOptions {
            match_case: false,
            whole_words: false,
        };
        let text = snapshot("🙂Straße STRASSE σςΣ İ sß");
        let hits = SearchState::with_options(&text, "strasse", options);
        assert_eq!(hits.matches(), &[range(4, 11), range(12, 19)]);
        assert_eq!(
            SearchState::with_options(&snapshot("σςΣ"), "Σ", options)
                .matches()
                .len(),
            3
        );
        assert!(SearchState::with_options(&snapshot("İ"), "i", options).is_empty());
        assert!(SearchState::with_options(&snapshot("ß"), "s", options).is_empty());
        assert_eq!(
            SearchState::with_options(&snapshot("sß"), "ss", options).matches(),
            &[range(1, 3)]
        );
        assert_eq!(
            SearchState::with_options(&snapshot("İ"), "i\u{307}", options).matches(),
            &[range(0, 2)]
        );
    }

    #[test]
    fn whole_words_observe_unicode_boundaries_not_ascii_neighbors() {
        let options = SearchOptions {
            match_case: false,
            whole_words: true,
        };
        let source = "Yu Yule _Yu Yu_ (YU) don't don e\u{301} e";
        assert_eq!(
            SearchState::with_options(&snapshot(source), "yu", options).matches(),
            &[range(0, 2), range(17, 19)]
        );
        assert_eq!(
            SearchState::with_options(&snapshot(source), "don", options)
                .matches()
                .len(),
            1
        );
        assert_eq!(
            SearchState::with_options(&snapshot("e\u{301} e"), "e", options).matches(),
            &[range(4, 5)]
        );
        assert_eq!(
            SearchState::with_options(&snapshot("xa a a"), "a a", options).matches(),
            &[range(3, 6)]
        );
        assert_eq!(
            SearchState::with_options(&snapshot("中文中文"), "中文", options)
                .matches()
                .len(),
            2
        );
    }

    #[test]
    fn options_survive_replace_undo_redo_and_source_reset() {
        let mut document = crate::EditorDocument::new("Yu YU Yule");
        let options = SearchOptions {
            match_case: false,
            whole_words: true,
        };
        let revision = document.revision();
        document.set_search_query_with_options("yu", options);
        assert_eq!(document.revision(), revision);
        assert_eq!(document.search().expect("search").matches().len(), 2);
        document.replace_search("羽", true).expect("replace");
        assert_eq!(document.snapshot().as_str(), "羽 羽 Yule");
        document.undo().expect("undo");
        assert_eq!(document.search().expect("search").options(), options);
        assert_eq!(document.search().expect("search").matches().len(), 2);
        document.redo().expect("redo");
        assert!(document.search().expect("search").is_empty());
        document.reset_source("yu Yu Yule").expect("reset");
        assert_eq!(document.search().expect("search").matches().len(), 2);
        let generation = document.search_generation();
        document.set_search_query_with_options("yu", SearchOptions::default());
        assert_ne!(document.search_generation(), generation);
        assert_eq!(document.search().expect("search").matches().len(), 1);
    }

    #[test]
    fn empty_query_matches_nothing() {
        let state = SearchState::new(&snapshot("abc"), "");
        assert!(state.is_empty());
        assert_eq!(state.matches(), &[]);
    }

    #[test]
    fn matches_are_ordered_and_do_not_overlap() {
        let state = SearchState::new(&snapshot("aaaa"), "aa");
        assert_eq!(state.matches(), &[range(0, 2), range(2, 4)]);
    }

    /// 偏移是字节，且必须落在字符边界上——非 BMP 字符前后各放一个匹配。
    #[test]
    fn matches_report_byte_offsets_around_multibyte_text() {
        let state = SearchState::new(&snapshot("x🙂x"), "x");
        assert_eq!(state.matches(), &[range(0, 1), range(5, 6)]);
    }

    #[test]
    fn matching_is_case_sensitive() {
        let state = SearchState::new(&snapshot("Yu yu YU"), "yu");
        assert_eq!(state.matches(), &[range(3, 5)]);
    }

    #[test]
    fn current_is_the_match_the_selection_sits_exactly_on() {
        let state = SearchState::new(&snapshot("aXbXc"), "X");
        assert_eq!(state.current(range(1, 2)), Some(0));
        assert_eq!(state.current(range(3, 4)), Some(1));
        // 起点对上、长度不对：那不是这一个匹配。
        assert_eq!(state.current(range(1, 3)), None);
        // 空选区（一个光标）不是任何匹配。
        assert_eq!(state.current(TextRange::empty(ByteOffset::new(1))), None);
        // 覆盖全部匹配的选区也不是「当前」，否则全选会点亮每一个。
        assert_eq!(state.current(range(0, 5)), None);
    }
}
