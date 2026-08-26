//! 行内解析。
//!
//! 移植自 `@lezer/markdown` 的 `DefaultInline` 与 `InlineContext`。
//!
//! # 字节而不是 UTF-16
//!
//! 上游在 JS 字符串上工作，位置是 UTF-16 code unit；Yu 全程是字节。绝大多数
//! 判断比的是 ASCII 字面量，换成字节比较等价（UTF-8 的后续字节都 >= 0x80，
//! 不可能等于任何 ASCII）。**不等价的只有两处**，都在这里显式处理：
//!
//! - emphasis 的 flanking 判定要看分隔符前后那一个**字符**的 Unicode 类别，
//!   见 [`char_before`] / [`char_at`]；
//! - 实体与 HTML 标签的长度上限，上游按 code unit 截断，这里按字节，
//!   而两者的合法内容都是 ASCII，截断点相同。
//!
//! # 与上游的一处偏差
//!
//! 上游用 JS 的 `/\s/` 判断空白，它包含 U+FEFF 而不含 U+0085。这里用 Rust 的
//! `char::is_whitespace`，即 Unicode `White_Space` 属性——正是 CommonMark
//! 「Unicode whitespace character」的定义。这一处**比上游更贴规范**。

use unicode_general_category::{GeneralCategory, get_general_category};

use crate::element::Element;
use crate::node::NodeKind;

/// ASCII 标点，反斜杠转义只对这些字符生效（CommonMark「backslash escapes」）。
const ESCAPABLE: &[u8] = b"!\"#$%&'()*+,-./:;<=>?@[\\]^_`{|}~";

/// 分隔符的身份。用来回答「这个开分隔符和那个闭分隔符是同一种吗」。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum DelimiterId {
    EmphasisUnderscore,
    EmphasisAsterisk,
    LinkStart,
    ImageStart,
}

impl DelimiterId {
    /// 匹配成功时是否自动把中间内容包起来。
    ///
    /// `LinkStart` / `ImageStart` 返回 `false`：它们由 `LinkEnd` 在遇到 `]`
    /// 时急切匹配，不参与 [`InlineContext::resolve_markers`] 的自动配对。
    const fn resolves(self) -> bool {
        matches!(self, Self::EmphasisUnderscore | Self::EmphasisAsterisk)
    }

    /// 分隔符字符本身要成为的节点类型。
    const fn mark(self) -> Option<NodeKind> {
        match self {
            Self::EmphasisUnderscore | Self::EmphasisAsterisk => Some(NodeKind::EmphasisMark),
            Self::LinkStart | Self::ImageStart => None,
        }
    }

    const fn is_emphasis(self) -> bool {
        matches!(self, Self::EmphasisUnderscore | Self::EmphasisAsterisk)
    }
}

const MARK_OPEN: u8 = 1;
const MARK_CLOSE: u8 = 2;

#[derive(Clone, Copy, Debug)]
pub(crate) struct InlineDelimiter {
    id: DelimiterId,
    from: u32,
    to: u32,
    side: u8,
}

/// `parts` 里的一格：要么是已经定型的元素，要么是还在等匹配的分隔符。
#[derive(Clone, Debug)]
enum Part {
    Element(Element),
    Delimiter(InlineDelimiter),
}

/// 行内解析的上下文。
///
/// `text` 是一个 leaf block 的完整内容（多行之间用 `\n` 连接，容器标记已被
/// [`crate::block::Line::scrub`] 换成等长的空格），`offset` 是它在文档里的
/// 起点。因此 `text` 的字节下标加上 `offset` 就是文档偏移——不变量 C1 要求的
/// 「有序、有效、完整覆盖」由此成立。
pub(crate) struct InlineContext<'a> {
    text: &'a str,
    offset: u32,
    parts: Vec<Option<Part>>,
}

impl<'a> InlineContext<'a> {
    fn new(text: &'a str, offset: u32) -> Self {
        Self {
            text,
            offset,
            parts: Vec::new(),
        }
    }

    /// 本段行内内容的结束位置（文档偏移）。
    fn end(&self) -> u32 {
        self.offset + u32::try_from(self.text.len()).unwrap_or(u32::MAX)
    }

    /// `pos` 处的字节。越界返回 `None`（对应上游的 `-1`）。
    fn byte(&self, pos: u32) -> Option<u8> {
        if pos < self.offset || pos >= self.end() {
            return None;
        }
        let index = usize::try_from(pos - self.offset).ok()?;
        self.text.as_bytes().get(index).copied()
    }

    /// `from..to` 的文本。越界的端点被夹住，不在字符边界上的端点**向内取整**。
    ///
    /// 向内取整而不是返回空串。这里的调用方是「往后看最多 N 个字节」这类
    /// 前瞻（实体最多 30 字节、HTML 标签到段落末尾），端点落在多字节字符
    /// 中间是常态。返回空串会让 `&#65;` 能不能被识别取决于它后面 30 字节内
    /// 有没有一个 CJK 字符——不报错，只是有时候不解析。这条是 comrak 差分
    /// 抓出来的。
    fn slice(&self, from: u32, to: u32) -> &'a str {
        let len = self.text.len();
        let mut start = usize::try_from(from.saturating_sub(self.offset))
            .unwrap_or(len)
            .min(len);
        let mut end = usize::try_from(to.saturating_sub(self.offset))
            .unwrap_or(len)
            .min(len);
        while start > 0 && !self.text.is_char_boundary(start) {
            start -= 1;
        }
        while end > start && !self.text.is_char_boundary(end) {
            end -= 1;
        }
        if end <= start {
            return "";
        }
        &self.text[start..end]
    }

    /// 跳过 `from` 之后的空白，返回下一个非空白位置或本段末尾。
    fn skip_space(&self, from: u32) -> u32 {
        let start = usize::try_from(from.saturating_sub(self.offset))
            .unwrap_or(self.text.len())
            .min(self.text.len());
        self.offset + u32::try_from(skip_space(self.text, start)).unwrap_or(0)
    }

    fn append_element(&mut self, element: Element) -> u32 {
        let to = element.to;
        self.parts.push(Some(Part::Element(element)));
        to
    }

    fn append_delimiter(&mut self, delimiter: InlineDelimiter) -> u32 {
        let to = delimiter.to;
        self.parts.push(Some(Part::Delimiter(delimiter)));
        to
    }

    /// 把 `from` 之后的分隔符配对成节点，返回剩下的元素。
    ///
    /// 这是 CommonMark 强调解析里最容易出错的一段，其中「三的倍数」规则
    /// （`**a*b***` 里哪一对先配上）没有直观解释，只能照抄规范。
    fn resolve_markers(&mut self, from: usize) -> Vec<Element> {
        let mut index = from;
        while index < self.parts.len() {
            let Some(Part::Delimiter(close)) = self.parts[index] else {
                index += 1;
                continue;
            };
            if !close.id.resolves() || close.side & MARK_CLOSE == 0 {
                index += 1;
                continue;
            }
            let emphasis = close.id.is_emphasis();
            let close_size = close.to - close.from;

            let mut opening: Option<(usize, InlineDelimiter)> = None;
            for candidate_index in (from..index).rev() {
                let Some(Part::Delimiter(open)) = self.parts[candidate_index] else {
                    continue;
                };
                if open.side & MARK_OPEN == 0 || open.id != close.id {
                    continue;
                }
                let open_size = open.to - open.from;
                // CommonMark 的「rule of three」：当一个分隔符既能开又能闭时，
                // 两段长度之和是 3 的倍数、且各自不是 3 的倍数的配对被排除。
                let blocked = emphasis
                    && (close.side & MARK_OPEN != 0 || open.side & MARK_CLOSE != 0)
                    && (open_size + close_size) % 3 == 0
                    && (open_size % 3 != 0 || close_size % 3 != 0);
                if blocked {
                    continue;
                }
                opening = Some((candidate_index, open));
                break;
            }
            let Some((open_index, open)) = opening else {
                index += 1;
                continue;
            };

            let mut kind = close.id.resolve_kind();
            let mut start = open.from;
            let mut end = close.to;
            if emphasis {
                // 强调消耗的字符数是两侧的较小值，最多 2。
                let size = (open.to - open.from).min(close_size).min(2);
                start = open.to - size;
                end = close.from + size;
                kind = if size == 1 {
                    NodeKind::Emphasis
                } else {
                    NodeKind::StrongEmphasis
                };
            }

            let mut content = Vec::new();
            if let Some(mark) = open.id.mark() {
                content.push(Element::leaf(mark, start, open.to));
            }
            for slot in open_index + 1..index {
                if let Some(Part::Element(element)) = self.parts[slot].take() {
                    content.push(element);
                }
            }
            if let Some(mark) = close.id.mark() {
                content.push(Element::leaf(mark, close.from, end));
            }
            let element = Element::new(kind, start, end, content);

            // 强调分隔符可能有剩余字符，剩下的继续参与后续配对。
            self.parts[open_index] =
                (emphasis && open.from != start).then_some(Part::Delimiter(InlineDelimiter {
                    id: open.id,
                    from: open.from,
                    to: start,
                    side: open.side,
                }));
            let keep = (emphasis && close.to != end).then_some(Part::Delimiter(InlineDelimiter {
                id: close.id,
                from: end,
                to: close.to,
                side: close.side,
            }));
            match keep {
                Some(keep) => {
                    self.parts[index] = Some(keep);
                    self.parts.insert(index, Some(Part::Element(element)));
                    // 下一轮落在剩余的闭分隔符上，让它继续找更外层的开分隔符。
                    index += 1;
                }
                None => {
                    self.parts[index] = Some(Part::Element(element));
                    index += 1;
                }
            }
        }

        self.parts[from..]
            .iter()
            .filter_map(|part| match part {
                Some(Part::Element(element)) => Some(element.clone()),
                _ => None,
            })
            .collect()
    }

    /// 取走 `start_index` 之后的全部内容（先配对），并把 `parts` 截断到那里。
    fn take_content(&mut self, start_index: usize) -> Vec<Element> {
        let content = self.resolve_markers(start_index);
        self.parts.truncate(start_index);
        content
    }
}

impl DelimiterId {
    /// 自动配对时包裹内容的节点类型。强调会在 `resolve_markers` 里按长度
    /// 改写成 `Emphasis` 或 `StrongEmphasis`，这里给的是占位。
    const fn resolve_kind(self) -> NodeKind {
        match self {
            Self::EmphasisUnderscore | Self::EmphasisAsterisk => NodeKind::Emphasis,
            Self::LinkStart => NodeKind::Link,
            Self::ImageStart => NodeKind::Image,
        }
    }
}

/// 解析一段行内内容，返回文档坐标下的元素序列。
///
/// `text` 的每个字节下标 `i` 对应文档偏移 `offset + i`。
pub(crate) fn parse_inline(text: &str, offset: u32) -> Vec<Element> {
    let mut cx = InlineContext::new(text, offset);
    let end = cx.end();
    let mut pos = offset;
    while pos < end {
        let Some(next) = cx.byte(pos) else { break };
        let mut advanced = None;
        for parser in INLINE_PARSERS {
            if let Some(after) = parser(&mut cx, next, pos) {
                advanced = Some(after);
                break;
            }
        }
        pos = match advanced {
            Some(after) if after > pos => after,
            // 没有解析器认领，或者认领了却没有前进：跳到下一个字符边界。
            // 按字符而不是按字节前进，是为了让 `char_at` 之类的判断永远落在
            // 合法边界上。
            _ => pos + char_len_at(text, pos.saturating_sub(offset)),
        };
    }
    cx.resolve_markers(0)
}

/// `text` 中 `index` 处那个字符的字节数；越界或落在非边界上返回 1。
fn char_len_at(text: &str, index: u32) -> u32 {
    let Ok(index) = usize::try_from(index) else {
        return 1;
    };
    text.get(index..)
        .and_then(|rest| rest.chars().next())
        .map_or(1, |ch| u32::try_from(ch.len_utf8()).unwrap_or(1))
}

type InlineParser = fn(&mut InlineContext<'_>, u8, u32) -> Option<u32>;

/// 顺序即优先级，与上游 `DefaultInline` 的键顺序一致。
///
/// `LinkEnd` 必须在 `Link` / `Image` 之后：它靠回扫 `parts` 找开标记，
/// 而开标记要先被放进去。
const INLINE_PARSERS: &[InlineParser] = &[
    parse_escape,
    parse_entity,
    parse_inline_code,
    parse_html_tag,
    parse_emphasis,
    parse_hard_break,
    parse_link_start,
    parse_image_start,
    parse_link_end,
];

fn parse_escape(cx: &mut InlineContext<'_>, next: u8, start: u32) -> Option<u32> {
    if next != b'\\' || start == cx.end() - 1 {
        return None;
    }
    let escaped = cx.byte(start + 1)?;
    ESCAPABLE
        .contains(&escaped)
        .then(|| cx.append_element(Element::leaf(NodeKind::Escape, start, start + 2)))
}

fn parse_entity(cx: &mut InlineContext<'_>, next: u8, start: u32) -> Option<u32> {
    if next != b'&' {
        return None;
    }
    // 上游取 30 个 code unit；实体名与数字实体都是 ASCII，按字节等价。
    let rest = cx.slice(start + 1, start + 31);
    let length = match_entity(rest)?;
    let length = u32::try_from(length).ok()?;
    Some(cx.append_element(Element::leaf(NodeKind::Entity, start, start + 1 + length)))
}

/// 匹配 `/^(?:#\d+|#x[a-f\d]+|\w+);/i`，返回含分号的长度。
///
/// **比上游多了位数上限。** 规范写明十进制引用是 1..=7 位数字、十六进制是
/// 1..=6 位，而 lezer 的正则 `#\d+` / `#x[a-f\d]+` 没有上限，于是
/// `&#87654321;` 会被它当成实体。规范用例 #28 直接考这一条。
const MAX_DECIMAL_DIGITS: usize = 7;
const MAX_HEX_DIGITS: usize = 6;

fn match_entity(rest: &str) -> Option<usize> {
    let bytes = rest.as_bytes();
    let index = if bytes.first() == Some(&b'#') {
        if matches!(bytes.get(1), Some(b'x' | b'X')) {
            let digits = bytes[2..]
                .iter()
                .take_while(|byte| byte.is_ascii_hexdigit())
                .count();
            if digits == 0 || digits > MAX_HEX_DIGITS {
                return None;
            }
            2 + digits
        } else {
            let digits = bytes[1..]
                .iter()
                .take_while(|byte| byte.is_ascii_digit())
                .count();
            if digits == 0 || digits > MAX_DECIMAL_DIGITS {
                return None;
            }
            1 + digits
        }
    } else {
        // `\w` 是 [A-Za-z0-9_]。
        let word = bytes
            .iter()
            .take_while(|byte| byte.is_ascii_alphanumeric() || **byte == b'_')
            .count();
        if word == 0 {
            return None;
        }
        word
    };
    (bytes.get(index) == Some(&b';')).then_some(index + 1)
}

fn parse_inline_code(cx: &mut InlineContext<'_>, next: u8, start: u32) -> Option<u32> {
    if next != b'`' || (start > cx.offset && cx.byte(start - 1) == Some(b'`')) {
        return None;
    }
    let mut pos = start + 1;
    while pos < cx.end() && cx.byte(pos) == Some(b'`') {
        pos += 1;
    }
    let size = pos - start;
    let mut run = 0_u32;
    while pos < cx.end() {
        if cx.byte(pos) == Some(b'`') {
            run += 1;
            if run == size && cx.byte(pos + 1) != Some(b'`') {
                let to = pos + 1;
                return Some(cx.append_element(Element::new(
                    NodeKind::InlineCode,
                    start,
                    to,
                    vec![
                        Element::leaf(NodeKind::CodeMark, start, start + size),
                        Element::leaf(NodeKind::CodeMark, to - size, to),
                    ],
                )));
            }
        } else {
            run = 0;
        }
        pos += 1;
    }
    None
}

fn parse_html_tag(cx: &mut InlineContext<'_>, next: u8, start: u32) -> Option<u32> {
    if next != b'<' || start == cx.end() - 1 {
        return None;
    }
    let after = cx.slice(start + 1, cx.end());

    if let Some(length) = match_autolink(after) {
        let length = u32::try_from(length).ok()?;
        // `length` 含收尾的 `>`，URL 节点要把它排除在外。
        return Some(cx.append_element(Element::new(
            NodeKind::Autolink,
            start,
            start + 1 + length,
            vec![
                Element::leaf(NodeKind::LinkMark, start, start + 1),
                Element::leaf(NodeKind::Url, start + 1, start + length),
                Element::leaf(NodeKind::LinkMark, start + length, start + 1 + length),
            ],
        )));
    }
    for (matcher, kind) in [
        (
            match_html_comment as fn(&str) -> Option<usize>,
            NodeKind::Comment,
        ),
        (
            match_processing_instruction,
            NodeKind::ProcessingInstruction,
        ),
        (match_html_tag, NodeKind::HtmlTag),
    ] {
        if let Some(length) = matcher(after) {
            let length = u32::try_from(length).ok()?;
            return Some(cx.append_element(Element::leaf(kind, start, start + 1 + length)));
        }
    }
    None
}

fn parse_emphasis(cx: &mut InlineContext<'_>, next: u8, start: u32) -> Option<u32> {
    if next != b'_' && next != b'*' {
        return None;
    }
    let mut pos = start + 1;
    while cx.byte(pos) == Some(next) {
        pos += 1;
    }

    let before = char_before(cx.text, start.saturating_sub(cx.offset) as usize);
    let after = char_at(cx.text, (pos - cx.offset) as usize);
    let punctuation_before = before.is_some_and(is_punctuation);
    let punctuation_after = after.is_some_and(is_punctuation);
    // 段首/段尾按空白算，与上游 `/\s|^$/` 一致。
    let space_before = before.is_none_or(char::is_whitespace);
    let space_after = after.is_none_or(char::is_whitespace);

    let left_flanking = !space_after && (!punctuation_after || space_before || punctuation_before);
    let right_flanking = !space_before && (!punctuation_before || space_after || punctuation_after);
    let can_open = left_flanking && (next == b'*' || !right_flanking || punctuation_before);
    let can_close = right_flanking && (next == b'*' || !left_flanking || punctuation_after);

    let id = if next == b'_' {
        DelimiterId::EmphasisUnderscore
    } else {
        DelimiterId::EmphasisAsterisk
    };
    Some(cx.append_delimiter(InlineDelimiter {
        id,
        from: start,
        to: pos,
        side: (u8::from(can_open) * MARK_OPEN) | (u8::from(can_close) * MARK_CLOSE),
    }))
}

fn parse_hard_break(cx: &mut InlineContext<'_>, next: u8, start: u32) -> Option<u32> {
    if next == b'\\'
        && let Some(end) = line_ending_at(cx, start + 1)
    {
        return Some(cx.append_element(Element::leaf(NodeKind::HardBreak, start, end)));
    }
    if next == b' ' {
        let mut pos = start + 1;
        while cx.byte(pos) == Some(b' ') {
            pos += 1;
        }
        if pos >= start + 2
            && let Some(end) = line_ending_at(cx, pos)
        {
            return Some(cx.append_element(Element::leaf(NodeKind::HardBreak, start, end)));
        }
    }
    None
}

/// `pos` 处是不是一个行尾符；是的话返回它之后的位置。
///
/// CommonMark 的 line ending 是 `\n`、`\r\n` 或单独的 `\r`。只认 `\n` 的话，
/// CRLF 文档里的硬换行整个失效——两个尾随空格变成可见内容，换行也不再是硬的。
/// 这件事不报错，只是画面不对，而 Windows 上存的文件全是 CRLF。
fn line_ending_at(cx: &InlineContext<'_>, pos: u32) -> Option<u32> {
    match cx.byte(pos)? {
        b'\n' => Some(pos + 1),
        b'\r' if cx.byte(pos + 1) == Some(b'\n') => Some(pos + 2),
        b'\r' => Some(pos + 1),
        _ => None,
    }
}

fn parse_link_start(cx: &mut InlineContext<'_>, next: u8, start: u32) -> Option<u32> {
    (next == b'[').then(|| {
        cx.append_delimiter(InlineDelimiter {
            id: DelimiterId::LinkStart,
            from: start,
            to: start + 1,
            side: MARK_OPEN,
        })
    })
}

fn parse_image_start(cx: &mut InlineContext<'_>, next: u8, start: u32) -> Option<u32> {
    if next != b'!' || cx.byte(start + 1) != Some(b'[') {
        return None;
    }
    Some(cx.append_delimiter(InlineDelimiter {
        id: DelimiterId::ImageStart,
        from: start,
        to: start + 2,
        side: MARK_OPEN,
    }))
}

fn parse_link_end(cx: &mut InlineContext<'_>, next: u8, start: u32) -> Option<u32> {
    if next != b']' {
        return None;
    }
    // 回扫最近的一个 link/image 开标记。
    let found = cx
        .parts
        .iter()
        .enumerate()
        .rev()
        .find_map(|(index, part)| match part {
            Some(Part::Delimiter(delimiter))
                if matches!(
                    delimiter.id,
                    DelimiterId::LinkStart | DelimiterId::ImageStart
                ) =>
            {
                Some((index, *delimiter))
            }
            _ => None,
        });
    let (index, delimiter) = found?;

    // side 被清零表示这个开标记已作废（会产出嵌套链接）；`[]` 后面既不是 `(`
    // 也不是 `[` 时也不成立。两种情况都把开标记丢掉，`]` 当普通字符。
    let empty_label =
        cx.skip_space(delimiter.to) == start && !matches!(cx.byte(start + 1), Some(b'(' | b'['));
    if delimiter.side == 0 || empty_label {
        cx.parts[index] = None;
        return None;
    }

    let content = cx.take_content(index);
    let kind = if delimiter.id == DelimiterId::LinkStart {
        NodeKind::Link
    } else {
        NodeKind::Image
    };
    let link = finish_link(cx, content, kind, delimiter.from, start + 1);
    let to = link.to;
    cx.parts.push(Some(Part::Element(link)));
    // 链接不能嵌套：把它左边所有未匹配的 `[` 作废。
    if delimiter.id == DelimiterId::LinkStart {
        for slot in &mut cx.parts[..index] {
            if let Some(Part::Delimiter(open)) = slot
                && open.id == DelimiterId::LinkStart
            {
                open.side = 0;
            }
        }
    }
    Some(to)
}

/// 补上链接的目标部分：`(url "title")` 或 `[label]`。
///
/// **不判断引用是否存在**（不变量 C6）。`[a][b]` 里的 `[b]` 只产出一个
/// `LinkLabel` 候选，`b` 有没有定义由装饰阶段的 reference table 决定。
fn finish_link(
    cx: &InlineContext<'_>,
    mut content: Vec<Element>,
    kind: NodeKind,
    start: u32,
    start_pos: u32,
) -> Element {
    let opening_len = if kind == NodeKind::Image { 2 } else { 1 };
    content.insert(
        0,
        Element::leaf(NodeKind::LinkMark, start, start + opening_len),
    );
    content.push(Element::leaf(NodeKind::LinkMark, start_pos - 1, start_pos));

    let mut end_pos = start_pos;
    match cx.byte(start_pos) {
        Some(b'(') => {
            let mut pos = cx.skip_space(start_pos + 1);
            let destination = parse_url(cx.text, pos - cx.offset, cx.offset);
            let mut title = None;
            if let Scan::Found(ref found) = destination {
                pos = cx.skip_space(found.to);
                // 目标与标题之间必须有空白。
                if pos != found.to
                    && let Scan::Found(found_title) =
                        parse_link_title(cx.text, pos - cx.offset, cx.offset)
                {
                    pos = cx.skip_space(found_title.to);
                    title = Some(found_title);
                }
            }
            if cx.byte(pos) == Some(b')') {
                content.push(Element::leaf(NodeKind::LinkMark, start_pos, start_pos + 1));
                end_pos = pos + 1;
                if let Scan::Found(found) = destination {
                    content.push(found);
                }
                if let Some(found) = title {
                    content.push(found);
                }
                content.push(Element::leaf(NodeKind::LinkMark, pos, end_pos));
            }
        }
        Some(b'[') => {
            if let Scan::Found(label) =
                parse_link_label(cx.text, start_pos - cx.offset, cx.offset, false)
            {
                end_pos = label.to;
                content.push(label);
            }
        }
        _ => {}
    }
    Element::new(kind, start, end_pos, content)
}

/// 扫描结果。`Incomplete` 表示扫到了输入末尾还没定论——增量的引用定义解析器
/// 靠它区分「这一行还没写完」和「这一行写错了」。
pub(crate) enum Scan {
    Found(Element),
    /// 到达输入末尾，还有可能在后续行里成立。
    Incomplete,
    /// 确定不成立。
    Failed,
}

pub(crate) fn parse_url(text: &str, start: u32, offset: u32) -> Scan {
    let bytes = text.as_bytes();
    let Ok(start_index) = usize::try_from(start) else {
        return Scan::Failed;
    };
    match bytes.get(start_index) {
        Some(b'<') => {
            let mut pos = start_index + 1;
            let mut escaped = false;
            while pos < bytes.len() {
                if escaped {
                    escaped = false;
                    pos += 1;
                    continue;
                }
                match bytes[pos] {
                    // 转义的 `>` 不结束目标。上游不认这条转义，于是
                    // `[link](<foo\>)` 在它那里成了链接（规范用例 #493）。
                    b'\\' => {
                        escaped = true;
                        pos += 1;
                    }
                    b'>' => {
                        return Scan::Found(Element::leaf(
                            NodeKind::Url,
                            start + offset,
                            u32::try_from(pos + 1).unwrap_or(u32::MAX) + offset,
                        ));
                    }
                    b'<' | b'\n' => return Scan::Failed,
                    _ => pos += 1,
                }
            }
            Scan::Incomplete
        }
        Some(_) => {
            let mut depth = 0_u32;
            let mut pos = start_index;
            let mut escaped = false;
            while pos < bytes.len() {
                let byte = bytes[pos];
                if is_space_byte(byte) {
                    break;
                } else if escaped {
                    escaped = false;
                } else if byte == b'(' {
                    depth += 1;
                } else if byte == b')' {
                    if depth == 0 {
                        break;
                    }
                    depth -= 1;
                } else if byte == b'\\' {
                    escaped = true;
                }
                pos += 1;
            }
            if pos > start_index {
                Scan::Found(Element::leaf(
                    NodeKind::Url,
                    start + offset,
                    u32::try_from(pos).unwrap_or(u32::MAX) + offset,
                ))
            } else if pos == bytes.len() {
                Scan::Incomplete
            } else {
                Scan::Failed
            }
        }
        None => Scan::Incomplete,
    }
}

pub(crate) fn parse_link_title(text: &str, start: u32, offset: u32) -> Scan {
    let bytes = text.as_bytes();
    let Ok(start_index) = usize::try_from(start) else {
        return Scan::Failed;
    };
    let Some(&opening) = bytes.get(start_index) else {
        return Scan::Failed;
    };
    let closing = match opening {
        b'(' => b')',
        b'"' | b'\'' => opening,
        _ => return Scan::Failed,
    };
    let mut pos = start_index + 1;
    let mut escaped = false;
    while pos < bytes.len() {
        let byte = bytes[pos];
        if escaped {
            escaped = false;
        } else if byte == closing {
            return Scan::Found(Element::leaf(
                NodeKind::LinkTitle,
                start + offset,
                u32::try_from(pos + 1).unwrap_or(u32::MAX) + offset,
            ));
        } else if byte == b'\\' {
            escaped = true;
        }
        pos += 1;
    }
    Scan::Incomplete
}

/// 扫描 `[label]`。`require_non_whitespace` 用于引用定义：`[   ]:` 不是合法
/// 的引用标签。
pub(crate) fn parse_link_label(
    text: &str,
    start: u32,
    offset: u32,
    require_non_whitespace: bool,
) -> Scan {
    let bytes = text.as_bytes();
    let Ok(start_index) = usize::try_from(start) else {
        return Scan::Failed;
    };
    let mut require_non_whitespace = require_non_whitespace;
    let mut escaped = false;
    // 上游的 999 上限来自 CommonMark：链接标签最多 999 个字符。
    let end = bytes.len().min(start_index + 1 + 999);
    let mut pos = start_index + 1;
    while pos < end {
        let byte = bytes[pos];
        if escaped {
            escaped = false;
        } else if byte == b']' {
            return if require_non_whitespace {
                Scan::Failed
            } else {
                Scan::Found(Element::leaf(
                    NodeKind::LinkLabel,
                    start + offset,
                    u32::try_from(pos + 1).unwrap_or(u32::MAX) + offset,
                ))
            };
        } else {
            if require_non_whitespace && !is_space_byte(byte) {
                require_non_whitespace = false;
            }
            if byte == b'[' {
                return Scan::Failed;
            } else if byte == b'\\' {
                escaped = true;
            }
        }
        pos += 1;
    }
    Scan::Incomplete
}

// ---------------------------------------------------------------------------
// 手写的扫描器，代替上游的正则
//
// 引入 `regex` 只为这几条模式不划算：它们都是定长前缀匹配，手写既没有编译期
// 开销也不用把整段文本切成 `String`。代价是可能与正则有细微出入，因此这一段
// 的守护是 CommonMark spec 里的 44 条 HTML block 与 20 条 raw HTML 用例。
// ---------------------------------------------------------------------------

/// `/^(?:[a-z][-\w+.]+:[^\s>]+|<email>)>/i`，返回含结尾 `>` 的长度。
fn match_autolink(text: &str) -> Option<usize> {
    match_absolute_uri(text).or_else(|| match_email(text))
}

fn match_absolute_uri(text: &str) -> Option<usize> {
    let bytes = text.as_bytes();
    if !bytes.first()?.is_ascii_alphabetic() {
        return None;
    }
    let mut index = 1_usize;
    // `[-\w+.]+` 至少一个字符。
    while index < bytes.len() {
        let byte = bytes[index];
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'+' | b'.') {
            index += 1;
        } else {
            break;
        }
    }
    if index < 2 || bytes.get(index) != Some(&b':') {
        return None;
    }
    index += 1;
    let body_start = index;
    while index < bytes.len() && !is_space_byte(bytes[index]) && bytes[index] != b'>' {
        index += 1;
    }
    if index == body_start || bytes.get(index) != Some(&b'>') {
        return None;
    }
    Some(index + 1)
}

fn match_email(text: &str) -> Option<usize> {
    let bytes = text.as_bytes();
    let mut index = 0_usize;
    while index < bytes.len()
        && (bytes[index].is_ascii_alphanumeric() || b".!#$%&'*+/=?^_`{|}~-".contains(&bytes[index]))
    {
        index += 1;
    }
    if index == 0 || bytes.get(index) != Some(&b'@') {
        return None;
    }
    index += 1;
    index = match_email_label(bytes, index)?;
    while bytes.get(index) == Some(&b'.') {
        index = match_email_label(bytes, index + 1)?;
    }
    (bytes.get(index) == Some(&b'>')).then_some(index + 1)
}

/// `[a-z\d](?:[a-z\d-]{0,61}[a-z\d])?`
fn match_email_label(bytes: &[u8], start: usize) -> Option<usize> {
    if !bytes.get(start)?.is_ascii_alphanumeric() {
        return None;
    }
    let mut index = start + 1;
    let mut last_alphanumeric = start;
    while index < bytes.len() && index - start <= 62 {
        let byte = bytes[index];
        if byte.is_ascii_alphanumeric() {
            last_alphanumeric = index;
            index += 1;
        } else if byte == b'-' {
            index += 1;
        } else {
            break;
        }
    }
    Some(last_alphanumeric + 1)
}

/// HTML 注释：`<!-->`、`<!--->`，或 `<!--` + 不含 `-->` 的内容 + `-->`。
///
/// 上游用的是 CommonMark 0.29 的正则 `/^!--[^>](?:-[^-]|[^-])*?-->/`，它拒绝
/// 内容里出现 `--`，也拒绝空注释。0.30 起规范改成了「只要不含 `-->` 就行」，
/// 规范用例 #625 与 #626 考这两条。这里按现行规范写。
fn match_html_comment(text: &str) -> Option<usize> {
    if !text.starts_with("!--") {
        return None;
    }
    // `<!-->` 与 `<!--->` 是两个特例形式的空注释。
    if text.starts_with("!-->") {
        return Some(4);
    }
    if text.starts_with("!--->") {
        return Some(5);
    }
    text[3..].find("-->").map(|at| 3 + at + 3)
}

/// `/^\?[^]*?\?>/`
fn match_processing_instruction(text: &str) -> Option<usize> {
    text.starts_with('?')
        .then(|| text[1..].find("?>").map(|at| 1 + at + 2))
        .flatten()
}

/// `/^(?:![A-Z][^]*?>|!\[CDATA\[[^]*?\]\]>|\/\s*[a-zA-Z][\w-]*\s*>|<open tag>)/`
fn match_html_tag(text: &str) -> Option<usize> {
    let bytes = text.as_bytes();
    match bytes.first()? {
        b'!' if text.starts_with("![CDATA[") => text.find("]]>").map(|at| at + 3),
        b'!' if bytes.get(1).is_some_and(u8::is_ascii_uppercase) => text.find('>').map(|at| at + 1),
        b'/' => match_closing_tag(bytes),
        _ => match_open_tag(bytes),
    }
}

/// `\/\s*[a-zA-Z][\w-]*\s*>`
fn match_closing_tag(bytes: &[u8]) -> Option<usize> {
    let mut index = skip_ascii_space(bytes, 1);
    if !bytes.get(index)?.is_ascii_alphabetic() {
        return None;
    }
    index += 1;
    while index < bytes.len()
        && (bytes[index].is_ascii_alphanumeric() || matches!(bytes[index], b'_' | b'-'))
    {
        index += 1;
    }
    index = skip_ascii_space(bytes, index);
    (bytes.get(index) == Some(&b'>')).then_some(index + 1)
}

/// `\s*[a-zA-Z][\w-]*(\s+[a-zA-Z:_][\w-.:]*(\s*=\s*(unquoted|'..'|".."))?)*\s*(\/\s*)?>`
fn match_open_tag(bytes: &[u8]) -> Option<usize> {
    // 规范要求 `<` 后紧跟标签名。上游的正则在这里有个 `\s*`，于是 `< a>`
    // 被它当成标签（规范用例 #621）。
    let mut index = 0_usize;
    if !bytes.get(index)?.is_ascii_alphabetic() {
        return None;
    }
    index += 1;
    while index < bytes.len()
        && (bytes[index].is_ascii_alphanumeric() || matches!(bytes[index], b'_' | b'-'))
    {
        index += 1;
    }
    loop {
        let after_space = skip_ascii_space(bytes, index);
        // 属性名前必须有至少一个空白。
        if after_space == index {
            break;
        }
        let Some(&first) = bytes.get(after_space) else {
            break;
        };
        if !(first.is_ascii_alphabetic() || matches!(first, b':' | b'_')) {
            break;
        }
        let mut attribute = after_space + 1;
        while attribute < bytes.len()
            && (bytes[attribute].is_ascii_alphanumeric()
                || matches!(bytes[attribute], b'_' | b'-' | b'.' | b':'))
        {
            attribute += 1;
        }
        index = attribute;
        let before_equals = skip_ascii_space(bytes, attribute);
        if bytes.get(before_equals) != Some(&b'=') {
            continue;
        }
        let value_start = skip_ascii_space(bytes, before_equals + 1);
        index = match_attribute_value(bytes, value_start)?;
    }
    index = skip_ascii_space(bytes, index);
    // 自闭合标签是 `/>`，`/` 与 `>` 之间不允许有空白。上游的 `(\/\s*)?` 允许，
    // 于是 `<bar/ >` 被它当成标签（规范用例 #621）。
    if bytes.get(index) == Some(&b'/') {
        index += 1;
    }
    (bytes.get(index) == Some(&b'>')).then_some(index + 1)
}

/// `[^\s"'=<>`]+` | `'[^']*'` | `"[^"]*"`
fn match_attribute_value(bytes: &[u8], start: usize) -> Option<usize> {
    match bytes.get(start)? {
        quote @ (b'\'' | b'"') => {
            let quote = *quote;
            let mut index = start + 1;
            while index < bytes.len() {
                if bytes[index] == quote {
                    return Some(index + 1);
                }
                index += 1;
            }
            None
        }
        _ => {
            let mut index = start;
            while index < bytes.len()
                && !is_space_byte(bytes[index])
                && !matches!(bytes[index], b'"' | b'\'' | b'=' | b'<' | b'>' | b'`')
            {
                index += 1;
            }
            (index > start).then_some(index)
        }
    }
}

fn skip_ascii_space(bytes: &[u8], mut index: usize) -> usize {
    while index < bytes.len() && is_space_byte(bytes[index]) {
        index += 1;
    }
    index
}

// ---------------------------------------------------------------------------
// 共用的小工具
// ---------------------------------------------------------------------------

/// CommonMark 的「whitespace character」：空格、制表、换行、回车。
/// 这是 ASCII 集合，与 Unicode 的 `White_Space` 不是一回事——后者用在
/// flanking 判定里，见 [`parse_emphasis`]。
pub(crate) const fn is_space_byte(byte: u8) -> bool {
    matches!(byte, b' ' | b'\t' | b'\n' | b'\r')
}

pub(crate) fn skip_space(text: &str, mut index: usize) -> usize {
    let bytes = text.as_bytes();
    while index < bytes.len() && is_space_byte(bytes[index]) {
        index += 1;
    }
    index
}

pub(crate) fn skip_space_back(text: &str, mut index: usize, to: usize) -> usize {
    let bytes = text.as_bytes();
    while index > to && is_space_byte(bytes[index - 1]) {
        index -= 1;
    }
    index
}

/// `index` 之前的那个字符。`index` 为 0 或不在字符边界上时返回 `None`。
fn char_before(text: &str, index: usize) -> Option<char> {
    text.get(..index)?.chars().next_back()
}

/// `index` 处的字符。越界或不在边界上时返回 `None`。
fn char_at(text: &str, index: usize) -> Option<char> {
    text.get(index..)?.chars().next()
}

/// Unicode 类别属于 `S*`（符号）或 `P*`（标点）。
///
/// 对应上游的 `new RegExp("[\\p{S}|\\p{P}]", "u")`。
fn is_punctuation(ch: char) -> bool {
    matches!(
        get_general_category(ch),
        GeneralCategory::MathSymbol
            | GeneralCategory::CurrencySymbol
            | GeneralCategory::ModifierSymbol
            | GeneralCategory::OtherSymbol
            | GeneralCategory::ConnectorPunctuation
            | GeneralCategory::DashPunctuation
            | GeneralCategory::OpenPunctuation
            | GeneralCategory::ClosePunctuation
            | GeneralCategory::InitialPunctuation
            | GeneralCategory::FinalPunctuation
            | GeneralCategory::OtherPunctuation
    )
}

/// HTML 块的第 7 条起始条件与行内开标签用同一套语法，块级那边复用这里。
pub(crate) fn match_open_tag_public(text: &str) -> Option<usize> {
    match_open_tag(text.as_bytes())
}
