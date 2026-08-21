//! 解析器读取源码的通道。
//!
//! 上游 lezer 的 `Input::chunk(pos)` 返回「从 pos 起可用的一段」，而
//! `lineChunkAt` 只在**这一段之内**找换行——如果换行不在这一段里，它得到的
//! 「一行」就在 chunk 边界处被截断。lezer 靠 CodeMirror 的 Input 每次正好
//! 返回一整行（`lineChunks: true`）来回避这件事。
//!
//! Yu 的源码在 rope 里，chunk 边界由 rope 决定，和行边界没有关系。所以这里
//! 不移植 `chunk` + `lineChunks` 这一对，而是直接把接口定成行粒度：
//! [`Input::read_line_into`] 负责跨 chunk 拼到下一个 LF 为止。截断一行会让
//! 块解析在一个看不见的位置改变判断，属于「静默地做错事」，不能靠调用方小心。

use std::ops::Range;

use yu_core::ByteOffset;
use yu_text::TextSnapshot;

/// 解析器读到的源码。位置一律是相对文档起点的字节偏移。
pub trait Input {
    /// 源码总字节数。
    fn len_bytes(&self) -> u32;

    /// 把 `from` 起、到下一个 LF（不含）或文档末尾为止的字节追加进 `out`。
    ///
    /// 实现必须跨越存储的 chunk 边界，不得在 chunk 结束处提前收手。
    fn read_line_into(&self, from: u32, out: &mut String);

    /// `pos` 处的字节；越界返回 `None`。
    fn byte_at(&self, pos: u32) -> Option<u8>;

    /// 读取 `range` 区间的文本。只用于诊断与测试，解析热路径不走这里。
    fn read(&self, range: Range<u32>) -> String;
}

impl Input for str {
    fn len_bytes(&self) -> u32 {
        u32::try_from(self.len()).unwrap_or(u32::MAX)
    }

    fn read_line_into(&self, from: u32, out: &mut String) {
        let from = usize::try_from(from).unwrap_or(usize::MAX).min(self.len());
        let rest = &self[from..];
        let end = rest.find('\n').unwrap_or(rest.len());
        out.push_str(&rest[..end]);
    }

    fn byte_at(&self, pos: u32) -> Option<u8> {
        self.as_bytes().get(usize::try_from(pos).ok()?).copied()
    }

    fn read(&self, range: Range<u32>) -> String {
        let start = usize::try_from(range.start)
            .unwrap_or(usize::MAX)
            .min(self.len());
        let end = usize::try_from(range.end)
            .unwrap_or(usize::MAX)
            .min(self.len());
        self[start..start.max(end)].to_owned()
    }
}

/// 不变量 E4 要求 ropey 的索引不逃逸出 `yu-text`，所以这里只用
/// `ByteOffset` 与 `ChunkCursor` 说话。
impl Input for TextSnapshot {
    fn len_bytes(&self) -> u32 {
        u32::try_from(self.len_bytes().get()).unwrap_or(u32::MAX)
    }

    fn read_line_into(&self, from: u32, out: &mut String) {
        let Some(cursor) = self.chunk_cursor(ByteOffset::new(u64::from(from))).ok() else {
            return;
        };
        for chunk in cursor {
            let chunk_start = chunk.start().get();
            // 第一个 chunk 可能从 `from` 之前开始。
            let skip = usize::try_from(u64::from(from).saturating_sub(chunk_start)).unwrap_or(0);
            let text = chunk.text();
            if skip >= text.len() {
                continue;
            }
            let text = &text[skip..];
            match text.find('\n') {
                Some(end) => {
                    out.push_str(&text[..end]);
                    return;
                }
                None => out.push_str(text),
            }
        }
    }

    fn byte_at(&self, pos: u32) -> Option<u8> {
        let offset = ByteOffset::new(u64::from(pos));
        let mut cursor = self.chunk_cursor(offset).ok()?;
        let chunk = cursor.next()?;
        let index = usize::try_from(u64::from(pos).checked_sub(chunk.start().get())?).ok()?;
        chunk.text().as_bytes().get(index).copied()
    }

    fn read(&self, range: Range<u32>) -> String {
        let mut out = String::new();
        let Ok(cursor) = self.chunk_cursor(ByteOffset::new(u64::from(range.start))) else {
            return out;
        };
        for chunk in cursor {
            let chunk_start = chunk.start().get();
            if chunk_start >= u64::from(range.end) {
                break;
            }
            let text = chunk.text();
            let skip =
                usize::try_from(u64::from(range.start).saturating_sub(chunk_start)).unwrap_or(0);
            if skip >= text.len() {
                continue;
            }
            let take = usize::try_from(u64::from(range.end).saturating_sub(chunk_start))
                .unwrap_or(text.len())
                .min(text.len());
            if take <= skip {
                break;
            }
            out.push_str(&text[skip..take]);
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::Input;
    use yu_text::TextBuffer;

    /// 一份跨越多个 rope chunk 的源码。ropey 的 chunk 上限是 1KB 量级，
    /// 这里做到远超它，好让「行跨 chunk」必然发生。
    fn multi_chunk_source() -> String {
        let mut text = String::new();
        for index in 0..40 {
            text.push_str(&"x".repeat(500));
            if index % 3 == 0 {
                text.push('\n');
            }
        }
        text.push('\n');
        text
    }

    #[test]
    fn snapshot_reads_a_line_across_chunk_boundaries() {
        let source = multi_chunk_source();
        let buffer = TextBuffer::new(source.clone());
        let snapshot = buffer.snapshot();
        assert!(
            snapshot.storage_stats().chunks() > 1,
            "这个用例要有多个 chunk 才有意义，否则它测不出跨边界"
        );

        let mut offset = 0_u32;
        for expected in source.split('\n') {
            let mut line = String::new();
            snapshot.read_line_into(offset, &mut line);
            assert_eq!(line, expected, "从 {offset} 起读到的行与源码不符");
            offset += u32::try_from(expected.len() + 1).expect("测试数据不会溢出");
        }
    }

    #[test]
    fn snapshot_and_str_agree_on_every_line_start() {
        let source = multi_chunk_source();
        let buffer = TextBuffer::new(source.clone());
        let snapshot = buffer.snapshot();
        let text = source.as_str();

        for offset in 0..u32::try_from(source.len()).expect("测试数据不会溢出") {
            let mut from_snapshot = String::new();
            let mut from_str = String::new();
            snapshot.read_line_into(offset, &mut from_snapshot);
            text.read_line_into(offset, &mut from_str);
            assert_eq!(from_snapshot, from_str, "偏移 {offset} 处两种输入不一致");
            assert_eq!(
                snapshot.byte_at(offset),
                text.byte_at(offset),
                "偏移 {offset} 处字节不一致"
            );
        }
        assert_eq!(snapshot.byte_at(text.len_bytes()), None);
    }

    #[test]
    fn read_returns_the_requested_range() {
        let source = multi_chunk_source();
        let buffer = TextBuffer::new(source.clone());
        let snapshot = buffer.snapshot();
        for (start, end) in [(0_u32, 10_u32), (1_400, 1_600), (0, 20_000)] {
            let end = end.min(source.as_str().len_bytes());
            assert_eq!(
                snapshot.read(start..end),
                source.as_str().read(start..end),
                "区间 {start}..{end} 不一致"
            );
        }
    }
}
