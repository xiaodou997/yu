//! `yu-markdown` 的 extension 集合与 v1 `BlockProjection` 的隐藏区间差分。
//!
//! # 这条测试回答的问题
//!
//! S6 把每一种 Markdown 语法改写成一个 extension。**换掉一个在产品里跑着的
//! 实现，删它之前它就是 oracle**——这条测试就是那个 oracle 的兑现处：同一份
//! 源码、同一批块，两边各自说「哪些字节不进视觉文本」，逐份比对。
//!
//! # 它为什么直到 extension 改用 `yu-syntax` 之后才有意义
//!
//! extension 最初建在 `yu-markdown::inline` 上——那是 **v1 自己的**行内扫描
//! 器。两条路共用同一个 `InlineDocument`，比对必然全绿：连
//! `    indented *em*` 这种 v1 公认判错的都逐字节一致。那正是这个项目点名的
//! 「共用代码路径的差分是自证的」。
//!
//! 换成 `yu-syntax` 的语法树之后两条路才真的分开，差异也随之全部翻到
//! **v1 错**的那一侧：`decoration_parity.rs` 登记为 `ProjectionBug` 的那些
//! 判断，现在由 extension 逐条兑现。
//!
//! # 口径
//!
//! 只比对**非焦点**（无光标）的规范投影。焦点块的「光标碰到语法就露出来」
//! 两边的粒度不同（v1 按 run，extension 按节点），拿它比对会得到一张解释不动
//! 的表；那件事由 `yu-markdown` 自己的用例压。
//!
//! 登记表是紧的：未登记的文档必须逐字节一致，已登记的文档必须**精确**等于
//! 登记值。后一条守的是「差异消失了但登记还留着」——表格 extension 落地时
//! 它正是这么把 `Pending` 那一行逼出来的。口径与 `docs/specs/invariants.md`
//! F 节一致。
//!
//! 这条测试随 `yu-projection` 一起消失。

use yu_core::TextRange;
use yu_markdown::{ExtensionSet, parse};
use yu_projection::{BlockProjection, VisualRunKind};
use yu_syntax::parse as parse_syntax;
use yu_text::TextBuffer;

/// 差异的归属。只剩一类——「extension 错」一条都没有，「还没做的语法」也
/// 随表格 extension 落地清空了。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Cause {
    /// v1 错，extension 对。删掉 v1 时一起消失。
    ProjectionBug,
}

struct Divergence {
    source: &'static str,
    /// extension 集合隐藏的 source 区间。
    extension: &'static [(u64, u64)],
    /// v1 `BlockProjection` 隐藏的 source 区间。
    projection: &'static [(u64, u64)],
    cause: Cause,
    why: &'static str,
}

const DIVERGENCES: &[Divergence] = &[
    Divergence {
        source: "# 标题 #",
        extension: &[(0, 2), (8, 10)],
        projection: &[(0, 2)],
        cause: Cause::ProjectionBug,
        why: "ATX 的收尾 `#` 序列是语法不是内容（CommonMark §4.2）。v1 只认前缀，\
              于是标题右边挂着一个用户没打算显示的 `#`",
    },
    Divergence {
        source: "> > 引用两层",
        extension: &[(0, 4)],
        projection: &[(0, 2)],
        cause: Cause::ProjectionBug,
        why: "两层引用有两个 `> ` 前缀。v1 的块序列只记了 depth=1，于是第二个 \
              `> ` 留在正文里；树里它是嵌套的第二个 `QuoteMark`",
    },
    Divergence {
        source: "***both***",
        extension: &[(0, 3), (7, 10)],
        projection: &[],
        cause: Cause::ProjectionBug,
        why: "三个以上连续定界符，v1 扫描器整段放弃",
    },
    Divergence {
        source: "***a***b",
        extension: &[(0, 3), (4, 7)],
        projection: &[],
        cause: Cause::ProjectionBug,
        why: "同上",
    },
    Divergence {
        source: "**a*b***",
        extension: &[(0, 2), (3, 4), (5, 8)],
        projection: &[],
        cause: Cause::ProjectionBug,
        why: "strong 里嵌 em、右侧收在一起。同样整段放弃",
    },
    Divergence {
        source: "    indented *em*\n",
        extension: &[],
        projection: &[(13, 14), (16, 17)],
        cause: Cause::ProjectionBug,
        why: "四空格缩进是代码块，里面不解析行内语法。v1 没有块级上下文，把代码\
              里的 `*` 当成强调并隐藏——用户看到的代码少了两个字符。树里它是 \
              `CodeBlock(CodeText)`，内部根本没有行内标记节点",
    },
    Divergence {
        source: "a\n\n    code *em*\n",
        extension: &[],
        projection: &[(12, 13), (15, 16)],
        cause: Cause::ProjectionBug,
        why: "同上，缩进代码块出现在段落之后而不是文档开头",
    },
    Divergence {
        source: "\tcode *em*\n",
        extension: &[],
        projection: &[(6, 7), (9, 10)],
        cause: Cause::ProjectionBug,
        why: "制表符缩进的代码块",
    },
    Divergence {
        source: "``a `b` c``",
        extension: &[(0, 2), (9, 11)],
        projection: &[(0, 2), (4, 5), (6, 7), (9, 11)],
        cause: Cause::ProjectionBug,
        why: "多重反引号的代码跨度里，单反引号是字面内容。v1 继续在里面找定界符",
    },
    Divergence {
        source: "<!-- comment *em* -->",
        extension: &[],
        projection: &[(13, 14), (16, 17)],
        cause: Cause::ProjectionBug,
        why: "HTML 注释是 raw HTML，内部不解析行内语法",
    },
    Divergence {
        source: "<http://a.com/*b*>",
        extension: &[(0, 1), (17, 18)],
        projection: &[(0, 1), (14, 15), (16, 18)],
        cause: Cause::ProjectionBug,
        why: "autolink 内部不解析行内语法。v1 在 URL 里找到了一对 `*` 并隐藏，\
              于是地址少掉两个字符",
    },
];

/// 语料。每种语法的常见写法，加上 `decoration_parity.rs` 已经登记过 v1 判错的
/// 那一批——后者是这条差分最该覆盖的地方。
const DOCUMENTS: &[&str] = &[
    "# 标题\n",
    "> 引用\n",
    "- 项目\n",
    "段落\n",
    "*斜体*\n",
    "```rust\nlet x = 1;\n```\n",
    "```\n未闭合\n",
    "```\n```\n",
    "# 标题\n段落\n",
    "- 项目\n\n段落\n",
    "# 标题",
    "## 二级 *斜体* 标题",
    "# 标题 #",
    "#   多空格",
    "> 引用一层",
    "> > 引用两层",
    "> a\n> b",
    "1. 有序",
    "1) 圆括号",
    "  - 缩进项",
    "- a\n- b",
    "- [ ] 待办",
    "- [x] 完成",
    "普通段落 *斜体* 与 **粗体** 与 `代码`",
    "[文字](目标)",
    "[a][b]",
    "![替代](图片)",
    "<http://a.com>",
    "行尾硬换行  \n第二行",
    "行尾\\\n第二行",
    "中文 *强调* 与 emoji 🙂",
    "***both***",
    "***a***b",
    "**a*b***",
    "    indented *em*\n",
    "a\n\n    code *em*\n",
    "\tcode *em*\n",
    "~~~\nfenced *em*\n~~~\n",
    "``a `b` c``",
    "<!-- comment *em* -->",
    "<http://a.com/*b*>",
    "autolink <http://a.com/b>",
    "*a* <http://x.y> *b*",
    "[link *em*](/uri)",
    "![img](/uri)",
    "line *em*  \nnext",
    "a | b\n--- | ---\n1 | 2",
    "",
];

/// 升序、不重叠、不相邻。两边各自的产出粒度不同（v1 按 run 切，extension 按
/// 节点切），不归一化就会比较到「怎么切」而不是「隐藏了哪些字节」。
fn merged(mut ranges: Vec<(u64, u64)>) -> Vec<(u64, u64)> {
    ranges.retain(|(from, to)| from < to);
    ranges.sort_unstable();
    let mut out: Vec<(u64, u64)> = Vec::new();
    for (from, to) in ranges {
        match out.last_mut() {
            Some(last) if from <= last.1 => last.1 = last.1.max(to),
            _ => out.push((from, to)),
        }
    }
    out
}

fn pair(range: TextRange) -> (u64, u64) {
    (range.start().get(), range.end().get())
}

/// 一份文档上两条路各自隐藏的 source 区间。
struct Hidden {
    extension: Vec<(u64, u64)>,
    projection: Vec<(u64, u64)>,
}

/// 两边各自跑一遍整份文档。
fn hidden_ranges(source: &str) -> Hidden {
    let buffer = TextBuffer::new(source.to_owned());
    let snapshot = buffer.snapshot();
    let document = parse(&snapshot);
    let definitions = document.reference_definitions();
    let tree = parse_syntax(&snapshot).expect("测试文档很短").into_tree();
    let extensions = ExtensionSet::markdown();

    let mut ours = Vec::new();
    let mut theirs = Vec::new();
    for block in document.blocks().iter() {
        let decorations = extensions
            .decorate(&snapshot, &tree, block, None)
            .expect("装饰产出不该失败");
        ours.extend(
            decorations
                .set()
                .all()
                .iter()
                .filter(|entry| entry.decoration.hides_source())
                .map(|entry| pair(entry.range)),
        );

        // v1 认不出的块（两层引用的 `InvalidBlockQuoteBlock` 就是一例）在这里
        // 什么都不贡献。那本身是一条差异，由登记表按最终的区间值压住。
        if let Ok(projection) =
            BlockProjection::from_block_with_definitions(&snapshot, block, definitions)
        {
            theirs.extend(
                projection
                    .visual()
                    .runs()
                    .iter()
                    .filter(|run| run.kind() == VisualRunKind::HiddenSyntax)
                    .map(|run| pair(run.source())),
            );
        }
    }
    Hidden {
        extension: merged(ours),
        projection: merged(theirs),
    }
}

fn registered(source: &str) -> Option<&'static Divergence> {
    DIVERGENCES
        .iter()
        .find(|divergence| divergence.source == source)
}

#[test]
fn unregistered_documents_hide_exactly_the_same_bytes() {
    for source in DOCUMENTS {
        if registered(source).is_some() {
            continue;
        }
        let hidden = hidden_ranges(source);
        assert_eq!(
            hidden.extension, hidden.projection,
            "{source:?} 两边隐藏的字节不一致。要么是 extension 错了，要么这是\
             一条新的差异——后者请登记进 DIVERGENCES 并写明归属"
        );
    }
}

/// 已登记的差异必须**精确**等于登记值。
///
/// 这一条比「允许不一致」严得多，守的是「差异消失了但登记还留着」：表格
/// widget 化之后 `Pending` 那一行会红，逼人删掉它而不是让它一直挂着。
#[test]
fn registered_divergences_are_exact() {
    for divergence in DIVERGENCES {
        let hidden = hidden_ranges(divergence.source);
        assert_eq!(
            hidden.extension, divergence.extension,
            "{:?} 的 extension 侧变了（{}）",
            divergence.source, divergence.why
        );
        assert_eq!(
            hidden.projection, divergence.projection,
            "{:?} 的 v1 侧变了（{}）",
            divergence.source, divergence.why
        );
        assert_ne!(
            hidden.extension, hidden.projection,
            "{:?} 两边已经一致了，请把这一行从 DIVERGENCES 删掉",
            divergence.source
        );
    }
}

/// 每一条登记都必须在语料里，否则它压不住任何东西。
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

/// 归属统计。差异的**形状**本身是这一阶段的结论，写成断言免得它悄悄变了。
///
/// 「extension 错」一条都没有——这是换掉 v1 的依据。
#[test]
fn divergence_causes_stay_accounted_for() {
    let count = |cause: Cause| {
        DIVERGENCES
            .iter()
            .filter(|divergence| divergence.cause == cause)
            .count()
    };
    assert_eq!(count(Cause::ProjectionBug), 11, "v1 判错的条数变了");
    assert_eq!(DIVERGENCES.len(), 11);
}
