//! ``` 围栏代码块。
//!
//! 围栏那两行不进视觉文本，中间的内容整段按等宽排。**内容不解析行内语法**
//! ——这件事不需要任何人判断：树里 `FencedCode` 的内容是一个 `CodeText`
//! 叶子，里面没有行内标记节点，遍历不到就产不出装饰。v1 的行内扫描器拿不到
//! 块级上下文，于是代码块里的 `*` 被当成强调隐藏掉了：用户看到的代码静静
//! 少掉两个字符。

use yu_core::{TextAttrs, TextStyle};
use yu_syntax::NodeKind;

use super::{BlockContext, Extension, ExtensionOutput};
use yu_core::TextRange;

pub struct FencedCode;

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
        if let Some(closing) = marks.last()
            && closing.range() != opening.range()
            && let Some(suffix) = TextRange::new(closing.range().start(), cx.range().end())
        {
            out.replace(suffix);
        }
    }
}
