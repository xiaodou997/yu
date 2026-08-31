use yu_core::{ByteOffset, Revision, TextRange};
use yu_text::{ChunkCursor, TextSnapshot};

use crate::block_sequence::{BlockKind, BlockSequence};

/// One source-backed link definition such as `[project]: /docs`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ReferenceDefinition {
    source: TextRange,
    label: TextRange,
    destination: TextRange,
    label_hash: u64,
    destination_hash: u64,
}

impl ReferenceDefinition {
    #[must_use]
    pub const fn source(self) -> TextRange {
        self.source
    }

    #[must_use]
    pub const fn label(self) -> TextRange {
        self.label
    }

    #[must_use]
    pub const fn destination(self) -> TextRange {
        self.destination
    }
}

/// Revision-bound definitions used to resolve reference links.
///
/// The index stores only source ranges and compact label/destination hashes.
/// It does not copy labels or destinations; lookup compares normalized bytes
/// against the immutable source snapshot when a label hash matches.
#[derive(Clone, Debug)]
pub struct ReferenceDefinitionIndex {
    source: TextSnapshot,
    definitions: Vec<ReferenceDefinition>,
    fingerprint: u64,
}

impl PartialEq for ReferenceDefinitionIndex {
    fn eq(&self, other: &Self) -> bool {
        self.definitions == other.definitions && self.fingerprint == other.fingerprint
    }
}

impl Eq for ReferenceDefinitionIndex {}

impl ReferenceDefinitionIndex {
    pub(crate) fn from_blocks(source: &TextSnapshot, blocks: &BlockSequence) -> Self {
        let definitions = blocks
            .iter()
            .filter_map(|record| {
                (record.kind() == BlockKind::ReferenceDefinition).then_some(record.range())
            })
            .filter_map(|range| parse_definition(source, range))
            .collect::<Vec<_>>();
        let fingerprint = fingerprint(&definitions);
        Self {
            source: source.clone(),
            definitions,
            fingerprint,
        }
    }

    #[must_use]
    pub fn revision(&self) -> Revision {
        self.source.revision()
    }

    #[must_use]
    pub fn definitions(&self) -> &[ReferenceDefinition] {
        &self.definitions
    }

    #[must_use]
    pub const fn fingerprint(&self) -> u64 {
        self.fingerprint
    }

    /// Resolves a label in the same source revision, using CommonMark-like
    /// ASCII case folding and whitespace collapsing for Phase 1.
    #[must_use]
    pub fn lookup(&self, source: &TextSnapshot, label: TextRange) -> Option<ReferenceDefinition> {
        if source.revision() != self.source.revision() {
            return None;
        }
        let normalized = normalized_label(source, label)?;
        let hash = hash_bytes(&normalized);
        self.definitions.iter().copied().find(|definition| {
            definition.label_hash == hash
                && normalized_label(&self.source, definition.label)
                    .is_some_and(|candidate| candidate == normalized)
        })
    }
}

/// Reports whether a root-level line is a link definition candidate.
pub(crate) fn is_reference_definition_line(source: &TextSnapshot, range: TextRange) -> bool {
    scan_definition(source, range).is_some()
}

fn parse_definition(source: &TextSnapshot, range: TextRange) -> Option<ReferenceDefinition> {
    let candidate = scan_definition(source, range)?;
    let label = byte_range(candidate.label_start, candidate.label_end)?;
    let destination = byte_range(candidate.destination_start, candidate.destination_end)?;
    let normalized = normalized_label(source, label)?;
    let destination_hash = hash_range(source, destination)?;
    Some(ReferenceDefinition {
        source: range,
        label,
        destination,
        label_hash: hash_bytes(&normalized),
        destination_hash,
    })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct DefinitionCandidate {
    label_start: usize,
    label_end: usize,
    destination_start: usize,
    destination_end: usize,
}

/// Scans only the small definition grammar without allocating a line buffer.
/// The parser calls this for every block boundary, so ordinary paragraph lines
/// must stay chunk-only and allocation-free.
fn scan_definition(source: &TextSnapshot, range: TextRange) -> Option<DefinitionCandidate> {
    let mut cursor = DefinitionByteCursor::new(source, range)?;
    let mut current = cursor.next()?;
    let mut leading_spaces = 0_usize;
    while current.1 == b' ' {
        leading_spaces = leading_spaces.saturating_add(1);
        if leading_spaces > 3 {
            return None;
        }
        current = cursor.next()?;
    }
    if current.1 != b'[' {
        return None;
    }
    let label_start = current.0.checked_add(1)?;
    let mut label_end = None;
    for (position, byte) in cursor.by_ref() {
        if matches!(byte, b'\r' | b'\n') {
            return None;
        }
        if byte == b']' {
            label_end = Some(position);
            break;
        }
    }
    let label_end = label_end?;
    if label_end == label_start || cursor.next()?.1 != b':' {
        return None;
    }

    let mut current = cursor.next()?;
    while matches!(current.1, b' ' | b'\t') {
        current = cursor.next()?;
    }
    if matches!(current.1, b'\r' | b'\n') {
        return None;
    }
    if current.1 == b'<' {
        let destination_start = current.0.checked_add(1)?;
        for (position, byte) in cursor.by_ref() {
            if byte == b'>' {
                if position == destination_start {
                    return None;
                }
                return Some(DefinitionCandidate {
                    label_start,
                    label_end,
                    destination_start,
                    destination_end: position,
                });
            }
            if matches!(byte, b'\r' | b'\n') {
                return None;
            }
        }
        return None;
    }

    let destination_start = current.0;
    let mut destination_end = destination_start.checked_add(1)?;
    for (position, byte) in cursor {
        if matches!(byte, b' ' | b'\t' | b'\r' | b'\n') {
            break;
        }
        destination_end = position.checked_add(1)?;
    }
    Some(DefinitionCandidate {
        label_start,
        label_end,
        destination_start,
        destination_end,
    })
}

/// 引用标签的归一化。CommonMark 的三条规则在这里都做全了。
///
/// 去掉首尾空白、把内部连续空白折成一个空格、做 **Unicode default case
/// fold**（`caseless::default_case_fold_str`）。第三条曾经是 `str::
/// to_lowercase`（simple lowercase），差别只在少数几个字符上——`ẞ` fold 成
/// `ss` 而 lowercase 成 `ß`，`ﬁ` fold 成 `fi`——但那几个字符正是不变量 F3
/// 登记的那条偏差。**S7 第六刀关掉了它**：`yu-markdown` 因此有了它的第一个
/// 外部依赖，理由与代价写在 Cargo.toml 与 overview 第 8 节 S7 第六刀。
///
/// # 折出来的东西只是一个查表键
///
/// 返回的字节串进 [`hash_bytes`]，再由 [`ReferenceDefinitionIndex::lookup`]
/// 逐字节比对。**它从来不映射回源码偏移**，所以 full fold 让 `ẞ` 变成两个
/// 字节这件事在这里没有任何后果。搜索的「不区分大小写」是另一回事：那条路
/// 要回报 `TextRange`，折叠必须给得出对齐信息，`caseless` 给不出——两件事
/// 只是碰巧都需要一份 case folding，见 `yu-editor::search` 的模块文档。
///
/// # 空白按 CommonMark 取，不按 `char::is_whitespace`
///
/// 见下面循环里的注释。comrak 在同一处用的是 `char::is_whitespace`
/// （`comrak::strings::normalize_label`），那是一处已知的、比规范宽的取法。
fn normalized_label(source: &TextSnapshot, range: TextRange) -> Option<Vec<u8>> {
    let text = String::from_utf8(read_range(source, range)?).ok()?;
    let mut collapsed = String::new();
    let mut pending_space = false;
    for character in text.chars() {
        // 空白按 CommonMark 取「空格、制表符、行结束符」，不是
        // `char::is_whitespace`——后者还包含 U+00A0 之类，那会让两个不同的
        // 标签折到一起。
        if matches!(character, ' ' | '\t' | '\n' | '\r') {
            if !collapsed.is_empty() {
                pending_space = true;
            }
            continue;
        }
        if pending_space {
            collapsed.push(' ');
            pending_space = false;
        }
        collapsed.push(character);
    }
    Some(caseless::default_case_fold_str(&collapsed).into_bytes())
}

fn hash_bytes(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf29ce484222325, |hash, byte| {
        hash.wrapping_mul(0x100000001b3)
            .wrapping_add(u64::from(*byte))
    })
}

fn hash_range(source: &TextSnapshot, range: TextRange) -> Option<u64> {
    Some(hash_bytes(&read_range(source, range)?))
}

fn fingerprint(definitions: &[ReferenceDefinition]) -> u64 {
    definitions
        .iter()
        .fold(0xcbf29ce484222325, |hash, definition| {
            hash.wrapping_mul(0x100000001b3)
                .wrapping_add(definition.label_hash)
                .wrapping_mul(0x100000001b3)
                .wrapping_add(definition.destination_hash)
        })
}

pub(crate) fn read_range(source: &TextSnapshot, range: TextRange) -> Option<Vec<u8>> {
    let mut bytes = Vec::new();
    let mut cursor = source.chunk_cursor(range.start()).ok()?;
    let start = usize::try_from(range.start()).ok()?;
    let end = usize::try_from(range.end()).ok()?;
    for chunk in &mut cursor {
        let chunk_start = usize::try_from(chunk.start()).ok()?;
        let chunk_end = chunk_start.checked_add(chunk.text().len())?;
        if chunk_start >= end {
            break;
        }
        let local_start = start.max(chunk_start).saturating_sub(chunk_start);
        let local_end = end.min(chunk_end).saturating_sub(chunk_start);
        if local_start < local_end {
            bytes.extend_from_slice(&chunk.text().as_bytes()[local_start..local_end]);
        }
    }
    Some(bytes)
}

struct DefinitionByteCursor<'a> {
    chunks: ChunkCursor<'a>,
    requested_start: usize,
    end: usize,
    current: Option<&'a str>,
    current_start: usize,
    current_index: usize,
}

impl<'a> DefinitionByteCursor<'a> {
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

impl Iterator for DefinitionByteCursor<'_> {
    type Item = (usize, u8);

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            if let Some(current) = self.current {
                if self.current_index < self.current_start + current.len()
                    && self.current_index < self.end
                {
                    let local = self.current_index - self.current_start;
                    let value = current.as_bytes()[local];
                    let position = self.current_index;
                    self.current_index += 1;
                    return Some((position, value));
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

fn byte_range(start: usize, end: usize) -> Option<TextRange> {
    TextRange::new(
        ByteOffset::try_from(start).ok()?,
        ByteOffset::try_from(end).ok()?,
    )
}
