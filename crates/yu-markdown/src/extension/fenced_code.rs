//! ``` 围栏代码块。
//!
//! 围栏那两行不进视觉文本，中间的内容整段按等宽排。**内容不解析行内语法**
//! ——这件事不需要任何人判断：树里 `FencedCode` 的内容是一个 `CodeText`
//! 叶子，里面没有行内标记节点，遍历不到就产不出装饰。v1 的行内扫描器拿不到
//! 块级上下文，于是代码块里的 `*` 被当成强调隐藏掉了：用户看到的代码静静
//! 少掉两个字符。
//!
//! # 着色也在这个文件里（S7 第五刀）
//!
//! 「哪一段是语言名、哪一段是正文」这个文件已经算出来了（见下面 `info` 与
//! `content_start`/`content_end`），另开一个 extension 就得**再算一遍**——而
//! 不变量 D6 明说 extension 之间不得相互感知，它读不到这里的 `BlockOrnament`。
//! 两份实现会在下一次改动时分叉，分叉的表现是颜色盖到围栏上。
//!
//! 着色本身在 `yu-highlight`：这里只知道「(语言名, 正文) → 一串带角色的
//! 区间」，不知道 tree-sitter 的存在。

use yu_core::{ByteOffset, TextAttrs, TextRange, TextStyle};
use yu_syntax::NodeKind;

use super::{BlockContext, BlockOrnament, Extension, ExtensionOutput};

#[derive(Debug, Default)]
pub struct FencedCode {
    highlighter: yu_highlight::Highlighter,
}

impl Extension for FencedCode {
    fn name(&self) -> &'static str {
        "fenced-code"
    }

    fn decorate(&self, cx: &BlockContext<'_>, out: &mut ExtensionOutput) {
        let Some(node) = cx.block_node(|kind| kind == NodeKind::FencedCode) else {
            return;
        };
        let style = out.style(TextAttrs::new(TextStyle::Code));
        out.mark(cx.range(), style);

        let marks: Vec<_> = node
            .children()
            .filter(|child| child.kind() == NodeKind::CodeMark)
            .collect();
        let Some(opening) = marks.first() else {
            return;
        };
        // 开围栏隐藏到内容起点，而不是只隐藏 ``` 本身：中间还夹着 `CodeInfo`
        // （语言名）和行尾的换行符，都不该出现在视觉文本里。
        //
        // 空代码块没有 `CodeText`，退到收尾围栏；连收尾围栏都没有（未闭合的
        // 空围栏）就退到块末。退成 `opening.end()` 是不行的——那会把开围栏
        // 后面那个换行符留在视觉文本里，画面上是一个空行。
        let content_start = node
            .children()
            .find(|child| child.kind() == NodeKind::CodeText)
            .map(|text| text.range().start())
            .or_else(|| marks.get(1).map(|closing| closing.range().start()))
            .unwrap_or_else(|| cx.range().end());
        if let Some(prefix) = TextRange::new(node.range().start(), content_start) {
            out.replace(prefix);
        }

        // 收尾围栏隐藏到**块**的末尾而不是节点的末尾：块比节点多出行尾那个
        // 换行符，而内容 `CodeText` 自己已经带着一个了。两个都留下的话每个
        // 代码块尾部会多出一个空行——不报错，只是画面里凭空多一行。
        //
        // 未闭合的围栏没有收尾行；只有一个标记时它就是开围栏，不能再算一次
        // 收尾，否则同一段 source 被隐藏两遍。
        let content_end = if let Some(closing) = marks.last()
            && closing.range() != opening.range()
            && let Some(suffix) = TextRange::new(closing.range().start(), cx.range().end())
        {
            out.replace(suffix);
            closing.range().start()
        } else {
            cx.range().end()
        };

        // 语言名与正文的区间。隐藏区间说得出「围栏那两行不进视觉文本」，
        // 说不出「哪一段是语言名」——而 KaTeX / Mermaid 那条路要按语言名
        // 决定这个块渲染成什么。
        let info = node
            .children()
            .find(|child| child.kind() == NodeKind::CodeInfo)
            .map_or_else(|| TextRange::empty(content_start), |child| child.range());
        if let Some(content) = TextRange::new(content_start, content_end.max(content_start)) {
            let style = out.line_style(BlockOrnament::FencedCode { info, content });
            out.line(cx.range(), style);
            self.highlight(cx, out, info, content);
        }
    }
}

impl FencedCode {
    /// 给正文里的 token 各加一条 Mark。
    ///
    /// **每一条都自带 `TextStyle::Code`。** `yu_editor::marks::winner_over` 是
    /// 「最窄的 Mark 赢，而且只赢一个」——Mark 不叠加，所以 token 那条会把上面
    /// 整段的 `Code` 整个盖掉。少写这半句的表现是高亮的字掉出等宽字体，不报错。
    ///
    /// 认不出语言、拿不到文本、着色器什么都没给，三种情况都是「不加任何
    /// Mark」，于是整块退回一段普通等宽文字。
    fn highlight(
        &self,
        cx: &BlockContext<'_>,
        out: &mut ExtensionOutput,
        info: TextRange,
        content: TextRange,
    ) {
        let Some(language) = cx
            .text(info)
            .as_deref()
            .and_then(yu_highlight::Language::from_info)
        else {
            return;
        };
        let Some(code) = cx.text(content) else {
            return;
        };
        let base = content.start().get();
        for span in self.highlighter.spans(language, &code) {
            // `RoleSpan` 的偏移是**正文那个 &str 的局部字节**，加上正文起点
            // 才是源码区间。`yu-highlight` 有意不用 `TextRange` 装它们，就是
            // 为了让这一次换算必须显式写出来。
            let Some(range) = TextRange::new(
                ByteOffset::new(base.saturating_add(span.start as u64)),
                ByteOffset::new(base.saturating_add(span.end as u64)),
            ) else {
                continue;
            };
            let style = out.style(TextAttrs::new(TextStyle::Code).with_role(span.role));
            out.mark(range, style);
        }
    }
}
