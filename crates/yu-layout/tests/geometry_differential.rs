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
//! bidi——而 S5 的验收要求补上这三样。补上之后新旧断点**必然不同**，
//! 24 个语料×宽度组合确实不同了。
//!
//! 所以这条差分按「有没有发生软换行」分成两个口径：
//!
//! - **没有软换行**（整块的换行全部来自强制换行符）：两条路本该给出同一份
//!   几何，逐点比对行盒、簇盒、caret 与 hit-test。这是 oracle 可信的那一面。
//! - **发生了软换行**：断点必然不同，比对退化为一条仍然成立的性质——
//!   **断行规则只改变 grapheme 被分到哪一行，不改变 grapheme 本身**。
//!   簇的视觉区间、宽度、样式、换行标记逐个相同，顺序相同。
//!
//! 两个口径各自钉了一个组合数。语料增删时会红，免得整批用例悄悄滑进
//! 弱口径那一边还看起来是绿的。
//!
//! # 断行规则本身的正确性不在这里
//!
//! 它的 oracle 不可能是 v1。UAX #14 的实现是 `unicode-linebreak`，它在上游
//! 用 Unicode 官方的 `LineBreakTest.txt` 逐条验证过；把那份文件搬进本仓库
//! 只会再测一遍那个依赖。这里要证明的是**我们用对了它**：断点落在断行机会
//! 上、强制换行被尊重、grapheme 不被劈开、行尾空白悬在行外、一个比整行还宽
//! 的词仍然排得出来、CJK 禁则生效。这些在 `src/block.rs` 的单元测试里。

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

/// 一块布局里有没有发生软换行：行数超过「强制换行数 + 1」就有。
fn soft_wrapped_old(layout: &LayoutSnapshot) -> bool {
    let breaks = layout
        .clusters()
        .iter()
        .filter(|cluster| cluster.is_line_break())
        .count();
    layout.lines().len() > breaks + 1
}

fn soft_wrapped_new(layout: &BlockLayout) -> bool {
    let breaks = layout
        .clusters()
        .iter()
        .filter(|cluster| cluster.is_line_break())
        .count();
    layout.lines().len() > breaks + 1
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

/// oracle 可信的那一面：没有软换行时，两条路的几何必须逐点相同。
#[test]
fn geometry_agrees_where_nothing_soft_wraps() {
    let mut compared = 0_usize;
    for source in CORPUS {
        for width in WIDTHS {
            let (old, new) = both_paths(source, *width);
            if soft_wrapped_old(&old) || soft_wrapped_new(&new) {
                continue;
            }
            compared += 1;
            let at = format!("语料 {source:?} 宽度 {width}");

            assert_eq!(old.lines().len(), new.lines().len(), "{at} 的行数");
            for (old_line, new_line) in old.lines().iter().zip(new.lines()) {
                let at = format!("{at} 第 {} 行", old_line.index());
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
                "{at} 的块高"
            );

            for (index, (old_cluster, new_cluster)) in
                old.clusters().iter().zip(new.clusters()).enumerate()
            {
                let at = format!("{at} 第 {index} 个簇");
                assert_eq!(old_cluster.line(), new_cluster.line(), "{at} 的行号");
                assert_eq!(old_cluster.x(), new_cluster.x(), "{at} 的 x");
            }

            // caret：每个 grapheme 边界、两种 affinity。grapheme 内部不是
            // 合法的 caret 位置，两条路在那里的行为不构成契约。
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
                    let at = format!("{at} 偏移 {} affinity {affinity:?}", offset.get());
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

            // hit-test：每个簇的左边缘、中点、右边缘，以及行首行尾之外。
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
                    let at = format!("{at} 第 {} 行 x={x}", line.index());
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
    assert_eq!(
        compared, 56,
        "没有软换行的组合数变了。语料或宽度动过就更新这个数，\
         但先确认不是有一批用例悄悄滑进了弱口径那一边"
    );
}

/// 发生软换行时 oracle 失效，但这条性质仍然成立：**断行规则只决定 grapheme
/// 被分到哪一行，不改变 grapheme 本身**。
///
/// 它压得住的东西：度量在两条路里一致、run 的样式没有错位、grapheme 切分
/// 没有因为换了断行算法而变。压不住的是断点选得对不对——那件事见文件头。
#[test]
fn soft_wrapping_changes_only_line_assignment() {
    let mut compared = 0_usize;
    for source in CORPUS {
        for width in WIDTHS {
            let (old, new) = both_paths(source, *width);
            if !soft_wrapped_old(&old) && !soft_wrapped_new(&new) {
                continue;
            }
            compared += 1;
            let at = format!("语料 {source:?} 宽度 {width}");
            assert_eq!(old.clusters().len(), new.clusters().len(), "{at} 的簇数");
            for (index, (old_cluster, new_cluster)) in
                old.clusters().iter().zip(new.clusters()).enumerate()
            {
                let at = format!("{at} 第 {index} 个簇");
                assert_eq!(
                    old_cluster.visual(),
                    new_cluster.visual(),
                    "{at} 的视觉区间"
                );
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
    assert_eq!(
        compared, 36,
        "发生软换行的组合数变了。语料或宽度动过就更新这个数（两边加起来必须是 23×4=92）"
    );
}
