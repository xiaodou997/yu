use std::ops::Range;

/// The source prefix of a Markdown list line. Offsets are byte offsets into
/// the line content (which excludes its line terminator).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ListLinePrefix {
    pub(crate) list_marker_end: usize,
    pub(crate) content_start: usize,
    pub(crate) task_marker: Option<Range<usize>>,
    ordered_digits: Option<Range<usize>>,
}

impl ListLinePrefix {
    /// Parses the conservative list prefix used by the Phase 1 block parser.
    /// The parser intentionally accepts only ASCII spaces before the marker;
    /// tabs and non-ASCII whitespace remain ordinary source text.
    pub(crate) fn parse(content: &str) -> Option<Self> {
        let bytes = content.as_bytes();
        let mut cursor = 0_usize;
        while cursor < bytes.len() && bytes[cursor] == b' ' {
            cursor += 1;
        }
        let digits_start = cursor;
        let ordered_digits = if bytes.get(cursor).is_some_and(u8::is_ascii_digit) {
            while cursor < bytes.len() && bytes[cursor].is_ascii_digit() {
                cursor += 1;
            }
            let digits_end = cursor;
            if !matches!(bytes.get(cursor), Some(b'.' | b')')) {
                return None;
            }
            cursor += 1;
            Some(digits_start..digits_end)
        } else if matches!(bytes.get(cursor), Some(b'-' | b'+' | b'*')) {
            cursor += 1;
            None
        } else {
            return None;
        };
        let list_marker_end = cursor;
        if !matches!(bytes.get(cursor), Some(b' ' | b'\t')) {
            return None;
        }
        while matches!(bytes.get(cursor), Some(b' ' | b'\t')) {
            cursor += 1;
        }

        let task_marker = if cursor.checked_add(2)? < bytes.len()
            && bytes[cursor] == b'['
            && matches!(bytes[cursor + 1], b' ' | b'x' | b'X')
            && bytes[cursor + 2] == b']'
            && (cursor + 3 == bytes.len() || matches!(bytes[cursor + 3], b' ' | b'\t'))
        {
            let marker = cursor..cursor + 3;
            cursor += 3;
            while matches!(bytes.get(cursor), Some(b' ' | b'\t')) {
                cursor += 1;
            }
            Some(marker)
        } else {
            None
        };

        Some(Self {
            list_marker_end,
            content_start: cursor,
            task_marker,
            ordered_digits,
        })
    }

    pub(crate) fn is_empty_item(&self, content: &str) -> bool {
        content
            .get(self.content_start..)
            .is_some_and(|rest| rest.trim().is_empty())
    }

    /// Builds the prefix for the next line. Task items intentionally restart
    /// as unchecked; ordered lists increment only when the number fits.
    pub(crate) fn continuation(&self, content: &str) -> String {
        let mut result = String::new();
        if let Some(digits) = &self.ordered_digits {
            result.push_str(&content[..digits.start]);
            if let Some(number) = content[digits.clone()]
                .parse::<u64>()
                .ok()
                .and_then(|value| value.checked_add(1))
            {
                result.push_str(&number.to_string());
            } else {
                result.push_str(&content[digits.clone()]);
            }
            result.push_str(&content[digits.end..self.list_marker_end]);
        } else {
            result.push_str(&content[..self.list_marker_end]);
        }

        if let Some(task) = &self.task_marker {
            result.push_str(&content[self.list_marker_end..task.start]);
            result.push_str("[ ]");
            result.push_str(&content[task.end..self.content_start]);
        } else {
            result.push_str(&content[self.list_marker_end..self.content_start]);
        }
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_task_and_plain_list_prefixes_without_attached_markers() {
        let task = ListLinePrefix::parse("  - [x] item").expect("task prefix");
        assert_eq!(task.content_start, "  - [x] ".len());
        assert_eq!(task.continuation("  - [x] item"), "  - [ ] ");
        assert!(!task.is_empty_item("  - [x] item"));
        assert!(ListLinePrefix::parse("- [x]attached").is_some());
        let attached = ListLinePrefix::parse("- [x]attached").expect("ordinary list prefix");
        assert_eq!(attached.content_start, "- ".len());
        assert!(!attached.is_empty_item("- [x]attached"));
    }

    #[test]
    fn continuation_increments_ordered_marker_and_exits_empty_item() {
        let ordered = ListLinePrefix::parse("9. item").expect("ordered prefix");
        assert_eq!(ordered.continuation("9. item"), "10. ");
        let empty = ListLinePrefix::parse("- [ ]  ").expect("empty task prefix");
        assert!(empty.is_empty_item("- [ ]  "));
    }
}
