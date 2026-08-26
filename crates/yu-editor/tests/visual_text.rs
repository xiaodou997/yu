//! `VisualText` 的性质。
//!
//! # 它压的是哪三件事
//!
//! `DecorationSet` 的双向映射本身已经有 oracle：
//! `crates/yu-decoration/tests/projection_differential.rs` 拿 v1 的
//! `Projection` 逐点比过。这里压的是 `VisualText` **加在它上面**的三样：
//!
//! 1. **换原点。** 装饰集合的视觉偏移是整篇文档的，块的视觉文本从 0 开始。
//!    差一个常量，而算错这个常量的表现是「点第二个块，光标落在第一个块里」。
//! 2. **拿出文本。** 拼出来的字节必须与映射说的长度一致。
//! 3. **叠 composition。** preedit 是往视觉文本里插入一段不在 source 里的
//!    文字，四种 `Decoration` 都表达不了。

use yu_core::{ByteOffset, TextRange};
use yu_decoration::Bias;
use yu_editor::{VisualText, VisualTextError};
use yu_markdown::{BlockDecorations, ExtensionSet};
use yu_syntax::parse as parse_syntax;
use yu_text::{TextBuffer, TextSnapshot};

fn offset(value: u64) -> ByteOffset {
    ByteOffset::new(value)
}

fn range(from: u64, to: u64) -> TextRange {
    TextRange::new(offset(from), offset(to)).expect("测试区间是升序的")
}

/// 第 `index` 个块的装饰与它的视觉文本。
fn block(snapshot: &TextSnapshot, index: usize) -> (BlockDecorations, VisualText) {
    let markdown = yu_markdown::parse(snapshot);
    let block = markdown.blocks().get(index).expect("块存在");
    let tree = parse_syntax(snapshot).expect("测试文档很短").into_tree();
    let decorations = ExtensionSet::markdown()
        .decorate(snapshot, &tree, block, None)
        .expect("装饰产出");
    let visual = VisualText::new(snapshot, decorations.range(), decorations.set().clone())
        .expect("视觉文本");
    (decorations, visual)
}

const CORPUS: &[&str] = &[
    "段落",
    "普通段落 *斜体* 与 **粗体** 与 `代码`",
    "# 一级标题",
    "## 二级 *斜体* 标题",
    "> 引用一层",
    "> > 引用两层",
    "- 项目",
    "- [ ] 待办",
    "```rust\nlet x = 1;\n```",
    "[文字](目标)",
    "![替代](图片)",
    "中文 *强调* 与 emoji 🙂",
    "a | b\n--- | ---\n1 | 2",
];

/// 视觉文本的长度必须与映射说的一致。
///
/// 「哪些字节被隐藏」与「隐藏之后有多长」如果用了两套算法，画面会比光标
/// 少几个字——不 panic、不报错。
#[test]
fn the_text_length_matches_what_the_mapping_says() {
    for source in CORPUS {
        let buffer = TextBuffer::new((*source).to_owned());
        let snapshot = buffer.snapshot();
        let (_, visual) = block(&snapshot, 0);
        assert_eq!(
            visual.visual_len().get(),
            visual.text().len() as u64,
            "语料 {source:?}"
        );
        assert_eq!(
            visual
                .source_to_visual(visual.source_range().end(), Bias::After)
                .expect("块末尾"),
            visual.visual_len(),
            "语料 {source:?} 的块末尾没落在视觉末尾"
        );
    }
}

/// 视觉文本恰好是块里没被隐藏的那些字节。
#[test]
fn the_text_is_the_block_minus_its_hidden_bytes() {
    for source in CORPUS {
        let buffer = TextBuffer::new((*source).to_owned());
        let snapshot = buffer.snapshot();
        let (decorations, visual) = block(&snapshot, 0);
        let mut hidden: Vec<(usize, usize)> = decorations
            .set()
            .all()
            .iter()
            .filter(|entry| entry.decoration.hides_source())
            .map(|entry| {
                (
                    entry.range.start().get() as usize,
                    entry.range.end().get() as usize,
                )
            })
            .collect();
        hidden.sort_unstable();
        let mut expected = String::new();
        let mut cursor = decorations.range().start().get() as usize;
        let end = decorations.range().end().get() as usize;
        for (from, to) in hidden {
            if from > cursor {
                expected.push_str(&source[cursor..from.min(end)]);
            }
            cursor = cursor.max(to);
        }
        if cursor < end {
            expected.push_str(&source[cursor..end]);
        }
        assert_eq!(visual.text(), expected, "语料 {source:?}");
    }
}

/// **块局部的视觉偏移从 0 开始。**
///
/// 装饰集合的视觉坐标是整篇文档的，而 `BlockLayout` 排的是一个块。少减一次
/// 原点的表现是「点第二个块，光标落进第一个块里」——不报错。
#[test]
fn a_block_visual_text_starts_at_zero_no_matter_where_the_block_is() {
    let source = "# 标题\n\n段落 *斜体*\n\n> 引用\n";
    let buffer = TextBuffer::new(source.to_owned());
    let snapshot = buffer.snapshot();
    let markdown = yu_markdown::parse(&snapshot);
    let mut seen = 0;
    for index in 0..markdown.blocks().len() {
        let Some(parsed) = markdown.blocks().get(index) else {
            continue;
        };
        if parsed.range().is_empty() {
            continue;
        }
        let (_, visual) = block(&snapshot, index);
        assert_eq!(
            visual
                .source_to_visual(visual.source_range().start(), Bias::After)
                .expect("块起点"),
            yu_core::VisualOffset::ZERO,
            "第 {index} 个块的视觉起点"
        );
        seen += 1;
    }
    assert!(seen >= 3, "语料里至少有三个非空块，实际 {seen}");
}

/// 可见字节上 source → visual → source 无损（不变量 D4）。
#[test]
fn visible_offsets_round_trip() {
    for source in CORPUS {
        let buffer = TextBuffer::new((*source).to_owned());
        let snapshot = buffer.snapshot();
        let (_, visual) = block(&snapshot, 0);
        for boundary in 0..=visual.text().len() {
            if !visual.text().is_char_boundary(boundary) {
                continue;
            }
            let at = yu_core::VisualOffset::new(boundary as u64);
            let source_offset = visual
                .visual_to_source(at, Bias::After)
                .expect("映射回源码");
            let back = visual
                .source_to_visual(source_offset, Bias::After)
                .expect("再映射回视觉");
            assert_eq!(back, at, "语料 {source:?} 的视觉 {boundary}");
        }
    }
}

/// 落在块外的源码偏移必须被拒绝，不能夹到边界上悄悄给个答案。
#[test]
fn a_source_offset_outside_the_block_is_rejected() {
    let source = "# 标题\n\n段落\n";
    let buffer = TextBuffer::new(source.to_owned());
    let snapshot = buffer.snapshot();
    let (_, visual) = block(&snapshot, 2);
    assert!(matches!(
        visual.source_to_visual(offset(0), Bias::After),
        Err(VisualTextError::SourceOutsideRange { .. })
    ));
}

// ------------------------------------------------------------------ preedit

fn composed(source: &str, replacement: TextRange, preedit: &str) -> VisualText {
    let buffer = TextBuffer::new(source.to_owned());
    let snapshot = buffer.snapshot();
    let (_, visual) = block(&snapshot, 0);
    visual
        .with_composition(replacement, preedit, TextRange::empty(ByteOffset::ZERO))
        .expect("叠 preedit")
}

/// preedit 的文字进视觉文本，被替换的那一段出去。
#[test]
fn a_preedit_replaces_its_range_in_the_visual_text() {
    let visual = composed("**粗体**", range(2, 8), "日本");
    assert_eq!(visual.text(), "日本");
    assert_eq!(visual.composition_text(), Some("日本"));
    assert_eq!(visual.composition_range(), Some(range(2, 8)));
}

/// **preedit 比被替换的文字短时，后面的视觉偏移往前挪。**
///
/// 平移量可以是负数。用无符号数算会饱和到 0——不 panic、不报错，只是
/// preedit 之后的每一个光标位置都差几个字节。
#[test]
fn a_shorter_preedit_pulls_the_following_offsets_back() {
    let source = "abcdef";
    let visual = composed(source, range(1, 4), "x");
    assert_eq!(visual.text(), "axef");
    // 断言必须落在替换区间**严格之后**：正好等于终点的那个偏移由
    // composition 分支直接回答，平移量算错了它也对。`f` 在 source 里是 5，
    // 叠 preedit 之后视觉上排在第 3 个字节。
    assert_eq!(
        visual
            .source_to_visual(offset(5), Bias::After)
            .expect("preedit 之后")
            .get(),
        3
    );
    assert_eq!(
        visual
            .visual_to_source(yu_core::VisualOffset::new(3), Bias::After)
            .expect("反过来")
            .get(),
        5
    );
}

/// preedit 更长时同理，往后挪。
#[test]
fn a_longer_preedit_pushes_the_following_offsets_forward() {
    let source = "abcdef";
    let visual = composed(source, range(1, 2), "xyz");
    assert_eq!(visual.text(), "axyzcdef");
    assert_eq!(
        visual
            .source_to_visual(offset(2), Bias::After)
            .expect("preedit 之后")
            .get(),
        4
    );
}

/// preedit 内部的每一个视觉边界都指回同一段 canonical 替换范围。
///
/// 那段文字根本不在 source 里，只能报它两端之一——报别的位置就是凭空造出
/// 一个源码偏移。
#[test]
fn every_offset_inside_a_preedit_maps_to_the_replaced_range() {
    let visual = composed("abcdef", range(1, 2), "xyz");
    let span = visual.composition_visual().expect("preedit 区间");
    assert_eq!(
        visual
            .visual_to_source(span.start(), Bias::After)
            .expect("起点")
            .get(),
        1
    );
    assert_eq!(
        visual
            .visual_to_source(span.end(), Bias::Before)
            .expect("终点")
            .get(),
        2
    );
    let middle = yu_core::VisualOffset::new(span.start().get() + 1);
    assert_eq!(
        visual
            .visual_to_source(middle, Bias::Before)
            .expect("中间")
            .get(),
        1
    );
    assert_eq!(
        visual
            .visual_to_source(middle, Bias::After)
            .expect("中间")
            .get(),
        2
    );
}

/// preedit 落在一段被隐藏的语法后面时，起点仍然按可见文本算。
#[test]
fn a_preedit_after_hidden_syntax_starts_at_the_visible_offset() {
    let visual = composed("# 标题", range(8, 8), "日");
    assert_eq!(visual.text(), "标题日");
    assert_eq!(
        visual
            .composition_visual()
            .expect("preedit 区间")
            .start()
            .get(),
        "标题".len() as u64
    );
}

/// 不能在已经叠了 preedit 的视觉文本上再叠一层。
///
/// 第二层会拿 canonical 的偏移去切**已经叠过**的文本，切出来的位置是错的，
/// 而且不报错。
#[test]
fn a_second_preedit_is_refused_instead_of_stacking() {
    let visual = composed("abcdef", range(1, 2), "xyz");
    assert!(matches!(
        visual.with_composition(range(1, 2), "q", TextRange::empty(ByteOffset::ZERO)),
        Err(VisualTextError::CompositionAlreadyActive)
    ));
}

/// preedit 内部的选中换算成视觉区间。
#[test]
fn the_preedit_selection_lands_inside_the_preedit() {
    let buffer = TextBuffer::new("abcdef".to_owned());
    let snapshot = buffer.snapshot();
    let (_, visual) = block(&snapshot, 0);
    let composed = visual
        .with_composition(range(1, 2), "日本", range(3, 6))
        .expect("叠 preedit");
    let span = composed.composition_visual().expect("preedit 区间");
    let selection = composed
        .composition_selection_visual()
        .expect("preedit 选中");
    assert_eq!(selection.start().get(), span.start().get() + 3);
    assert_eq!(selection.end().get(), span.start().get() + 6);
}

/// preedit 内部的选中落在字符中间必须被拒绝。
#[test]
fn a_preedit_selection_off_a_char_boundary_is_rejected() {
    let buffer = TextBuffer::new("abcdef".to_owned());
    let snapshot = buffer.snapshot();
    let (_, visual) = block(&snapshot, 0);
    assert!(matches!(
        visual.with_composition(range(1, 2), "日", range(1, 1)),
        Err(VisualTextError::CompositionSelectionNotUtf8Boundary { .. })
    ));
}
