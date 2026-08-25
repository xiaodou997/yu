//! 两条产出「隐藏哪些字节」的路，逐份文档比对。
//!
//! # 这条测试回答的问题
//!
//! `crates/yu-decoration/tests/projection_differential.rs` 把隐藏区间**从真实
//! Projection 里取**，再喂给 `DecorationSet`，这样两边输入完全一致，差异只能
//! 来自映射。那条测试刻意回避了另一半问题，这条负责回答它：
//!
//! **`yu-syntax` 的标记节点范围，能不能真的驱动「隐藏语法」？**
//!
//! S3 结束时 `yu-syntax` 零消费者，这个问题没有答案。它不能拖到 S5——那时它会
//! 和布局重写一起爆。这里用 v1 的行内扫描器（本 crate）当 oracle 提前回答：
//! 一个已经在产品里跑着的实现，比任何自证性质都强。
//!
//! # 结论
//!
//! 76 份语料里 60 份逐字节一致，16 份登记在下面的 `DIVERGENCES` 里，而**没有
//! 一条是 `yu-syntax` 错**。差异分两类：
//!
//! - `ProjectionBug`：v1 扫描器没有块级上下文，在不该解析行内语法的地方解析了
//!   （缩进代码块、`~~~` 围栏、HTML 注释、autolink 内部、代码跨度内部），
//!   或者遇到三个以上连续定界符就整段放弃。这正是 overview-v2 §2.1
//!   「Markdown 语义泄漏」要换掉扫描器的原因。
//! - `ByDesign`：v1 隐藏的语法**种类**更多（链接括号、图片、autolink 尖括号、
//!   硬换行的尾随空格）。`yu-markdown::decorations` 现在只做强调与行内代码，
//!   其余种类是 S6 逐个 extension 的工作。
//!
//! # 登记表是紧的
//!
//! 未登记的文档必须逐字节一致；已登记的文档必须**精确**等于登记值。后一条守的
//! 是「差异消失了但登记还留着」——S6 给链接补上隐藏之后，这条测试会红，逼人
//! 把那一行删掉。口径与 `docs/specs/invariants.md` F 节的偏差表一致。
//!
//! 这条测试随 `yu-projection` 一起消失。

use yu_core::{ByteOffset, Revision, TextRange, VisualOffset};
use yu_decoration::{Bias, Decoration};
use yu_markdown::{inline_syntax_decoration_set, inline_syntax_decorations};
use yu_projection::{Projection, ProjectionBias, VisualRunKind};
use yu_text::TextBuffer;

/// 差异的归属。交接时的三选一里少了「`yu-syntax` 的 bug」——一条都没有。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Cause {
    /// v1 扫描器错，`yu-syntax` 对。删掉 v1 时一起消失。
    ProjectionBug,
    /// v1 隐藏的语法种类更多。`yu-markdown` 补齐后这一行要删。
    ByDesign,
    /// 同一份文档里两者都有。
    Mixed,
}

struct Divergence {
    source: &'static str,
    /// v1 行内扫描器隐藏的 source 区间。
    projection: &'static [(u64, u64)],
    /// `yu-syntax` 的标记节点撑出的隐藏区间。
    syntax: &'static [(u64, u64)],
    cause: Cause,
    why: &'static str,
}

const DIVERGENCES: &[Divergence] = &[
    Divergence {
        source: "***both***",
        projection: &[],
        syntax: &[(0, 1), (1, 3), (7, 9), (9, 10)],
        cause: Cause::ProjectionBug,
        why: "三个连续 `*` 是 em 套 strong。v1 不实现 CommonMark 的 \
              delimiter run 拆分，遇到就整段放弃，一个字节都不隐藏",
    },
    Divergence {
        source: "***a***b",
        projection: &[],
        syntax: &[(0, 1), (1, 3), (4, 6), (6, 7)],
        cause: Cause::ProjectionBug,
        why: "同上，且证明它与「定界符是否贴着文档末尾」无关",
    },
    Divergence {
        source: "*a* **b** ***c***",
        projection: &[(0, 1), (2, 3), (4, 6), (7, 9)],
        syntax: &[
            (0, 1),
            (2, 3),
            (4, 6),
            (7, 9),
            (10, 11),
            (11, 13),
            (14, 16),
            (16, 17),
        ],
        cause: Cause::ProjectionBug,
        why: "放弃的是三重定界符那一段，同一行里前面的 em 与 strong 仍然正确。\
              于是失败是局部的、静默的——正是这个项目最危险的那类",
    },
    Divergence {
        source: "**a*b***",
        projection: &[],
        syntax: &[(0, 2), (3, 4), (5, 6), (6, 8)],
        cause: Cause::ProjectionBug,
        why: "strong 里嵌 em、右侧收在一起。同样整段放弃",
    },
    Divergence {
        source: "    indented *em*\n",
        projection: &[(13, 14), (16, 17)],
        syntax: &[],
        cause: Cause::ProjectionBug,
        why: "四空格缩进是代码块，里面不解析行内语法。v1 没有块级上下文，\
              把代码里的 `*` 当成了强调并隐藏——用户看到的代码少了两个字符",
    },
    Divergence {
        source: "a\n\n    code *em*\n",
        projection: &[(12, 13), (15, 16)],
        syntax: &[],
        cause: Cause::ProjectionBug,
        why: "同上，缩进代码块出现在段落之后而不是文档开头",
    },
    Divergence {
        source: "\tcode *em*\n",
        projection: &[(6, 7), (9, 10)],
        syntax: &[],
        cause: Cause::ProjectionBug,
        why: "制表符缩进的代码块。注意 yu-syntax 这里判对了块类型——\
              不变量 F2 的制表符偏差只影响标记与内容的边界，不影响块识别",
    },
    Divergence {
        source: "~~~\nfenced *em*\n~~~\n",
        projection: &[(11, 12), (14, 15)],
        syntax: &[(0, 3), (16, 19)],
        cause: Cause::ProjectionBug,
        why: "两边完全对调：v1 不认识 `~~~` 围栏，于是漏隐藏围栏本身、\
              反而隐藏了代码内容里的 `*`。``` 围栏两边一致，见语料",
    },
    Divergence {
        source: "``a `b` c``",
        projection: &[(0, 2), (4, 5), (6, 7), (9, 11)],
        syntax: &[(0, 2), (9, 11)],
        cause: Cause::ProjectionBug,
        why: "多重反引号的代码跨度里，单反引号是字面内容。v1 继续在里面找定界符",
    },
    Divergence {
        source: "<!-- comment *em* -->",
        projection: &[(13, 14), (16, 17)],
        syntax: &[],
        cause: Cause::ProjectionBug,
        why: "HTML 注释是 raw HTML，内部不解析行内语法",
    },
    Divergence {
        source: "<http://a.com/*b*>",
        projection: &[(0, 1), (14, 15), (16, 17), (17, 18)],
        syntax: &[],
        cause: Cause::Mixed,
        why: "尖括号 (0,1)/(17,18) 是 ByDesign——v1 把 autolink 呈现为链接文本；\
              里面的 (14,15)/(16,17) 是 bug——autolink 内部不解析强调",
    },
    Divergence {
        source: "autolink <http://a.com/b>",
        projection: &[(9, 10), (24, 25)],
        syntax: &[],
        cause: Cause::ByDesign,
        why: "把上一条的两种原因分开：只隐藏尖括号时，差异纯粹是种类差异",
    },
    Divergence {
        source: "*a* <http://x.y> *b*",
        projection: &[(0, 1), (2, 3), (4, 5), (15, 16), (17, 18), (19, 20)],
        syntax: &[(0, 1), (2, 3), (17, 18), (19, 20)],
        cause: Cause::ByDesign,
        why: "autolink 与强调同行：强调部分两边一致，多出来的只有尖括号",
    },
    Divergence {
        source: "[link *em*](/uri)",
        projection: &[(0, 1), (6, 7), (9, 10), (10, 17)],
        syntax: &[(6, 7), (9, 10)],
        cause: Cause::ByDesign,
        why: "链接括号与目标由 v1 隐藏。链接文本里的强调两边一致——\
              这一条同时说明 yu-syntax 在链接内部照常解析行内语法",
    },
    Divergence {
        source: "![img](/uri)",
        projection: &[(0, 2), (5, 12)],
        syntax: &[],
        cause: Cause::ByDesign,
        why: "图片在 v2 里是 Widget（第 5.3 节），不是「隐藏一段字符」，\
              所以它不会由 inline_syntax_decorations 产出",
    },
    Divergence {
        source: "line *em*  \nnext",
        projection: &[(5, 6), (8, 9), (9, 11)],
        syntax: &[(5, 6), (8, 9)],
        cause: Cause::ByDesign,
        why: "(9,11) 是硬换行的两个尾随空格。硬换行的呈现属于 S6",
    },
];

/// 两边一致的语料。挑的是会真正压到定界符规则的写法。
///
/// 与 `DIVERGENCES` 合起来构成全部语料——任何一份文档只能出现在一处，
/// 由 `every_document_appears_once` 守。
const AGREED: &[&str] = &[
    "",
    "plain text",
    "*",
    "**",
    "***\n\n*em*\n",
    "*emphasis*",
    "**strong**",
    "`code`",
    "a*b*c",
    "a**b**c**d**e",
    "`code` `code`",
    "*a*b*c*",
    "**a**b**c**",
    "text*with*stars",
    "*a **b** c*",
    "**a *b* c**",
    "*a**b*",
    "`a``b`",
    "`` ` ``",
    "`` a `` b",
    "*`a`*",
    "`*a*`",
    "**`a`**",
    "`code with *em* inside`",
    "text `code *em* more` text",
    "```\nfenced *em*\n```\n",
    "a * b * c",
    "a_b_c",
    "_underscore_",
    "__strong underscore__",
    "snake_case_word",
    "_a_ __b__",
    "a_b_ _c_d",
    "*a _b_ c*",
    r"\*not em\*",
    r"a\\*b*c",
    "unmatched *delimiter",
    "a`b",
    "# heading *em*\n",
    "> quote *em*\n",
    "> quote *em*\n> more **strong**\n",
    "- item *em*\n",
    "- item *em*\n- item `code`\n",
    "1. item *em*\n",
    "* list item\n",
    "title\n=====\n\n*em*\n",
    "| a | *b* |\n| - | - |\n| `c` | d |\n",
    "a <b>*c*</b> d",
    "*multi\nline*",
    "a *b\nc* d",
    "**a\nb**",
    "*a*\n*b*",
    "line one *em*\nline two `code`\n",
    "*em* at end\n",
    "\n\n*em*\n\n",
    "**紧邻**`的`*三段*",
    "中文 *强调* 混排",
    "中*文*强调",
    "emoji 🙂 *后面*",
    "🙂*a*🙂",
];

/// v1 行内扫描器隐藏的 source 区间。
fn projection_hidden(projection: &Projection) -> Vec<(u64, u64)> {
    projection
        .runs()
        .iter()
        .filter(|run| run.kind() == VisualRunKind::HiddenSyntax)
        .map(|run| run.source())
        .filter(|range| !range.is_empty())
        .map(|range| (range.start().get(), range.end().get()))
        .collect()
}

/// `yu-syntax` 的标记节点撑出的隐藏区间。
fn syntax_hidden(source: &str) -> Vec<(u64, u64)> {
    let parsed = yu_syntax::parse(source).expect("测试文档很短");
    inline_syntax_decorations(parsed.tree())
        .into_iter()
        .filter(|entry| entry.decoration == Decoration::Replace)
        .map(|entry| (entry.range.start().get(), entry.range.end().get()))
        .collect()
}

fn projection_of(source: &str) -> Option<Projection> {
    let buffer = TextBuffer::new(source.to_owned());
    let snapshot = buffer.snapshot();
    let range = TextRange::new(
        ByteOffset::ZERO,
        ByteOffset::try_from(source.len()).expect("测试文档很短"),
    )?;
    Projection::inline(&snapshot, range).ok()
}

/// 一份文档在两条路上的产出。
struct BothPaths {
    projection: Projection,
    /// v1 行内扫描器的隐藏区间。
    theirs: Vec<(u64, u64)>,
    /// `yu-syntax` 的隐藏区间。
    ours: Vec<(u64, u64)>,
}

fn both_paths(source: &str) -> Option<BothPaths> {
    let projection = projection_of(source)?;
    let theirs = projection_hidden(&projection);
    let ours = syntax_hidden(source);
    Some(BothPaths {
        projection,
        theirs,
        ours,
    })
}

/// 语料不能有重复：一份文档同时出现在两张表里，会让「登记表是紧的」失效。
#[test]
fn every_document_appears_once() {
    let mut seen = std::collections::HashSet::new();
    for source in AGREED
        .iter()
        .copied()
        .chain(DIVERGENCES.iter().map(|entry| entry.source))
    {
        assert!(seen.insert(source), "语料重复：{source:?}");
    }
}

/// 未登记的文档，两边必须逐字节一致。
#[test]
fn undocumented_documents_agree_byte_for_byte() {
    for source in AGREED {
        let Some(both) = both_paths(source) else {
            panic!("{source:?}：投影失败");
        };
        assert_eq!(
            both.ours, both.theirs,
            "{source:?}：yu-syntax 与 v1 扫描器隐藏的区间不同。\
             要么是真的回归，要么该往 DIVERGENCES 里加一行并写明原因"
        );
    }
}

/// 已登记的差异必须精确成立——**两个方向都紧**。
///
/// 差异变了要更新，差异消失了要删行。后者是这条测试的主要价值：
/// S6 给链接、图片、autolink、硬换行补上装饰之后，对应的行会红。
#[test]
fn registered_divergences_are_exact() {
    for entry in DIVERGENCES {
        let Some(both) = both_paths(entry.source) else {
            panic!("{:?}：投影失败", entry.source);
        };
        assert_eq!(
            both.theirs,
            entry.projection.to_vec(),
            "{:?}：v1 扫描器的产出变了（{:?}）",
            entry.source,
            entry.cause
        );
        assert_eq!(
            both.ours,
            entry.syntax.to_vec(),
            "{:?}：yu-syntax 的产出变了（{:?}）",
            entry.source,
            entry.cause
        );
        assert_ne!(
            both.ours, both.theirs,
            "{:?}：这条差异已经不存在了，把它从 DIVERGENCES 里删掉。原因栏写的是：{}",
            entry.source, entry.why
        );
    }
}

/// `yu-syntax` 隐藏的每一个字节都必须是定界符字符。
///
/// 这是「range 能不能驱动隐藏语法」最直接的机械检验：范围偏一个字节，
/// 就会有正文字符落进隐藏区间，用户看到的文本会静静少掉一个字。
///
/// 只对 `yu-syntax` 这一侧断言。v1 那侧不满足也不该满足——它隐藏链接目标
/// 与硬换行空格，那些本来就不是定界符字符。
#[test]
fn syntax_hides_only_delimiter_bytes() {
    const DELIMITERS: &[char] = &['*', '_', '`', '~'];
    for source in AGREED
        .iter()
        .copied()
        .chain(DIVERGENCES.iter().map(|entry| entry.source))
    {
        for (start, end) in syntax_hidden(source) {
            let start = usize::try_from(start).expect("测试文档很短");
            let end = usize::try_from(end).expect("测试文档很短");
            let slice = source
                .get(start..end)
                .unwrap_or_else(|| panic!("{source:?}：隐藏区间 {start}..{end} 不在字符边界上"));
            assert!(
                !slice.is_empty() && slice.chars().all(|ch| DELIMITERS.contains(&ch)),
                "{source:?}：隐藏区间 {start}..{end} 是 {slice:?}，里面有不是定界符的字节"
            );
        }
    }
}

/// 语料规模与差异归类的计数，钉在测试里。
///
/// 文档（overview-v2 第 2.1 节与第 8 节 S4）引用了这几个数字。写死它们是为了
/// 让语料增删时必须回去改文档，而不是让文档里的数字慢慢腐烂。
#[test]
fn the_corpus_and_the_divergence_counts_are_what_the_docs_say() {
    let agreed = AGREED.len();
    let divergent = DIVERGENCES.len();
    assert_eq!(agreed, 60, "一致语料的份数变了");
    assert_eq!(divergent, 16, "登记差异的条数变了");
    assert_eq!(agreed + divergent, 76, "语料总份数变了");

    let count = |cause| {
        DIVERGENCES
            .iter()
            .filter(|entry| entry.cause == cause)
            .count()
    };
    assert_eq!(count(Cause::ProjectionBug), 10, "「v1 扫描器错」的条数变了");
    assert_eq!(count(Cause::ByDesign), 5, "「有意的不同」的条数变了");
    assert_eq!(count(Cause::Mixed), 1, "「两者兼有」的条数变了");
}

/// 语料必须真的产生了隐藏，否则上面几条比的是一堆空表。
#[test]
fn the_corpus_actually_hides_something() {
    let hiding = AGREED
        .iter()
        .filter(|source| !syntax_hidden(source).is_empty())
        .count();
    assert!(
        hiding >= 40,
        "只有 {hiding} 份一致语料产生了隐藏区间，一致性基本上是空对空"
    );
}

/// 端到端：`yu-syntax → yu-markdown → yu-decoration` 整条链的 source↔visual
/// 映射，必须与 v1 的整条链一致。
///
/// 这比 `projection_differential.rs` 更进一步：那条测试给两边喂同一组隐藏
/// 区间，只比映射；这条让两边**各自从源码走完自己的路**，比的是最终结果。
/// 不变量 D4 的 round-trip 是自证性质，这里才是有 oracle 的那一半。
///
/// 只跑两边隐藏区间一致的语料——区间都不同的时候比映射没有意义，
/// 那些文档的归因已经写在 `DIVERGENCES` 里了。
#[test]
fn end_to_end_mapping_matches_the_v1_projection() {
    for source in AGREED {
        let Some(both) = both_paths(source) else {
            panic!("{source:?}：投影失败");
        };
        assert_eq!(
            both.ours, both.theirs,
            "{source:?}：前置条件——隐藏区间应当一致"
        );
        let projection = &both.projection;

        let parsed = yu_syntax::parse(*source).expect("测试文档很短");
        let decorations = inline_syntax_decoration_set(
            Revision::INITIAL,
            ByteOffset::try_from(source.len()).expect("测试文档很短"),
            parsed.tree(),
        );

        assert_eq!(
            decorations.visual_len(),
            projection.visual_len(),
            "{source:?}：两条链对视觉长度的理解不一致"
        );

        // 只走字符边界：v1 会拒绝落在字符中间的偏移，DecorationSet 不持有
        // 源码、做不了这个校验。两边契约不同的地方不拿来比。
        for (offset, _) in source
            .char_indices()
            .chain(std::iter::once((source.len(), ' ')))
        {
            let offset = ByteOffset::new(u64::try_from(offset).expect("测试文档很短"));
            assert_eq!(
                decorations.source_to_visual(offset),
                projection
                    .source_to_visual(offset, ProjectionBias::After)
                    .expect("整篇范围内的偏移都合法"),
                "{source:?} 的 source {offset:?}：两条链的 source→visual 不同"
            );
        }

        for visual in 0..=decorations.visual_len().get() {
            for (ours_bias, theirs_bias) in [
                (Bias::Before, ProjectionBias::Before),
                (Bias::After, ProjectionBias::After),
            ] {
                assert_eq!(
                    decorations.visual_to_source(VisualOffset::new(visual), ours_bias),
                    projection
                        .visual_to_source(VisualOffset::new(visual), theirs_bias)
                        .expect("投影长度之内的偏移都合法"),
                    "{source:?} 的 visual {visual} / {ours_bias:?}：\
                     两条链的 visual→source 不同"
                );
            }
        }
    }
}

/// 隐藏之后读起来对不对——人工写死几份期望文本。
///
/// 上面几条都是「和另一个实现一样」，这条是「和人的期望一样」。
/// 两者都需要：oracle 自己也可能错，`DIVERGENCES` 里有九条就是证据。
#[test]
fn visual_text_reads_as_expected() {
    const CASES: &[(&str, &str)] = &[
        ("*a* **b** `c`", "a b c"),
        ("***both***", "both"),
        ("**a*b***", "ab"),
        ("~~~\nfenced *em*\n~~~\n", "\nfenced *em*\n\n"),
        ("    indented *em*\n", "    indented *em*\n"),
        ("``a `b` c``", "a `b` c"),
        ("中文 *强调* 混排", "中文 强调 混排"),
    ];
    for (source, expected) in CASES {
        let hidden = syntax_hidden(source);
        let mut visual = String::new();
        let mut cursor = 0usize;
        for (start, end) in hidden {
            let start = usize::try_from(start).expect("测试文档很短");
            let end = usize::try_from(end).expect("测试文档很短");
            visual.push_str(&source[cursor..start]);
            cursor = end;
        }
        visual.push_str(&source[cursor..]);
        assert_eq!(&visual, expected, "{source:?}：隐藏语法之后的文本不对");
    }
}
