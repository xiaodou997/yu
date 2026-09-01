//! UTF-16 code unit 下标 → 文本内的 UTF-8 字节偏移。
//!
//! 原本住在 `yu-font-macos`：CoreText 的 `CTRunGetStringIndices` 给的是
//! UTF-16 下标，而 [`yu_core::Glyph`] 的 source 是 UTF-8 字节。**这条换算
//! 跟平台无关**——DirectWrite 的 `clusterMap` 同样以 UTF-16 code unit 为索引
//! 单位（S7 第七刀的 spike），第二端会一字不差地需要它。所以它提到了这里。
//!
//! 代理对的低位没有对应的字节边界：那一格是 `None`，问到它就是问错了，
//! 不是回一个最近的边界。**「就近取整」在这里等于把一个字符劈成两半，而它
//! 不报错**——I4 那条「surrogate 中间位置不得穿过 ABI」守的是同一件事。

use yu_core::TextRange;

/// 一段文本的 UTF-16 → UTF-8 边界表。
///
/// 建一次用多次：一次 shaping 里每个 run、每个字形都要问它，逐次现算是
/// O(文本长度) 乘以字形数。
#[derive(Clone, Debug)]
pub struct Utf16Map {
    /// 下标是 UTF-16 code unit 位置，值是对应的 UTF-8 字节偏移。
    /// 代理对低位那一格是 `None`。长度是 UTF-16 长度 + 1。
    boundaries: Vec<Option<usize>>,
}

impl Utf16Map {
    #[must_use]
    pub fn new(text: &str) -> Self {
        let mut boundaries = Vec::with_capacity(text.len() + 1);
        boundaries.push(Some(0_usize));
        let mut byte = 0_usize;
        for character in text.chars() {
            byte = byte.saturating_add(character.len_utf8());
            if character.len_utf16() == 2 {
                boundaries.push(None);
            }
            boundaries.push(Some(byte));
        }
        Self { boundaries }
    }

    /// 这段文本有多少个 UTF-16 code unit。
    #[must_use]
    pub fn len(&self) -> usize {
        self.boundaries.len().saturating_sub(1)
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// 一个 UTF-16 位置对应的字节偏移。越界或落在代理对中间时是 `None`。
    #[must_use]
    pub fn byte_offset(&self, utf16: usize) -> Option<usize> {
        *self.boundaries.get(utf16)?
    }

    /// 把一段 UTF-16 区间搬到以 `source.start()` 为基址的字节区间上。
    ///
    /// `source` 是**这段文本整体**在调用方坐标系里的位置（`ShapingProvider`
    /// 契约的 C8：字形区间是 `source.start()` 加上局部偏移）。
    #[must_use]
    pub fn range(&self, start: usize, end: usize, source: TextRange) -> Option<TextRange> {
        let start = self.byte_offset(start)?;
        let end = self.byte_offset(end)?;
        if start > end || u64::try_from(end).ok()? > source.len() {
            return None;
        }
        let source_start = source.start().checked_add(u64::try_from(start).ok()?)?;
        let source_end = source.start().checked_add(u64::try_from(end).ok()?)?;
        TextRange::new(source_start, source_end)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use yu_core::ByteOffset;

    fn source(len: u64) -> TextRange {
        TextRange::new(ByteOffset::ZERO, ByteOffset::new(len)).expect("有序")
    }

    #[test]
    fn ascii_is_one_to_one() {
        let map = Utf16Map::new("abc");
        assert_eq!(map.len(), 3);
        assert_eq!(map.byte_offset(0), Some(0));
        assert_eq!(map.byte_offset(3), Some(3));
        assert_eq!(map.byte_offset(4), None);
    }

    #[test]
    fn bmp_multibyte_characters_advance_more_bytes_than_units() {
        // 「中」是 3 字节 / 1 个 UTF-16 单位。
        let map = Utf16Map::new("a\u{4e2d}b");
        assert_eq!(map.len(), 3);
        assert_eq!(map.byte_offset(1), Some(1));
        assert_eq!(map.byte_offset(2), Some(4));
        assert_eq!(map.byte_offset(3), Some(5));
    }

    /// 代理对的低位没有字节边界。**回一个最近的边界会把一个字符劈成两半，
    /// 而那不报错**——所以这里必须是 `None`。
    #[test]
    fn the_low_half_of_a_surrogate_pair_has_no_byte_boundary() {
        let map = Utf16Map::new("\u{1f600}");
        assert_eq!(map.len(), 2);
        assert_eq!(map.byte_offset(0), Some(0));
        assert_eq!(map.byte_offset(1), None);
        assert_eq!(map.byte_offset(2), Some(4));
        assert_eq!(map.range(0, 1, source(4)), None);
        assert_eq!(map.range(1, 2, source(4)), None);
    }

    #[test]
    fn ranges_are_rebased_onto_the_requested_source() {
        let map = Utf16Map::new("a\u{4e2d}b");
        let based = TextRange::new(ByteOffset::new(100), ByteOffset::new(105)).expect("有序");
        let range = map.range(1, 2, based).expect("区间");
        assert_eq!(range.start().get(), 101);
        assert_eq!(range.end().get(), 104);
    }

    #[test]
    fn reversed_or_overlong_ranges_are_rejected() {
        let map = Utf16Map::new("abc");
        assert_eq!(map.range(2, 1, source(3)), None);
        assert_eq!(map.range(0, 3, source(2)), None);
        assert_eq!(map.range(0, 9, source(3)), None);
    }

    #[test]
    fn an_empty_text_still_has_a_zero_boundary() {
        let map = Utf16Map::new("");
        assert!(map.is_empty());
        assert_eq!(map.byte_offset(0), Some(0));
        assert_eq!(map.range(0, 0, source(0)).map(|range| range.len()), Some(0));
    }
}
