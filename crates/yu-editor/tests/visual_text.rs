//! `VisualText` 的性质。
//!
//! # 它压的是哪三件事
//!
//! `DecorationSet` 的双向映射本身另有守护：`yu-decoration` 的 `hidden.rs`
//! 里有一份不走树的线性参照实现，树的下降与它逐点一致（S4 时期还拿 v1 的
//! `Projection` 逐点比过，那条差分随 v1 一起删了）。这里压的是 `VisualText`
//! **加在它上面**的三样：
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
        .decorate(
            snapshot,
            &tree,
            markdown.reference_definitions(),
            markdown.presentation(),
            block,
            None,
        )
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

#[test]
fn replacement_text_and_geometry_share_normalized_atoms() {
    use yu_decoration::{Decoration, DecorationRange, DecorationSet};
    let snapshot = TextBuffer::new("a<br>z").snapshot();
    let set = DecorationSet::new(
        snapshot.revision(),
        snapshot.len_bytes(),
        [DecorationRange::new(
            range(1, 5),
            Decoration::Substitute { text: '\n'.into() },
        )],
    );
    let visual = VisualText::new(&snapshot, range(0, 6), set.clone()).expect("projection");
    assert_eq!(visual.text(), "a\nz");
    assert_eq!(visual.visual_len().get(), set.visual_len().get());
    for bias in [Bias::Before, Bias::After] {
        assert_eq!(
            visual
                .visual_to_source(yu_core::VisualOffset::new(1), bias)
                .expect("before break"),
            offset(1)
        );
        assert_eq!(
            visual
                .visual_to_source(yu_core::VisualOffset::new(2), bias)
                .expect("after break"),
            offset(5)
        );
    }
    for from in 0..=6 {
        for to in from..=6 {
            let sliced = VisualText::new(&snapshot, range(from, to), set.clone()).expect("slice");
            assert_eq!(
                sliced.text().len() as u64,
                set.source_to_visual(offset(to)).get() - set.source_to_visual(offset(from)).get(),
                "slice {from}..{to}"
            );
        }
    }
    let composed = visual
        .with_composition(range(1, 5), "中文", range(0, 0))
        .expect("preedit over atom");
    assert_eq!(composed.text(), "a中文z");
    assert_eq!(snapshot.as_str(), "a<br>z");
    assert_eq!(
        composed
            .visual_to_source(yu_core::VisualOffset::new(7), Bias::After)
            .expect("after preedit"),
        offset(5)
    );
}

#[test]
fn unicode_substitutions_expand_without_moving_source_boundaries() {
    use yu_decoration::{Decoration, DecorationRange, DecorationSet};
    let snapshot = TextBuffer::new("AxZ").snapshot();
    let set = DecorationSet::new(
        snapshot.revision(),
        snapshot.len_bytes(),
        [DecorationRange::new(
            range(1, 2),
            Decoration::Substitute {
                text: '🪶'.into()
            },
        )],
    );
    let visual = VisualText::new(&snapshot, range(0, 3), set).expect("Unicode atom");
    assert_eq!(visual.text(), "A🪶Z");
    assert_eq!(
        visual
            .source_to_visual(offset(2), Bias::After)
            .expect("after atom")
            .get(),
        5
    );
    assert_eq!(
        visual
            .visual_to_source(yu_core::VisualOffset::new(5), Bias::After)
            .expect("source edge"),
        offset(2)
    );
}

#[test]
fn native_document_projects_plain_html_break_tags_but_preserves_code_and_escapes() {
    use yu_editor::EditorDocument;
    use yu_layout::LayoutConfig;
    for (source, expected) in [
        ("a<br>b", "a\nb"),
        ("a<BR />b<br/>c", "a\nb\nc"),
        ("a`<br>`b", "a<br>b"),
        (r"a\<br>b", "a<br>b"),
        ("a<br class=\"x\">b", "a<br class=\"x\">b"),
        ("a</br>b", "a</br>b"),
    ] {
        let mut document = EditorDocument::new(source);
        let layout = document
            .block_layout(0, LayoutConfig::new(400.0, 16.0))
            .expect("native layout");
        assert_eq!(layout.visual().text(), expected, "source={source:?}");
        assert_eq!(document.snapshot().as_str(), source);
    }
}

#[test]
fn table_breaks_raise_cell_height_and_keep_caret_rows_distinct() {
    use yu_editor::EditorDocument;
    use yu_layout::LayoutConfig;
    for prefix in ["", "> ", "> > "] {
        for zoom in [0.75, 1.0, 1.5] {
            let source = format!(
                "{prefix}| H | V |\n{prefix}| --- | --- |\n{prefix}| first<br>second | x |\n"
            );
            let mut document = EditorDocument::new(source.clone());
            let layout = document
                .block_layout(0, LayoutConfig::new(500.0 * zoom, 16.0 * zoom))
                .expect("table layout");
            assert!(layout.visual().text().contains("first\nsecond"));
            let from = layout
                .caret_for_source(
                    offset(source.find("first").expect("first") as u64),
                    Bias::After,
                )
                .expect("first caret");
            let to = layout
                .caret_for_source(
                    offset(source.find("second").expect("second") as u64),
                    Bias::After,
                )
                .expect("second caret");
            assert!(to.point().y() > from.point().y() + 8.0 * zoom);
            let tall = layout
                .table()
                .expect("table")
                .cells()
                .iter()
                .find(|cell| cell.row() == 1)
                .expect("body cell")
                .bounds()
                .height();
            let plain = source.replace("<br>", " ");
            let mut plain = EditorDocument::new(plain);
            let plain = plain
                .block_layout(0, LayoutConfig::new(500.0 * zoom, 16.0 * zoom))
                .expect("single line");
            let short = plain
                .table()
                .expect("table")
                .cells()
                .iter()
                .find(|cell| cell.row() == 1)
                .expect("body cell")
                .bounds()
                .height();
            assert!(tall > short + 8.0 * zoom);
        }
    }
}

#[test]
fn table_break_deletion_removes_one_atom_and_undo_restores_source() {
    use yu_editor::{CaretAffinity, EditorCommand, EditorDocument, EditorSelection};
    for prefix in ["", "> "] {
        for tag in ["<br>", "<BR />", "<br/>", "&#32;", "&#9;", "&#x2003;"] {
            for backward in [false, true] {
                let source = format!("{prefix}| H |\n{prefix}| --- |\n{prefix}| a{tag}b |\n");
                let mut document = EditorDocument::new(source.clone());
                let at = source.find(tag).expect("tag") + if backward { tag.len() } else { 0 };
                let selection = EditorSelection::cursor(
                    &document.snapshot(),
                    offset(at as u64),
                    CaretAffinity::Downstream,
                )
                .expect("caret");
                document.set_selection(selection).expect("selection");
                document
                    .execute(if backward {
                        EditorCommand::DeleteBackward
                    } else {
                        EditorCommand::DeleteForward
                    })
                    .expect("delete atom");
                assert_eq!(document.snapshot().as_str(), source.replace(tag, ""));
                document.execute(EditorCommand::Undo).expect("undo");
                assert_eq!(document.snapshot().as_str(), source);
            }
        }
    }
}

#[test]
fn table_arrow_steps_and_shift_selection_keep_projection_atoms_whole() {
    use yu_editor::{CaretAffinity, EditorCommand, EditorDocument, EditorSelection};
    for prefix in ["", "> ", "> > "] {
        let atoms = [
            "<br>",
            "<br>",
            "&#32;",
            "&#x2003;",
            "&amp;",
            "&NotEqualTilde;",
            "&fjlig;",
            "\\|",
            "👨‍👩‍👧‍👦",
        ];
        let source = format!(
            "{prefix}| H |\n{prefix}| --- |\n{prefix}| a{}z |",
            atoms.concat()
        );
        let mut document = EditorDocument::new(source.clone());
        let start = source.find("<br>").expect("first atom");
        let cursor = EditorSelection::cursor(
            &document.snapshot(),
            offset(start as u64),
            CaretAffinity::Downstream,
        )
        .expect("cursor");
        document.set_selection(cursor).expect("selection");
        let mut at = start;
        for atom in atoms {
            document.execute(EditorCommand::MoveRight).expect("right");
            at += atom.len();
            assert_eq!(document.selection().focus(), offset(at as u64));
        }
        for atom in atoms.into_iter().rev() {
            document.execute(EditorCommand::MoveLeft).expect("left");
            at -= atom.len();
            assert_eq!(document.selection().focus(), offset(at as u64));
        }
        document
            .execute(EditorCommand::ExtendHorizontal {
                forward: true,
                word: false,
            })
            .expect("select break");
        let selected = document.selection().ordered_range();
        assert_eq!(selected.start(), offset(start as u64));
        assert_eq!(selected.end(), offset((start + 4) as u64));
        document
            .execute(EditorCommand::DeleteSelections)
            .expect("delete selected break");
        assert_eq!(document.snapshot().as_str(), source.replacen("<br>", "", 1));
        document.execute(EditorCommand::Undo).expect("undo");
        assert_eq!(document.snapshot().as_str(), source);
    }
}

#[test]
fn projected_table_graphemes_combine_after_escapes_but_not_after_line_breaks() {
    use yu_editor::{CaretAffinity, EditorCommand, EditorDocument, EditorSelection};
    for (text, start_delta, end_delta) in [
        ("\\*\u{301}", 0, 4),
        ("&#32;\u{301}", 0, 7),
        ("<br>\u{301}", 4, 6),
        ("&amp;", 0, 5),
        ("&NotEqualTilde;", 0, 15),
        ("&fjlig;", 0, 7),
    ] {
        for forward in [false, true] {
            let source = format!("| H |\n| --- |\n| a{text}z |");
            let base = source.find(text).expect("text");
            let from = base + if forward { start_delta } else { end_delta };
            let mut document = EditorDocument::new(source.clone());
            let cursor = EditorSelection::cursor(
                &document.snapshot(),
                offset(from as u64),
                CaretAffinity::Downstream,
            )
            .expect("cursor");
            document.set_selection(cursor).expect("selection");
            document
                .execute(if forward {
                    EditorCommand::MoveRight
                } else {
                    EditorCommand::MoveLeft
                })
                .expect("move");
            assert_eq!(
                document.selection().focus(),
                offset((base + if forward { end_delta } else { start_delta }) as u64),
                "{text} forward={forward}"
            );
            document.set_selection(cursor).expect("reset");
            document
                .execute(if forward {
                    EditorCommand::DeleteForward
                } else {
                    EditorCommand::DeleteBackward
                })
                .expect("delete");
            let mut expected = source.clone();
            expected.replace_range(base + start_delta..base + end_delta, "");
            assert_eq!(document.snapshot().as_str(), expected);
            document.execute(EditorCommand::Undo).expect("undo");
            assert_eq!(document.snapshot().as_str(), source);
        }
    }
}

#[test]
fn ordinary_escaped_punctuation_projects_without_changing_source() {
    for (source, expected) in [
        (r"literal \<br\> \& \*word\*", "literal <br> & *word*"),
        (r"中文 \[文字\] \\ 🪶", "中文 [文字] \\ 🪶"),
        (r"`\*code\*` and \*text\*", r"\*code\* and *text*"),
        (r"unknown \q", r"unknown \q"),
    ] {
        let snapshot = TextBuffer::new(source).snapshot();
        let (_, visual) = block(&snapshot, 0);
        assert_eq!(visual.text(), expected, "source={source:?}");
        assert_eq!(snapshot.as_str(), source);
    }
}

#[test]
fn escaped_punctuation_reveals_only_when_the_source_selection_touches_it() {
    let snapshot = TextBuffer::new(r"a\*b").snapshot();
    let markdown = yu_markdown::parse(&snapshot);
    let tree = parse_syntax(&snapshot)
        .expect("valid escape fixture")
        .into_tree();
    for (active, expected) in [(range(0, 0), "a*b"), (range(2, 2), r"a\*b")] {
        let decorations = ExtensionSet::markdown()
            .decorate(
                &snapshot,
                &tree,
                markdown.reference_definitions(),
                markdown.presentation(),
                markdown.blocks().get(0).expect("valid escape fixture"),
                Some(active),
            )
            .expect("valid escape fixture");
        let visual = VisualText::new(&snapshot, decorations.range(), decorations.set().clone())
            .expect("valid escape fixture");
        assert_eq!(visual.text(), expected);
        assert_eq!(snapshot.as_str(), r"a\*b");
    }
}

#[test]
fn character_references_use_canonical_projection_and_preserve_source() {
    for (source, expected) in [
        ("# **A &amp; B**", "A & B"),
        ("&copy; &#169; &#x1F600; &NotEqualTilde;", "© © 😀 ≂\u{338}"),
        ("&CounterClockwiseContourIntegral;", "∳"),
        ("&#0; &#xD800; &#1114112;", "� � �"),
        (
            r"\&amp; `&amp;` &unknown; &amp",
            "&amp; &amp; &unknown; &amp",
        ),
        ("```\n&amp;\n```", "&amp;\n"),
        ("[&amp;](target)", "&"),
        (
            "| &amp; | &NotEqualTilde; |\n| --- | --- |\n| &#32; | &#x1F600; |",
            "&≂\u{338} 😀",
        ),
    ] {
        let snapshot = TextBuffer::new(source).snapshot();
        let (_, visual) = block(&snapshot, 0);
        assert_eq!(visual.text(), expected, "{source}");
        assert_eq!(snapshot.as_str(), source);
    }
}

#[test]
fn multi_scalar_reference_has_atomic_source_edges_and_reveals_for_editing() {
    let source = "甲&NotEqualTilde;😀";
    let snapshot = TextBuffer::new(source).snapshot();
    let (decorations, visual) = block(&snapshot, 0);
    assert_eq!(visual.text(), "甲≂\u{338}😀");
    let set = decorations.set();
    // The internal combining-mark boundary must never map inside the entity.
    assert_eq!(
        set.visual_to_source(yu_core::VisualOffset::new(6), Bias::Before),
        offset(3)
    );
    assert_eq!(
        set.visual_to_source(yu_core::VisualOffset::new(6), Bias::After),
        offset(18)
    );
    assert_eq!(set.source_to_visual(offset(18)).get(), 8);
    let markdown = yu_markdown::parse(&snapshot);
    let tree = parse_syntax(&snapshot)
        .expect("valid entity fixture")
        .into_tree();
    let decorations = ExtensionSet::markdown()
        .decorate(
            &snapshot,
            &tree,
            markdown.reference_definitions(),
            markdown.presentation(),
            markdown.blocks().get(0).expect("valid entity fixture"),
            Some(range(5, 5)),
        )
        .expect("valid entity fixture");
    let revealed = VisualText::new(&snapshot, decorations.range(), decorations.set().clone())
        .expect("valid entity fixture");
    assert_eq!(revealed.text(), source);
}
