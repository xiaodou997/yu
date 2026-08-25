//! 新旧两条布局路的几何差分。
//!
//! # 这条测试回答的问题
//!
//! S5 换掉 `yu-layout` 的输入契约：从 `yu_projection::Projection`（一个认识
//! 标题、引用、表格的类型）换成 [`LayoutInput`]（视觉文本 + 不透明的
//! [`StyledRun`]）。**换了契约之后，排出来的还是同一份几何吗？**
//!
//! 做法与 S4 的 `projection_differential.rs` 相同：两条路喂**同一个**
//! `Projection`——新路的输入由它派生，所以任何差异都只能来自布局本身，
//! 不会来自输入。
//!
//! # 这个 oracle 只在这一面可信
//!
//! `LayoutSnapshot` 的断行是**按 grapheme 贪心**：`line_width + advance >
//! max_width` 就地断，不找断行机会。它没有 UAX #14，没有 CJK 禁则，也没有
//! bidi——S5 的验收要求补上这三样，补上之后新旧断点**必然不同**。
//!
//! 所以这条差分只在「两条路本该给出同一个答案」的那一面成立：**同一套
//! 贪心规则下的几何累加、行盒边界、caret 与 hit-test**。它证明不了断行
//! 规则对不对，那件事的 oracle 是 UAX #14 的官方测试套件，不是这里。
//!
//! `oracle_boundary` 那一组用例把这条边界钉成可执行的：它断言现有实现会在
//! 词中间断行。等 UAX #14 落地，那条断言会红，逼人回来把差分的范围重新划
//! 清楚，而不是默默把新实现改回旧行为。

use yu_core::{ByteOffset, CaretAffinity, StyleId, TextAttrs, TextRange, TextStyle, VisualOffset};
use yu_layout::{
    BlockLayout, LayoutConfig, LayoutSnapshot, MonospaceMetrics, StyleTable, StyledRun,
};
use yu_projection::{Projection, ProjectionBias, VisualRunKind};
use yu_text::TextBuffer;

/// 纯段落语料。这一刀只做「样式化文本」，标题/引用/列表标记/表格是
/// `LineStyle` 与 widget 的事（S5 后续几刀），它们的几何还没有新路径。
const CORPUS: &[&str] = &[
    "",
    "a",
    "plain text",
    "*emphasis* and **strong** and `code`",
    "a*b*c",
    "`code with *em* inside`",
    "one two three four five six seven eight",
    "line one\nline two\nline three",
    "*multi\nline*",
    "a *b\nc* d",
    "trailing newline\n",
    "\n",
    "\n\n",
    "中文 *强调* 混排",
    "中*文*强调",
    "emoji 🙂 *后面*",
    "🙂*a*🙂",
    "combining e\u{0301}\u{0301} mark",
    "crlf one\r\ncrlf two",
    "a\u{200b}b\u{200b}c",
    "\u{1f469}\u{200d}\u{1f4bb} zwj family",
    "unmatched *delimiter",
    "text `code *em* more` text",
];

/// 每份语料在这些宽度下各比一遍。3.0 与 5.0 一定会触发折行，80.0 不会。
const WIDTHS: &[f32] = &[3.0, 5.0, 12.0, 80.0];

/// `TextStyle` ↔ `StyleId` 的对照。
///
/// 这张表就是 S5 要回答的「`StyleId` 的解释权归谁」的一个实例：产出装饰的
/// 那一层给 id 赋含义并提供表，布局层只按 id 查表拿到排版属性。这里由测试
/// 扮演那一层——真正的表在 S6 由 `yu-markdown` 的 extension 提供。
struct ProjectionStyleTable;

impl ProjectionStyleTable {
    fn id_for(style: TextStyle) -> StyleId {
        match style {
            TextStyle::Plain => StyleId(0),
            TextStyle::Emphasis => StyleId(1),
            TextStyle::Strong => StyleId(2),
            TextStyle::Code => StyleId(3),
        }
    }
}

impl StyleTable for ProjectionStyleTable {
    fn attrs(&self, style: StyleId) -> Option<TextAttrs> {
        let style = match style {
            StyleId(0) => TextStyle::Plain,
            StyleId(1) => TextStyle::Emphasis,
            StyleId(2) => TextStyle::Strong,
            StyleId(3) => TextStyle::Code,
            _ => return None,
        };
        Some(TextAttrs::new(style))
    }
}

/// 从一个 `Projection` 派生新路径的输入。
///
/// 隐藏的语法不进视觉文本；其余每个 run 原样变成一段 `StyledRun`。这样新旧
/// 两条路看的是同一份视觉字节与同一组样式。
fn derive_input(projection: &Projection) -> (String, Vec<StyledRun>) {
    let mut text = String::new();
    let mut runs = Vec::new();
    for run in projection.runs().iter().copied() {
        if run.kind() == VisualRunKind::HiddenSyntax {
            continue;
        }
        let piece = projection.text_for_run(run).expect("run 的文本可读");
        assert_eq!(
            run.visual().end().get() - run.visual().start().get(),
            piece.len() as u64,
            "run 的视觉长度必须等于它的文本字节数"
        );
        assert_eq!(
            text.len() as u64,
            run.visual().start().get(),
            "可见 run 必须无缝铺满视觉文本"
        );
        text.push_str(&piece);
        runs.push(StyledRun::new(
            run.visual(),
            ProjectionStyleTable::id_for(run.style()),
        ));
    }
    (text, runs)
}

fn projection_of(source: &str) -> Projection {
    let buffer = TextBuffer::new(source.to_owned());
    let snapshot = buffer.snapshot();
    let range = TextRange::new(
        ByteOffset::ZERO,
        ByteOffset::try_from(source.len()).expect("测试文档很短"),
    )
    .expect("有序");
    Projection::inline(&snapshot, range).expect("行内投影")
}

fn both_paths(source: &str, width: f32) -> (LayoutSnapshot, BlockLayout) {
    let projection = projection_of(source);
    let config = LayoutConfig::new(width, 1.0);
    let metrics = MonospaceMetrics::new(config.default_advance());
    let old = LayoutSnapshot::from_projection_with_metrics(&projection, config, &metrics)
        .expect("v1 布局");
    let (text, runs) = derive_input(&projection);
    let new = BlockLayout::build(
        yu_layout::LayoutInput::new(&text, &runs),
        config,
        &ProjectionStyleTable,
        &metrics,
    )
    .expect("v2 布局");
    (old, new)
}

/// 视觉文本本身必须一致。这是几何比对的前提：字节都不一样就没什么好比的。
#[test]
fn derived_visual_text_matches_the_projection() {
    for source in CORPUS {
        let projection = projection_of(source);
        let (text, runs) = derive_input(&projection);
        assert_eq!(
            VisualOffset::try_from(text.len()).expect("短"),
            projection.visual_len(),
            "语料 {source:?} 的视觉长度不一致"
        );
        let tiled = runs
            .last()
            .map_or(VisualOffset::ZERO, |run| run.visual().end());
        assert_eq!(
            tiled,
            projection.visual_len(),
            "语料 {source:?} 的 run 没铺满"
        );
    }
}

#[test]
fn line_boxes_agree() {
    for source in CORPUS {
        for width in WIDTHS {
            let (old, new) = both_paths(source, *width);
            assert_eq!(
                old.lines().len(),
                new.lines().len(),
                "语料 {source:?} 宽度 {width} 的行数不一致"
            );
            for (old_line, new_line) in old.lines().iter().zip(new.lines()) {
                let at = format!("语料 {source:?} 宽度 {width} 第 {} 行", old_line.index());
                assert_eq!(old_line.index(), new_line.index(), "{at} 的行号");
                assert_eq!(old_line.visual(), new_line.visual(), "{at} 的视觉区间");
                assert_eq!(old_line.y(), new_line.y(), "{at} 的 y");
                assert_eq!(old_line.width(), new_line.width(), "{at} 的宽度");
                assert_eq!(
                    old_line.cluster_range(),
                    new_line.cluster_range(),
                    "{at} 的簇区间"
                );
            }
            assert_eq!(
                old.lines().len() as f32 * old.config().line_height(),
                new.height(),
                "语料 {source:?} 宽度 {width} 的块高"
            );
        }
    }
}

#[test]
fn cluster_boxes_agree() {
    for source in CORPUS {
        for width in WIDTHS {
            let (old, new) = both_paths(source, *width);
            assert_eq!(
                old.clusters().len(),
                new.clusters().len(),
                "语料 {source:?} 宽度 {width} 的簇数不一致"
            );
            for (index, (old_cluster, new_cluster)) in
                old.clusters().iter().zip(new.clusters()).enumerate()
            {
                let at = format!("语料 {source:?} 宽度 {width} 第 {index} 个簇");
                assert_eq!(
                    old_cluster.visual(),
                    new_cluster.visual(),
                    "{at} 的视觉区间"
                );
                assert_eq!(old_cluster.line(), new_cluster.line(), "{at} 的行号");
                assert_eq!(old_cluster.x(), new_cluster.x(), "{at} 的 x");
                assert_eq!(old_cluster.width(), new_cluster.width(), "{at} 的宽度");
                assert_eq!(
                    old_cluster.is_line_break(),
                    new_cluster.is_line_break(),
                    "{at} 的换行标记"
                );
                assert_eq!(
                    ProjectionStyleTable::id_for(old_cluster.style()),
                    new_cluster.style(),
                    "{at} 的样式"
                );
            }
        }
    }
}

/// caret 在每个 grapheme 边界上、两种 affinity 下都要落在同一处。
///
/// 只取 grapheme 边界：grapheme 内部的视觉偏移不是合法的 caret 位置，
/// 两条路在那里的行为不构成契约。
#[test]
fn caret_positions_agree_at_cluster_boundaries() {
    for source in CORPUS {
        for width in WIDTHS {
            let (old, new) = both_paths(source, *width);
            let mut offsets = vec![VisualOffset::ZERO];
            for cluster in new.clusters() {
                offsets.push(cluster.visual().end());
            }
            offsets.push(new.visual_len());
            offsets.dedup();

            for offset in offsets {
                for (affinity, bias) in [
                    (CaretAffinity::Upstream, ProjectionBias::Before),
                    (CaretAffinity::Downstream, ProjectionBias::After),
                ] {
                    let at = format!(
                        "语料 {source:?} 宽度 {width} 偏移 {} affinity {affinity:?}",
                        offset.get()
                    );
                    let old_caret = old
                        .caret_for_visual(offset, bias)
                        .unwrap_or_else(|error| panic!("{at} 的 v1 caret: {error}"));
                    let new_caret = new
                        .caret(offset, affinity)
                        .unwrap_or_else(|error| panic!("{at} 的 v2 caret: {error}"));
                    assert_eq!(old_caret.line(), new_caret.line(), "{at} 的行号");
                    assert_eq!(old_caret.point(), new_caret.point(), "{at} 的位置");
                }
            }
        }
    }
}

/// hit-test 在每个簇的左边缘、中点、右边缘上都要落在同一处。
#[test]
fn hit_tests_agree() {
    for source in CORPUS {
        for width in WIDTHS {
            let (old, new) = both_paths(source, *width);
            for line in new.lines() {
                let y = line.y();
                let mut xs = vec![-1.0_f32, 0.0, line.width(), line.width() + 3.0];
                for index in line.cluster_range() {
                    let cluster = new.clusters()[index];
                    xs.push(cluster.x());
                    xs.push(cluster.x() + cluster.width() / 2.0);
                    xs.push(cluster.x() + cluster.width());
                }
                for x in xs {
                    let point = yu_layout::LayoutPoint::new(x, y);
                    let at = format!("语料 {source:?} 宽度 {width} 第 {} 行 x={x}", line.index());
                    let old_hit = old
                        .hit_test(point)
                        .unwrap_or_else(|error| panic!("{at} 的 v1 hit: {error}"));
                    let new_hit = new
                        .hit(point)
                        .unwrap_or_else(|error| panic!("{at} 的 v2 hit: {error}"));
                    assert_eq!(old_hit.visual(), new_hit.visual(), "{at} 的视觉偏移");
                    assert_eq!(old_hit.line(), new_hit.line(), "{at} 的行号");
                    assert_eq!(old_hit.point(), new_hit.point(), "{at} 的位置");
                }
            }
        }
    }
}

/// oracle 的边界：现有实现在**词中间**断行。
///
/// 这一条不是「期望它这样」，是「记下它现在这样」。UAX #14 落地之后
/// `"one two"` 在宽度 5 下应当断在空格处而不是 `"one t"`，那时这条断言会红——
/// 它的作用就是让上面几条差分的适用范围必须被重新审一遍。
#[test]
fn oracle_boundary_current_wrapping_breaks_inside_words() {
    let (old, new) = both_paths("one two three", 5.0);
    let first_old = old.lines().first().expect("至少一行");
    let first_new = new.lines().first().expect("至少一行");
    assert_eq!(first_old.visual(), first_new.visual());
    assert_eq!(
        first_new.visual().end(),
        VisualOffset::new(5),
        "贪心断行会切在 \"one t\" 之后，而不是空格处"
    );
}
