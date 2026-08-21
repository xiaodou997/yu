use yu_core::{ByteOffset, Utf16Offset};

/// Additive source metrics stored on persistent text-tree nodes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TextSummary {
    bytes: u64,
    utf16_units: u64,
    line_breaks: u64,
}

impl TextSummary {
    pub const EMPTY: Self = Self {
        bytes: 0,
        utf16_units: 0,
        line_breaks: 0,
    };

    /// 由后端已经维护好的累计量直接构造。
    ///
    /// 用于 summary 本来就是 O(1) 常驻在树根上的存储后端；语义必须与
    /// `from_text` 对同一段文本的结果逐字段相等，跨后端差分测试守护这一点。
    pub(crate) const fn from_parts(bytes: u64, utf16_units: u64, line_breaks: u64) -> Self {
        Self {
            bytes,
            utf16_units,
            line_breaks,
        }
    }

    #[must_use]
    pub fn from_text(text: &str) -> Self {
        Self {
            bytes: u64::try_from(text.len()).unwrap_or(u64::MAX),
            utf16_units: u64::try_from(text.encode_utf16().count()).unwrap_or(u64::MAX),
            line_breaks: u64::try_from(text.bytes().filter(|byte| *byte == b'\n').count())
                .unwrap_or(u64::MAX),
        }
    }

    #[must_use]
    pub const fn bytes(self) -> ByteOffset {
        ByteOffset::new(self.bytes)
    }

    #[must_use]
    pub const fn utf16_units(self) -> Utf16Offset {
        Utf16Offset::new(self.utf16_units)
    }

    #[must_use]
    pub const fn line_breaks(self) -> u64 {
        self.line_breaks
    }

    #[must_use]
    pub const fn line_count(self) -> u64 {
        self.line_breaks.saturating_add(1)
    }

    pub(crate) const fn bytes_usize(self) -> usize {
        self.bytes as usize
    }

    pub(crate) const fn from_char(character: char) -> Self {
        Self {
            bytes: character.len_utf8() as u64,
            utf16_units: character.len_utf16() as u64,
            line_breaks: if character == '\n' { 1 } else { 0 },
        }
    }

    pub(crate) const fn utf16_u64(self) -> u64 {
        self.utf16_units
    }

    pub(crate) const fn plus(self, other: Self) -> Self {
        Self {
            bytes: self.bytes + other.bytes,
            utf16_units: self.utf16_units + other.utf16_units,
            line_breaks: self.line_breaks + other.line_breaks,
        }
    }

    pub(crate) const fn minus(self, other: Self) -> Self {
        Self {
            bytes: self.bytes - other.bytes,
            utf16_units: self.utf16_units - other.utf16_units,
            line_breaks: self.line_breaks - other.line_breaks,
        }
    }
}

pub(crate) fn byte_offset_for_utf16(text: &str, target: u64) -> Option<usize> {
    let mut utf16 = 0_u64;
    for (byte, character) in text.char_indices() {
        if utf16 == target {
            return Some(byte);
        }
        let next = utf16 + character.len_utf16() as u64;
        if target < next {
            return None;
        }
        utf16 = next;
    }
    (utf16 == target).then_some(text.len())
}

pub(crate) fn byte_after_line_break(text: &str, target: u64) -> Option<usize> {
    if target == 0 {
        return Some(0);
    }
    let mut line_breaks = 0_u64;
    for (byte, value) in text.bytes().enumerate() {
        if value == b'\n' {
            line_breaks += 1;
            if line_breaks == target {
                return Some(byte + 1);
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn summary_distinguishes_utf8_and_utf16_lengths() {
        let summary = TextSummary::from_text("羽🙂\r\n");

        assert_eq!(summary.bytes().get(), 9);
        assert_eq!(summary.utf16_units().get(), 5);
        assert_eq!(summary.line_breaks(), 1);
        assert_eq!(summary.line_count(), 2);
    }
}
