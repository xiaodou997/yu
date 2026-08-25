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
//! # 为什么它住在 `yu-editor`
//!
//! 新路的输入由 [`BlockLayoutInput`] 派生，而那是 `yu-editor` 的模块——
//! Markdown 的解释权在 S5 这一轮从 `yu-layout` 搬到了这里。差分的两端
//! 因此一端在 `yu-layout`（v1 的 `LayoutSnapshot`），一端在这里。
//!
//! 它随 `LayoutSnapshot` 一起消失。删之前要想清楚：删掉之后新引擎就没有
//! 外部 oracle 了，剩下的保障是那些性质测试与真实窗口。
//!
//! # 断行规则本身的正确性不在这里
//!
//! 它的 oracle 不可能是 v1。UAX #14 的实现是 `unicode-linebreak`，它在上游
//! 用 Unicode 官方的 `LineBreakTest.txt` 逐条验证过；把那份文件搬进本仓库
//! 只会再测一遍那个依赖。这里要证明的是**我们用对了它**：断点落在断行机会
//! 上、强制换行被尊重、grapheme 不被劈开、行尾空白悬在行外、一个比整行还宽
//! 的词仍然排得出来、CJK 禁则生效。这些在 `src/block.rs` 的单元测试里。

use yu_core::{ByteOffset, CaretAffinity, TextRange, VisualOffset};
use yu_editor::BlockLayoutInput;
use yu_layout::{BlockLayout, LayoutConfig, LayoutSnapshot, MonospaceMetrics, NoWidgets};
use yu_projection::{BlockProjection, Projection, ProjectionBias};
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

/// 块级语料的宽度。默认推进量是 2.0，所以这几个数比上面那组大一倍。
const BLOCK_WIDTHS: &[f32] = &[6.0, 10.0, 24.0, 160.0];

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

fn v2(projection: &Projection, config: LayoutConfig) -> BlockLayout {
    let metrics = MonospaceMetrics::new(config.default_advance());
    let input = BlockLayoutInput::derive(projection, config, &metrics).expect("派生输入");
    BlockLayout::build_all(
        input.layout_input(),
        config,
        input.styles(),
        &NoWidgets,
        input.line_styles(),
        &metrics,
    )
    .expect("v2 布局")
}

fn both_paths(source: &str, width: f32) -> (LayoutSnapshot, BlockLayout) {
    let projection = projection_of(source);
    let config = LayoutConfig::new(width, 1.0);
    let metrics = MonospaceMetrics::new(config.default_advance());
    let old = LayoutSnapshot::from_projection_with_metrics(&projection, config, &metrics)
        .expect("v1 布局");
    (old, v2(&projection, config))
}

/// 块级语料：标题、引用、列表标记各自带来一组几何（字号倍率、行高倍率、
/// 悬挂缩进），它们在 v2 里分别落到 `TextAttrs::size_scale`、
/// `LineAttrs::line_height_scale` 与 `LineAttrs::indent` 上。
const BLOCK_CORPUS: &[&str] = &[
    "# h1 title\n",
    "## h2 *em* title\n",
    "###### h6 title\n",
    "> quoted text\n",
    "> first\n> second\n",
    "> > nested quote\n",
    "- item one\n",
    "  - indented item\n",
    "1. ordered item\n",
    "- [ ] task item\n",
    "- [x] done item\n",
    "plain paragraph\n",
    "`code span` in a paragraph\n",
];

fn block_projection_of(source: &str) -> BlockProjection {
    let buffer = TextBuffer::new(source.to_owned());
    let snapshot = buffer.snapshot();
    let markdown = yu_markdown::parse(&snapshot);
    let block = markdown.blocks().get(0).expect("至少一个块");
    BlockProjection::from_block_with_definitions(&snapshot, block, markdown.reference_definitions())
        .expect("块投影")
}

fn both_block_paths(source: &str, width: f32) -> (LayoutSnapshot, BlockLayout) {
    let projection = block_projection_of(source);
    let config = LayoutConfig::new(width, 1.0).with_default_advance(2.0);
    let metrics = MonospaceMetrics::new(config.default_advance());
    let old = LayoutSnapshot::from_block_projection_with_metrics(&projection, config, &metrics)
        .expect("v1 块布局");
    (old, v2(projection.visual(), config))
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
    let config = LayoutConfig::new(80.0, 1.0);
    let metrics = MonospaceMetrics::new(config.default_advance());
    for source in CORPUS.iter().chain(BLOCK_CORPUS) {
        let projection = projection_of(source);
        let input = BlockLayoutInput::derive(&projection, config, &metrics).expect("派生输入");
        assert_eq!(
            VisualOffset::try_from(input.text().len()).expect("短"),
            projection.visual_len(),
            "语料 {source:?} 的视觉长度不一致"
        );
        let tiled = input
            .layout_input()
            .runs()
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
                    yu_editor::style_id(old_cluster.style()),
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

/// 块级几何：标题、引用、列表标记。
///
/// v1 把这三样都做在布局里（`heading_config` 改 config、`marker_gutter` 与
/// `block_quote_metrics` 算出 gutter 再整体右移）；v2 把它们翻译成
/// `TextAttrs::size_scale`、`LineAttrs::line_height_scale` 与
/// `LineAttrs::indent`，翻译发生在 `yu-editor`。**翻译对不对**由这条差分守。
#[test]
fn block_geometry_agrees_where_nothing_soft_wraps() {
    let mut compared = 0_usize;
    for source in BLOCK_CORPUS {
        for width in BLOCK_WIDTHS {
            let (old, new) = both_block_paths(source, *width);
            if soft_wrapped_old(&old) || soft_wrapped_new(&new) {
                continue;
            }
            compared += 1;
            let at = format!("语料 {source:?} 宽度 {width}");

            assert_eq!(old.lines().len(), new.lines().len(), "{at} 的行数");
            for (old_line, new_line) in old.lines().iter().zip(new.lines()) {
                let at = format!("{at} 第 {} 行", old_line.index());
                assert_eq!(old_line.visual(), new_line.visual(), "{at} 的视觉区间");
                assert_eq!(old_line.y(), new_line.y(), "{at} 的 y");
                assert_eq!(old_line.width(), new_line.width(), "{at} 的宽度");
                // v1 把标题的行高塞进 config，v2 塞进 LineAttrs。两条路的
                // 行盒高度必须一样，否则整块的高度会错，滚动范围跟着错。
                assert_eq!(old.config().line_height(), new_line.height(), "{at} 的行高");
            }
            assert_eq!(old.block_height(), new.height(), "{at} 的块高");

            for (index, (old_cluster, new_cluster)) in
                old.clusters().iter().zip(new.clusters()).enumerate()
            {
                let at = format!("{at} 第 {index} 个簇");
                assert_eq!(old_cluster.line(), new_cluster.line(), "{at} 的行号");
                assert_eq!(old_cluster.x(), new_cluster.x(), "{at} 的 x");
                assert_eq!(old_cluster.width(), new_cluster.width(), "{at} 的宽度");
            }

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

            for line in new.lines() {
                let y = line.y();
                let mut xs = vec![-1.0_f32, 0.0, line.width(), line.width() + 3.0];
                for index in line.cluster_range() {
                    let cluster = new.clusters()[index];
                    xs.push(cluster.x());
                    // 不取中点。标题的推进量是 2.0×1.7，中点落在两条边缘
                    // **等距**的地方，谁近谁远由最后一位尾数决定——那是一个
                    // 真正的平局，两条路怎么打破它都不构成契约。
                    xs.push(cluster.x() + cluster.width() * 0.25);
                    xs.push(cluster.x() + cluster.width() * 0.75);
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
                    if x > 0.0 {
                        assert_eq!(old_hit.point(), new_hit.point(), "{at} 的位置");
                    } else {
                        // 点在 gutter 里（或它左边）时 v1 不可信：它直接返回
                        // x=0，而同一个偏移的 `caret_for_visual` 返回的是缩进
                        // 之后的位置。两处各写了一遍规则，于是对不上——正是
                        // S5 在 bidi 那一刀记下的同一种毛病。v2 只有一条规则，
                        // 这里改为自证。
                        let caret = new
                            .caret(new_hit.visual(), CaretAffinity::Downstream)
                            .unwrap_or_else(|error| panic!("{at} 的 v2 caret: {error}"));
                        assert_eq!(new_hit.point(), caret.point(), "{at} 的 hit/caret 一致");
                    }
                }
            }
        }
    }
    assert_eq!(
        compared, 18,
        "块级语料里没有软换行的组合数变了。语料或宽度动过就更新这个数，\
         但先确认不是有一批用例悄悄滑进了弱口径那一边"
    );
}

/// 「长什么样」那部分：v1 把它们留在 `LayoutSnapshot` 上，v2 留在
/// `BlockOrnaments` 里。两边必须说的是同一件事。
#[test]
fn ornaments_carry_what_the_v1_snapshot_carried() {
    // 行高取 20 而不是 1：引用竖条的宽度是 `line_height * 0.12` 再 clamp 到
    // [1, 3]，行高为 1 时怎么算都是 1，公式写错也看不出来。
    let config = LayoutConfig::new(160.0, 20.0).with_default_advance(2.0);
    let metrics = MonospaceMetrics::new(config.default_advance());
    let mut headings = 0_usize;
    let mut quotes = 0_usize;
    let mut markers = 0_usize;
    for source in BLOCK_CORPUS {
        let projection = block_projection_of(source);
        let old = LayoutSnapshot::from_block_projection_with_metrics(&projection, config, &metrics)
            .expect("v1 块布局");
        let input =
            BlockLayoutInput::derive(projection.visual(), config, &metrics).expect("派生输入");
        let ornaments = input.ornaments();
        let at = format!("语料 {source:?}");

        match (old.heading(), ornaments.heading()) {
            (Some(old_heading), Some(new_heading)) => {
                headings += 1;
                assert_eq!(
                    old_heading.source(),
                    new_heading.source(),
                    "{at} 的标题范围"
                );
                assert_eq!(old_heading.level(), new_heading.level(), "{at} 的标题级别");
                assert_eq!(
                    old_heading.font_scale(),
                    new_heading.font_scale(),
                    "{at} 的字号倍率"
                );
                assert_eq!(
                    old_heading.line_height_scale(),
                    new_heading.line_height_scale(),
                    "{at} 的行高倍率"
                );
            }
            (None, None) => {}
            (old_heading, new_heading) => {
                panic!("{at} 的标题装饰对不上: {old_heading:?} / {new_heading:?}")
            }
        }

        match (old.block_quote(), ornaments.quote()) {
            (Some(old_quote), Some(new_quote)) => {
                quotes += 1;
                assert_eq!(old_quote.source(), new_quote.source(), "{at} 的引用范围");
                assert_eq!(old_quote.depth(), new_quote.depth(), "{at} 的引用层数");
                let new_bars = new_quote.bars(old.block_height()).expect("竖条");
                assert_eq!(old_quote.bars(), new_bars.as_slice(), "{at} 的竖条几何");
            }
            (None, None) => {}
            (old_quote, new_quote) => {
                panic!("{at} 的引用装饰对不上: {old_quote:?} / {new_quote:?}")
            }
        }

        match (projection.visual().leading_marker(), ornaments.marker()) {
            (Some(old_marker), Some(new_marker)) => {
                markers += 1;
                assert_eq!(old_marker.source(), new_marker.source(), "{at} 的标记范围");
                assert_eq!(old_marker.text(), new_marker.text(), "{at} 的标记文本");
                // v1 把标记画在 `indent * default_advance` 处（引用 gutter 之后）。
                assert_eq!(
                    new_marker.x(),
                    f32::from(old_marker.indent()) * config.default_advance(),
                    "{at} 的标记 x"
                );
            }
            (None, None) => {}
            (old_marker, new_marker) => {
                panic!("{at} 的标记装饰对不上: {old_marker:?} / {new_marker:?}")
            }
        }
    }
    assert_eq!((headings, quotes, markers), (3, 3, 3), "块级语料的构成变了");
}

/// 块级语料在窄宽度下同样只改变行归属。
#[test]
fn block_soft_wrapping_changes_only_line_assignment() {
    let mut compared = 0_usize;
    for source in BLOCK_CORPUS {
        for width in BLOCK_WIDTHS {
            let (old, new) = both_block_paths(source, *width);
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
            }
        }
    }
    assert_eq!(
        compared, 34,
        "块级语料里发生软换行的组合数变了（两边加起来必须是 13×4=52）"
    );
}
