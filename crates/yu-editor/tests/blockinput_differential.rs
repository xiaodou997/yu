//! 两条派生布局输入的路，逐份文档比对。
//!
//! # 这条测试回答的问题
//!
//! `BlockLayoutInput` 现在有两个来源：v1 的 `Projection`，与 `yu-markdown`
//! 的 extension 产出（`DecorationSet`）。**`Projection` 还在产品里跑着，
//! 删它之前它就是 oracle**——这条测试是那个 oracle 在装配这一层的兑现处。
//!
//! # 两条路真的分开了吗
//!
//! 分开的是**派生**：视觉文本怎么拼、哪些字节进得去、每一段排什么字型、
//! 行级装饰是什么。这几样两边各写各的。
//!
//! 不分开的是**几何算术**：`heading_metrics` / `block_quote_metrics` /
//! `measure_marker_parts` 两条路共用同一批函数。那是有意的——比对
//! 「2.0 倍字号抄了两遍还相等」什么都证明不了，而共用之后差分能把注意力
//! 全放在真正会分叉的地方。
//!
//! # 口径
//!
//! 未登记的文档必须**逐项**一致：视觉文本、样式段（连同它解析出来的
//! `TextAttrs`）、行级几何、三种装饰。已登记的必须精确等于登记值。
//!
//! 登记表与 `yu-projection/tests/extension_parity.rs` 是同一批差异的两种
//! 表现：那边比「隐藏了哪些字节」，这边比「隐藏之后排出了什么」。一条差异
//! 在那边登记了，这边通常也会有。**切换消费者那一刀，这张表就是「画面预期
//! 会变哪些地方」的清单**——S5 那种「前后截图只应有零处不同」的验收法在那一
//! 刀不成立，因为它本来就该变。

use yu_core::{ClusterMetrics, TextAttrs, TextStyle};
use yu_editor::{BlockLayoutInput, BlockOrnaments};
use yu_layout::{LayoutConfig, LineStyleTable, StyleTable};
use yu_markdown::{ExtensionSet, parse};
use yu_projection::BlockProjection;
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
        }
    }
}

/// 一个块派生出来的、两条路都说得出的那些东西。
///
/// 比的是**解析之后**的 `TextAttrs`，不是 `StyleId`：两边的 id 空间本来就
/// 不同（v1 是固定四个，extension 是自己编号），比 id 只会比出这件已知的事。
#[derive(Debug, PartialEq)]
struct Derived {
    text: String,
    runs: Vec<(u64, u64, TextAttrs)>,
    indent: f32,
    line_height_scale: f32,
    heading: Option<u8>,
    quote: Option<u8>,
    /// （标记文本，画在哪，占多宽）。`x` 与 `advance` 是几何，两条路共用
    /// 同一批函数算，所以精确相等是应该的。
    marker: Option<(String, f32, f32)>,
}

type Ornaments = (Option<u8>, Option<u8>, Option<(String, f32, f32)>);

fn ornaments_of(ornaments: &BlockOrnaments) -> Ornaments {
    (
        ornaments.heading().map(|heading| heading.level()),
        ornaments.quote().map(|quote| quote.depth()),
        ornaments
            .marker()
            .map(|marker| (marker.text().to_owned(), marker.x(), marker.advance())),
    )
}

fn describe(input: &BlockLayoutInput) -> Derived {
    let layout = input.layout_input();
    // 相邻且**解析后属性相同**的两段合成一段。两条路把同一片文字切成几段
    // 是各自的实现细节（v1 按 `VisualRun` 切，新那条按 Mark 边界切），比对
    // 要问的是「每个字节排什么」，不是「怎么切的」。不归一化的话
    // `- [ ] 待办` 会因为 v1 多切了一刀而假红，真差异反而淹掉。
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
    let (heading, quote, marker) = ornaments_of(input.ornaments());
    Derived {
        text: input.text().to_owned(),
        runs,
        indent: line.indent(),
        line_height_scale: line.line_height_scale(),
        heading,
        quote,
        marker,
    }
}

/// 两条路各自派生一遍第 `index` 个块。v1 认不出那个块时返回 `None`。
fn derive_both(source: &str, index: usize) -> Option<(Derived, Derived)> {
    let config = LayoutConfig::new(400.0, 10.0);
    let buffer = TextBuffer::new(source.to_owned());
    let snapshot = buffer.snapshot();
    let document = parse(&snapshot);
    let definitions = document.reference_definitions();
    let tree = parse_syntax(&snapshot).expect("测试文档很短").into_tree();
    let block = document.blocks().get(index)?;

    let decorations = ExtensionSet::markdown()
        .decorate(&snapshot, &tree, block, None)
        .expect("装饰产出不该失败");
    let ours = BlockLayoutInput::from_decorations(&decorations, &snapshot, config, &StyleSensitive)
        .expect("从装饰派生");

    let projection =
        BlockProjection::from_block_with_definitions(&snapshot, block, definitions).ok()?;
    let theirs =
        BlockLayoutInput::derive(projection.visual(), config, &StyleSensitive).expect("从投影派生");

    Some((describe(&ours), describe(&theirs)))
}

/// 差异的归属。只剩一类——没有「extension 错」，「还没做的语法」也随表格
/// extension 落地清空了。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Cause {
    /// v1 判错，extension 对。删掉 v1 时一起消失。
    ProjectionBug,
}

struct Divergence {
    source: &'static str,
    /// extension 那条路排出来的视觉文本。
    extension: &'static str,
    /// v1 那条路排出来的视觉文本。
    projection: &'static str,
    cause: Cause,
    why: &'static str,
}

/// 登记的是**视觉文本**，因为它是这一层最直观的产物：用户看见的就是它。
const DIVERGENCES: &[Divergence] = &[
    Divergence {
        source: "# 标题 #",
        extension: "标题",
        projection: "标题 #",
        cause: Cause::ProjectionBug,
        why: "ATX 的收尾 `#` 是语法不是内容。v1 只认前缀，于是标题右边挂着\
              一个用户没打算显示的 `#`",
    },
    Divergence {
        source: "> > 引用两层",
        extension: "引用两层",
        projection: "> 引用两层",
        cause: Cause::ProjectionBug,
        why: "两层引用有两个 `> ` 前缀，v1 的块序列只记了 depth=1。注意 v1 \
              连缩进也只让一层——视觉文本与 gutter 同时错，方向还相反",
    },
    Divergence {
        source: "***both***",
        extension: "both",
        projection: "***both***",
        cause: Cause::ProjectionBug,
        why: "三个以上连续定界符，v1 扫描器整段放弃，于是 `***` 原样画出来",
    },
    Divergence {
        source: "    indented *em*\n",
        extension: "    indented *em*\n",
        projection: "    indented em\n",
        cause: Cause::ProjectionBug,
        why: "四空格缩进是代码块，里面不解析行内语法。v1 把代码里的 `*` 当成\
              强调隐藏——**用户看到的代码少了两个字符**，这是最该修的一条",
    },
    Divergence {
        source: "``a `b` c``",
        extension: "a `b` c",
        projection: "a b c",
        cause: Cause::ProjectionBug,
        why: "多重反引号的代码跨度里，单反引号是字面内容。v1 继续在里面找\
              定界符，于是代码里的反引号消失",
    },
    Divergence {
        source: "<!-- comment *em* -->",
        extension: "<!-- comment *em* -->",
        projection: "<!-- comment em -->",
        cause: Cause::ProjectionBug,
        why: "HTML 注释是 raw HTML，内部不解析行内语法",
    },
    Divergence {
        source: "<http://a.com/*b*>",
        extension: "http://a.com/*b*",
        projection: "http://a.com/b",
        cause: Cause::ProjectionBug,
        why: "autolink 内部不解析行内语法。v1 在 URL 里找到一对 `*` 并隐藏，\
              于是**地址少掉两个字符**",
    },
];

/// 语料。只放单块文档，比对才对得上「第几个块」。
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
];

fn registered(source: &str) -> Option<&'static Divergence> {
    DIVERGENCES
        .iter()
        .find(|divergence| divergence.source == source)
}

#[test]
fn unregistered_documents_derive_identical_layout_input() {
    for source in DOCUMENTS {
        if registered(source).is_some() {
            continue;
        }
        let Some((ours, theirs)) = derive_both(source, 0) else {
            panic!("{source:?} 的第一个块 v1 认不出来——那本身是一条差异，请登记");
        };
        assert_eq!(
            ours, theirs,
            "{source:?} 两条路派生出的布局输入不一致。要么是新那条错了，\
             要么这是一条新差异——后者请登记进 DIVERGENCES 并写明归属"
        );
    }
}

/// 已登记的差异必须**精确**等于登记值。
///
/// 守的是「差异消失了但登记还留着」：表格 widget 化之后 `Pending` 那一行
/// 会红，逼人删掉它。
#[test]
fn registered_divergences_are_exact() {
    for divergence in DIVERGENCES {
        let (ours, theirs) = derive_both(divergence.source, 0)
            .unwrap_or_else(|| panic!("{:?} 两条路都要派生得出来", divergence.source));
        assert_eq!(
            ours.text, divergence.extension,
            "{:?} 的 extension 侧变了（{}）",
            divergence.source, divergence.why
        );
        assert_eq!(
            theirs.text, divergence.projection,
            "{:?} 的 v1 侧变了（{}）",
            divergence.source, divergence.why
        );
        assert_ne!(
            ours.text, theirs.text,
            "{:?} 两边已经一致了，请把这一行从 DIVERGENCES 删掉",
            divergence.source
        );
    }
}

#[test]
fn every_registered_divergence_is_in_the_corpus() {
    for divergence in DIVERGENCES {
        assert!(
            DOCUMENTS.contains(&divergence.source),
            "{:?} 登记了差异却不在语料里",
            divergence.source
        );
    }
}

/// 归属统计。差异的形状是这一刀的结论，写成断言免得它悄悄变了。
#[test]
fn divergence_causes_stay_accounted_for() {
    let count = |cause: Cause| {
        DIVERGENCES
            .iter()
            .filter(|divergence| divergence.cause == cause)
            .count()
    };
    assert_eq!(count(Cause::ProjectionBug), 7, "v1 判错的条数变了");
    assert_eq!(DIVERGENCES.len(), 7);
}

// ------------------------------------------------------ 只有新那条说得出的事

/// 标题的字号倍率盖在**整张**样式表上，不靠 heading 产一条覆盖全块的 Mark。
///
/// 「几级标题」是语义，归 `yu-markdown`；「1.7 倍、排粗体」是呈现，只有这一
/// 层有 `LayoutConfig` 说得出来。让 extension 产一条 `Strong` 的 Mark 也能
/// work，但那等于把呈现决定塞回刚划清界限的那一层。
#[test]
fn a_heading_scales_every_style_in_the_table() {
    let config = LayoutConfig::new(400.0, 10.0);
    let buffer = TextBuffer::new("## h2 *em* `code`".to_owned());
    let snapshot = buffer.snapshot();
    let document = parse(&snapshot);
    let tree = parse_syntax(&snapshot).expect("测试文档很短").into_tree();
    let block = document.blocks().get(0).expect("至少一个块");
    let decorations = ExtensionSet::markdown()
        .decorate(&snapshot, &tree, block, None)
        .expect("装饰产出");
    let input =
        BlockLayoutInput::from_decorations(&decorations, &snapshot, config, &StyleSensitive)
            .expect("派生");

    let layout = input.layout_input();
    assert!(!layout.runs().is_empty(), "标题里有三段不同字型的文字");
    for run in layout.runs() {
        let attrs = input.styles().attrs(run.style()).expect("查得到");
        assert_eq!(attrs.style(), TextStyle::Strong, "标题一律排粗体");
        assert_eq!(attrs.size_scale(), 1.7, "二级标题的字号倍率");
    }
    assert_eq!(input.ornaments().heading().map(|h| h.level()), Some(2));
}

/// 嵌套时窄的赢：`**[文字](目标)**` 里链接正文排正文字型，不继承外层加粗。
///
/// 这条与 `unregistered_documents_...` 里那份语料重合，但那边只保证「两条路
/// 一样」——两边一起错也会绿。这里正面钉住它该是什么。
#[test]
fn link_text_inside_bold_is_not_bold() {
    let (ours, _) = derive_both("**[文字](目标)**", 0).expect("两条路都派生得出来");
    assert_eq!(ours.text, "文字");
    assert_eq!(
        ours.runs,
        vec![(0, 6, TextAttrs::new(TextStyle::Plain))],
        "整段链接正文都是正文字型"
    );
}

/// 样式段必须无缝铺满视觉文本。
///
/// 漏掉半段的后果是画面上少几个字——既不 panic 也不报错。
#[test]
fn styled_runs_tile_the_visual_text() {
    for source in DOCUMENTS {
        let Some((ours, _)) = derive_both(source, 0) else {
            continue;
        };
        let mut cursor = 0_u64;
        for (from, to, _) in &ours.runs {
            assert_eq!(*from, cursor, "{source:?} 的样式段之间有空档");
            cursor = *to;
        }
        assert_eq!(
            cursor,
            ours.text.len() as u64,
            "{source:?} 的样式段没铺到视觉文本末尾"
        );
    }
}

/// 视觉文本恰好是「源码去掉被隐藏的字节」。
#[test]
fn the_visual_text_is_the_source_minus_the_hidden_bytes() {
    let source = "## 二级 *斜体*";
    let buffer = TextBuffer::new(source.to_owned());
    let snapshot = buffer.snapshot();
    let document = parse(&snapshot);
    let tree = parse_syntax(&snapshot).expect("测试文档很短").into_tree();
    let block = document.blocks().get(0).expect("至少一个块");
    let decorations = ExtensionSet::markdown()
        .decorate(&snapshot, &tree, block, None)
        .expect("装饰产出");

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
    let mut cursor = 0_usize;
    for (from, to) in hidden {
        if from > cursor {
            expected.push_str(&source[cursor..from]);
        }
        cursor = cursor.max(to);
    }
    expected.push_str(&source[cursor..]);

    let input = BlockLayoutInput::from_decorations(
        &decorations,
        &snapshot,
        LayoutConfig::new(400.0, 10.0),
        &StyleSensitive,
    )
    .expect("派生");
    assert_eq!(input.text(), expected);
}

/// 空块不该 panic，也不该产出半个 run。
#[test]
fn an_empty_block_derives_an_empty_input() {
    let buffer = TextBuffer::new(String::new());
    let snapshot = buffer.snapshot();
    let document = parse(&snapshot);
    let tree = parse_syntax(&snapshot).expect("空文档").into_tree();
    let Some(block) = document.blocks().get(0) else {
        return;
    };
    let decorations = ExtensionSet::markdown()
        .decorate(&snapshot, &tree, block, None)
        .expect("装饰产出");
    let input = BlockLayoutInput::from_decorations(
        &decorations,
        &snapshot,
        LayoutConfig::new(400.0, 10.0),
        &StyleSensitive,
    )
    .expect("派生");
    assert_eq!(input.text(), "");
    assert!(input.layout_input().runs().is_empty());
}

/// 未知 id 查不到就是查不到，不给默认字型。
#[test]
fn an_unknown_style_id_is_absent_not_defaulted() {
    let buffer = TextBuffer::new("段落".to_owned());
    let snapshot = buffer.snapshot();
    let document = parse(&snapshot);
    let tree = parse_syntax(&snapshot).expect("测试文档很短").into_tree();
    let block = document.blocks().get(0).expect("至少一个块");
    let decorations = ExtensionSet::markdown()
        .decorate(&snapshot, &tree, block, None)
        .expect("装饰产出");
    let input = BlockLayoutInput::from_decorations(
        &decorations,
        &snapshot,
        LayoutConfig::new(400.0, 10.0),
        &StyleSensitive,
    )
    .expect("派生");
    assert!(input.styles().attrs(yu_core::StyleId(u32::MAX)).is_none());
}

/// 两个 extension 盖在同一段 source 上时，视觉文本不能把那一段算两遍。
///
/// `- [x] 完成` 里 task 隐藏整个 `[x]`、link 另外隐藏它的两个 `LinkMark`，
/// 三条隐藏区间彼此重叠。装配层**不**预先合并它们——重叠由 `visible_pieces`
/// 那句 `cursor.max(to)` 吃掉。删掉预合并时一条测试都没红（变异验证发现的），
/// 所以这条性质现在只剩这一处压着，得说清楚。
#[test]
fn overlapping_hidden_ranges_are_not_counted_twice() {
    let source = "- [x] 完成";
    let buffer = TextBuffer::new(source.to_owned());
    let snapshot = buffer.snapshot();
    let document = parse(&snapshot);
    let tree = parse_syntax(&snapshot).expect("测试文档很短").into_tree();
    let block = document.blocks().get(0).expect("至少一个块");
    let decorations = ExtensionSet::markdown()
        .decorate(&snapshot, &tree, block, None)
        .expect("装饰产出");

    let hidden: Vec<(u64, u64)> = decorations
        .set()
        .all()
        .iter()
        .filter(|entry| entry.decoration.hides_source())
        .map(|entry| (entry.range.start().get(), entry.range.end().get()))
        .collect();
    assert!(
        hidden.len() > 1
            && hidden
                .iter()
                .any(
                    |(from, to)| hidden.iter().any(|(other_from, other_to)| (from, to)
                        != (other_from, other_to)
                        && from < other_to
                        && other_from < to)
                ),
        "这份语料的前提是隐藏区间真的重叠，实际是 {hidden:?}"
    );

    let input = BlockLayoutInput::from_decorations(
        &decorations,
        &snapshot,
        LayoutConfig::new(400.0, 10.0),
        &StyleSensitive,
    )
    .expect("派生");
    assert_eq!(input.text(), "-  完成", "`- ` 留着，`[x]` 整个消失");
}
