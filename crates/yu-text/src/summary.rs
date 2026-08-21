use yu_core::{ByteOffset, Utf16Offset};

/// Additive source metrics carried by the text tree.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TextSummary {
    bytes: u64,
    utf16_units: u64,
    line_breaks: u64,
}

impl TextSummary {
    /// 由 rope 已经维护好的累计量直接构造。
    ///
    /// 这些量常驻在树根与节点上，取用是 O(1)/O(log n)；语义必须与
    /// `from_text` 对同一段文本的结果逐字段相等，
    /// `byte_utf16_and_line_queries_match_string_model` 守护这一点。
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
