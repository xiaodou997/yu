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
//!
//! # 区分大小写
//!
//! 这一版是**字面、区分大小写**的子串匹配。不区分大小写要么改变偏移
//! （`to_lowercase` 会让某些字符变长，`İ` → `i̇`），要么逐字符折叠。
//!
//! **这条登记以前挂在 F3 上（「等 F3 接外部依赖时一起做」），S7 第六刀查下来
//! 那个挂法是错的。** F3 关掉了，依赖也接进来了（`caseless`），而这一条一步
//! 都没走近：两者只是碰巧都需要一份 case folding，另一半是反的——
//!
//! | | F3 的引用标签 | 这里 |
//! | --- | --- | --- |
//! | 折出来的东西要不要映射回源码偏移 | **不要**，它只是一个查表键 | **要**，命中要回报 `TextRange` |
//! | 缺的是什么 | 一个依赖 | 一个**给得出对齐信息**的匹配算法 |
//!
//! `caseless::default_case_fold_str` 给不出「折叠后第 k 个字符落在源码第几
//! 个字节」。**新的触发条件就是这件事本身**：有人要不区分大小写的搜索时，
//! 要做的是那个匹配算法，不是再接一个 crate。

use yu_core::{Revision, TextRange};
use yu_text::TextSnapshot;

/// 一份查询在一版源码上的全部匹配。
///
/// 它是 `(TextSnapshot, query)` 的纯函数，跟着 Revision 失效。匹配不重叠：
/// 一次命中之后从它的末尾继续找，所以 `aa` 在 `aaa` 里有一个匹配，不是两个。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SearchState {
    revision: Revision,
    query: String,
    matches: Vec<TextRange>,
}

impl SearchState {
    /// 扫一遍源码。空查询没有匹配。
    #[must_use]
    pub fn new(snapshot: &TextSnapshot, query: impl Into<String>) -> Self {
        let query = query.into();
        let revision = snapshot.revision();
        let matches = if query.is_empty() {
            Vec::new()
        } else {
            snapshot
                .as_str()
                .match_indices(query.as_str())
                .filter_map(|(start, hit)| {
                    let start = yu_core::ByteOffset::new(start as u64);
                    let end = yu_core::ByteOffset::new(start.get() + hit.len() as u64);
                    TextRange::new(start, end)
                })
                .collect()
        };
        Self {
            revision,
            query,
            matches,
        }
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
