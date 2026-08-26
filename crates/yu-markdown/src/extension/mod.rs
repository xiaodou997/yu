//! 语义 extension 集合。
//!
//! # 这一层要回答的问题
//!
//! overview-v2 第 4.3 节给 `yu-markdown` 的职责是「Markdown 语法定义与
//! decoration 产出」，S6 的验收是「新增一种语法的 diff 只落在 `yu-markdown`
//! 内，且 < 200 行」。要让那句话成立，一种语法必须能**只写一个文件**就接进
//! 来：它自己认识自己的语法，自己产出自己的装饰，不知道别的语法存在。
//!
//! 于是这里定义三样东西：
//!
//! - [`BlockContext`]：一个块在装饰阶段能看见的全部输入；
//! - [`Extension`]：一种语法的产出器；
//! - [`ExtensionSet`]：注册表，按固定顺序跑一遍，把各家的产出合并成
//!   [`BlockDecorations`]。
//!
//! # 为什么每个 extension 有自己的 id 空间
//!
//! 不变量 D6 要求「extension 之间不得相互感知」。如果它们共用一张样式表，
//! `StyleId(1)` 是谁的就取决于谁先跑——那就是一种相互感知，而且是**静默**
//! 的那种：换个注册顺序，斜体会变成等宽，不报错。
//!
//! 所以 [`ExtensionOutput`] 的 id 是**局部**的（从 0 开始，自己去重），
//! 合并时由 [`ExtensionSet`] 按注册顺序整体平移。平移是纯函数，所以单个
//! extension 的产出仍然可以单独缓存、单独重算——D6 说的「独立的单位」。
//!
//! # 什么不在这里
//!
//! **几何不在这里。** 标题几号字、引用竖条多宽、列表标记的 gutter 让多少，
//! 都要 `LayoutConfig` 才算得出来，而那是 `yu-editor` 的事（S5 已经把这条
//! 边界划好了，见 `yu-editor/src/blockinput.rs` 的模块文档）。这里产出的
//! 是[`BlockOrnament`]——「这是二级标题」「这是两层引用」——由上面那层翻译
//! 成「1.7 倍字号」「缩进 8.0」。
//!
//! **composition 不在这里。** IME 的 preedit 是往视觉文本里**插入**一段
//! 不在 source 里的文字，而 [`Decoration`] 表达不了插入（第 5.1 节的四个
//! 变体都是对既有 source 的处理）。不变量 H1 说它是 transient overlay，
//! 不进 canonical source——那就不该是装饰。它由装配层单独叠上去。

use core::error::Error;
use core::fmt;

use yu_core::{ByteOffset, Revision, StyleId, TextAttrs, TextRange};
use yu_decoration::{Decoration, DecorationRange, DecorationSet, LineStyleId, MergeError};
use yu_syntax::{NodeKind, Tree};
use yu_text::TextSnapshot;

use crate::block_line_ranges;
use crate::block_sequence::Block;
use crate::reference::read_range;

mod code_span;
mod emphasis;
mod fenced_code;
mod heading;
mod image;
mod line_break;
mod link;
mod list;
mod quote;
mod syntax;
mod task;

pub use syntax::{DelimitedSpan, SyntaxNode};

/// 扫空白时一次读多少字节。
///
/// 空白 run 通常只有一两个字节，一个窗口就够；长的按它自己的长度分几次读。
/// 这个数字没有魔力，换成 16 或 128 都能工作。
const SPACE_WINDOW: u64 = 64;

/// 读 `from..to`，`from` 落在字符中间时往后挪到最近的边界。
///
/// 窗口的起点是按**字节**算出来的（`cursor - 64`），完全可能落在一个多字节
/// 字符中间；`read_range` 在那里会失败，而调用方能做的只有就地放弃。那正是
/// 静默地做错事：`# 标题` 后面跟 63 个空格时，收尾标记前的空白一个都不隐藏
/// ——不 panic、不报错，只是画面里多出一串空格。这条是被用例抓到的。
///
/// UTF-8 一个字符最多 4 字节，所以最多往后挪 3 次。
fn read_window(source: &TextSnapshot, from: u64, to: u64) -> Option<(u64, Vec<u8>)> {
    let mut from = from;
    while from < to {
        if let Some(range) = TextRange::new(ByteOffset::new(from), ByteOffset::new(to))
            && let Some(bytes) = read_range(source, range)
        {
            return Some((from, bytes));
        }
        from += 1;
    }
    None
}

/// 一个块在装饰阶段能看见的全部输入。
///
/// 语法树整篇只解析一次，各家共用——每个 extension 自己再解析一遍会让
/// 「同一份源码在两个 extension 眼里不一样」成为可能。
pub struct BlockContext<'a> {
    source: &'a TextSnapshot,
    block: Block,
    syntax: SyntaxNode<'a>,
    active: Option<TextRange>,
}

impl<'a> BlockContext<'a> {
    #[must_use]
    pub const fn source(&self) -> &'a TextSnapshot {
        self.source
    }

    #[must_use]
    pub const fn block(&self) -> Block {
        self.block
    }

    #[must_use]
    pub fn range(&self) -> TextRange {
        self.block.range()
    }

    /// 完整包含这个块的最深语法节点。
    ///
    /// 它可能比块**大**：块的边界由 `block_sequence` 定，语法树的块结构由
    /// `yu-syntax` 定，两者不保证逐字节相同。要遍历的话用
    /// [`BlockContext::nodes`]，它已经把块外的节点滤掉了。
    #[must_use]
    pub const fn syntax(&self) -> SyntaxNode<'a> {
        self.syntax
    }

    /// 落在这个块**之内**的全部语法节点，前序。
    ///
    /// 「之内」是完整包含：半个落在块外的节点一个都不给。extension 因此
    /// 不需要自己判断边界——漏判的后果是装饰跨到邻块上，而装饰是按块缓存的，
    /// 那会变成「改了这一块，另一块的样子也变了」。
    pub fn nodes(&self) -> impl Iterator<Item = SyntaxNode<'a>> + use<'a> {
        let (from, to) = (self.range().start().get(), self.range().end().get());
        self.syntax
            .descendants()
            .filter(move |node| from <= u64::from(node.start()) && u64::from(node.end()) <= to)
    }

    /// 块自身对应的那个语法节点。
    ///
    /// 先看包含块的那个节点，再在块内按前序找第一个——两头都找是因为块与
    /// 语法节点的边界不保证对齐：块通常带着行尾的换行符，节点通常不带。
    pub fn block_node(&self, wanted: impl Fn(NodeKind) -> bool) -> Option<SyntaxNode<'a>> {
        if wanted(self.syntax.kind()) {
            return Some(self.syntax);
        }
        self.nodes().find(|node| wanted(node.kind()))
    }

    /// 读一段源码。
    ///
    /// 树里没有节点的语法（`==高亮==` 就是一例，lezer 不认识它）只能自己扫
    /// 文本。给它一个口子，「新增一种语法的 diff 只落在 `yu-markdown` 内」
    /// 才成立——否则加一种语法就得先改解析器。
    #[must_use]
    pub fn text(&self, range: TextRange) -> Option<String> {
        String::from_utf8(read_range(self.source, range)?).ok()
    }

    /// 从 `offset` 起跳过 ASCII 空格与制表符，不越出块的末尾。
    ///
    /// 结构标记与内容之间的那几个空格属于语法：`#   多空格` 的 `HeaderMark`
    /// 只有 `#` 一个字节，三个空格是树不表示的部分。留着它们的话标题会顶着
    /// 三个空格往右挪——不报错，只是画得不对。
    ///
    /// 按窗口分段读，不是一口气读到块末：引用块每个 `QuoteMark` 都要调一次，
    /// 一次读到块末的话，一个五百行的引用块要复制五百次半个块。空白run 通常
    /// 只有一两个字节，一个窗口就够，长的也只按它自己的长度付钱。
    #[must_use]
    pub fn skip_spaces(&self, offset: ByteOffset) -> ByteOffset {
        let end = self.range().end().get();
        let mut cursor = offset.get();
        while cursor < end {
            let stop = end.min(cursor.saturating_add(SPACE_WINDOW));
            let scanned = stop - cursor;
            let Some(window) = TextRange::new(ByteOffset::new(cursor), ByteOffset::new(stop))
            else {
                break;
            };
            let Some(bytes) = read_range(self.source, window) else {
                break;
            };
            let run = bytes
                .iter()
                .take_while(|byte| matches!(byte, b' ' | b'\t'))
                .count() as u64;
            cursor += run;
            // 没跑满这个窗口，说明撞上了非空白。
            if run < scanned {
                break;
            }
        }
        ByteOffset::new(cursor)
    }

    /// 从 `offset` 起**往回**跳过 ASCII 空格与制表符，不越出块的起点。
    ///
    /// `# 标题 #` 的收尾标记前面那个空格也是语法。分段读的理由同
    /// [`BlockContext::skip_spaces`]。
    #[must_use]
    pub fn skip_spaces_back(&self, offset: ByteOffset) -> ByteOffset {
        let start = self.range().start().get();
        let mut cursor = offset.get();
        while cursor > start {
            let wanted = start.max(cursor.saturating_sub(SPACE_WINDOW));
            let Some((from, bytes)) = read_window(self.source, wanted, cursor) else {
                break;
            };
            let run = bytes
                .iter()
                .rev()
                .take_while(|byte| matches!(byte, b' ' | b'\t'))
                .count() as u64;
            let scanned = cursor - from;
            cursor -= run;
            if run < scanned {
                break;
            }
        }
        ByteOffset::new(cursor)
    }

    /// 这个块第一行的起点。列表标记的缩进由它算出来。
    #[must_use]
    pub fn first_line_start(&self) -> ByteOffset {
        block_line_ranges(self.source, self.range())
            .first()
            .map_or_else(|| self.range().start(), |line| line.start())
    }

    /// 光标所在的区间，只有**焦点块**有。
    ///
    /// 语法标记在光标碰到它的时候要露出来，否则用户没法编辑自己写的 `**`。
    #[must_use]
    pub const fn active(&self) -> Option<TextRange> {
        self.active
    }

    /// 这个块是不是焦点块。结构性前缀（`#`、`>`、`- `）整块一起露出来。
    #[must_use]
    pub const fn is_focus(&self) -> bool {
        self.active.is_some()
    }
}

/// 行级/块级「长什么样」的那部分装饰，由 [`LineStyleId`] 指向。
///
/// 它是**语义**，不是几何：`Heading { level: 2 }` 而不是「1.7 倍字号」。
/// 翻译成几何是 `yu-editor` 的事——那一层才有 `LayoutConfig`。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BlockOrnament {
    /// ATX 标题，1..=6 级。
    Heading { level: u8 },
    /// 引用，`depth` 层竖条。
    QuoteBar { depth: u8 },
    /// 列表的行首标记。`text` 不在 source 里（`•` 是 `-` 的替代呈现），
    /// 它替代掉的那段源码由 [`MarkerOrnament::source`] 指着。
    ///
    /// [`MarkerOrnament::source`]: crate::extension::MarkerOrnament::source
    Marker(MarkerOrnament),
}

/// 列表标记的替代呈现。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MarkerOrnament {
    source: TextRange,
    text: String,
    indent: u8,
}

impl MarkerOrnament {
    #[must_use]
    pub fn new(source: TextRange, text: impl Into<String>, indent: u8) -> Self {
        Self {
            source,
            text: text.into(),
            indent,
        }
    }

    /// 被替代掉的那段源码。选中与编辑仍然走它（不变量 A2）。
    #[must_use]
    pub const fn source(&self) -> TextRange {
        self.source
    }

    #[must_use]
    pub fn text(&self) -> &str {
        &self.text
    }

    /// 标记左边空出多少列。
    #[must_use]
    pub const fn indent(&self) -> u8 {
        self.indent
    }
}

/// 一个 extension 的产出：一组装饰，加上它自己那份 id 表。
///
/// id 是**局部**的，从 0 开始。合并时由 [`ExtensionSet`] 整体平移，
/// 所以 extension 写代码时不需要知道别人占了哪些号。
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ExtensionOutput {
    ranges: Vec<DecorationRange>,
    styles: Vec<TextAttrs>,
    line_styles: Vec<BlockOrnament>,
}

impl ExtensionOutput {
    /// 登记一种字型，拿到它的局部 id。相同的属性只占一个号。
    pub fn style(&mut self, attrs: TextAttrs) -> StyleId {
        let index = self
            .styles
            .iter()
            .position(|existing| *existing == attrs)
            .unwrap_or_else(|| {
                self.styles.push(attrs);
                self.styles.len() - 1
            });
        StyleId(u32::try_from(index).unwrap_or(u32::MAX))
    }

    /// 登记一种行级装饰，拿到它的局部 id。
    pub fn line_style(&mut self, ornament: BlockOrnament) -> LineStyleId {
        let index = self
            .line_styles
            .iter()
            .position(|existing| *existing == ornament)
            .unwrap_or_else(|| {
                self.line_styles.push(ornament);
                self.line_styles.len() - 1
            });
        LineStyleId(u32::try_from(index).unwrap_or(u32::MAX))
    }

    /// 让这段 source 从视觉文本里消失。它仍然可被光标穿越（不变量 D5）。
    pub fn replace(&mut self, range: TextRange) {
        if range.is_empty() {
            return;
        }
        self.ranges
            .push(DecorationRange::new(range, Decoration::Replace));
    }

    /// 给这段 source 换一种字型。
    pub fn mark(&mut self, range: TextRange, style: StyleId) {
        self.mark_with_priority(range, style, 0);
    }

    /// 带优先级的 [`ExtensionOutput::mark`]。
    ///
    /// 重叠的 mark 由装配层按「优先级高的赢，同级窄的赢」压平。标题用一个
    /// 高优先级的 mark 盖住整段，于是标题里的斜体也排成标题的字型——v1 的
    /// `HeadingClusterMetrics` 就是这个行为。
    pub fn mark_with_priority(&mut self, range: TextRange, style: StyleId, priority: i32) {
        if range.is_empty() {
            return;
        }
        self.ranges
            .push(DecorationRange::new(range, Decoration::Mark { style }).with_priority(priority));
    }

    /// 给这段 source 覆盖的每一行加一段行级装饰。
    pub fn line(&mut self, range: TextRange, style: LineStyleId) {
        self.ranges
            .push(DecorationRange::new(range, Decoration::Line { style }));
    }

    #[must_use]
    pub fn ranges(&self) -> &[DecorationRange] {
        &self.ranges
    }

    /// 把局部 id 平移到全局 id 空间。
    fn rebased(self, style_base: u32, line_style_base: u32) -> Vec<DecorationRange> {
        self.ranges
            .into_iter()
            .map(|mut entry| {
                entry.decoration = match entry.decoration {
                    Decoration::Mark { style } => Decoration::Mark {
                        style: StyleId(style.0.saturating_add(style_base)),
                    },
                    Decoration::Line { style } => Decoration::Line {
                        style: LineStyleId(style.0.saturating_add(line_style_base)),
                    },
                    other => other,
                };
                entry
            })
            .collect()
    }
}

/// 一种 Markdown 语法的装饰产出器。
///
/// 实现者只认识自己那一种语法。它拿不到别的 extension 的产出，也拿不到
/// 排版几何——两者都是有意的。
pub trait Extension: Send + Sync {
    /// 诊断用的名字。合并顺序由注册顺序决定，与名字无关。
    fn name(&self) -> &'static str;

    /// 产出这个块上属于本语法的装饰。
    fn decorate(&self, cx: &BlockContext<'_>, out: &mut ExtensionOutput);
}

/// 一个块装饰完之后的全部结果：装饰集合 + 两张解释 id 的表。
#[derive(Clone)]
pub struct BlockDecorations {
    range: TextRange,
    set: DecorationSet,
    styles: Vec<TextAttrs>,
    line_styles: Vec<BlockOrnament>,
}

impl BlockDecorations {
    /// 这份装饰覆盖的块。
    #[must_use]
    pub const fn range(&self) -> TextRange {
        self.range
    }

    #[must_use]
    pub const fn set(&self) -> &DecorationSet {
        &self.set
    }

    #[must_use]
    pub fn revision(&self) -> Revision {
        self.set.revision()
    }

    /// 查不到的 id 返回 `None`，由调用方报错而不是取一个默认值。
    ///
    /// 「装饰产出与样式表脱节」的 bug 应该响。给默认值的话它只会画得不对。
    #[must_use]
    pub fn attrs(&self, style: StyleId) -> Option<TextAttrs> {
        usize::try_from(style.0)
            .ok()
            .and_then(|index| self.styles.get(index))
            .copied()
    }

    #[must_use]
    pub fn ornament(&self, style: LineStyleId) -> Option<&BlockOrnament> {
        usize::try_from(style.0)
            .ok()
            .and_then(|index| self.line_styles.get(index))
    }

    /// 这个块上的全部行级装饰，按定序。
    #[must_use]
    pub fn line_ornaments(&self) -> Vec<(TextRange, &BlockOrnament)> {
        self.set
            .all()
            .iter()
            .filter_map(|entry| match entry.decoration {
                Decoration::Line { style } => {
                    self.ornament(style).map(|ornament| (entry.range, ornament))
                }
                _ => None,
            })
            .collect()
    }
}

impl fmt::Debug for BlockDecorations {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("BlockDecorations")
            .field("range", &self.range)
            .field("decorations", &self.set.all())
            .field("styles", &self.styles)
            .field("line_styles", &self.line_styles)
            .finish()
    }
}

/// 装饰产出过程中的错误。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ExtensionError {
    /// 各 extension 的集合合不到一起——revision 或源码长度不一致。
    Merge(MergeError),
    /// id 空间溢出。一个块上的样式数量本来就该是个位数。
    IdOverflow,
}

impl fmt::Display for ExtensionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Merge(error) => write!(formatter, "装饰集合合并失败：{error}"),
            Self::IdOverflow => formatter.write_str("一个块上的样式 id 溢出"),
        }
    }
}

impl Error for ExtensionError {}

impl From<MergeError> for ExtensionError {
    fn from(error: MergeError) -> Self {
        Self::Merge(error)
    }
}

/// 注册表：一组按固定顺序跑的 extension。
///
/// 顺序只决定 id 的分配，不决定装饰的定序——后者由
/// [`DecorationRange::order_key`] 全序钉死（不变量 D6）。
pub struct ExtensionSet {
    extensions: Vec<Box<dyn Extension>>,
}

impl ExtensionSet {
    /// Yu 目前支持的全部 Markdown 语法。
    ///
    /// 加一种语法就是加一个文件加这里一行。
    #[must_use]
    pub fn markdown() -> Self {
        Self {
            extensions: vec![
                Box::new(heading::Heading),
                Box::new(quote::Quote),
                Box::new(list::List),
                Box::new(task::Task),
                Box::new(fenced_code::FencedCode),
                Box::new(emphasis::Emphasis),
                Box::new(code_span::CodeSpan),
                Box::new(link::Link),
                Box::new(image::Image),
                Box::new(line_break::LineBreak),
            ],
        }
    }

    /// 空注册表。测试用：单独跑一个 extension 才能看出它自己产了什么。
    #[must_use]
    pub const fn empty() -> Self {
        Self {
            extensions: Vec::new(),
        }
    }

    #[must_use]
    pub fn with(mut self, extension: impl Extension + 'static) -> Self {
        self.extensions.push(Box::new(extension));
        self
    }

    #[must_use]
    pub fn names(&self) -> Vec<&'static str> {
        self.extensions
            .iter()
            .map(|extension| extension.name())
            .collect()
    }

    /// 跑一遍注册表，产出这个块的装饰。
    ///
    /// `tree` 是**整篇文档**的语法树，由调用方按 revision 缓存。这里不自己
    /// 解析：每块解析一次是 O(块数 × 文档长度)，而且十个 extension 各解析
    /// 一遍还会让「同一份源码在两个 extension 眼里不一样」成为可能。
    ///
    /// # Errors
    ///
    /// 各家的集合合不到一起（revision 或源码长度不一致）。
    pub fn decorate(
        &self,
        source: &TextSnapshot,
        tree: &Tree,
        block: Block,
        active: Option<TextRange>,
    ) -> Result<BlockDecorations, ExtensionError> {
        // 「代码块里不解析行内语法」不再是这里的一条分支：语法树里
        // `CodeBlock` / `FencedCode` / `CommentBlock` 内部根本没有行内标记
        // 节点，遍历不到就产不出装饰。v1 的扫描器没有块级上下文，需要调用方
        // 逐个块判断，漏一个就是「代码里的星号被吃掉了」；换成语法树之后
        // 这件事由树的形状保证，不再依赖任何人记得判断。
        let cx = BlockContext {
            source,
            block,
            syntax: SyntaxNode::new(tree, 0).deepest_containing(content_range(source, block)),
            active,
        };

        let source_len = source.len_bytes();
        let revision = source.revision();
        let mut sets = Vec::with_capacity(self.extensions.len());
        let mut styles = Vec::new();
        let mut line_styles = Vec::new();
        let bounds = block.range();
        for extension in &self.extensions {
            let mut out = ExtensionOutput::default();
            extension.decorate(&cx, &mut out);
            // 兜底：装饰不得越出这个块。`BlockContext::nodes` 已经把块外的
            // 节点滤掉了，所以正常情况下这里一条都不该滤掉；留着它是因为
            // extension 也可以自己造区间（列表标记就是），而越界的后果是
            // 「改了这一块，另一块的样子也变了」——按块缓存会把它藏起来。
            out.ranges.retain(|entry| {
                entry.range.start() >= bounds.start() && entry.range.end() <= bounds.end()
            });
            if out.ranges.is_empty() {
                continue;
            }
            let style_base = u32::try_from(styles.len()).map_err(|_| ExtensionError::IdOverflow)?;
            let line_style_base =
                u32::try_from(line_styles.len()).map_err(|_| ExtensionError::IdOverflow)?;
            styles.extend(out.styles.iter().copied());
            line_styles.extend(out.line_styles.iter().cloned());
            sets.push(DecorationSet::new(
                revision,
                source_len,
                out.rebased(style_base, line_style_base),
            ));
        }
        let set = DecorationSet::merge(revision, source_len, sets.iter())?;
        Ok(BlockDecorations {
            range: block.range(),
            set,
            styles,
            line_styles,
        })
    }
}

impl Default for ExtensionSet {
    fn default() -> Self {
        Self::markdown()
    }
}

impl fmt::Debug for ExtensionSet {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ExtensionSet")
            .field("extensions", &self.names())
            .finish()
    }
}

/// 块的 range 去掉行尾的空白，用来在语法树里定位它。
///
/// **块带着行尾的换行符，语法节点不带。** 拿没修剪的 range 去找「完整包含它
/// 的最深节点」，几乎每个块都会一路退到 `Document`：`# 标题\n` 的块是 0..10，
/// 而 `AtxHeading1` 只有 0..9，包不住。
///
/// 退到 `Document` 之后结果仍然是对的——[`BlockContext::nodes`] 会把块外的
/// 节点裁掉——所以这件事不报错、不画错，只是每个块都要遍历整篇文档一遍，
/// 长文档变成 O(块数 × 文档长度)。这正是「静默地做错事」的一种：唯一的症状
/// 是慢。
fn content_range(source: &TextSnapshot, block: Block) -> TextRange {
    /// 行尾空白最多几个字节。`\r\n` 加几个尾随空格，8 个足够，而读整块只为
    /// 修剪尾巴对大代码块是浪费。
    const WINDOW: u64 = 8;

    let range = block.range();
    let (start, end) = (range.start().get(), range.end().get());
    // 窗口起点按字节算，可能落在多字节字符中间；`read_window` 会挪到边界。
    // 就地放弃的话这里会静静地退回未修剪的 range，症状只有慢。
    let Some((_, bytes)) = read_window(source, end.saturating_sub(WINDOW).max(start), end) else {
        return range;
    };
    let trailing = bytes
        .iter()
        .rev()
        .take_while(|byte| matches!(byte, b'\n' | b'\r' | b' ' | b'\t'))
        .count() as u64;
    TextRange::new(range.start(), ByteOffset::new(end.saturating_sub(trailing))).unwrap_or(range)
}

/// 光标碰到这一段行内语法时，它的定界符要露出来。
///
/// 空的 active（一个光标位置）用**严格**包含：光标停在 `*a*` 的外边缘不算
/// 碰到它，否则 `*a**b*` 中间那一处会让两段语法一起露出来。非空的 active
/// （一段选区）用相交。
#[must_use]
pub fn reveals(active: Option<TextRange>, span: TextRange) -> bool {
    let Some(active) = active else {
        return false;
    };
    if active.is_empty() {
        span.start() < active.start() && active.start() < span.end()
    } else {
        active.start() < span.end() && span.start() < active.end()
    }
}
