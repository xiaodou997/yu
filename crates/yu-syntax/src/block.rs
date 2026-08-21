//! 块级解析。
//!
//! 移植自 `@lezer/markdown` 的 `BlockContext` / `Line` / `DefaultBlockParsers`。
//!
//! # 一遍、不回溯
//!
//! 算法逐行前进，只看当前行和已经打开的容器栈，从不回头改已经产出的节点。
//! 这是增量解析能成立的前提：一个「解析到第 N 行时的状态」可以被完整地表示
//! 成容器栈，于是从任意块边界重新开始都能得到与全量解析相同的结果
//! （不变量 C3）。
//!
//! # 与上游的结构差异
//!
//! 上游区分 `lineStart`（文档坐标）与 `absoluteLineStart`（输入流坐标），
//! 因为它支持在一份输入里只解析若干不连续 range（`parseMixed`）。Yu 不用那个
//! 能力（见 crate 文档），两个坐标恒等，这里合并成一个 `line_start`。
//! 一并不移植的还有 `injectGaps` / `toRelative` / `moveRangeI` /
//! `reusePlaceholders`——它们全部只服务多 range。

use crate::element::{Element, inject_marks, wrap};
use crate::inline::{
    Scan, is_space_byte, parse_inline, parse_link_label, parse_link_title, parse_url, skip_space,
    skip_space_back,
};
use crate::input::Input;
use crate::node::NodeKind;
use crate::tree::Tree;

/// 一个打开着的容器块。
/// 列表项「以空行开头」的状态机。
///
/// 规范：一个列表项最多以一个空行开头。也就是说 `-` 后面直接是空行时，
/// 这个项是空的，后面的内容不再属于它。上游没有这条规则，于是
/// `-\n\n  foo` 里的 `foo` 会被并进列表项（规范用例 #280）。
#[derive(Clone, Copy, PartialEq, Eq)]
enum BlankStart {
    /// 标记后面有内容，这条规则不适用。
    No,
    /// 标记独占一行，还没看到下一行。
    Pending,
    /// 下一行也是空的，这个项到此为止。
    Exhausted,
}

struct CompositeBlock {
    kind: NodeKind,
    /// 列表项用它存内容缩进，列表用它存标记字符。
    value: u32,
    blank_start: BlankStart,
    from: u32,
    /// 上下文哈希：这个块所处的容器路径的摘要。见 [`crate::tree`]。
    hash: u32,
    end: u32,
    children: Vec<Tree>,
    positions: Vec<u32>,
}

impl CompositeBlock {
    fn create(kind: NodeKind, value: u32, from: u32, parent_hash: u32, end: u32) -> Self {
        // 与上游一致的 32 位环绕运算。哈希只要求「同样的容器路径给同样的值」，
        // 不要求抗碰撞——碰撞的后果是复用了一个本不该复用的块，而复用的结果
        // 立刻会被 C3 的差分测试抓到。
        let hash = parent_hash
            .wrapping_add(parent_hash << 8)
            .wrapping_add(kind as u32)
            .wrapping_add(value << 4);
        Self {
            kind,
            value,
            blank_start: BlankStart::No,
            from,
            hash,
            end,
            children: Vec::new(),
            positions: Vec::new(),
        }
    }

    /// 加一个子节点，`position` 是绝对位置。
    fn add_child(&mut self, child: Tree, position: u32) {
        self.children.push(child.with_context_hash(self.hash));
        self.positions.push(position.saturating_sub(self.from));
    }

    fn into_tree(self, end: u32) -> Tree {
        let mut end = end;
        if let (Some(&position), Some(child)) = (self.positions.last(), self.children.last()) {
            end = end.max(self.from + position + child.len_bytes());
        }
        Tree::new(
            self.kind,
            end.saturating_sub(self.from),
            0,
            self.children,
            self.positions,
        )
    }
}

/// 正在被解析的一行。
///
/// `pos` / `indent` / `next` 描述的是**跳过所有已处理容器标记之后**的位置：
/// 在 `> - foo` 的第二层容器里，`pos` 指向 `foo`。
pub(crate) struct Line {
    /// 整行文本，不含换行符。
    pub text: String,
    /// 已处理的容器提供的基准缩进（列数）。
    pub base_indent: usize,
    /// 基准缩进对应的字节位置。
    pub base_pos: usize,
    /// 已处理的容器层数。
    pub depth: usize,
    /// 已处理容器的标记节点（`>`、列表符号等）。
    pub markers: Vec<Element>,
    /// 下一个非空白字符的字节位置。
    pub pos: usize,
    /// 下一个非空白字符的列号。
    pub indent: usize,
    /// `pos` 处的字节，行尾为 `None`。
    pub next: Option<u8>,
}

impl Line {
    fn new() -> Self {
        Self {
            text: String::new(),
            base_indent: 0,
            base_pos: 0,
            depth: 0,
            markers: Vec::new(),
            pos: 0,
            indent: 0,
            next: None,
        }
    }

    fn forward(&mut self) {
        if self.base_pos > self.pos {
            self.forward_inner();
        }
    }

    fn forward_inner(&mut self) {
        let new_pos = skip_space(&self.text, self.base_pos);
        self.indent = self.count_indent(new_pos, self.pos, self.indent);
        self.pos = new_pos;
        self.next = self.text.as_bytes().get(new_pos).copied();
    }

    fn reset(&mut self) {
        self.base_indent = 0;
        self.base_pos = 0;
        self.pos = 0;
        self.indent = 0;
        self.forward_inner();
        self.depth = 1;
        self.markers.clear();
    }

    /// 把基准位置推进到 `to`（字节位置）。
    fn move_base(&mut self, to: usize) {
        self.base_pos = to;
        self.base_indent = self.count_indent(to, self.pos, self.indent);
    }

    /// 把基准位置推进到第 `indent` 列。
    fn move_base_column(&mut self, indent: usize) {
        self.base_indent = indent;
        self.base_pos = self.find_column(indent);
    }

    /// `from` 列号为 `indent` 时，`to` 处的列号。
    ///
    /// 制表符对齐到 4 的倍数。按**字符**计数而不是按字节：UTF-8 的后续字节
    /// （`0b10xxxxxx`）不单独占一列。实践中这段范围里只有空格和制表符，
    /// 这样写是为了让它对任何输入都不会算错，而不是靠调用点保证。
    pub(crate) fn count_indent(&self, to: usize, from: usize, indent: usize) -> usize {
        let bytes = self.text.as_bytes();
        let mut indent = indent;
        for &byte in &bytes[from.min(bytes.len())..to.min(bytes.len())] {
            if byte == b'\t' {
                indent += 4 - indent % 4;
            } else if byte & 0xC0 != 0x80 {
                indent += 1;
            }
        }
        indent
    }

    /// 第 `goal` 列对应的字节位置。
    /// 第 `goal` 列对应的字节位置。
    ///
    /// **返回值一定落在字符边界上。** 按字符而不是按字节前进：一个多字节
    /// 字符占一列，达到 `goal` 时如果停在它的首字节之后，返回的位置就在字符
    /// 中间，而 `scrub()` 会拿这个位置去切字符串。
    ///
    /// 目前没有能走到那里的输入——所有调用点的 `goal` 都落在行首的空白里，
    /// 而空白全是 ASCII。这里写成无条件成立的契约而不是依赖那个论证：
    /// 论证依赖「容器缩进永远是空白」这条别处的性质，它随时可能被改动，
    /// 而改动的人不会知道这里有个隐含前提。
    pub(crate) fn find_column(&self, goal: usize) -> usize {
        let bytes = self.text.as_bytes();
        let mut index = 0_usize;
        let mut indent = 0_usize;
        while index < bytes.len() && indent < goal {
            let byte = bytes[index];
            if byte == b'\t' {
                indent += 4 - indent % 4;
            } else {
                indent += 1;
            }
            index += 1;
            // 跳过这个字符剩下的续字节。
            while index < bytes.len() && bytes[index] & 0xC0 == 0x80 {
                index += 1;
            }
        }
        index
    }

    /// 供 leaf block 累积用的行内容：把容器标记换成等长的空格。
    ///
    /// 等长是关键。leaf 的内容会被 [`parse_inline`] 按「内容下标 + 起点 =
    /// 文档偏移」的方式定位，任何长度变化都会让行内节点的 range 整体错位。
    fn scrub(&self) -> String {
        if self.base_indent == 0 {
            return self.text.clone();
        }
        let mut result = " ".repeat(self.base_pos);
        result.push_str(&self.text[self.base_pos.min(self.text.len())..]);
        result
    }

    fn len(&self) -> usize {
        self.text.len()
    }
}

/// 一个段落样式的块，边界还没确定。
struct LeafBlock {
    start: u32,
    content: String,
    marks: Vec<Element>,
}

/// 块解析器的返回。
enum BlockResult {
    /// 本规则不适用，交给下一个。
    NotApplicable,
    /// 已经解析完一个 leaf block，并把当前行推进到它之后。
    Consumed,
    /// 打开了一个容器块，本行继续解析。
    Opened,
}

// ---------------------------------------------------------------------------
// 行首识别：这些函数只看不改
// ---------------------------------------------------------------------------

/// 围栏代码块的围栏结束位置，不是围栏则 `None`。
fn is_fenced_code(line: &Line) -> Option<usize> {
    let next = line.next?;
    if next != b'`' && next != b'~' {
        return None;
    }
    let bytes = line.text.as_bytes();
    let mut pos = line.pos + 1;
    while pos < bytes.len() && bytes[pos] == next {
        pos += 1;
    }
    if pos < line.pos + 3 {
        return None;
    }
    // 反引号围栏的 info string 里不能再出现反引号。
    if next == b'`' && bytes[pos..].contains(&b'`') {
        return None;
    }
    Some(pos)
}

/// 引用标记的宽度（`>` 或 `> `）。
fn is_blockquote(line: &Line) -> Option<usize> {
    if line.next != Some(b'>') {
        return None;
    }
    Some(if line.text.as_bytes().get(line.pos + 1) == Some(&b' ') {
        2
    } else {
        1
    })
}

fn is_horizontal_rule(line: &Line, stack_len: usize, breaking: bool) -> bool {
    let Some(next) = line.next else {
        return false;
    };
    if next != b'*' && next != b'-' && next != b'_' {
        return false;
    }
    let bytes = line.text.as_bytes();
    let mut count = 1_usize;
    for &byte in &bytes[line.pos + 1..] {
        if byte == next {
            count += 1;
        } else if !is_space_byte(byte) {
            return false;
        }
    }
    // setext 标题优先：`---` 在一个段落下面是 h2 而不是分隔线。
    if breaking && next == b'-' && is_setext_underline(line).is_some() && line.depth == stack_len {
        return false;
    }
    count >= 3
}

fn in_list(stack: &[CompositeBlock], kind: NodeKind) -> bool {
    stack.iter().rev().any(|block| block.kind == kind)
}

fn is_bullet_list(line: &Line, stack: &[CompositeBlock], breaking: bool) -> Option<usize> {
    let next = line.next?;
    if next != b'-' && next != b'+' && next != b'*' {
        return None;
    }
    let bytes = line.text.as_bytes();
    if line.pos != bytes.len() - 1 && !bytes.get(line.pos + 1).copied().is_some_and(is_space_byte) {
        return None;
    }
    // 打断段落时，空的列表项不算——`foo\n-\n` 里的 `-` 是 setext 下划线。
    if breaking
        && !in_list(stack, NodeKind::BulletList)
        && skip_space(&line.text, line.pos + 2) >= bytes.len()
    {
        return None;
    }
    Some(1)
}

fn is_ordered_list(line: &Line, stack: &[CompositeBlock], breaking: bool) -> Option<usize> {
    let bytes = line.text.as_bytes();
    let mut pos = line.pos;
    let mut next = line.next?;
    while next.is_ascii_digit() {
        pos += 1;
        if pos == bytes.len() {
            return None;
        }
        next = bytes[pos];
    }
    if pos == line.pos || pos > line.pos + 9 || (next != b'.' && next != b')') {
        return None;
    }
    if pos < bytes.len() - 1 && !bytes.get(pos + 1).copied().is_some_and(is_space_byte) {
        return None;
    }
    // 打断段落时只允许 `1.`：`2.` 起头的一行接在段落后面仍是段落。
    if breaking
        && !in_list(stack, NodeKind::OrderedList)
        && (skip_space(&line.text, pos + 1) == bytes.len()
            || pos > line.pos + 1
            || line.next != Some(b'1'))
    {
        return None;
    }
    Some(pos + 1 - line.pos)
}

/// ATX 标题的 `#` 个数。
fn is_atx_heading(line: &Line) -> Option<usize> {
    if line.next != Some(b'#') {
        return None;
    }
    let bytes = line.text.as_bytes();
    let mut pos = line.pos + 1;
    while pos < bytes.len() && bytes[pos] == b'#' {
        pos += 1;
    }
    // 规范允许 `#` 后面跟空格或制表符。上游只认空格（规范用例 #10 考这一条），
    // 这里按规范放宽——这是修正 lezer，不是偏离规范。
    if pos < bytes.len() && bytes[pos] != b' ' && bytes[pos] != b'\t' {
        return None;
    }
    let size = pos - line.pos;
    (size <= 6).then_some(size)
}

/// setext 下划线的结束位置。
fn is_setext_underline(line: &Line) -> Option<usize> {
    let next = line.next?;
    if (next != b'-' && next != b'=') || line.indent >= line.base_indent + 4 {
        return None;
    }
    let bytes = line.text.as_bytes();
    let mut pos = line.pos + 1;
    while pos < bytes.len() && bytes[pos] == next {
        pos += 1;
    }
    let end = pos;
    while pos < bytes.len() && is_space_byte(bytes[pos]) {
        pos += 1;
    }
    (pos == bytes.len()).then_some(end)
}

/// HTML 块的七种起始条件（CommonMark「HTML blocks」）与各自的结束条件。
#[derive(Clone, Copy, PartialEq, Eq)]
enum HtmlBlockEnd {
    /// `</script>` `</pre>` `</style>`
    RawTextClose,
    CommentClose,
    ProcessingInstructionClose,
    AngleClose,
    CdataClose,
    /// 空行结束，且空行本身不属于这个块。
    BlankLine,
}

/// 返回起始条件的编号（0..=6）。`breaking` 时排除第 7 条——它不能打断段落。
fn is_html_block(line: &Line, breaking: bool) -> Option<usize> {
    if line.next != Some(b'<') {
        return None;
    }
    let rest = &line.text[line.pos..];
    let limit = if breaking { 6 } else { 7 };
    (0..limit).find(|&condition| html_block_starts(rest, condition))
}

fn html_block_end(condition: usize) -> HtmlBlockEnd {
    match condition {
        0 => HtmlBlockEnd::RawTextClose,
        1 => HtmlBlockEnd::CommentClose,
        2 => HtmlBlockEnd::ProcessingInstructionClose,
        3 => HtmlBlockEnd::AngleClose,
        4 => HtmlBlockEnd::CdataClose,
        _ => HtmlBlockEnd::BlankLine,
    }
}

/// 第 6 条起始条件里的标签名，与上游一字不差。
const HTML_BLOCK_TAGS: &[&str] = &[
    "address",
    "article",
    "aside",
    "base",
    "basefont",
    "blockquote",
    "body",
    "caption",
    "center",
    "col",
    "colgroup",
    "dd",
    "details",
    "dialog",
    "dir",
    "div",
    "dl",
    "dt",
    "fieldset",
    "figcaption",
    "figure",
    "footer",
    "form",
    "frame",
    "frameset",
    "h1",
    "h2",
    "h3",
    "h4",
    "h5",
    "h6",
    "head",
    "header",
    "hr",
    "html",
    "iframe",
    "legend",
    "li",
    "link",
    "main",
    "menu",
    "menuitem",
    "nav",
    "noframes",
    "ol",
    "optgroup",
    "option",
    "p",
    "param",
    "section",
    "source",
    "summary",
    "table",
    "tbody",
    "td",
    "tfoot",
    "th",
    "thead",
    "title",
    "tr",
    "track",
    "ul",
];

fn html_block_starts(rest: &str, condition: usize) -> bool {
    let bytes = rest.as_bytes();
    match condition {
        // /^<(?:script|pre|style)(?:\s|>|$)/i
        // 规范 0.30 起把 `textarea` 加进了第 1 条，上游停在 0.29（规范用例 #171）。
        0 => ["script", "pre", "style", "textarea"].iter().any(|tag| {
            // 按字节比较而不是切字符串：`<` 后面可能是多字节字符，
            // 按长度切会落进字符中间。
            let after = 1 + tag.len();
            bytes.len() >= after
                && bytes[1..after].eq_ignore_ascii_case(tag.as_bytes())
                && bytes
                    .get(after)
                    .is_none_or(|byte| is_space_byte(*byte) || *byte == b'>')
        }),
        1 => rest.starts_with("<!--"),
        2 => rest.starts_with("<?"),
        3 => rest.starts_with("<!") && bytes.get(2).is_some_and(u8::is_ascii_uppercase),
        4 => rest.starts_with("<![CDATA["),
        // /^\s*<\/?(?:tag)(?:\s|\/?>|$)/i
        5 => {
            let name_start = if rest.starts_with("</") { 2 } else { 1 };
            HTML_BLOCK_TAGS.iter().any(|tag| {
                let after = name_start + tag.len();
                bytes.len() >= after
                    && bytes[name_start..after].eq_ignore_ascii_case(tag.as_bytes())
                    && match bytes.get(after) {
                        None => true,
                        Some(b'>') => true,
                        Some(b'/') => bytes.get(after + 1) == Some(&b'>'),
                        Some(byte) => is_space_byte(*byte),
                    }
            })
        }
        // /^\s*(?:<\/[a-z][\w-]*\s*>|<open tag>)\s*$/i
        6 => match_condition_seven(rest),
        _ => false,
    }
}

fn match_condition_seven(rest: &str) -> bool {
    let bytes = rest.as_bytes();
    let matched = if rest.starts_with("</") {
        let mut index = 2;
        if !bytes.get(index).is_some_and(u8::is_ascii_alphabetic) {
            return false;
        }
        index += 1;
        while index < bytes.len()
            && (bytes[index].is_ascii_alphanumeric() || matches!(bytes[index], b'_' | b'-'))
        {
            index += 1;
        }
        while index < bytes.len() && is_space_byte(bytes[index]) {
            index += 1;
        }
        if bytes.get(index) != Some(&b'>') {
            return false;
        }
        index + 1
    } else {
        // 复用行内的开标签扫描器：条件 7 的开标签语法与它一致。
        match crate::inline::match_open_tag_public(&rest[1..]) {
            Some(length) => 1 + length,
            None => return false,
        }
    };
    // 结尾的 `\s*$`。
    rest[matched..].bytes().all(is_space_byte)
}

fn html_block_ends(end: HtmlBlockEnd, text: &str) -> bool {
    match end {
        HtmlBlockEnd::RawTextClose => ["</script>", "</pre>", "</style>", "</textarea>"]
            .iter()
            .any(|needle| contains_ignore_ascii_case(text, needle)),
        HtmlBlockEnd::CommentClose => text.contains("-->"),
        HtmlBlockEnd::ProcessingInstructionClose => text.contains("?>"),
        HtmlBlockEnd::AngleClose => text.contains('>'),
        HtmlBlockEnd::CdataClose => text.contains("]]>"),
        HtmlBlockEnd::BlankLine => text.bytes().all(|byte| byte == b' ' || byte == b'\t'),
    }
}

fn contains_ignore_ascii_case(haystack: &str, needle: &str) -> bool {
    let haystack = haystack.as_bytes();
    let needle_bytes = needle.as_bytes();
    haystack
        .windows(needle_bytes.len())
        .any(|window| window.eq_ignore_ascii_case(needle_bytes))
}

/// 列表项内容的缩进列号。
fn get_list_indent(line: &Line, pos: usize) -> usize {
    let indent_after = line.count_indent(pos, line.pos, line.indent);
    let indented = line.count_indent(skip_space(&line.text, pos), pos, indent_after);
    // 两种情况下内容缩进都按「标记后 1 列」算：
    //
    // - 标记后超过 4 个空格：多出来的部分属于内容（那是个缩进代码块）；
    // - 标记后到行尾只有空白：规范说「列表项以空行开头时，标记后的空格数不
    //   影响所需缩进」，也就是 W+1。上游漏了这一条，于是 `-` 或 `-   ` 单独
    //   一行时内容缩进算错，随后的缩进代码块与围栏内容都会跟着错
    //   （规范用例 #278 / #279）。
    let blank_after_marker = skip_space(&line.text, pos) >= line.text.len();
    if indented >= indent_after + 5 || blank_after_marker {
        indent_after + 1
    } else {
        indented
    }
}

/// 把相邻的 `CodeText` 合并，避免每行产生一个节点。
fn add_code_text(marks: &mut Vec<Element>, from: u32, to: u32) {
    if let Some(last) = marks.last_mut()
        && last.to == from
        && last.kind == NodeKind::CodeText
    {
        last.to = to;
        return;
    }
    marks.push(Element::leaf(NodeKind::CodeText, from, to));
}

// ---------------------------------------------------------------------------
// BlockContext
// ---------------------------------------------------------------------------

/// 块级解析的状态机。
pub(crate) struct BlockContext<'a, I: Input + ?Sized> {
    input: &'a I,
    stack: Vec<CompositeBlock>,
    line: Line,
    at_end: bool,
    to: u32,
    /// 当前行起点（文档偏移）。
    line_start: u32,
    /// 当前行的换行符位置，或文档末尾。
    line_end: u32,
    fragments: Option<crate::fragment::FragmentCursor<'a>>,
    /// 本次解析实际重新扫描过的字节数，供不变量 J1 的上界断言使用。
    reparsed_bytes: u32,
}

impl<'a, I: Input + ?Sized> BlockContext<'a, I> {
    pub(crate) fn new(
        input: &'a I,
        fragments: Option<crate::fragment::FragmentCursor<'a>>,
    ) -> Self {
        let to = input.len_bytes();
        let mut context = Self {
            input,
            stack: vec![CompositeBlock::create(NodeKind::Document, 0, 0, 0, 0)],
            line: Line::new(),
            at_end: false,
            to,
            line_start: 0,
            line_end: 0,
            fragments,
            reparsed_bytes: 0,
        };
        context.read_line();
        context
    }

    fn block(&self) -> &CompositeBlock {
        self.stack.last().expect("Document 永远在栈底")
    }

    fn block_mut(&mut self) -> &mut CompositeBlock {
        self.stack.last_mut().expect("Document 永远在栈底")
    }

    /// 上一行的结束位置。
    fn prev_line_end(&self) -> u32 {
        if self.at_end {
            self.line_start
        } else {
            self.line_start.saturating_sub(1)
        }
    }

    /// 读入下一行。返回 `false` 表示已经到文档末尾。
    fn next_line(&mut self) -> bool {
        self.line_start += u32::try_from(self.line.len()).unwrap_or(0);
        if self.line_end >= self.to {
            self.line_start = self.line_end;
            self.at_end = true;
            self.read_line();
            false
        } else {
            self.line_start += 1;
            self.read_line();
            true
        }
    }

    /// 把 `line_start` 起的一行读进 `self.line`，并跳过所有仍然成立的容器标记。
    fn read_line(&mut self) {
        let mut text = std::mem::take(&mut self.line.text);
        text.clear();
        self.input.read_line_into(self.line_start, &mut text);
        self.line_end = self.line_start + u32::try_from(text.len()).unwrap_or(0);
        self.reparsed_bytes = self
            .reparsed_bytes
            .saturating_add(u32::try_from(text.len()).unwrap_or(0));
        self.line.text = text;
        self.line.reset();

        while self.line.depth < self.stack.len() {
            let depth = self.line.depth;
            if !self.skip_context_markup(depth) {
                break;
            }
            self.line.forward();
            self.line.depth += 1;
        }
    }

    /// 第 `depth` 层容器在本行是否继续，并顺带记录它的标记。
    ///
    /// 对应上游的 `DefaultSkipMarkup`。
    fn skip_context_markup(&mut self, depth: usize) -> bool {
        let kind = self.stack[depth].kind;
        match kind {
            NodeKind::Document => true,
            NodeKind::Blockquote => {
                if self.line.next != Some(b'>') {
                    return false;
                }
                let pos = self.line.pos;
                let mark_from = self.line_start + u32::try_from(pos).unwrap_or(0);
                self.line.markers.push(Element::leaf(
                    NodeKind::QuoteMark,
                    mark_from,
                    mark_from + 1,
                ));
                let after_space = self
                    .line
                    .text
                    .as_bytes()
                    .get(pos + 1)
                    .copied()
                    .is_some_and(is_space_byte);
                self.line.move_base(pos + if after_space { 2 } else { 1 });
                self.stack[depth].end =
                    self.line_start + u32::try_from(self.line.len()).unwrap_or(0);
                true
            }
            NodeKind::ListItem => {
                match self.stack[depth].blank_start {
                    BlankStart::Exhausted => return false,
                    BlankStart::Pending => {
                        let blank_line = self.line.pos == self.line.len();
                        self.stack[depth].blank_start = if blank_line {
                            BlankStart::Exhausted
                        } else {
                            BlankStart::No
                        };
                        if blank_line {
                            return false;
                        }
                    }
                    BlankStart::No => {}
                }
                let value = usize::try_from(self.stack[depth].value).unwrap_or(0);
                if self.line.indent < self.line.base_indent + value && self.line.next.is_some() {
                    return false;
                }
                self.line.move_base_column(self.line.base_indent + value);
                true
            }
            NodeKind::BulletList | NodeKind::OrderedList => self.skip_for_list(depth),
            other => unreachable!("{} 不是容器块，不该进入容器栈", other.name()),
        }
    }

    fn skip_for_list(&mut self, depth: usize) -> bool {
        // 空行、或者缩进已经深到属于列表项内容，列表都继续。
        let innermost = depth + 1 == self.stack.len();
        if self.line.pos == self.line.len() {
            return true;
        }
        if !innermost {
            let inner_value = usize::try_from(self.stack[depth + 1].value).unwrap_or(0);
            if self.line.indent >= inner_value + self.line.base_indent {
                return true;
            }
        }
        if self.line.indent >= self.line.base_indent + 4 {
            return false;
        }
        let kind = self.stack[depth].kind;
        let size = if kind == NodeKind::OrderedList {
            is_ordered_list(&self.line, &self.stack, false)
        } else {
            is_bullet_list(&self.line, &self.stack, false)
        };
        let Some(size) = size else {
            return false;
        };
        // `- - -` 是分隔线而不是嵌套列表。
        if kind == NodeKind::BulletList && is_horizontal_rule(&self.line, self.stack.len(), false) {
            return false;
        }
        // 同一个列表必须用同一个标记字符。
        let marker = self
            .line
            .text
            .as_bytes()
            .get(self.line.pos + size - 1)
            .copied();
        marker.is_some_and(|byte| u32::from(byte) == self.stack[depth].value)
    }

    // -- 树的组装 ---------------------------------------------------------

    fn start_context(&mut self, kind: NodeKind, start: usize, value: u32) {
        let parent_hash = self.block().hash;
        let from = self.line_start + u32::try_from(start).unwrap_or(0);
        let end = self.line_start + u32::try_from(self.line.len()).unwrap_or(0);
        self.stack
            .push(CompositeBlock::create(kind, value, from, parent_hash, end));
    }

    fn finish_context(&mut self) {
        let block = self.stack.pop().expect("Document 不会被 finish");
        let from = block.from;
        let end = block.end;
        let tree = block.into_tree(end);
        self.block_mut().add_child(tree, from);
    }

    fn add_leaf_node(&mut self, kind: NodeKind, from: u32, to: u32) {
        let tree = Tree::leaf(kind, to.saturating_sub(from), 0);
        self.block_mut().add_child(tree, from);
    }

    fn add_tree(&mut self, tree: Tree, from: u32) {
        self.block_mut().add_child(tree, from);
    }

    /// 供增量复用直接塞入一棵现成子树。
    pub(crate) fn reuse_tree(&mut self, tree: Tree, from: u32) {
        self.add_tree(tree, from);
    }

    pub(crate) fn block_children_len(&self) -> usize {
        self.block().children.len()
    }

    pub(crate) fn truncate_block_children(&mut self, len: usize) {
        let block = self.block_mut();
        block.children.truncate(len);
        block.positions.truncate(len);
    }

    pub(crate) fn block_hash(&self) -> u32 {
        self.block().hash
    }

    fn finish(mut self) -> (Tree, u32) {
        while self.stack.len() > 1 {
            self.finish_context();
        }
        let line_start = self.line_start;
        let reparsed = self.reparsed_bytes;
        let document = self.stack.pop().expect("Document 永远在栈底");
        (document.into_tree(line_start), reparsed)
    }

    // -- 主循环 -----------------------------------------------------------

    pub(crate) fn parse(mut self) -> (Tree, u32) {
        while !self.advance() {}
        self.finish()
    }

    /// 前进一个块。返回 `true` 表示已经到文档末尾。
    fn advance(&mut self) -> bool {
        loop {
            // 关掉本行已经不再继续的容器，并把它们的标记写进树。
            let mut mark_index = 0_usize;
            loop {
                let next_end = (self.line.depth < self.stack.len()).then(|| self.block().end);
                while mark_index < self.line.markers.len()
                    && next_end.is_none_or(|end| self.line.markers[mark_index].from < end)
                {
                    let mark = self.line.markers[mark_index].clone();
                    mark_index += 1;
                    self.add_leaf_node(mark.kind, mark.from, mark.to);
                }
                if next_end.is_none() {
                    break;
                }
                self.finish_context();
            }
            if self.line.pos < self.line.len() {
                break;
            }
            // 空行。
            if !self.next_line() {
                return true;
            }
        }

        if self.try_reuse_fragment() {
            return false;
        }

        'restart: loop {
            for index in 0..BLOCK_PARSER_COUNT {
                match self.run_block_parser(index) {
                    BlockResult::NotApplicable => {}
                    BlockResult::Consumed => return false,
                    BlockResult::Opened => {
                        self.line.forward();
                        continue 'restart;
                    }
                }
            }
            break;
        }

        // 容器标记独占一行（`-` 或 `>` 后面什么都没有）时不开段落：树里会多出
        // 一个零长度的 Paragraph，而后续行会被并进它，`-\n      baz` 里的
        // 缩进代码块就此丢失（规范用例 #278）。上游在这里没有这道检查。
        if self.line.pos >= self.line.len() {
            return !self.next_line();
        }

        self.parse_leaf_block();
        false
    }

    fn try_reuse_fragment(&mut self) -> bool {
        let Some(mut fragments) = self.fragments.take() else {
            return false;
        };
        let pos = self.line_start + u32::try_from(self.line.base_pos).unwrap_or(0);
        let line_start = self.line_start;
        let taken = fragments.try_take(self, pos, line_start);
        self.fragments = Some(fragments);
        let Some(taken) = taken else {
            return false;
        };
        self.line_start += taken;
        if self.line_start < self.to {
            self.line_start += 1;
        } else {
            self.at_end = true;
        }
        self.read_line();
        true
    }

    /// 供 [`crate::fragment`] 回溯行边界用。
    pub(crate) fn input_byte_at(&self, pos: u32) -> Option<u8> {
        self.input.byte_at(pos)
    }

    /// 段落式的 leaf block：一直吃到被空行或别的构造打断为止。
    fn parse_leaf_block(&mut self) {
        let start = self.line_start + u32::try_from(self.line.pos).unwrap_or(0);
        let mut leaf = LeafBlock {
            start,
            content: self.line.text[self.line.pos..].to_owned(),
            marks: Vec::new(),
        };
        let mut parsers = Vec::new();
        if leaf.content.as_bytes().first() == Some(&b'[') {
            parsers.push(LeafParser::LinkReference(LinkReferenceParser::new(&leaf)));
        }
        parsers.push(LeafParser::SetextHeading);

        while self.next_line() {
            if self.line.pos == self.line.len() {
                break;
            }
            if self.line.indent < self.line.base_indent + 4 && self.ends_leaf_block() {
                break;
            }
            let mut finished = false;
            for parser in &mut parsers {
                if self.leaf_next_line(parser, &mut leaf) {
                    finished = true;
                    break;
                }
            }
            if finished {
                return;
            }
            leaf.content.push('\n');
            leaf.content.push_str(&self.line.scrub());
            leaf.marks.extend(self.line.markers.iter().cloned());
        }
        self.finish_leaf(&mut parsers, leaf);
    }

    fn finish_leaf(&mut self, parsers: &mut [LeafParser], leaf: LeafBlock) {
        for parser in parsers.iter_mut() {
            if self.leaf_finish(parser, &leaf) {
                return;
            }
        }
        let inline = inject_marks(parse_inline(&leaf.content, leaf.start), leaf.marks);
        let to = leaf.start + u32::try_from(leaf.content.len()).unwrap_or(0);
        let tree = wrap(NodeKind::Paragraph, leaf.start, to, inline, 0);
        self.add_tree(tree, leaf.start);
    }

    /// 把一个 leaf 解析器产出的元素接进树，顺带把容器标记插回去。
    fn add_leaf_element(&mut self, leaf: &LeafBlock, element: Element) {
        let from = element.from;
        let to = element.to;
        let kind = element.kind;
        let children = inject_marks(element.children, leaf.marks.clone());
        let tree = wrap(kind, from, to, children, 0);
        self.add_tree(tree, from);
    }
}

/// 块解析器的个数，见 [`BlockContext::run_block_parser`]。
const BLOCK_PARSER_COUNT: usize = 8;

/// leaf block 的观察者。段落在被完全读入之前，有可能变成别的东西。
enum LeafParser {
    LinkReference(LinkReferenceParser),
    SetextHeading,
}

impl<I: Input + ?Sized> BlockContext<'_, I> {
    /// 块解析器的固定顺序，对应上游 `DefaultBlockParsers` 的键顺序。
    ///
    /// 顺序即优先级，且**不能重排**：缩进代码块必须先于列表判断，否则
    /// 深缩进的列表标记会被当成代码；分隔线必须先于无序列表，否则 `- - -`
    /// 会被当成嵌套列表。
    ///
    /// 上游把这些放在一个可配置的数组里（`configure` / `before` / `after`），
    /// 那套机制是给扩展用的，S6 落地 `yu-markdown` 的 extension 时再建；
    /// 现在建它只会得到一份没有使用者、也没有测试的抽象。
    fn run_block_parser(&mut self, index: usize) -> BlockResult {
        match index {
            0 => self.parse_indented_code(),
            1 => self.parse_fenced_code(),
            2 => self.parse_blockquote(),
            3 => self.parse_horizontal_rule(),
            4 => self.parse_bullet_list(),
            5 => self.parse_ordered_list(),
            6 => self.parse_atx_heading(),
            7 => self.parse_html_block(),
            _ => BlockResult::NotApplicable,
        }
    }

    /// 能打断段落的构造。对应上游的 `DefaultEndLeaf`。
    fn ends_leaf_block(&self) -> bool {
        is_atx_heading(&self.line).is_some()
            || is_fenced_code(&self.line).is_some()
            || is_blockquote(&self.line).is_some()
            || is_bullet_list(&self.line, &self.stack, true).is_some()
            || is_ordered_list(&self.line, &self.stack, true).is_some()
            || is_horizontal_rule(&self.line, self.stack.len(), true)
            || is_html_block(&self.line, true).is_some()
    }

    fn parse_indented_code(&mut self) -> BlockResult {
        let base = self.line.base_indent + 4;
        if self.line.indent < base {
            return BlockResult::NotApplicable;
        }
        let start = self.line.find_column(base);
        let from = self.line_start + u32::try_from(start).unwrap_or(0);
        let mut to = self.line_start + u32::try_from(self.line.len()).unwrap_or(0);
        let mut marks = Vec::new();
        let mut pending = Vec::new();
        add_code_text(&mut marks, from, to);

        while self.next_line() && self.line.depth >= self.stack.len() {
            if self.line.pos == self.line.len() {
                // 空行先攒着：它属于代码块，当且仅当后面还有缩进的代码行。
                add_code_text(&mut pending, self.line_start - 1, self.line_start);
                pending.extend(self.line.markers.iter().cloned());
            } else if self.line.indent < base {
                break;
            } else {
                for mark in pending.drain(..) {
                    if mark.kind == NodeKind::CodeText {
                        add_code_text(&mut marks, mark.from, mark.to);
                    } else {
                        marks.push(mark);
                    }
                }
                add_code_text(&mut marks, self.line_start - 1, self.line_start);
                marks.extend(self.line.markers.iter().cloned());
                to = self.line_start + u32::try_from(self.line.len()).unwrap_or(0);
                let code_start = self.line_start
                    + u32::try_from(self.line.find_column(self.line.base_indent + 4)).unwrap_or(0);
                if code_start < to {
                    add_code_text(&mut marks, code_start, to);
                }
            }
        }
        // 攒下的空行没有归宿，但它们携带的容器标记（`>`）还得进树。
        pending.retain(|mark| mark.kind != NodeKind::CodeText);
        if !pending.is_empty() {
            pending.append(&mut self.line.markers);
            self.line.markers = pending;
        }

        let tree = wrap(NodeKind::CodeBlock, from, to, marks, 0);
        self.add_tree(tree, from);
        BlockResult::Consumed
    }

    fn parse_fenced_code(&mut self) -> BlockResult {
        let Some(fence_end) = is_fenced_code(&self.line) else {
            return BlockResult::NotApplicable;
        };
        let from = self.line_start + u32::try_from(self.line.pos).unwrap_or(0);
        let fence_char = self.line.next.expect("is_fenced_code 已经确认有字符");
        let len = fence_end - self.line.pos;
        let info_from = skip_space(&self.line.text, fence_end);
        let info_to = skip_space_back(&self.line.text, self.line.len(), info_from);
        let mut marks = vec![Element::leaf(
            NodeKind::CodeMark,
            from,
            from + u32::try_from(len).unwrap_or(0),
        )];
        if info_from < info_to {
            marks.push(Element::leaf(
                NodeKind::CodeInfo,
                self.line_start + u32::try_from(info_from).unwrap_or(0),
                self.line_start + u32::try_from(info_to).unwrap_or(0),
            ));
        }

        let mut first = true;
        while self.next_line() && self.line.depth >= self.stack.len() {
            let mut index = self.line.pos;
            let bytes_len = self.line.len();
            // 上游写的是 `line.indent - line.baseIndent < 4`。JS 的减法允许负数，
            // Rust 的 usize 不允许——移项成加法，对所有取值都等价。
            if self.line.indent < self.line.base_indent + 4 {
                let bytes = self.line.text.as_bytes();
                while index < bytes_len && bytes[index] == fence_char {
                    index += 1;
                }
            }
            if index - self.line.pos >= len && skip_space(&self.line.text, index) == bytes_len {
                // 最后一个内容行与闭合围栏之间的换行属于代码正文。上游在这里
                // 直接跳出，那个换行不进任何节点——于是围栏代码的最后一行没有
                // 换行，而不闭合的围栏反而有（它多走了文档末尾那个空行）。
                if !first {
                    add_code_text(&mut marks, self.line_start - 1, self.line_start);
                }
                marks.extend(self.line.markers.iter().cloned());
                marks.push(Element::leaf(
                    NodeKind::CodeMark,
                    self.line_start + u32::try_from(self.line.pos).unwrap_or(0),
                    self.line_start + u32::try_from(index).unwrap_or(0),
                ));
                self.next_line();
                break;
            }
            if !first {
                add_code_text(&mut marks, self.line_start - 1, self.line_start);
            }
            marks.extend(self.line.markers.iter().cloned());
            let text_start = self.line_start + u32::try_from(self.line.base_pos).unwrap_or(0);
            let text_end = self.line_start + u32::try_from(bytes_len).unwrap_or(0);
            if text_start < text_end {
                add_code_text(&mut marks, text_start, text_end);
            }
            first = false;
        }

        let to = self.prev_line_end();
        let tree = wrap(NodeKind::FencedCode, from, to, marks, 0);
        self.add_tree(tree, from);
        BlockResult::Consumed
    }

    fn parse_blockquote(&mut self) -> BlockResult {
        let Some(size) = is_blockquote(&self.line) else {
            return BlockResult::NotApplicable;
        };
        let pos = self.line.pos;
        self.start_context(NodeKind::Blockquote, pos, 0);
        let mark_from = self.line_start + u32::try_from(pos).unwrap_or(0);
        self.add_leaf_node(NodeKind::QuoteMark, mark_from, mark_from + 1);
        self.line.move_base(pos + size);
        BlockResult::Opened
    }

    fn parse_horizontal_rule(&mut self) -> BlockResult {
        if !is_horizontal_rule(&self.line, self.stack.len(), false) {
            return BlockResult::NotApplicable;
        }
        let from = self.line_start + u32::try_from(self.line.pos).unwrap_or(0);
        self.next_line();
        let to = self.prev_line_end();
        self.add_leaf_node(NodeKind::HorizontalRule, from, to);
        BlockResult::Consumed
    }

    fn parse_bullet_list(&mut self) -> BlockResult {
        let Some(size) = is_bullet_list(&self.line, &self.stack, false) else {
            return BlockResult::NotApplicable;
        };
        self.open_list_item(
            NodeKind::BulletList,
            size,
            u32::from(self.line.next.unwrap_or(b'-')),
        )
    }

    fn parse_ordered_list(&mut self) -> BlockResult {
        let Some(size) = is_ordered_list(&self.line, &self.stack, false) else {
            return BlockResult::NotApplicable;
        };
        // 有序列表用「结束符」（`.` 或 `)`）作为身份，`1.` 与 `1)` 是两个列表。
        let delimiter = self
            .line
            .text
            .as_bytes()
            .get(self.line.pos + size - 1)
            .copied()
            .unwrap_or(b'.');
        self.open_list_item(NodeKind::OrderedList, size, u32::from(delimiter))
    }

    fn open_list_item(&mut self, list: NodeKind, size: usize, value: u32) -> BlockResult {
        if self.block().kind != list {
            let base_pos = self.line.base_pos;
            self.start_context(list, base_pos, value);
        }
        let new_base = get_list_indent(&self.line, self.line.pos + size);
        let base_pos = self.line.base_pos;
        let item_value = u32::try_from(new_base.saturating_sub(self.line.base_indent)).unwrap_or(0);
        let starts_blank = skip_space(&self.line.text, self.line.pos + size) >= self.line.len();
        self.start_context(NodeKind::ListItem, base_pos, item_value);
        if starts_blank {
            self.block_mut().blank_start = BlankStart::Pending;
        }
        let mark_from = self.line_start + u32::try_from(self.line.pos).unwrap_or(0);
        self.add_leaf_node(
            NodeKind::ListMark,
            mark_from,
            mark_from + u32::try_from(size).unwrap_or(0),
        );
        self.line.move_base_column(new_base);
        BlockResult::Opened
    }

    fn parse_atx_heading(&mut self) -> BlockResult {
        let Some(size) = is_atx_heading(&self.line) else {
            return BlockResult::NotApplicable;
        };
        let hash = self.line.next.unwrap_or(b'#');
        let off = self.line.pos;
        let from = self.line_start + u32::try_from(off).unwrap_or(0);
        let text_len = self.line.len();
        let end_of_space = skip_space_back(&self.line.text, text_len, off);
        let bytes = self.line.text.as_bytes();
        let mut after = end_of_space;
        while after > off && bytes[after - 1] == hash {
            after -= 1;
        }
        // 收尾的 `#` 串只有在前面有空白、且不是整行都是 `#` 时才算标记。
        if after == end_of_space || after == off || !is_space_byte(bytes[after - 1]) {
            after = text_len;
        }

        // `#` 与正文之间那一个空白字符不属于正文。
        let content_start = (off + size + 1).min(text_len);
        let content_end = after.max(content_start);
        let mut children = vec![Element::leaf(
            NodeKind::HeaderMark,
            from,
            from + u32::try_from(size).unwrap_or(0),
        )];
        children.extend(parse_inline(
            &self.line.text[content_start..content_end],
            self.line_start + u32::try_from(content_start).unwrap_or(0),
        ));
        if after < text_len {
            children.push(Element::leaf(
                NodeKind::HeaderMark,
                self.line_start + u32::try_from(after).unwrap_or(0),
                self.line_start + u32::try_from(end_of_space).unwrap_or(0),
            ));
        }
        let to = self.line_start + u32::try_from(text_len).unwrap_or(0);
        let kind = NodeKind::atx_heading(u8::try_from(size).unwrap_or(1))
            .expect("is_atx_heading 只返回 1..=6");
        let tree = wrap(kind, from, to, children, 0);
        self.next_line();
        self.add_tree(tree, from);
        BlockResult::Consumed
    }

    fn parse_html_block(&mut self) -> BlockResult {
        let Some(condition) = is_html_block(&self.line, false) else {
            return BlockResult::NotApplicable;
        };
        let from = self.line_start + u32::try_from(self.line.pos).unwrap_or(0);
        let end = html_block_end(condition);
        let mut marks = Vec::new();
        // 空行结束的两种条件里，空行本身不属于这个块，所以不吃掉它。
        let mut consumes_terminator = end != HtmlBlockEnd::BlankLine;
        while !html_block_ends(end, &self.line.text) {
            if !self.next_line() {
                break;
            }
            if self.line.depth < self.stack.len() {
                consumes_terminator = false;
                break;
            }
            marks.extend(self.line.markers.iter().cloned());
        }
        if consumes_terminator {
            self.next_line();
        }
        let kind = match end {
            HtmlBlockEnd::CommentClose => NodeKind::CommentBlock,
            HtmlBlockEnd::ProcessingInstructionClose => NodeKind::ProcessingInstructionBlock,
            _ => NodeKind::HtmlBlock,
        };
        let to = self.prev_line_end();
        let tree = wrap(kind, from, to, marks, 0);
        self.add_tree(tree, from);
        BlockResult::Consumed
    }

    // -- leaf 解析器 ------------------------------------------------------

    fn leaf_next_line(&mut self, parser: &mut LeafParser, leaf: &mut LeafBlock) -> bool {
        match parser {
            LeafParser::LinkReference(state) => {
                if state.stage == RefStage::Failed {
                    return false;
                }
                let mut content = leaf.content.clone();
                content.push('\n');
                content.push_str(&self.line.scrub());
                let finish = state.advance(&content);
                let Some(finish) = finish else { return false };
                if finish >= content.len() {
                    return false;
                }
                let element = state.complete(finish);
                self.add_leaf_element(leaf, element);
                true
            }
            LeafParser::SetextHeading => {
                if self.line.depth < self.stack.len() {
                    return false;
                }
                let Some(underline) = is_setext_underline(&self.line) else {
                    return false;
                };
                let level_one = self.line.next == Some(b'=');
                let mark = Element::leaf(
                    NodeKind::HeaderMark,
                    self.line_start + u32::try_from(self.line.pos).unwrap_or(0),
                    self.line_start + u32::try_from(underline).unwrap_or(0),
                );
                self.next_line();
                let mut children = parse_inline(&leaf.content, leaf.start);
                children.push(mark);
                let kind = if level_one {
                    NodeKind::SetextHeading1
                } else {
                    NodeKind::SetextHeading2
                };
                let element = Element::new(kind, leaf.start, self.prev_line_end(), children);
                self.add_leaf_element(leaf, element);
                true
            }
        }
    }

    fn leaf_finish(&mut self, parser: &mut LeafParser, leaf: &LeafBlock) -> bool {
        match parser {
            LeafParser::LinkReference(state) => {
                if !matches!(state.stage, RefStage::Link | RefStage::Title) {
                    return false;
                }
                let pos = usize::try_from(state.pos).unwrap_or(usize::MAX);
                if skip_space(&leaf.content, pos.min(leaf.content.len())) != leaf.content.len() {
                    return false;
                }
                let element = state.complete(leaf.content.len());
                self.add_leaf_element(leaf, element);
                true
            }
            LeafParser::SetextHeading => false,
        }
    }
}

// ---------------------------------------------------------------------------
// 引用定义的增量状态机
// ---------------------------------------------------------------------------

/// `[label]: url "title"` 的解析进度。
///
/// 引用定义可以跨行，而块解析是单遍不回溯的，所以不能「读完再判断」。
/// 这个状态机每来一行就试着往前推一格，推不动就说明定义在上一行结束。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RefStage {
    Failed,
    Start,
    Label,
    Link,
    Title,
}

struct LinkReferenceParser {
    stage: RefStage,
    elements: Vec<Element>,
    /// 已消费到的位置，相对 `start`。
    pos: u32,
    start: u32,
}

impl LinkReferenceParser {
    fn new(leaf: &LeafBlock) -> Self {
        let mut parser = Self {
            stage: RefStage::Start,
            elements: Vec::new(),
            pos: 0,
            start: leaf.start,
        };
        parser.advance(&leaf.content);
        parser
    }

    /// 推进状态机。返回引用定义的结束位置（相对 `content` 起点），
    /// `None` 表示还没有定论或已经失败。
    fn advance(&mut self, content: &str) -> Option<usize> {
        loop {
            match self.stage {
                RefStage::Failed => return None,
                RefStage::Start => {
                    if !self.next_stage(parse_link_label(content, self.pos, self.start, true)) {
                        return None;
                    }
                    let pos = usize::try_from(self.pos).unwrap_or(usize::MAX);
                    if content.as_bytes().get(pos) != Some(&b':') {
                        self.stage = RefStage::Failed;
                        return None;
                    }
                    self.elements.push(Element::leaf(
                        NodeKind::LinkMark,
                        self.pos + self.start,
                        self.pos + self.start + 1,
                    ));
                    self.pos += 1;
                }
                RefStage::Label => {
                    let from = skip_space(content, usize::try_from(self.pos).unwrap_or(0));
                    let from = u32::try_from(from).unwrap_or(u32::MAX);
                    if !self.next_stage(parse_url(content, from, self.start)) {
                        return None;
                    }
                }
                RefStage::Link => {
                    let pos = usize::try_from(self.pos).unwrap_or(0);
                    let skip = skip_space(content, pos);
                    let mut end = 0_usize;
                    if skip > pos
                        && let Scan::Found(title) = parse_link_title(
                            content,
                            u32::try_from(skip).unwrap_or(u32::MAX),
                            self.start,
                        )
                    {
                        let title_end =
                            usize::try_from(title.to.saturating_sub(self.start)).unwrap_or(0);
                        if let Some(title_end) = line_end(content, title_end)
                            && title_end > 0
                        {
                            self.next_stage(Scan::Found(title));
                            end = title_end;
                        }
                    }
                    if end == 0 {
                        end = line_end(content, pos)?;
                    }
                    return (end > 0 && end < content.len()).then_some(end);
                }
                RefStage::Title => {
                    return line_end(content, usize::try_from(self.pos).unwrap_or(0));
                }
            }
        }
    }

    fn next_stage(&mut self, scan: Scan) -> bool {
        match scan {
            Scan::Found(element) => {
                self.pos = element.to.saturating_sub(self.start);
                self.elements.push(element);
                self.stage = match self.stage {
                    RefStage::Start => RefStage::Label,
                    RefStage::Label => RefStage::Link,
                    RefStage::Link | RefStage::Title => RefStage::Title,
                    RefStage::Failed => RefStage::Failed,
                };
                true
            }
            Scan::Failed => {
                self.stage = RefStage::Failed;
                false
            }
            // 到了输入末尾还没定论：后续行可能补上。
            Scan::Incomplete => false,
        }
    }

    fn complete(&self, len: usize) -> Element {
        Element::new(
            NodeKind::LinkReference,
            self.start,
            self.start + u32::try_from(len).unwrap_or(0),
            self.elements.clone(),
        )
    }
}

/// 从 `pos` 到行尾之间只有空白时返回换行符（或文本末尾）的位置，否则 `None`。
fn line_end(text: &str, pos: usize) -> Option<usize> {
    let bytes = text.as_bytes();
    let mut pos = pos;
    while pos < bytes.len() {
        if bytes[pos] == b'\n' {
            break;
        }
        if !is_space_byte(bytes[pos]) {
            return None;
        }
        pos += 1;
    }
    Some(pos)
}
