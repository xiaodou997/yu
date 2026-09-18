//! `BlockLayoutInput` 的性质。
//!
//! # 这个文件顶替了什么
//!
//! S6 全程靠一条差分守着：同一个块，一边从 v1 的 `Projection` 派生布局输入，
//! 一边从 `DecorationSet` 派生，逐项比对视觉文本、样式段（连同解析出来的
//! `TextAttrs`）、行级几何与三种装饰。28 份语料里 21 份逐项完全一致，
//! 7 条登记差异全部是 v1 判错。
//!
//! **`yu-projection` 删掉之后那个 oracle 就没有了。** 那 7 条差异的内容留在
//! `docs/architecture/overview-v2.md` 第 8 节 S6 一节里，删除前它们在真实
//! 窗口逐条兑现过（见那一节的「真实窗口」）。差分本身留在 git 里：
//! `git log --oneline -- crates/yu-editor/tests/blockinput_differential.rs`。
//!
//! # 剩下的保障
//!
//! - **语料仍然在。** 下面这一份是差分那 28 份原样搬过来的，只是断言从
//!   「与 v1 一致」换成了自洽性质：样式段无缝铺满视觉文本、视觉文本等于
//!   源码减去被隐藏的字节、每个 id 都查得到。它压不住「隐藏错了字节」，
//!   那件事的 oracle 现在是 CommonMark 官方用例（`yu-syntax`）加
//!   `yu-markdown/tests/extension_decorations.rs`。
//! - **正面钉住的规则。** 标题的字号倍率盖在整张样式表上、链接正文继承
//!   外层加粗——这两条差分时期就是单独写的，因为「两边一起错」也会绿。

use yu_core::{ClusterMetrics, StyleId, TextAttrs, TextRole, TextStyle};
use yu_editor::layout_tokens::LINE_HEIGHT_BODY;
use yu_editor::{BlockLayoutInput, BlockOrnaments, ThematicBreakOrnament, VisualText};
use yu_layout::{LayoutConfig, LineStyleTable, StyleTable};
use yu_markdown::{ExtensionSet, parse};
use yu_syntax::parse as parse_syntax;
use yu_text::TextBuffer;

/// 一份能分辨字型的度量。`MonospaceMetrics` 分辨不了——用它做断言，
/// 「标题排成粗体」这条规则去掉之后一条用例都不会红。
struct StyleSensitive;

impl ClusterMetrics for StyleSensitive {
    fn advance(&self, _cluster: &str, style: TextStyle) -> f32 {
        match style {
            TextStyle::Plain => 1.0,
            TextStyle::Emphasis => 2.0,
            TextStyle::Strong => 4.0,
            TextStyle::Code => 8.0,
            style => {
                if style.is_code() {
                    8.0
                } else if style.is_strong() {
                    4.0
                } else {
                    2.0
                }
            }
        }
    }
}

/// 一个块派生出来的那些东西。
#[derive(Debug, PartialEq)]
struct Derived {
    text: String,
    runs: Vec<(u64, u64, TextAttrs)>,
    indent: f32,
    line_height_scale: f32,
    heading: Option<u8>,
    quote: Option<u8>,
    /// （标记文本，画在哪，占多宽）。
    marker: Option<(String, f32, f32)>,
    /// 分隔线那条横线。不是分隔线就是 `None`。
    rule: Option<ThematicBreakOrnament>,
}

type Ornaments = (
    Option<u8>,
    Option<u8>,
    Option<(String, f32, f32)>,
    Option<ThematicBreakOrnament>,
);

fn ornaments_of(ornaments: &BlockOrnaments) -> Ornaments {
    (
        ornaments.heading().map(|heading| heading.level()),
        ornaments.quote().map(|quote| quote.depth()),
        ornaments
            .marker()
            .map(|marker| (marker.text().to_owned(), marker.x(), marker.advance())),
        ornaments.rule(),
    )
}

fn describe(input: &BlockLayoutInput) -> Derived {
    let layout = input.layout_input();
    // 相邻且**解析后属性相同**的两段合成一段：怎么切是实现细节，要问的是
    // 「每个字节排什么」。
    let mut runs: Vec<(u64, u64, TextAttrs)> = Vec::new();
    for run in layout.runs() {
        let attrs = input
            .styles()
            .attrs(run.style())
            .expect("每一段的样式 id 都必须查得到");
        let (from, to) = (run.visual().start().get(), run.visual().end().get());
        match runs.last_mut() {
            Some(last) if last.2 == attrs && last.1 == from => last.1 = to,
            _ => runs.push((from, to, attrs)),
        }
    }
    let line = input
        .line_styles()
        .attrs(yu_core::LineStyleId(0))
        .expect("整块共用的那一段行级样式");
    let (heading, quote, marker, rule) = ornaments_of(input.ornaments());
    Derived {
        text: input.text().to_owned(),
        runs,
        indent: line.indent(),
        line_height_scale: line.line_height_scale(),
        heading,
        quote,
        marker,
        rule,
    }
}

/// 第 `index` 个块的装饰、视觉文本与派生出来的布局输入。
fn derive(source: &str, index: usize) -> Option<(Derived, BlockLayoutInput, String)> {
    derive_with_theme(source, index, yu_core::ThemeId::Github)
}

fn derive_with_theme(
    source: &str,
    index: usize,
    theme: yu_core::ThemeId,
) -> Option<(Derived, BlockLayoutInput, String)> {
    let config = LayoutConfig::new(400.0, 10.0).with_theme(theme);
    let buffer = TextBuffer::new(source.to_owned());
    let snapshot = buffer.snapshot();
    let document = parse(&snapshot);
    let tree = parse_syntax(&snapshot).expect("测试文档很短").into_tree();
    let block = document.blocks().get(index)?;

    let decorations = ExtensionSet::markdown()
        .decorate(
            &snapshot,
            &tree,
            document.reference_definitions(),
            document.presentation(),
            block,
            None,
        )
        .expect("装饰产出不该失败");
    let visual = VisualText::new(&snapshot, decorations.range(), decorations.set().clone())
        .expect("视觉文本");
    let input = BlockLayoutInput::from_decorations(
        block.kind(),
        &decorations,
        &visual,
        config,
        &StyleSensitive,
    )
    .expect("从装饰派生");
    let expected = visible_source(source, &decorations);
    Some((describe(&input), input, expected))
}

/// 源码去掉被隐藏的字节。**重叠的隐藏区间只能算一次**——`- [x]` 里 task 与
/// link 各盖一层，数两遍会少几个字。
fn visible_source(source: &str, decorations: &yu_editor::BlockDecorations) -> String {
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
    let mut visible = String::new();
    let mut cursor = decorations.range().start().get() as usize;
    let end = decorations.range().end().get() as usize;
    for (from, to) in hidden {
        if from > cursor {
            visible.push_str(&source[cursor..from.min(end)]);
        }
        cursor = cursor.max(to);
    }
    if cursor < end {
        visible.push_str(&source[cursor..end]);
    }
    visible
}

/// 语料。差分时期那 28 份原样搬过来——每一种语法的常见写法，加上 v1 判错
/// 的那一批（缩进代码块、多重反引号、HTML 注释、autolink 内部）。
const DOCUMENTS: &[&str] = &[
    "段落",
    "普通段落 *斜体* 与 **粗体** 与 `代码`",
    "# 一级标题",
    "## 二级 *斜体* 标题",
    "###### 六级",
    "#   多空格",
    "# 标题 #",
    "> 引用一层",
    "> > 引用两层",
    "- 项目",
    "1. 有序",
    "  - 缩进项",
    "- [ ] 待办",
    "- [x] 完成",
    "```rust\nlet x = 1;\n```",
    "[文字](目标)",
    "[a][b]",
    "![替代](图片)",
    "<http://a.com>",
    "**[文字](目标)**",
    "中文 *强调* 与 emoji 🙂",
    "***both***",
    "    indented *em*\n",
    "``a `b` c``",
    "<!-- comment *em* -->",
    "<http://a.com/*b*>",
    "行尾硬换行  \n第二行",
    "a | b\n--- | ---\n1 | 2",
    "---\n",
];

/// 视觉文本恰好是源码减去被隐藏的字节。
#[test]
fn the_visual_text_is_the_source_minus_the_hidden_bytes() {
    for source in DOCUMENTS {
        let Some((derived, _, expected)) = derive(source, 0) else {
            continue;
        };
        assert_eq!(derived.text, expected, "语料 {source:?}");
    }
}

/// 样式段必须无缝铺满视觉文本。
///
/// 漏掉半段的后果是画面上少几个字——既不 panic 也不报错。
#[test]
fn styled_runs_tile_the_visual_text() {
    for source in DOCUMENTS {
        let Some((derived, _, _)) = derive(source, 0) else {
            continue;
        };
        let mut cursor = 0_u64;
        for (from, to, _) in &derived.runs {
            assert_eq!(*from, cursor, "{source:?} 的样式段之间有空档");
            cursor = *to;
        }
        assert_eq!(
            cursor,
            derived.text.len() as u64,
            "{source:?} 的样式段没铺到视觉文本末尾"
        );
    }
}

/// 每一段样式指向的 id 都必须查得到。
///
/// 查不到不给默认字型：一个「装饰产出与样式表脱节」的 bug 应该响，不应该
/// 只是画得不对。`describe` 里那句 `expect` 已经压着它，这里正面写一遍，
/// 顺带确认表**不是**长到无穷。
#[test]
fn every_style_id_resolves_and_unknown_ones_do_not() {
    for source in DOCUMENTS {
        let Some((_, input, _)) = derive(source, 0) else {
            continue;
        };
        for run in input.layout_input().runs() {
            assert!(
                input.styles().attrs(run.style()).is_some(),
                "{source:?} 的样式 id {:?} 查不到",
                run.style()
            );
        }
        assert!(
            input.styles().attrs(StyleId(u32::MAX)).is_none(),
            "{source:?} 的样式表不该认这个 id"
        );
    }
}

/// 空块不该 panic，也不该产出半个 run。
#[test]
fn an_empty_block_derives_an_empty_input() {
    let Some((derived, input, _)) = derive("", 0) else {
        return;
    };
    assert_eq!(derived.text, "");
    assert!(input.layout_input().runs().is_empty());
}

/// 标题的字号倍率盖在**整张**样式表上，不靠 heading 产一条覆盖全块的 Mark。
///
/// 「几级标题」是语义，归 `yu-markdown`；「1.6 倍、排粗体」是呈现，只有这一
/// 层有 `LayoutConfig` 说得出来。让 extension 产一条 `Strong` 的 Mark 也能
/// work，但那等于把呈现决定塞回刚划清界限的那一层。
///
/// 倍率本身（h2 = 1.6）住在 `layout_tokens::HEADING_FONT_SCALE`，那里的注释
/// 讲为什么是这个数。
#[test]
fn a_heading_scales_every_style_in_the_table() {
    let (_, input, _) = derive("## h2 *em* `code`", 0).expect("派生");
    let layout = input.layout_input();
    assert!(!layout.runs().is_empty(), "标题里有三段不同字型的文字");
    for run in layout.runs() {
        let attrs = input.styles().attrs(run.style()).expect("查得到");
        assert!(
            attrs.style().is_strong(),
            "all heading runs inherit heading weight"
        );
        assert_eq!(
            attrs.size_scale(),
            yu_editor::layout_tokens::HEADING_FONT_SCALE[1],
            "heading scale with inline-code ratio"
        );
    }
    assert_eq!(input.ornaments().heading().map(|h| h.level()), Some(2));
}

/// 嵌套的任务项也要往右让。
///
/// 缩进此前挂在**标记装饰**上，而任务项按设计不产标记（`- ` 原样留在正文里，
/// 这样任务项画成 `- ☐ 待办`、普通项画成 `• 项目`）。于是同一层的普通列表项
/// 缩进了，任务项贴着左边缘——不报错、不少字，只有画面上看得见。
///
/// 断言写成**相对**关系，不写死列宽：要问的是「缩进只有一个来源」。
#[test]
fn a_nested_task_item_is_indented_like_a_nested_list_item() {
    let (top_task, _, _) = derive("- [x] 顶层\n", 0).expect("派生");
    let (nested_task, _, _) = derive("- 外\n  - [x] 内\n", 1).expect("派生");
    let (top_item, _, _) = derive("- 外\n- 第二\n", 1).expect("派生");
    let (nested_item, _, _) = derive("- 外\n  - 内\n", 1).expect("派生");

    assert!(
        nested_task.indent > top_task.indent,
        "嵌套的任务项要比顶层的往右让"
    );
    assert_eq!(
        nested_task.indent - top_task.indent,
        nested_item.indent - top_item.indent,
        "两种块「多让了多少」是同一个数——缩进只有一个来源"
    );
    assert!(nested_task.marker.is_none(), "任务项不该有替代标记");

    assert!(
        nested_task.indent < nested_item.indent,
        "checkbox occupies space before the aligned list text"
    );
}

/// 分隔线：横线横跨正文那一栏，竖直居中于块高。
///
/// **三个数各自说一件事，写死它们是有意的。** 常数没有断言就等于没有约定
/// （第十刀学到的那一条）：x 写错线会悬空、宽度写错线会比正文长出一截、
/// 居中改成贴边线会紧挨着上一块的文字看上去像那一块的下划线——三种错法都
/// 不 panic、不报错，只有画面上看得见。
///
/// `LayoutConfig::new(400.0, 10.0)`：正文栏宽 400，行高 10，于是粗细是
/// `clamp(10 × 0.06, 1, 2) = 1`。
#[test]
fn a_thematic_break_draws_a_centered_rule_across_the_text_column() {
    let (derived, _, _) = derive("---\n", 0).expect("派生");
    let rule = derived.rule.expect("分隔线块要带上那条横线");
    assert_eq!(rule.thickness(), 1.0);

    let bounds = rule.bounds(10.0).expect("矩形");
    assert_eq!(bounds.x(), 0.0, "线从正文那一栏的左沿起");
    assert_eq!(bounds.width(), 400.0, "横跨整栏");
    assert_eq!(bounds.height(), 1.0);
    assert_eq!(bounds.y(), 4.5, "竖直居中于块高");
}

/// 分隔线不改变排版：它那一块排出来就是一个空行。
///
/// 线是**装饰**，一个字节都不进 `LayoutInput`。这条钉住的是那句话的下半段
/// ——把线做成一段占位文字也能画出来，代价是光标能走进一个 source 里没有的
/// 位置。
#[test]
fn a_thematic_break_lays_out_as_a_blank_line() {
    let (derived, input, _) = derive("---\n", 0).expect("派生");
    assert_eq!(derived.text, "\n", "三个减号不进视觉文本，换行符留着");
    assert_eq!(derived.indent, 0.0);
    assert!(input.layout_input().widgets().is_empty());
}

/// 块级盒模型的水平内边距：代码块与引用块收窄断行宽度、内容右移；
/// 行高倍率按块类给（正文 1.6、代码 1.65、标题 1.3/1.25）。
///
/// 内边距的数值住在 `layout_tokens`，这里钉的是「收窄与右移是同一个数」——
/// 两个数分叉的表现是文字溢出背景盒或短一截，不报错。
#[test]
fn code_and_quote_blocks_shrink_the_wrap_width_by_their_padding() {
    // derive 的配置是 `LayoutConfig::new(400.0, 10.0)`。
    let (code, input, _) = derive("```rust\nlet x = 1;\n```", 0).expect("代码块");
    assert_eq!(
        input.layout_config().max_width(),
        400.0 - 5.625,
        "右侧边距收窄布局右边界；左边距只由缩进扣除"
    );
    assert_eq!(code.indent, 8.125, "代码内容右移一个内边距");
    assert!((code.line_height_scale - 1.44).abs() < 0.00001);

    // 缩进代码与围栏是同一种盒模型待遇。
    let (indented, input, _) = derive("    let x = 1;\n", 0).expect("缩进代码块");
    assert_eq!(input.layout_config().max_width(), 400.0 - 5.625);
    assert_eq!(indented.indent, 8.125);
    assert!((indented.line_height_scale - 1.44).abs() < 0.00001);

    let (quote, input, _) = derive("> quoted\n", 0).expect("引用块");
    assert_eq!(
        input.layout_config().max_width(),
        400.0 - 15.0 * 10.0 / 16.0,
        "引用块断行宽度收窄两个水平内边距"
    );
    assert!(
        quote.indent > 0.0,
        "引用内容的缩进 = 内边距 + 竖条 gutter：{}",
        quote.indent
    );
    assert_eq!(quote.line_height_scale, LINE_HEIGHT_BODY);

    let (item, input, _) = derive("- 项目\n", 0).expect("列表项");
    assert_eq!(input.layout_config().max_width(), 400.0, "列表项不收窄");
    assert_eq!(item.line_height_scale, LINE_HEIGHT_BODY);

    let (paragraph, input, _) = derive("plain\n", 0).expect("段落");
    assert_eq!(input.layout_config().max_width(), 400.0, "段落不收窄");
    assert_eq!(paragraph.indent, 0.0);
    assert_eq!(
        paragraph.line_height_scale, LINE_HEIGHT_BODY,
        "正文行高 1.6"
    );

    let (h1, _, _) = derive("# h1\n", 0).expect("h1");
    assert_eq!(h1.line_height_scale, 2.7);
    let (h2, _, _) = derive("## h2\n", 0).expect("h2");
    assert!((h2.line_height_scale - 2.14375).abs() < 0.00001);
    for level in 3..=6 {
        let marks = "#".repeat(level);
        let (heading, _, _) = derive(&format!("{marks} h{level}\n"), 0).expect("h3–h6");
        assert!(
            (heading.line_height_scale - [2.145, 1.75, 1.4, 1.4][level - 3]).abs() < 0.00001,
            "h{level} 的行高倍率"
        );
    }
}

/// 光标进这一块，线就没了。
///
/// `derive` 一律不给焦点，所以这一条自己造一个焦点块。它压的是「露出源码」
/// 与「不画线」由**同一个事实**带着走：`yu-markdown` 不产那条装饰，这一层
/// 自然就没有可画的东西，不需要在这里再判一次焦点。
#[test]
fn a_focused_thematic_break_carries_no_rule() {
    let config = LayoutConfig::new(400.0, 10.0);
    let buffer = TextBuffer::new("---\n".to_owned());
    let snapshot = buffer.snapshot();
    let document = parse(&snapshot);
    let tree = parse_syntax(&snapshot).expect("测试文档很短").into_tree();
    let block = document.blocks().get(0).expect("一个块");
    let active = yu_core::TextRange::new(yu_core::ByteOffset::new(1), yu_core::ByteOffset::new(1))
        .expect("空区间");
    let decorations = ExtensionSet::markdown()
        .decorate(
            &snapshot,
            &tree,
            document.reference_definitions(),
            document.presentation(),
            block,
            Some(active),
        )
        .expect("装饰产出不该失败");
    let visual = VisualText::new(&snapshot, decorations.range(), decorations.set().clone())
        .expect("视觉文本");
    let input = BlockLayoutInput::from_decorations(
        block.kind(),
        &decorations,
        &visual,
        config,
        &StyleSensitive,
    )
    .expect("从装饰派生");
    assert_eq!(input.text(), "---\n", "焦点块按源码排");
    assert!(input.ornaments().rule().is_none(), "露着源码就不画线");
}

/// 标记与正文之间空一列。
///
/// 这个常数没有断言的时候，把它从几何里拿掉一条用例都不红——**常数没有断言
/// 就等于没有约定**（第十刀学到的那一条）。拿掉之后 `•项目` 会挤在一起。
///
/// `LayoutConfig::new` 的 `default_advance` 默认是 1.0，这里的 `1.0` 就是
/// 「一列」。
#[test]
fn a_list_marker_and_its_text_are_one_column_apart() {
    for source in ["- 项目", "1. 有序", "  - 缩进项"] {
        let (derived, _, _) = derive(source, 0).expect("派生");
        let (_, x, advance) = derived.marker.expect("列表项该有标记");
        assert_eq!(
            derived.indent - (x + advance),
            1.0,
            "{source:?}：正文该从标记右边一列处起"
        );
    }
}

/// 普通段落各段各排各的字型，倍率是 1。
#[test]
fn a_paragraph_keeps_every_style_at_its_own_face() {
    let (derived, _, _) = derive("plain *em* `code`", 0).expect("派生");
    let faces: Vec<_> = derived
        .runs
        .iter()
        .map(|(_, _, attrs)| (attrs.style(), attrs.size_scale()))
        .collect();
    assert!(faces.contains(&(TextStyle::Plain, 1.0)));
    assert!(faces.contains(&(TextStyle::Emphasis, 1.0)));
    assert!(faces.contains(&(TextStyle::Code, 0.9)));
}

#[test]
fn link_text_inside_bold_inherits_weight_and_preserves_link_role() {
    let (derived, _, _) = derive("**[文字](目标)**", 0).expect("派生");
    assert_eq!(derived.text, "文字");
    assert_eq!(
        derived.runs,
        vec![(
            0,
            6,
            TextAttrs::new(TextStyle::Strong)
                .with_role(TextRole::Link)
                .with_font(yu_core::ThemeFont::OpenSans)
        )],
        "链接正文保留加粗字型 + 链接角色"
    );
}

/// 两个 extension 盖在同一段 source 上时，视觉文本不能把那一段算两遍。
///
/// `- [x] 完成` 里 task 隐藏整个 `[x]`、link 另外隐藏它的两个 `LinkMark`，
/// 三条隐藏区间彼此重叠。重叠由 `DecorationSet` 在构造期合并掉——装配层
/// 不预先合并，那是死代码（变异验证发现的）。
#[test]
fn overlapping_hidden_ranges_are_not_counted_twice() {
    let (derived, _, _) = derive("- [x] 完成", 0).expect("派生");
    assert_eq!(
        derived.text, "完成",
        "task prefix is replaced by a checkbox"
    );
}

#[test]
fn native_list_markers_follow_theme_and_container_depth_without_changing_source() {
    let source = "- root\n  - child\n    - grandchild\n1. ordered\n";
    for (theme, expected) in [
        (yu_core::ThemeId::Github, ["•", "◦", "▪", "1."]),
        (yu_core::ThemeId::Night, ["▪", "▪", "▪", "1."]),
    ] {
        for (index, expected) in expected.into_iter().enumerate() {
            let (derived, _input, visible) =
                derive_with_theme(source, index, theme).expect("list block");
            assert_eq!(derived.marker.expect("marker").0, expected);
            assert_eq!(
                derived.text, visible,
                "marker styling must preserve projection"
            );
        }
    }
}

#[test]
fn inline_code_boxes_survive_markdown_projection_with_distinct_identity() {
    let source = "`a`*`b`*";
    for theme in [yu_core::ThemeId::Github, yu_core::ThemeId::Night] {
        let (_, input, visible) = derive_with_theme(source, 0, theme).expect("paragraph");
        assert_eq!(visible, "ab");
        assert_eq!(input.text(), "ab");
        let layout = yu_layout::BlockLayout::build_all(
            input.layout_input(),
            input.layout_config(),
            input.styles(),
            &yu_layout::NoWidgets,
            input.line_styles(),
            &StyleSensitive,
        )
        .expect("layout");
        let boxes = layout.inline_boxes();
        assert_eq!(
            boxes.len(),
            2,
            "adjacent projected code spans stay separate"
        );
        assert_ne!(boxes[0].id, boxes[1].id);
        for fragment in boxes {
            assert!(fragment.left_edge && fragment.right_edge);
            let cluster = layout
                .clusters()
                .iter()
                .find(|cluster| cluster.visual() == fragment.range)
                .expect("source cluster");
            assert!(fragment.bounds.x() < cluster.x());
            assert!(fragment.bounds.x() + fragment.bounds.width() > cluster.x() + cluster.width());
        }
    }
}

#[test]
fn quote_right_padding_applies_once_to_mixed_container_leaves() {
    for source in ["> - word\n", "- > word\n", "> > word\n", "> ## heading\n"] {
        let (_, github, _) = derive_with_theme(source, 0, yu_core::ThemeId::Github).expect("quote");
        assert_eq!(github.layout_config().max_width(), 390.625, "{source:?}");
        let (_, night, _) = derive_with_theme(source, 0, yu_core::ThemeId::Night).expect("quote");
        assert_eq!(night.layout_config().max_width(), 400.0, "{source:?}");
    }
}

#[test]
fn alternating_list_quote_prefixes_are_all_projected_out() {
    let source = "- > - > first word\n";
    let (derived, _, _) = derive(source, 0).expect("nested item");
    assert_eq!(derived.text.trim_end(), "first word");
}

#[test]
fn nested_font_traits_compose_without_changing_visual_text() {
    for (source, word, expected) in [
        ("***both***", "both", TextStyle::StrongEmphasis),
        ("**a *both* c**", "both", TextStyle::StrongEmphasis),
        ("*a **both** c*", "both", TextStyle::StrongEmphasis),
        ("## *both*", "both", TextStyle::StrongEmphasis),
        ("## `code`", "code", TextStyle::CodeStrong),
        ("## *`code`*", "code", TextStyle::CodeStrongEmphasis),
        ("**`code`**", "code", TextStyle::CodeStrong),
        ("*[**both**](target)*", "both", TextStyle::StrongEmphasis),
    ] {
        let (derived, _, _) = derive(source, 0).expect("derive nested traits");
        let start = derived.text.find(word).expect("visible text remains");
        let relevant: Vec<_> = derived
            .runs
            .iter()
            .filter(|(a, b, _)| *a < (start + word.len()) as u64 && *b > start as u64)
            .collect();
        assert!(!relevant.is_empty());
        assert!(
            relevant
                .iter()
                .all(|(_, _, attrs)| attrs.style() == expected),
            "{source}: {:?}",
            derived.runs
        );
        if source.contains("target") {
            assert!(
                relevant
                    .iter()
                    .all(|(_, _, attrs)| attrs.role() == TextRole::Link)
            );
        }
        if expected.is_code() {
            assert!(
                relevant
                    .iter()
                    .all(|(_, _, attrs)| attrs.inline_box_id().is_some())
            );
        }
    }
}

#[test]
fn heading_code_inherits_theme_weight_and_keeps_explicit_traits() {
    for (theme, weights) in [
        (yu_core::ThemeId::Github, [true; 6]),
        (
            yu_core::ThemeId::Night,
            [false, true, true, false, true, false],
        ),
    ] {
        for (index, bold) in weights.into_iter().enumerate() {
            for (markup, explicit_bold, italic) in [
                ("`code`", false, false),
                ("*`code`*", false, true),
                ("**`code`**", true, false),
            ] {
                let source = format!("{} {markup}", "#".repeat(index + 1));
                let (derived, _, _) = derive_with_theme(&source, 0, theme).expect("heading");
                assert_eq!(derived.text, "code");
                for (_, _, attrs) in derived.runs {
                    assert!(attrs.style().is_code());
                    assert_eq!(
                        attrs.style().is_strong(),
                        bold || explicit_bold,
                        "{source} {theme:?}"
                    );
                    assert_eq!(attrs.style().is_emphasis(), italic);
                    assert_eq!(attrs.font(), theme.spec().code_font);
                    let code_scale = match theme {
                        yu_core::ThemeId::Github
                        | yu_core::ThemeId::YuLight
                        | yu_core::ThemeId::YuDark => 1.0,
                        yu_core::ThemeId::Night => 0.875,
                    };
                    assert_eq!(
                        attrs.size_scale(),
                        theme.spec().heading_sizes[index] * code_scale
                    );
                    assert!(attrs.inline_box_id().is_some());
                }
            }
        }
    }
}
