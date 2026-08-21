//! ropey 适配层：整个仓库里唯一允许出现 `ropey` 这个名字的 Rust 文件。
//!
//! 不变量 E4 要求 ropey 的索引与类型不逃逸出 `yu-text`。ropey 2.x 是全字节
//! 索引的——`insert` / `remove` / `slice` / `is_char_boundary` 收的都是字节
//! 偏移，没有 char index 这个概念——所以这里根本不存在 byte↔char 转换点。
//! 这正是选 2.x 而不是 1.6 的理由：1.6 的主索引是 char，每一处转换都是一个
//! 「在某个 emoji 上悄悄切错位置」的机会，而这里一个都没有。
//!
//! 本文件对外只暴露 `pub(super)`。加上 `tools/check-rope-leak.py` 保证
//! `ropey` 不出现在别处，E4 就成了机械可查的事实而不是约定。

use std::ops::Range;

use ropey::iter::Chunks;
use ropey::{LineType, Rope};

use super::{AllocationCollector, ChunkCursor, StorageChunk, StorageStats};
use crate::TextSummary;

/// Yu 的换行计数口径是「`\n` 的个数」（见 `TextSummary::from_text`）。
/// `LineType::LF` 忽略裸 CR，于是 `\r\n` 记一次、单独的 `\r` 记零次——同一口径。
const LINES: LineType = LineType::LF;

fn as_u64(value: usize) -> u64 {
    u64::try_from(value).unwrap_or(u64::MAX)
}

#[derive(Debug)]
pub(crate) struct RopeyStore {
    rope: Rope,
}

impl RopeyStore {
    pub(crate) fn new(text: String) -> Self {
        Self {
            rope: Rope::from_str(&text),
        }
    }

    pub(crate) fn len_bytes(&self) -> usize {
        self.rope.len()
    }

    pub(crate) fn is_char_boundary(&self, offset: usize) -> bool {
        // ropey 越界会 panic；不变量 I4 要求 panic 不穿过 ABI，所以先自己拦。
        offset <= self.rope.len() && self.rope.is_char_boundary(offset)
    }

    pub(crate) fn slice(&self, range: Range<usize>) -> String {
        self.rope.slice(range).to_string()
    }

    pub(crate) fn replace_range(&mut self, range: Range<usize>, inserted: &str) {
        let start = range.start;
        if !range.is_empty() {
            self.rope.remove(range);
        }
        if !inserted.is_empty() {
            self.rope.insert(start, inserted);
        }
    }

    pub(crate) fn snapshot(&self) -> RopeySnapshot {
        RopeySnapshot {
            rope: self.rope.clone(),
        }
    }

    pub(crate) fn stats(&self) -> StorageStats {
        self.snapshot().stats()
    }
}

/// `Rope` 的 clone 是 O(1) 的结构共享（内部是 `Arc` 的 CoW 树），
/// 所以快照不需要额外包一层。
#[derive(Clone, Debug)]
pub(crate) struct RopeySnapshot {
    rope: Rope,
}

impl RopeySnapshot {
    pub(crate) fn len_bytes(&self) -> usize {
        self.rope.len()
    }

    pub(crate) fn write_to(&self, output: &mut String) {
        for chunk in self.rope.chunks() {
            output.push_str(chunk);
        }
    }

    /// chunk 数只能靠遍历，是 O(chunk 数) 的。因此它只服务诊断与 bench，
    /// 不在任何热路径上——热路径用 `len_bytes` / `summary`，都是 O(1)。
    pub(crate) fn stats(&self) -> StorageStats {
        let chunks = self.rope.chunks().filter(|chunk| !chunk.is_empty()).count();
        StorageStats::new(self.rope.len(), chunks)
    }

    pub(crate) fn summary(&self) -> TextSummary {
        TextSummary::from_parts(
            as_u64(self.rope.len()),
            as_u64(self.rope.len_utf16()),
            as_u64(self.rope.len_lines(LINES)).saturating_sub(1),
        )
    }

    pub(crate) fn is_char_boundary(&self, offset: usize) -> bool {
        offset <= self.rope.len() && self.rope.is_char_boundary(offset)
    }

    pub(crate) fn prefix_summary(&self, offset: usize) -> TextSummary {
        TextSummary::from_parts(
            as_u64(offset),
            as_u64(self.rope.byte_to_utf16_idx(offset)),
            as_u64(self.rope.byte_to_line_idx(offset, LINES)),
        )
    }

    pub(crate) fn byte_offset_for_utf16(&self, offset: u64) -> Option<usize> {
        let offset = usize::try_from(offset).ok()?;
        if offset > self.rope.len_utf16() {
            return None;
        }
        // `utf16_to_byte_idx` 在 UTF-16 索引落在代理对中间时向下取整到该字符
        // 的起点，不报错。回代一次才能把「落在代理对中间」和「正好在边界上」
        // 区分开——契约要求前者返回 None。
        let byte = self.rope.utf16_to_byte_idx(offset);
        (self.rope.byte_to_utf16_idx(byte) == offset).then_some(byte)
    }

    pub(crate) fn byte_offset_for_line(&self, line: u64) -> Option<usize> {
        let line = usize::try_from(line).ok()?;
        (line < self.rope.len_lines(LINES)).then(|| self.rope.line_to_byte_idx(line, LINES))
    }

    /// ropey 的节点是私有的，走不进去。但 chunk 的数据指针就是叶子分配的地址：
    /// 两个快照共享同一片叶子时拿到同一个指针，编辑过的叶子是新分配、新指针。
    /// 按指针去重得到的正是「去重后的叶子文本字节数」。
    pub(crate) fn collect_allocations(&self, collector: &mut AllocationCollector) {
        for chunk in self.rope.chunks() {
            collector.add_chunk_text(chunk);
        }
    }

    pub(crate) fn chunks_from(&self, offset: usize) -> ChunkCursor<'_> {
        let (chunks, start) = self.rope.chunks_at(offset);
        ChunkCursor::new(RopeyChunkCursor {
            chunks,
            next_start: start,
        })
    }

    pub(crate) fn chunk_before(&self, offset: usize) -> Option<(usize, &str)> {
        if self.rope.len() == 0 {
            return None;
        }
        let mut cursor = self.rope.chunk_cursor_at(offset);
        // `chunk_cursor_at(len)` 停在最后一个 chunk 上，它正好结束于 offset，
        // 本身就是答案；其余情况游标停在「包含 offset」的 chunk 上，答案是它
        // 的前一个。
        if cursor.byte_offset() + cursor.chunk().len() > offset && !cursor.prev() {
            return None;
        }
        let text = cursor.chunk();
        (!text.is_empty()).then(|| (cursor.byte_offset(), text))
    }
}

pub(crate) struct RopeyChunkCursor<'a> {
    chunks: Chunks<'a>,
    next_start: usize,
}

impl<'a> Iterator for RopeyChunkCursor<'a> {
    type Item = StorageChunk<'a>;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            let text = self.chunks.next()?;
            if text.is_empty() {
                continue;
            }
            let start = self.next_start;
            self.next_start += text.len();
            return Some(StorageChunk { start, text });
        }
    }
}
