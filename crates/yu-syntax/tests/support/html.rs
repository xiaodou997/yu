//! 把 `yu-syntax` 的语法树渲染成 CommonMark 规范格式的 HTML。
//!
//! # 这份代码只属于测试
//!
//! Yu 的产品链路**不渲染 HTML**：编辑器是 source projection，导出走 comrak
//! （overview-v2 第 6.1 节）。它存在的唯一理由是：CommonMark 的 652 条规范用例
//! 给的期望值是 HTML，不比 HTML 就没法用它们度量解析器。
//!
//! # 它顺带验证了什么
//!
//! 三件 S4 也要做的事，在这里先做了一遍：
//!
//! 1. **gap 就是正文。** 树里没有「文本节点」，相邻节点之间的空隙才是文字。
//!    渲染必须能从 position 精确推导出这些空隙——这正是不变量 C2 对
//!    「lossless」的定义，这份渲染器是它的第一个真实消费者。
//! 2. **引用链接由文档级的表判定，不由 parser 判定**（不变量 C6）。
//!    [`Html::collect_references`] 先扫一遍全树建表，渲染时再查；
//!    parser 只给了候选。S4 的 reference table facet 是同一个形状。
//! 3. **紧凑/松散列表不在树里。** 与上游 lezer 一致，树不记这件事，
//!    它由块之间有没有空行推导得出。
//!
//! # 它不验证什么
//!
//! 这里的实体解码、URL 百分号编码、HTML 转义都是 CommonMark 的**渲染**规则，
//! 与解析无关。它们出错会表现为 spec 用例失败，因此每一条失败都必须先分清
//! 是解析错了还是这里错了，再决定是修还是登记进不变量第 F 节。

use std::collections::HashMap;

use percent_encoding::{AsciiSet, CONTROLS, percent_encode};
use yu_syntax::{NodeKind, Tree};

/// 一条引用定义。
struct Reference {
    destination: String,
    title: Option<String>,
}

pub fn render(source: &str, tree: &Tree) -> String {
    let mut renderer = Html {
        source,
        references: HashMap::new(),
        out: String::new(),
        at_line_start: true,
        pending: String::new(),
        block_empty: true,
    };
    renderer.collect_references(tree, 0);
    renderer.block_children(tree, 0, false);
    renderer.out
}

struct Html<'a> {
    source: &'a str,
    references: HashMap<String, Reference>,
    out: String,
    /// 输出是否停在一个视觉行的开头。
    ///
    /// 必须是渲染器的状态而不是 [`Html::push_text`] 的局部变量：一个段落的
    /// 正文会被 QuoteMark 之类的标记节点切成几段 gap，软换行两侧的空白要被
    /// 去掉，而「刚过完一个换行」这件事得跨过中间那个标记节点。
    at_line_start: bool,
    /// 还没决定要不要输出的空白。
    ///
    /// 软换行前后的空格、块首块尾的空白都要去掉，但**只去掉源码里的空白**：
    /// `&#9;foo` 解码出来的制表符是正文，不能一起修剪。所以修剪不能在渲染
    /// 完成后对结果串做，只能在这里、对来自 gap 的字符做。
    pending: String,
    /// 当前块还没输出过任何正文。
    block_empty: bool,
}

/// 树里的一个节点及其绝对范围。
#[derive(Clone, Copy)]
struct Node<'a> {
    tree: &'a Tree,
    from: usize,
    to: usize,
}

impl<'a> Node<'a> {
    fn kind(self) -> NodeKind {
        self.tree.kind()
    }

    fn children(self) -> Vec<Node<'a>> {
        (0..self.tree.child_count())
            .filter_map(|index| {
                let (child, position) = self.tree.child(index)?;
                let from = self.from + position as usize;
                Some(Node {
                    tree: child,
                    from,
                    to: from + child.len_bytes() as usize,
                })
            })
            .collect()
    }
}

fn root(tree: &Tree, from: usize) -> Node<'_> {
    Node {
        tree,
        from,
        to: from + tree.len_bytes() as usize,
    }
}

impl<'a> Html<'a> {
    fn text(&self, from: usize, to: usize) -> &'a str {
        let from = from.min(self.source.len());
        let to = to.clamp(from, self.source.len());
        self.source.get(from..to).unwrap_or("")
    }

    // -- 引用定义 --------------------------------------------------------

    fn collect_references(&mut self, tree: &Tree, from: usize) {
        let node = root(tree, from);
        if node.kind() == NodeKind::LinkReference {
            self.record_reference(node);
            return;
        }
        for child in node.children() {
            self.collect_references(child.tree, child.from);
        }
    }

    fn record_reference(&mut self, node: Node<'_>) {
        let mut label = None;
        let mut destination = None;
        let mut title = None;
        for child in node.children() {
            match child.kind() {
                NodeKind::LinkLabel => {
                    label = Some(normalize_label(self.text(child.from + 1, child.to - 1)));
                }
                NodeKind::Url => destination = Some(self.destination_text(child)),
                NodeKind::LinkTitle => {
                    title = Some(unescape(self.text(child.from + 1, child.to - 1)));
                }
                _ => {}
            }
        }
        if let (Some(label), Some(destination)) = (label, destination)
            && !label.is_empty()
        {
            // 先定义的胜出，与规范一致。
            self.references
                .entry(label)
                .or_insert(Reference { destination, title });
        }
    }

    /// URL 节点的文本，去掉 `<>` 包裹并处理转义。
    fn destination_text(&self, node: Node<'_>) -> String {
        let raw = self.text(node.from, node.to);
        let inner = raw
            .strip_prefix('<')
            .and_then(|rest| rest.strip_suffix('>'))
            .unwrap_or(raw);
        unescape(inner)
    }

    // -- 块级 ------------------------------------------------------------

    fn block_children(&mut self, tree: &Tree, from: usize, tight: bool) {
        for child in root(tree, from).children() {
            self.block(child, tight);
        }
    }

    /// 换行归一：只有在输出不是行首时才补一个换行。
    ///
    /// 这是 cmark 的 `cr()`，整套块级换行都由它决定。照搬它而不是在每个分支
    /// 里手写换行，是因为「`<li>` 后面什么时候跟换行」的规则分散在紧凑/松散、
    /// 首个子块是不是段落、嵌套列表等好几个维度上，手写必然漏。
    fn cr(&mut self) {
        if !self.out.is_empty() && !self.out.ends_with('\n') {
            self.out.push('\n');
        }
    }

    fn block(&mut self, node: Node<'_>, tight: bool) {
        match node.kind() {
            NodeKind::Paragraph => {
                let body = self.render_inline_to_string(node);
                // 空段落不产出任何东西。树里会出现零长度的 Paragraph——
                // `>` 单独一行就是——但 CommonMark 的输出里没有 `<p></p>`。
                if body.is_empty() {
                    return;
                }
                // 紧凑列表里的段落不加 `<p>`。
                if tight {
                    self.out.push_str(&body);
                } else {
                    self.cr();
                    self.out.push_str("<p>");
                    self.out.push_str(&body);
                    self.out.push_str("</p>\n");
                }
            }
            NodeKind::AtxHeading1
            | NodeKind::AtxHeading2
            | NodeKind::AtxHeading3
            | NodeKind::AtxHeading4
            | NodeKind::AtxHeading5
            | NodeKind::AtxHeading6 => {
                let level = node.kind().atx_heading_level().unwrap_or(1);
                self.heading(node, level);
            }
            NodeKind::SetextHeading1 | NodeKind::SetextHeading2 => {
                let level = u8::from(node.kind() == NodeKind::SetextHeading2) + 1;
                self.heading(node, level);
            }
            NodeKind::Blockquote => {
                self.cr();
                self.out.push_str("<blockquote>\n");
                for child in node.children() {
                    if child.kind() == NodeKind::QuoteMark {
                        continue;
                    }
                    self.block(child, false);
                }
                self.cr();
                self.out.push_str("</blockquote>\n");
            }
            NodeKind::BulletList => self.list(node, "ul", None),
            NodeKind::OrderedList => {
                let start = self.ordered_list_start(node);
                self.list(node, "ol", start);
            }
            NodeKind::HorizontalRule => {
                self.cr();
                self.out.push_str("<hr />\n");
            }
            NodeKind::CodeBlock => {
                self.cr();
                self.out.push_str("<pre><code>");
                let body = self.indented_code_body(node);
                self.push_code_body(&body);
                self.out.push_str("</code></pre>\n");
            }
            NodeKind::FencedCode => self.fenced_code(node),
            NodeKind::HtmlBlock | NodeKind::CommentBlock | NodeKind::ProcessingInstructionBlock => {
                self.html_block(node);
            }
            // 引用定义不产出任何 HTML。
            NodeKind::LinkReference => {}
            _ => self.inline_children(node),
        }
    }

    /// 规范：标题正文去掉首尾空白。ATX 的 `#` 与正文之间的空格、setext
    /// 正文与下划线之间的换行都在这里被吃掉——它们在树里是 gap，是正文的一部分。
    fn heading(&mut self, node: Node<'_>, level: u8) {
        let body = self.render_inline_to_string(node);
        self.cr();
        self.out.push_str(&format!("<h{level}>"));
        self.out.push_str(&body);
        self.out.push_str(&format!("</h{level}>\n"));
    }

    /// 渲染一个块的全部行内内容，首尾空白按规范去掉。
    ///
    /// 段落与标题都适用：CommonMark 的输出里，两者的正文都不带首尾空白，
    /// 而树里的 gap 是原样的源码。
    fn render_inline_to_string(&mut self, node: Node<'_>) -> String {
        let start = self.out.len();
        self.at_line_start = true;
        self.block_empty = true;
        self.pending.clear();
        self.inline_range(node, node.from, node.to);
        // 块尾还挂着的空白一律丢弃。
        self.pending.clear();
        self.out.split_off(start)
    }

    /// 把攒着的空白落到输出里。块首的空白直接丢掉。
    fn flush_pending(&mut self) {
        if self.block_empty {
            self.pending.clear();
            return;
        }
        let pending = std::mem::take(&mut self.pending);
        self.out.push_str(&pending);
    }

    /// 一个会产出正文的行内节点即将输出：先结算待定空白，再复位行首状态。
    fn begin_content(&mut self) {
        self.flush_pending();
        self.at_line_start = false;
        self.block_empty = false;
    }

    /// HTML 块原样输出，但要还原两件树里没直说的事。
    ///
    /// - **行首缩进属于内容。** 节点从第一个非空白字符开始（块解析器就是这么
    ///   定位的），而规范保留最多 3 格缩进。
    /// - **容器标记不属于内容。** `> <div>` 里的 `> ` 是引用块的标记，
    ///   它在树里是 QuoteMark 子节点，输出时要跳过。
    fn html_block(&mut self, node: Node<'_>) {
        self.cr();
        let from = self.line_start_indent(node.from);
        let quote_marks: Vec<Node<'_>> = node
            .children()
            .into_iter()
            .filter(|child| child.kind() == NodeKind::QuoteMark)
            .collect();
        let mut raw = String::new();
        let mut cursor = from;
        for mark in quote_marks {
            if mark.from > cursor {
                raw.push_str(self.text(cursor, mark.from));
            }
            // 标记后面那一个空格也是标记的一部分。
            cursor = mark.to + usize::from(self.text(mark.to, mark.to + 1) == " ");
        }
        raw.push_str(self.text(cursor, node.to));
        self.out.push_str(raw.trim_end_matches('\n'));
        self.out.push('\n');
    }

    /// 把位置回退到本行的行首空白之前。
    fn line_start_indent(&self, from: usize) -> usize {
        let bytes = self.source.as_bytes();
        let mut start = from;
        while start > 0 && matches!(bytes[start - 1], b' ' | b'\t') {
            start -= 1;
        }
        // 只回退到行首为止；碰不到换行说明这是文档第一行。
        if start == 0 || bytes[start - 1] == b'\n' {
            start
        } else {
            from
        }
    }

    fn list(&mut self, node: Node<'_>, tag: &str, start: Option<u32>) {
        let loose = self.list_is_loose(node);
        self.cr();
        match start {
            Some(start) if start != 1 => {
                self.out.push_str(&format!("<{tag} start=\"{start}\">\n"));
            }
            _ => self.out.push_str(&format!("<{tag}>\n")),
        }
        for item in node.children() {
            if item.kind() != NodeKind::ListItem {
                continue;
            }
            self.list_item(item, !loose);
        }
        self.out.push_str(&format!("</{tag}>\n"));
    }

    fn list_item(&mut self, node: Node<'_>, tight: bool) {
        self.cr();
        self.out.push_str("<li>");
        for child in node.children() {
            if matches!(child.kind(), NodeKind::ListMark | NodeKind::QuoteMark) {
                continue;
            }
            self.block(child, tight);
        }
        self.out.push_str("</li>\n");
    }

    fn ordered_list_start(&self, node: Node<'_>) -> Option<u32> {
        let item = node
            .children()
            .into_iter()
            .find(|child| child.kind() == NodeKind::ListItem)?;
        let mark = item
            .children()
            .into_iter()
            .find(|child| child.kind() == NodeKind::ListMark)?;
        let text = self.text(mark.from, mark.to);
        text.trim_end_matches(['.', ')']).parse().ok()
    }

    /// 列表松散当且仅当它的任意两个相邻块之间隔着空行。
    ///
    /// 树里没有这个信息（与上游一致），只能回到源码上判断。
    fn list_is_loose(&self, node: Node<'_>) -> bool {
        let items: Vec<Node<'_>> = node
            .children()
            .into_iter()
            .filter(|child| child.kind() == NodeKind::ListItem)
            .collect();
        for pair in items.windows(2) {
            if self.gap_has_blank_line(pair[0].to, pair[1].from) {
                return true;
            }
        }
        items.iter().any(|item| {
            let blocks: Vec<Node<'_>> = item
                .children()
                .into_iter()
                .filter(|child| !matches!(child.kind(), NodeKind::ListMark | NodeKind::QuoteMark))
                .collect();
            blocks
                .windows(2)
                .any(|pair| self.gap_has_blank_line(pair[0].to, pair[1].from))
        })
    }

    /// `from..to` 之间是否夹着一个空行。
    ///
    /// 容器标记（`>`）算空白：`> - a\n>\n> - b` 是松散列表，中间那个 `>` 行
    /// 在语义上就是空行。
    fn gap_has_blank_line(&self, from: usize, to: usize) -> bool {
        let gap = self.text(from, to);
        let mut newlines = 0_usize;
        for byte in gap.bytes() {
            match byte {
                b'\n' => {
                    newlines += 1;
                    if newlines >= 2 {
                        return true;
                    }
                }
                b' ' | b'\t' | b'>' => {}
                _ => newlines = 0,
            }
        }
        false
    }

    /// 缩进代码块的正文。
    ///
    /// 树里 CodeText 之间会留下空隙：空行的空白没有被块解析器收进节点（它只
    /// 记了那一个换行），续行的缩进也在节点之外。这些空隙都是纯空白，把它们
    /// 填回去再按列剥掉缩进，就还原出规范要求的正文（规范用例 #112）。
    fn indented_code_body(&self, node: Node<'_>) -> String {
        let texts: Vec<Node<'_>> = node
            .children()
            .into_iter()
            .filter(|child| child.kind() == NodeKind::CodeText)
            .collect();
        let Some(first) = texts.first() else {
            return String::new();
        };
        // 第一行的缩进已经被块解析器剥掉了（CodeText 从内容列开始），
        // 只有被填回来的续行还带着完整缩进。
        let strip = self.column_within_line(first.from);
        let mut raw = String::new();
        let mut cursor = first.from;
        for text in &texts {
            if text.from > cursor {
                let gap = self.text(cursor, text.from);
                // 只填纯空白的空隙。带内容的空隙是容器标记，不属于代码。
                if gap.bytes().all(|byte| byte == b' ' || byte == b'\t') {
                    raw.push_str(gap);
                }
            }
            raw.push_str(self.text(text.from, text.to));
            cursor = text.to.max(cursor);
        }
        strip_columns_after_first_line(&raw, strip)
    }

    /// 围栏代码块的正文。
    ///
    /// 与缩进代码块不同，这里**不填空隙**：围栏内的续行空隙是容器缩进
    /// （列表项的 `  `），填回去会让正文整体右移。要剥掉的只有围栏自己相对
    /// 内容的那点缩进（规范用例 #131–#133）。
    fn fenced_code_body(&self, node: Node<'_>) -> String {
        let children = node.children();
        let texts: Vec<Node<'_>> = children
            .iter()
            .copied()
            .filter(|child| child.kind() == NodeKind::CodeText)
            .collect();
        let Some(first_text) = texts.first() else {
            return String::new();
        };
        let fence_column = children
            .iter()
            .find(|child| child.kind() == NodeKind::CodeMark)
            .map_or(0, |mark| self.column_within_line(mark.from));
        let strip = fence_column.saturating_sub(self.column_within_line(first_text.from));
        let raw: String = texts
            .iter()
            .map(|text| self.text(text.from, text.to))
            .collect();
        strip_columns(&raw, strip)
    }

    /// `offset` 在它所在行里的列号，制表符按 4 对齐。
    fn column_within_line(&self, offset: usize) -> usize {
        let bytes = self.source.as_bytes();
        let mut start = offset.min(bytes.len());
        while start > 0 && bytes[start - 1] != b'\n' {
            start -= 1;
        }
        let mut column = 0_usize;
        for &byte in &bytes[start..offset.min(bytes.len())] {
            if byte == b'\t' {
                column += 4 - column % 4;
            } else if byte & 0xC0 != 0x80 {
                column += 1;
            }
        }
        column
    }

    /// 输出代码正文：每一行都以换行结尾。
    fn push_code_body(&mut self, body: &str) {
        if body.is_empty() {
            return;
        }
        escape_html_into(body, &mut self.out);
        if !body.ends_with('\n') {
            self.out.push('\n');
        }
    }

    fn fenced_code(&mut self, node: Node<'_>) {
        self.cr();
        let info = node
            .children()
            .into_iter()
            .find(|child| child.kind() == NodeKind::CodeInfo)
            .map(|child| self.text(child.from, child.to));
        match info.and_then(|info| info.split_whitespace().next()) {
            Some(language) => {
                self.out.push_str("<pre><code class=\"language-");
                let language = unescape(language);
                escape_html_into(&language, &mut self.out);
                self.out.push_str("\">");
            }
            None => self.out.push_str("<pre><code>"),
        }
        let body = self.fenced_code_body(node);
        self.push_code_body(&body);
        self.out.push_str("</code></pre>\n");
    }

    // -- 行内 ------------------------------------------------------------

    fn inline_children(&mut self, node: Node<'_>) {
        let body = self.render_inline_to_string(node);
        self.out.push_str(&body);
    }

    /// 渲染 `node` 落在 `from..to` 里的子节点，以及它们之间的 gap 文本。
    ///
    /// 必须按范围裁剪：渲染链接的标签时 `from..to` 只是 `[` 与 `]` 之间那一段，
    /// 而 `node.children()` 还包含 URL、标题和它们之间的空格。不裁剪的话那个
    /// 空格会作为 gap 混进标签文本里——`<a>link </a>`。
    fn inline_range(&mut self, node: Node<'_>, from: usize, to: usize) {
        let mut cursor = from;
        for child in node.children() {
            if child.to <= from || child.from >= to {
                continue;
            }
            if child.from > cursor {
                let gap = self.text(cursor, child.from.min(to)).to_owned();
                self.push_text(&gap);
            }
            self.inline(child);
            cursor = child.to.max(cursor);
        }
        if cursor < to {
            let gap = self.text(cursor, to).to_owned();
            self.push_text(&gap);
        }
    }

    fn inline(&mut self, node: Node<'_>) {
        // 标记节点不产出内容，不能动空白状态；其余节点都要先结算待定空白。
        if !is_mark(node.kind())
            && !matches!(
                node.kind(),
                NodeKind::Url | NodeKind::LinkTitle | NodeKind::LinkLabel
            )
        {
            self.begin_content();
        }
        match node.kind() {
            NodeKind::Escape => {
                let text = self.text(node.from + 1, node.to).to_owned();
                escape_html_into(&text, &mut self.out);
            }
            NodeKind::Entity => {
                let raw = self.text(node.from, node.to);
                let decoded = decode_entity(raw).unwrap_or_else(|| raw.to_owned());
                escape_html_into(&decoded, &mut self.out);
            }
            NodeKind::HardBreak => {
                self.out.push_str("<br />\n");
                self.at_line_start = true;
            }
            NodeKind::Emphasis => {
                self.out.push_str("<em>");
                self.inline_marked(node);
                self.out.push_str("</em>");
            }
            NodeKind::StrongEmphasis => {
                self.out.push_str("<strong>");
                self.inline_marked(node);
                self.out.push_str("</strong>");
            }
            NodeKind::InlineCode => self.inline_code(node),
            NodeKind::HtmlTag | NodeKind::Comment | NodeKind::ProcessingInstruction => {
                let raw = self.text(node.from, node.to).to_owned();
                self.out.push_str(&raw);
            }
            NodeKind::Autolink => self.autolink(node),
            NodeKind::Link => self.link(node),
            NodeKind::Image => self.image(node),
            // 语法标记本身不产出内容。
            NodeKind::EmphasisMark
            | NodeKind::LinkMark
            | NodeKind::CodeMark
            | NodeKind::HeaderMark
            | NodeKind::QuoteMark
            | NodeKind::ListMark
            | NodeKind::Url
            | NodeKind::LinkTitle
            | NodeKind::LinkLabel => {}
            _ => self.inline_range(node, node.from, node.to),
        }
    }

    /// 渲染一个带标记的节点：跳过标记，正文照旧。
    fn inline_marked(&mut self, node: Node<'_>) {
        let children = node.children();
        let content_from = children
            .iter()
            .find(|child| !is_mark(child.kind()))
            .map_or(node.to, |child| child.from);
        let content_to = children
            .iter()
            .rev()
            .find(|child| !is_mark(child.kind()))
            .map_or(node.from, |child| child.to);
        // 标记之间的正文范围。
        let from = children
            .first()
            .filter(|child| is_mark(child.kind()))
            .map_or(node.from, |child| child.to)
            .min(content_from.max(node.from));
        let to = children
            .last()
            .filter(|child| is_mark(child.kind()))
            .map_or(node.to, |child| child.from)
            .max(content_to.min(node.to));
        self.inline_range(node, from, to);
    }

    fn inline_code(&mut self, node: Node<'_>) {
        let marks: Vec<Node<'_>> = node
            .children()
            .into_iter()
            .filter(|child| child.kind() == NodeKind::CodeMark)
            .collect();
        let (from, to) = match (marks.first(), marks.last()) {
            (Some(first), Some(last)) if marks.len() >= 2 => (first.to, last.from),
            _ => (node.from, node.to),
        };
        let raw = self.text(from, to);
        // 规范：代码跨行时换行变空格；首尾各恰好一个空格且不全是空格时剥掉。
        let collapsed: String = raw
            .chars()
            .map(|ch| if ch == '\n' { ' ' } else { ch })
            .collect();
        let stripped = if collapsed.starts_with(' ')
            && collapsed.ends_with(' ')
            && collapsed.chars().any(|ch| ch != ' ')
        {
            &collapsed[1..collapsed.len() - 1]
        } else {
            collapsed.as_str()
        };
        self.out.push_str("<code>");
        let stripped = stripped.to_owned();
        escape_html_into(&stripped, &mut self.out);
        self.out.push_str("</code>");
    }

    fn autolink(&mut self, node: Node<'_>) {
        let Some(url) = node
            .children()
            .into_iter()
            .find(|child| child.kind() == NodeKind::Url)
        else {
            return;
        };
        let text = self.text(url.from, url.to).to_owned();
        let destination = if text.contains('@') && !text.contains(':') {
            format!("mailto:{text}")
        } else {
            text.clone()
        };
        // 自动链接的目标里反斜杠不是转义符：`<https://x?find=\\*>` 的目标
        // 就是带反斜杠的那一串（规范用例 #20 / #603）。
        self.out.push_str("<a href=\"");
        self.out.push_str(&encode_href(&destination));
        self.out.push_str("\">");
        escape_html_into(&text, &mut self.out);
        self.out.push_str("</a>");
    }

    fn link(&mut self, node: Node<'_>) {
        let Some(target) = self.link_target(node) else {
            // 引用不成立：`[a][b]` 里的 `b` 没有定义。按普通文本渲染，
            // 这正是不变量 C6 说的「成立与否不由 parser 决定」。
            self.render_as_plain(node);
            return;
        };
        self.out.push_str("<a href=\"");
        self.out.push_str(&encode_href(&target.destination));
        self.out.push('"');
        if let Some(title) = &target.title {
            self.out.push_str(" title=\"");
            let title = title.clone();
            escape_html_into(&title, &mut self.out);
            self.out.push('"');
        }
        self.out.push('>');
        self.link_label_content(node);
        self.out.push_str("</a>");
    }

    fn image(&mut self, node: Node<'_>) {
        let Some(target) = self.link_target(node) else {
            self.render_as_plain(node);
            return;
        };
        self.out.push_str("<img src=\"");
        self.out.push_str(&encode_href(&target.destination));
        self.out.push_str("\" alt=\"");
        let alt = self.plain_text_of_label(node);
        escape_html_into(&alt, &mut self.out);
        self.out.push('"');
        if let Some(title) = &target.title {
            self.out.push_str(" title=\"");
            let title = title.clone();
            escape_html_into(&title, &mut self.out);
            self.out.push('"');
        }
        self.out.push_str(" />");
    }

    /// 链接的目标：行内式直接读，引用式查表。
    fn link_target(&self, node: Node<'_>) -> Option<Reference> {
        let children = node.children();
        // 行内式的标志是第三个 LinkMark 就是 `(`。不能用「有没有 URL 子节点」
        // 判断——`[link]()` 是合法的空目标行内链接，它没有 URL 节点。
        let marks: Vec<Node<'_>> = children
            .iter()
            .copied()
            .filter(|child| child.kind() == NodeKind::LinkMark)
            .collect();
        let is_inline = marks
            .get(2)
            .is_some_and(|mark| self.text(mark.from, mark.to) == "(");
        if is_inline {
            let destination = children
                .iter()
                .find(|child| child.kind() == NodeKind::Url)
                .map(|child| self.destination_text(*child))
                .unwrap_or_default();
            let title = children
                .iter()
                .find(|child| child.kind() == NodeKind::LinkTitle)
                .map(|child| unescape(self.text(child.from + 1, child.to - 1)));
            return Some(Reference { destination, title });
        }
        // 引用式：显式标签 `[text][label]`，或折叠/简写 `[label]`。
        let explicit = children
            .iter()
            .find(|child| child.kind() == NodeKind::LinkLabel)
            .map(|child| normalize_label(self.text(child.from + 1, child.to - 1)))
            .filter(|label| !label.is_empty());
        let label = explicit.unwrap_or_else(|| normalize_label(&self.label_source(node)));
        let found = self.references.get(&label)?;
        Some(Reference {
            destination: found.destination.clone(),
            title: found.title.clone(),
        })
    }

    /// `[` 与配对 `]` 之间的源码。
    fn label_source(&self, node: Node<'_>) -> String {
        let marks: Vec<Node<'_>> = node
            .children()
            .into_iter()
            .filter(|child| child.kind() == NodeKind::LinkMark)
            .collect();
        match (marks.first(), marks.get(1)) {
            (Some(open), Some(close)) => self.text(open.to, close.from).to_owned(),
            _ => String::new(),
        }
    }

    fn link_label_content(&mut self, node: Node<'_>) {
        let marks: Vec<Node<'_>> = node
            .children()
            .into_iter()
            .filter(|child| child.kind() == NodeKind::LinkMark)
            .collect();
        let (from, to) = match (marks.first(), marks.get(1)) {
            (Some(open), Some(close)) => (open.to, close.from),
            _ => (node.from, node.to),
        };
        self.inline_range(node, from, to);
    }

    /// 图片 alt 是标签的**字面文本**：行内标记去掉，嵌套图片取它自己的 alt。
    ///
    /// 不能拿渲染结果去标签——嵌套图片渲染成 `<img alt="bar" />`，
    /// 去完标签什么都不剩（规范用例 #574）。
    fn plain_text_of_label(&mut self, node: Node<'_>) -> String {
        let marks: Vec<Node<'_>> = node
            .children()
            .into_iter()
            .filter(|child| child.kind() == NodeKind::LinkMark)
            .collect();
        let (from, to) = match (marks.first(), marks.get(1)) {
            (Some(open), Some(close)) => (open.to, close.from),
            _ => (node.from, node.to),
        };
        let mut out = String::new();
        self.plain_text_range(node, from, to, &mut out);
        out
    }

    fn plain_text_range(&self, node: Node<'_>, from: usize, to: usize, out: &mut String) {
        let mut cursor = from;
        for child in node.children() {
            if child.to <= from || child.from >= to {
                continue;
            }
            if child.from > cursor {
                out.push_str(self.text(cursor, child.from.min(to)));
            }
            match child.kind() {
                NodeKind::Image | NodeKind::Link => {
                    out.push_str(&self.plain_text_of_label_inner(child));
                }
                NodeKind::Escape => out.push_str(self.text(child.from + 1, child.to)),
                NodeKind::Entity => {
                    let raw = self.text(child.from, child.to);
                    out.push_str(&decode_entity(raw).unwrap_or_else(|| raw.to_owned()));
                }
                kind if is_mark(kind) => {}
                NodeKind::Url | NodeKind::LinkTitle | NodeKind::LinkLabel => {}
                _ => self.plain_text_range(child, child.from, child.to, out),
            }
            cursor = child.to.max(cursor);
        }
        if cursor < to {
            out.push_str(self.text(cursor, to));
        }
    }

    fn plain_text_of_label_inner(&self, node: Node<'_>) -> String {
        let marks: Vec<Node<'_>> = node
            .children()
            .into_iter()
            .filter(|child| child.kind() == NodeKind::LinkMark)
            .collect();
        let (from, to) = match (marks.first(), marks.get(1)) {
            (Some(open), Some(close)) => (open.to, close.from),
            _ => (node.from, node.to),
        };
        let mut out = String::new();
        self.plain_text_range(node, from, to, &mut out);
        out
    }

    /// 引用不成立时按普通文本渲染：`[` 与 `]` 都照原样出现，中间的标签
    /// 内容仍然是行内内容（`[*a*][x]` 里的 `*a*` 还是强调）。
    fn render_as_plain(&mut self, node: Node<'_>) {
        let marks: Vec<Node<'_>> = node
            .children()
            .into_iter()
            .filter(|child| child.kind() == NodeKind::LinkMark)
            .collect();
        let (Some(open), Some(close)) = (marks.first(), marks.get(1)) else {
            let text = self.text(node.from, node.to).to_owned();
            self.push_text(&text);
            return;
        };
        let opening = self.text(node.from, open.to).to_owned();
        self.push_text(&opening);
        self.inline_range(node, open.to, close.from);
        // 尾部是 `][label]` 这样的原始源码，里面的反斜杠转义没有被 parser
        // 拆成节点（`finish_link` 直接吃掉了整个标签），这里补上（规范用例 #545）。
        let trailing = unescape(self.text(close.from, node.to));
        self.push_text(&trailing);
    }

    /// 输出一段正文：软换行两侧的空白按规范去掉。
    fn push_text(&mut self, text: &str) {
        for ch in text.chars() {
            match ch {
                // 续行开头的空白：整段丢掉。
                ' ' | '\t' if self.at_line_start => {}
                ' ' | '\t' => self.pending.push(ch),
                '\n' => {
                    // 换行前攒下的空格是行尾空白，丢掉。
                    self.pending.clear();
                    self.pending.push('\n');
                    self.at_line_start = true;
                }
                _ => {
                    self.begin_content();
                    escape_html_char(ch, &mut self.out);
                }
            }
        }
    }
}

fn is_mark(kind: NodeKind) -> bool {
    matches!(
        kind,
        NodeKind::EmphasisMark
            | NodeKind::LinkMark
            | NodeKind::CodeMark
            | NodeKind::HeaderMark
            | NodeKind::QuoteMark
            | NodeKind::ListMark
    )
}

// ---------------------------------------------------------------------------
// CommonMark 的渲染规则
// ---------------------------------------------------------------------------

fn escape_html_char(ch: char, out: &mut String) {
    match ch {
        '&' => out.push_str("&amp;"),
        '<' => out.push_str("&lt;"),
        '>' => out.push_str("&gt;"),
        '"' => out.push_str("&quot;"),
        other => out.push(other),
    }
}

fn escape_html_into(text: &str, out: &mut String) {
    for ch in text.chars() {
        escape_html_char(ch, out);
    }
}

/// cmark 的 `HREF_SAFE`：这些字符原样保留，其余百分号编码。
const HREF_UNSAFE: &AsciiSet = &CONTROLS
    .add(b' ')
    .add(b'"')
    .add(b'<')
    .add(b'>')
    .add(b'[')
    .add(b'\\')
    .add(b']')
    .add(b'^')
    .add(b'`')
    .add(b'{')
    .add(b'|')
    .add(b'}');

fn encode_href(url: &str) -> String {
    let encoded = percent_encode(url.as_bytes(), HREF_UNSAFE).to_string();
    let mut out = String::with_capacity(encoded.len());
    for ch in encoded.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '\'' => out.push_str("&#x27;"),
            other => out.push(other),
        }
    }
    out
}

/// 处理反斜杠转义与实体引用。用于链接目标、标题与代码块的 info string——
/// 这些位置的转义没有被 parser 拆成节点。
fn unescape(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = String::with_capacity(text.len());
    let mut index = 0_usize;
    while index < bytes.len() {
        match bytes[index] {
            b'\\' if index + 1 < bytes.len() && bytes[index + 1].is_ascii_punctuation() => {
                out.push(bytes[index + 1] as char);
                index += 2;
            }
            b'&' => {
                let limit = (index + 32).min(bytes.len());
                match text.get(index..limit).and_then(entity_prefix_len) {
                    Some((length, decoded)) => {
                        out.push_str(&decoded);
                        index += length;
                    }
                    None => {
                        out.push('&');
                        index += 1;
                    }
                }
            }
            _ => {
                let ch = text[index..].chars().next().unwrap_or('&');
                out.push(ch);
                index += ch.len_utf8();
            }
        }
    }
    out
}

/// 把一个完整的实体引用解码成字符。
fn decode_entity(raw: &str) -> Option<String> {
    let (length, decoded) = entity_prefix_len(raw)?;
    (length == raw.len()).then_some(decoded)
}

/// 从 `text` 开头匹配一个实体引用，返回 (长度, 解码结果)。
fn entity_prefix_len(text: &str) -> Option<(usize, String)> {
    let bytes = text.as_bytes();
    if bytes.first() != Some(&b'&') {
        return None;
    }
    if bytes.get(1) == Some(&b'#') {
        let (radix, digits_start) = if matches!(bytes.get(2), Some(b'x' | b'X')) {
            (16, 3)
        } else {
            (10, 2)
        };
        let digits: String = text[digits_start..]
            .chars()
            .take_while(|ch| ch.is_digit(radix))
            .collect();
        if digits.is_empty() || bytes.get(digits_start + digits.len()) != Some(&b';') {
            return None;
        }
        let code = u32::from_str_radix(&digits, radix).ok()?;
        // 规范：0 与非法码位一律变成 U+FFFD。
        let ch = char::from_u32(code)
            .filter(|ch| *ch != '\0')
            .unwrap_or('\u{FFFD}');
        return Some((digits_start + digits.len() + 1, ch.to_string()));
    }
    let end = text.find(';')? + 1;
    let candidate = &text[..end];
    entities::ENTITIES
        .iter()
        .find(|entity| entity.entity == candidate)
        .map(|entity| (end, entity.characters.to_owned()))
}

/// 引用标签的规范化：折叠空白、大小写不敏感。
fn normalize_label(label: &str) -> String {
    let mut out = String::with_capacity(label.len());
    let mut in_space = false;
    for ch in label.trim().chars() {
        if ch.is_whitespace() {
            in_space = true;
        } else {
            if in_space && !out.is_empty() {
                out.push(' ');
            }
            in_space = false;
            out.extend(ch.to_lowercase());
        }
    }
    out
}

/// 从每一行开头剥掉最多 `columns` 列的空白，制表符按 4 对齐。
fn strip_columns(text: &str, columns: usize) -> String {
    strip_columns_from(text, columns, 0)
}

/// 同上，但跳过第一行。
fn strip_columns_after_first_line(text: &str, columns: usize) -> String {
    strip_columns_from(text, columns, 1)
}

fn strip_columns_from(text: &str, columns: usize, skip_lines: usize) -> String {
    if columns == 0 {
        return text.to_owned();
    }
    let mut out = String::with_capacity(text.len());
    for (index, line) in text.split('\n').enumerate() {
        if index > 0 {
            out.push('\n');
        }
        if index < skip_lines {
            out.push_str(line);
            continue;
        }
        let mut removed = 0_usize;
        let mut rest = line;
        while removed < columns {
            match rest.as_bytes().first() {
                Some(b' ') => {
                    removed += 1;
                    rest = &rest[1..];
                }
                Some(b'\t') => {
                    let width = 4 - removed % 4;
                    if removed + width > columns {
                        break;
                    }
                    removed += width;
                    rest = &rest[1..];
                }
                _ => break,
            }
        }
        out.push_str(rest);
    }
    out
}
