use yu_core::{ByteOffset, TextRange};
use yu_text::{ChunkCursor, TextSnapshot};

use crate::block_sequence::TaskState;

/// A parser-owned task marker, for example `[ ]` or `[x]`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct TaskMarker {
    state: TaskState,
    range: TextRange,
}

impl TaskMarker {
    #[must_use]
    pub const fn state(self) -> TaskState {
        self.state
    }

    #[must_use]
    pub const fn range(self) -> TextRange {
        self.range
    }
}

/// `[x]` / `[X]` 是勾上的，`[ ]` 是没勾的。别的形状不是复选框。
///
/// 这三个字节的读法只有这一处：`extension/task.rs` 按树给的 `TaskMarker`
/// 区间读，`classify` 按同一个区间读，两处读出来的必须是同一个状态。
pub(crate) fn checkbox_state(marker: &[u8]) -> Option<TaskState> {
    match marker {
        [b'[', b' ', b']'] => Some(TaskState::Todo),
        [b'[', b'x' | b'X', b']'] => Some(TaskState::Done),
        _ => None,
    }
}

/// Finds the first-line task marker of a list item without materializing the
/// source snapshot. The grammar intentionally accepts only the GFM-shaped
/// `[ ]`/`[x]` marker followed by whitespace or the line ending.
pub(crate) fn parse_task_marker(
    source: &TextSnapshot,
    range: TextRange,
    ordered: bool,
) -> Option<TaskMarker> {
    let mut cursor = TaskByteCursor::new(source, range)?;
    let mut current = cursor.next()?;
    let mut leading_spaces = 0_usize;
    while current.1 == b' ' {
        leading_spaces = leading_spaces.saturating_add(1);
        if leading_spaces > 3 {
            return None;
        }
        current = cursor.next()?;
    }

    if ordered {
        if !current.1.is_ascii_digit() {
            return None;
        }
        while current.1.is_ascii_digit() {
            current = cursor.next()?;
        }
        if !matches!(current.1, b'.' | b')') {
            return None;
        }
    } else if !matches!(current.1, b'-' | b'+' | b'*') {
        return None;
    }

    current = cursor.next()?;
    if !matches!(current.1, b' ' | b'\t') {
        return None;
    }
    while matches!(current.1, b' ' | b'\t') {
        current = cursor.next()?;
    }
    if current.1 != b'[' {
        return None;
    }
    let marker_start = current.0;
    let state = match cursor.next()?.1 {
        b' ' => TaskState::Todo,
        b'x' | b'X' => TaskState::Done,
        _ => return None,
    };
    if cursor.next()?.1 != b']' {
        return None;
    }
    if let Some((_, following)) = cursor.next()
        && !matches!(following, b' ' | b'\t' | b'\r' | b'\n')
    {
        return None;
    }
    Some(TaskMarker {
        state,
        range: TextRange::new(
            ByteOffset::try_from(marker_start).ok()?,
            ByteOffset::try_from(marker_start.checked_add(3)?).ok()?,
        )?,
    })
}

struct TaskByteCursor<'a> {
    chunks: ChunkCursor<'a>,
    requested_start: usize,
    end: usize,
    current: Option<&'a str>,
    current_start: usize,
    current_index: usize,
}

impl<'a> TaskByteCursor<'a> {
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

impl Iterator for TaskByteCursor<'_> {
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

#[cfg(test)]
mod tests {
    use super::*;
    use yu_text::{TextBuffer, Transaction};

    #[test]
    fn task_marker_parser_keeps_state_and_range_source_backed() {
        let source = "  - [x] done\n1. [ ] todo\n- [X] done\n- [x]attached\n";
        let buffer = TextBuffer::new(source);
        let snapshot = buffer.snapshot();
        let mut offset = 0_u64;
        let ranges = source
            .split_inclusive('\n')
            .map(|line| {
                let start = offset;
                offset += line.len() as u64;
                TextRange::new(ByteOffset::new(start), ByteOffset::new(offset)).expect("line range")
            })
            .collect::<Vec<_>>();
        assert_eq!(
            parse_task_marker(&snapshot, ranges[0], false).map(|marker| marker.state()),
            Some(TaskState::Done)
        );
        assert_eq!(
            parse_task_marker(&snapshot, ranges[1], true).map(|marker| marker.state()),
            Some(TaskState::Todo)
        );
        assert_eq!(
            parse_task_marker(&snapshot, ranges[2], false).map(|marker| marker.state()),
            Some(TaskState::Done)
        );
        assert_eq!(parse_task_marker(&snapshot, ranges[3], false), None);
        assert_eq!(
            &snapshot.as_str()[usize::try_from(
                parse_task_marker(&snapshot, ranges[0], false)
                    .expect("marker")
                    .range()
                    .start()
            )
            .expect("offset")
                ..usize::try_from(
                    parse_task_marker(&snapshot, ranges[0], false)
                        .expect("marker")
                        .range()
                        .end()
                )
                .expect("offset")],
            "[x]"
        );

        let mut chunked = TextBuffer::new("- [ ] item\n");
        chunked
            .apply(&Transaction::new(
                chunked.revision(),
                [yu_text::Edit::new(TextRange::empty(ByteOffset::new(2)), "")],
            ))
            .expect("empty edit should apply");
        assert_eq!(
            parse_task_marker(
                &chunked.snapshot(),
                TextRange::new(ByteOffset::ZERO, chunked.snapshot().len_bytes()).expect("range"),
                false
            )
            .expect("chunked marker")
            .state(),
            TaskState::Todo
        );
    }
}
