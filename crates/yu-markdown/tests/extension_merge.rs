//! 两个 extension 各自产出、再合并，与一次性产出比对。
//!
//! # 这条测试真正压的是 `merge`，不是拆分本身
//!
//! `emphasis_decorations` 与 `code_decorations` **共用同一个 `collect`**，只是
//! 认的节点类型不同。所以「拆开再合并等于不拆」这件事，在产出这一侧几乎是
//! 恒真的——拿它当拆分的验证会是一条什么都没测的测试，那是这个项目点名的
//! 危险类别。
//!
//! 它有价值的地方在另一头：这两个 extension 提供了一组**区间交错**的真实
//! 输入。`` *`a`* `` 产出的是 Emphasis(0,1) / Code(1,2) / Code(3,4) /
//! Emphasis(4,5)，分属两个集合、彼此相邻。合并要把它们正确定序，并把跨集合
//! 相邻的隐藏区间接成一个视觉位置。这两件事都只有 `merge` 负责，而
//! `DecorationSet::new` 是它的 oracle。
//!
//! 不变量 D6，以及 `docs/architecture/overview-v2.md` 第 5.2 节第 4 条。
//!
//! # 产出器住在这个文件里
//!
//! 它们此前是 `yu-markdown::decorations` 的公开函数——S4 为了把
//! `yu-syntax → yu-decoration` 这条链路端到端接通一次而提前落地的一小块。
//! S6 之后产品链路上的产出器是 `ExtensionSet`，这三个函数就只剩这条测试
//! 一个使用者了。留在产品里的话它是一段没人调用的公开 API；搬进来它就是
//! 它实际的身份：一组给 `merge` 喂真实交错输入的夹具。

use yu_core::{ByteOffset, Revision, TextRange, VisualOffset};
use yu_decoration::{Bias, Decoration, DecorationRange, DecorationSet};
use yu_syntax::{NodeKind, Tree, parse};

fn inline_syntax_decorations(tree: &Tree) -> Vec<DecorationRange> {
    let mut out = Vec::new();
    collect(tree, 0, hides, &mut out);
    out
}

/// 只隐藏强调的定界符。**一个 extension 的产出。**
fn emphasis_decorations(tree: &Tree) -> Vec<DecorationRange> {
    let mut out = Vec::new();
    collect(
        tree,
        0,
        |kind| matches!(kind, NodeKind::EmphasisMark),
        &mut out,
    );
    out
}

/// 只隐藏行内代码与围栏的定界符。**另一个 extension 的产出。**
fn code_decorations(tree: &Tree) -> Vec<DecorationRange> {
    let mut out = Vec::new();
    collect(tree, 0, |kind| matches!(kind, NodeKind::CodeMark), &mut out);
    out
}

/// 两个 extension 各自的产出，各装进一个绑定 revision 的集合。
///
/// 交给 `DecorationSet::merge` 就该得到与 `inline_syntax_decoration_set`
/// 相同的结果。
fn extension_decoration_sets(
    revision: Revision,
    source_len: ByteOffset,
    tree: &Tree,
) -> Vec<DecorationSet> {
    vec![
        DecorationSet::new(revision, source_len, emphasis_decorations(tree)),
        DecorationSet::new(revision, source_len, code_decorations(tree)),
    ]
}

/// 一次性产出，装进一个绑定 revision 的集合。
fn inline_syntax_decoration_set(
    revision: Revision,
    source_len: ByteOffset,
    tree: &Tree,
) -> DecorationSet {
    DecorationSet::new(revision, source_len, inline_syntax_decorations(tree))
}

fn collect(
    tree: &Tree,
    from: u32,
    hides: impl Fn(NodeKind) -> bool + Copy,
    out: &mut Vec<DecorationRange>,
) {
    if hides(tree.kind())
        && let Some(range) = TextRange::new(
            ByteOffset::from(from),
            ByteOffset::from(from + tree.len_bytes()),
        )
    {
        out.push(DecorationRange::new(range, Decoration::Replace));
        // 标记节点没有子节点，也不该有——继续下降只会重复覆盖。
        return;
    }
    for index in 0..tree.child_count() {
        let Some((child, position)) = tree.child(index) else {
            break;
        };
        collect(child, from + position, hides, out);
    }
}

/// 哪些节点算「可以整段隐藏的语法字符」。只列强调与代码的定界符。
const fn hides(kind: NodeKind) -> bool {
    matches!(kind, NodeKind::EmphasisMark | NodeKind::CodeMark)
}

/// 挑的都是强调与代码交错、或定界符彼此相邻的写法。
const DOCUMENTS: &[&str] = &[
    "*`a`*",
    "`*a*`",
    "**`a`**",
    "*a* `b` **c**",
    "**紧邻**`的`*三段*",
    "`code` *em* `code` *em*",
    "*a **b** `c` d*",
    "```\nfenced *em*\n```\n",
    "a *b* c",
    "`a`",
    "plain text",
    "",
    "*a*`b`*c*`d*`",
    "***both*** `and` code",
];

fn source_len(source: &str) -> ByteOffset {
    ByteOffset::try_from(source.len()).expect("测试文档很短")
}

#[test]
fn merging_the_two_extensions_equals_producing_them_together() {
    for source in DOCUMENTS {
        let parsed = parse(*source).expect("测试文档很短");
        let len = source_len(source);
        let sets = extension_decoration_sets(Revision::INITIAL, len, parsed.tree());
        let merged = DecorationSet::merge(Revision::INITIAL, len, sets.iter())
            .expect("同一个 revision 与长度");
        let together = inline_syntax_decoration_set(Revision::INITIAL, len, parsed.tree());

        assert_eq!(
            merged.all(),
            together.all(),
            "{source:?}：合并后的装饰列表与一次性产出不同"
        );
        assert_eq!(
            merged.visual_len(),
            together.visual_len(),
            "{source:?}：合并后的视觉长度与一次性产出不同"
        );

        // 映射也要逐点一致——跨集合相邻的隐藏区间没接上的话，
        // 装饰列表相同而映射不同，只比 all() 看不出来。
        for offset in 0..=len.get() {
            assert_eq!(
                merged.source_to_visual(ByteOffset::new(offset)),
                together.source_to_visual(ByteOffset::new(offset)),
                "{source:?} 的 source {offset}：合并后的 source→visual 不同"
            );
        }
        for visual in 0..=merged.visual_len().get() {
            for bias in [Bias::Before, Bias::After] {
                assert_eq!(
                    merged.visual_to_source(VisualOffset::new(visual), bias),
                    together.visual_to_source(VisualOffset::new(visual), bias),
                    "{source:?} 的 visual {visual} / {bias:?}：合并后的 visual→source 不同"
                );
            }
        }
    }
}

/// 每个 extension 只认自己那一种标记，谁都不产出对方的。
///
/// 两边**分开**断言。写成「合起来都是定界符字符」会宽松到抓不住越界：
/// 代码 extension 多产出一条强调标记时，那也是个定界符字符，测试照样绿。
#[test]
fn each_extension_produces_only_its_own_marks() {
    for source in DOCUMENTS {
        let parsed = parse(*source).expect("测试文档很短");
        let check = |entries: Vec<yu_decoration::DecorationRange>, allowed: &[char], who: &str| {
            for entry in entries {
                let start = usize::try_from(entry.range.start().get()).expect("测试文档很短");
                let end = usize::try_from(entry.range.end().get()).expect("测试文档很短");
                let slice = &source[start..end];
                assert!(
                    !slice.is_empty() && slice.chars().all(|ch| allowed.contains(&ch)),
                    "{source:?}：{who} extension 产出了 {slice:?}，那不是它该认的标记"
                );
            }
        };
        check(emphasis_decorations(parsed.tree()), &['*', '_'], "强调");
        check(code_decorations(parsed.tree()), &['`', '~'], "代码");
    }
}

/// 语料必须真的出现「两个 extension 的区间彼此相邻」，
/// 否则跨集合的相邻合并根本没被走到。
#[test]
fn the_corpus_actually_interleaves_the_two_extensions() {
    let interleaved = DOCUMENTS
        .iter()
        .filter(|source| {
            let parsed = parse(**source).expect("测试文档很短");
            let emphasis = emphasis_decorations(parsed.tree());
            let code = code_decorations(parsed.tree());
            emphasis.iter().any(|left| {
                code.iter().any(|right| {
                    left.range.end() == right.range.start()
                        || right.range.end() == left.range.start()
                })
            })
        })
        .count();
    assert!(
        interleaved >= 3,
        "只有 {interleaved} 份语料让两个 extension 的区间相邻，\
         跨集合的相邻合并没有被覆盖"
    );
}

/// 夹具本身的前提：产出必须升序且不重叠。
///
/// 树的遍历顺序是否保证这一点并不显然，而 `merge` 的输入靠它。这条不在
/// 测 `merge`，它守的是「上面那些断言比的是有效输入」。
#[test]
fn the_fixture_produces_ordered_disjoint_ranges() {
    for source in DOCUMENTS {
        let parsed = parse(*source).expect("测试文档很短");
        let found: Vec<_> = inline_syntax_decorations(parsed.tree())
            .into_iter()
            .map(|entry| (entry.range.start().get(), entry.range.end().get()))
            .collect();
        for pair in found.windows(2) {
            assert!(
                pair[0].1 <= pair[1].0,
                "{source:?} 的产出不是升序不重叠的：{found:?}"
            );
        }
    }
}

/// 不成对的定界符不产生标记节点，因此也不该被隐藏——否则用户打下第一个
/// `*` 的瞬间它就消失了。
#[test]
fn unmatched_delimiters_stay_visible() {
    for source in ["*a", "a`b"] {
        let parsed = parse(source).expect("测试文档很短");
        assert!(
            inline_syntax_decorations(parsed.tree()).is_empty(),
            "{source:?} 里的定界符没有配对，不该被隐藏"
        );
    }
}

/// 块级标记不在这一批里：它们由各自的 extension 管，行为是替换而不是隐藏。
#[test]
fn block_prefixes_are_not_hidden_here() {
    for source in ["# title\n", "> quote\n", "- item\n"] {
        let parsed = parse(source).expect("测试文档很短");
        assert!(
            inline_syntax_decorations(parsed.tree()).is_empty(),
            "{source:?} 的块级前缀不该由行内标记的产出器隐藏"
        );
    }
}
