//! 围栏代码块的着色装饰。
//!
//! `yu-highlight` 自己的用例压的是「(语言, 代码) → 哪几段是什么角色」。
//! **这里压的是另一件事**：那些区间搬到源码坐标之后指对了没有，以及它们带着
//! 的排版属性对不对。两者的判据必须分开——拿「着色器说这里是关键字」去证
//! 「装饰指着 `fn`」是自证。
//!
//! 所以这个文件里每一条断言都把 `&source[range]` 切出来跟一个字面量比。

use yu_core::{StyleId, TextAttrs, TextRange, TextRole, TextStyle};
use yu_decoration::Decoration;
use yu_markdown::{BlockDecorations, BlockOrnament, ExtensionSet, parse};
use yu_syntax::parse as parse_syntax;
use yu_text::TextBuffer;

fn decorate(source: &str) -> BlockDecorations {
    let buffer = TextBuffer::new(source.to_owned());
    let snapshot = buffer.snapshot();
    let document = parse(&snapshot);
    let tree = parse_syntax(&snapshot).expect("测试文档很短").into_tree();
    let block = document.blocks().get(0).expect("至少有一个块");
    ExtensionSet::markdown()
        .decorate(
            &snapshot,
            &tree,
            document.reference_definitions(),
            block,
            None,
        )
        .expect("装饰产出不该失败")
}

/// 这个块上的 `(源码文本, 排版属性)`，只取带角色的那些。
///
/// 整段的 `Code` 那一条不带角色，于是自动被滤掉——判据因此不必知道「除了
/// 高亮之外还有几条 Mark」，加一条别的 Mark 不会让这些用例假红。
fn highlighted<'a>(source: &'a str, decorations: &BlockDecorations) -> Vec<(&'a str, TextAttrs)> {
    decorations
        .set()
        .all()
        .iter()
        .filter_map(|entry| match entry.decoration {
            Decoration::Mark { style } => Some((entry.range, style)),
            _ => None,
        })
        .filter_map(|(range, style)| {
            let attrs = attrs_of(decorations, style);
            (attrs.role() != TextRole::Plain).then(|| (text_of(source, range), attrs))
        })
        .collect()
}

fn attrs_of(decorations: &BlockDecorations, style: StyleId) -> TextAttrs {
    decorations
        .attrs(style)
        .expect("装饰指向的 StyleId 必须查得到")
}

fn text_of(source: &str, range: TextRange) -> &str {
    let start = usize::try_from(range.start().get()).expect("测试文档很短");
    let end = usize::try_from(range.end().get()).expect("测试文档很短");
    &source[start..end]
}

/// 这个块的正文区间，由 `BlockOrnament::FencedCode` 给。
fn content_of(decorations: &BlockDecorations) -> Option<TextRange> {
    decorations
        .line_styles()
        .iter()
        .find_map(|ornament| match ornament {
            BlockOrnament::FencedCode { content, .. } => Some(*content),
            _ => None,
        })
}

const RUST_BLOCK: &str = "```rust\nfn main() {\n    // hi\n    let x: u32 = 1;\n}\n```\n";

/// 装饰指着源码里的哪一段。
#[test]
fn highlight_marks_point_at_the_right_source_text() {
    let decorations = decorate(RUST_BLOCK);
    let spans = highlighted(RUST_BLOCK, &decorations);
    let keywords: Vec<_> = spans
        .iter()
        .filter(|(_, attrs)| attrs.role() == TextRole::Keyword)
        .map(|(text, _)| *text)
        .collect();
    assert_eq!(keywords, vec!["fn", "let"]);
    let comments: Vec<_> = spans
        .iter()
        .filter(|(_, attrs)| attrs.role() == TextRole::Comment)
        .map(|(text, _)| *text)
        .collect();
    assert_eq!(comments, vec!["// hi"]);
    let types: Vec<_> = spans
        .iter()
        .filter(|(_, attrs)| attrs.role() == TextRole::Type)
        .map(|(text, _)| *text)
        .collect();
    assert_eq!(types, vec!["u32"]);
}

/// **每一条高亮 Mark 都必须带着 `TextStyle::Code`。**
///
/// 这是这一刀最容易犯的那个错，而且它不报错：`yu_editor::marks::winner_over`
/// 让**最窄的** Mark 赢，而且只赢一个——token 那条比整段的 `Code` 窄，会把它
/// 整个盖掉。少了这半句，高亮的字全部掉出等宽字体，代码块里对齐的空格与
/// 变宽的字混在一起，画面歪掉但一条断言都不响。
#[test]
fn every_highlight_mark_keeps_the_monospace_typeface() {
    for source in [
        RUST_BLOCK,
        "```json\n{ \"a\": [1, true] }\n```\n",
        "```py\ndef f():\n    return None\n```\n",
    ] {
        let decorations = decorate(source);
        let spans = highlighted(source, &decorations);
        assert!(!spans.is_empty(), "这份语料该有高亮：{source:?}");
        for (text, attrs) in spans {
            assert_eq!(
                attrs.style(),
                TextStyle::Code,
                "{text:?} 的高亮属性丢了等宽字面"
            );
            assert_eq!(attrs.size_scale(), 1.0, "{text:?} 的字号被动过");
        }
    }
}

/// 高亮不许碰围栏那两行。
///
/// 判据是 `BlockOrnament::FencedCode` 给的正文区间——同一个 extension 里的
/// 另一样产出，与着色走的不是同一段代码。着色的区间是「正文起点 + 局部偏移」
/// 算出来的，起点加错或者局部偏移越界都会顶到这一条上。
#[test]
fn highlight_stays_inside_the_content_range() {
    for source in [
        RUST_BLOCK,
        // 开围栏带参数，收尾围栏缩进——两处都会挪动正文的起止。
        "```rust,ignore\nfn a() {}\n  ```\n",
        // 未闭合：正文一直到块末。
        "```rust\nfn a() {}\n",
    ] {
        let decorations = decorate(source);
        let content = content_of(&decorations).expect("围栏块必有正文区间");
        let spans = highlighted(source, &decorations);
        assert!(!spans.is_empty(), "这份语料该有高亮：{source:?}");
        for (text, _) in &spans {
            let start = source.find(text).expect("文本来自这份源码");
            assert!(
                start as u64 >= content.start().get(),
                "{text:?} 跑到开围栏上去了：{source:?}"
            );
            assert!(
                (start + text.len()) as u64 <= content.end().get(),
                "{text:?} 跑到收尾围栏上去了：{source:?}"
            );
        }
    }
}

/// 没有语言名、认不出语言、以及根本不是代码块，都不产高亮。
///
/// 「没有语言名」那一条特别重要：`BlockOrnament::FencedCode` 在无 info 时给的
/// 是一段**空区间**，把它当成一个语言名去查会得到空串——`Language::from_info`
/// 对空串返回 `None` 才让这一条成立。
#[test]
fn blocks_without_a_known_language_get_no_highlight() {
    for source in [
        "```\nfn main() {}\n```\n",
        "```brainfuck\n+++[->+++<]\n```\n",
        "```   \nfn main() {}\n```\n",
        "普通段落里的 fn let return\n",
        "    fn 缩进代码块() {}\n",
    ] {
        let decorations = decorate(source);
        assert!(
            highlighted(source, &decorations).is_empty(),
            "不该有高亮：{source:?}"
        );
    }
}

/// 空的与只有空白的代码块不产高亮，也不 panic。
#[test]
fn empty_fenced_blocks_are_quiet() {
    for source in ["```rust\n```\n", "```rust\n\n```\n", "```rust\n"] {
        let decorations = decorate(source);
        assert!(
            highlighted(source, &decorations).is_empty(),
            "不该有高亮：{source:?}"
        );
    }
}

/// 多字节源码上区间不切错。
///
/// 装饰这一步是把**字节**偏移搬进 `TextRange`，中文注释里差一个字节就会让
/// 后面 `VisualText` 的映射落在半个汉字上。
#[test]
fn multibyte_code_blocks_keep_char_boundaries() {
    let source = "```rust\nlet 变量 = \"你好\"; // 中文注释\n```\n";
    let decorations = decorate(source);
    let spans = highlighted(source, &decorations);
    let literals: Vec<_> = spans
        .iter()
        .filter(|(_, attrs)| attrs.role() == TextRole::Literal)
        .map(|(text, _)| *text)
        .collect();
    assert_eq!(literals, vec!["\"你好\""]);
    let comments: Vec<_> = spans
        .iter()
        .filter(|(_, attrs)| attrs.role() == TextRole::Comment)
        .map(|(text, _)| *text)
        .collect();
    assert_eq!(comments, vec!["// 中文注释"]);
}

/// 光标停在代码块里不改变高亮。
///
/// 围栏块本来就没有「光标露出」这回事（`fenced_code.rs` 不看 `active`），
/// 但着色带着一条 memo，而 memo 的键里没有 `active`。这一条把「带焦点的那份
/// 产出与不带焦点的完全相同」钉死——不相同就说明有人把焦点掺进了着色。
#[test]
fn moving_the_caret_inside_a_code_block_changes_nothing() {
    let buffer = TextBuffer::new(RUST_BLOCK.to_owned());
    let snapshot = buffer.snapshot();
    let document = parse(&snapshot);
    let tree = parse_syntax(&snapshot).expect("短").into_tree();
    let block = document.blocks().get(0).expect("有块");
    let extensions = ExtensionSet::markdown();
    let canonical = extensions
        .decorate(
            &snapshot,
            &tree,
            document.reference_definitions(),
            block,
            None,
        )
        .expect("产出");
    for caret in [10_u64, 20, 30, 40] {
        let active = TextRange::empty(yu_core::ByteOffset::new(caret));
        let revealed = extensions
            .decorate(
                &snapshot,
                &tree,
                document.reference_definitions(),
                block,
                Some(active),
            )
            .expect("产出");
        assert_eq!(
            highlighted(RUST_BLOCK, &canonical),
            highlighted(RUST_BLOCK, &revealed),
            "光标在 {caret} 时高亮变了"
        );
    }
}
