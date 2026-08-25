//! 从 `yu-syntax` 的语法树产出装饰。
//!
//! # 这是 S6 的形状，提前落地一小块
//!
//! 第 4.3 节给 `yu-markdown` 的职责是「Markdown 语法定义与 decoration 产出」。
//! S6 会把每一种语法改写成一个 extension，各自产出自己的装饰；这里先只做
//! **隐藏行内语法标记**这一件事，目的是把 `yu-syntax → yu-decoration` 这条
//! 链路端到端接通一次。
//!
//! 接通它的价值不在功能，在于回答一个到 S5 才会暴露的问题：**`yu-syntax`
//! 的标记节点范围，能不能真的驱动「隐藏语法」？** S3 结束时 `yu-syntax`
//! 零消费者，这个问题没有答案；再拖到 S5 就要和布局重写一起爆。
//!
//! 答案是能。`crates/yu-projection/tests/decoration_parity.rs` 拿 v1 的行内
//! 扫描器当 oracle，76 份语料逐字节比对，没有一处是这里的范围错了；判错的
//! 11 份全是 v1 缺块级上下文。
//!
//! 老的扫描器（本 crate 的其余部分）仍然是产品链路，两者在 S6 之前并存。

use yu_core::{ByteOffset, Revision, TextRange};
use yu_decoration::{Decoration, DecorationRange, DecorationSet};
use yu_syntax::{NodeKind, Tree};

/// 隐藏行内语法标记的装饰。
///
/// 现在只覆盖强调与行内代码的定界符——挑这两种是因为 `yu-projection` 的
/// `Projection::inline` 恰好也只隐藏它们，于是两者可以逐份文档比对
/// （见 `crates/yu-projection/tests/decoration_parity.rs`）。
///
/// 块级前缀（`#`、`>`、列表标记）与链接的括号暂不隐藏：它们在 v1 里是靠
/// 「语义 marker」而不是单纯隐藏来呈现的，属于 S6 逐个 extension 的工作。
#[must_use]
pub fn inline_syntax_decorations(tree: &Tree) -> Vec<DecorationRange> {
    let mut out = Vec::new();
    collect(tree, 0, hides, &mut out);
    out
}

/// 只隐藏强调的定界符。**一个 extension 的产出。**
///
/// 与 [`code_decorations`] 一起构成 [`inline_syntax_decorations`] 的全部内容，
/// 但两者是各自独立遍历树得出的，谁都不知道对方存在——第 4.3 节与不变量 D6
/// 要求的就是这个。S6 会把每一种语法都变成这个形状。
#[must_use]
pub fn emphasis_decorations(tree: &Tree) -> Vec<DecorationRange> {
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
#[must_use]
pub fn code_decorations(tree: &Tree) -> Vec<DecorationRange> {
    let mut out = Vec::new();
    collect(tree, 0, |kind| matches!(kind, NodeKind::CodeMark), &mut out);
    out
}

/// 上面两个 extension 各自的产出，各装进一个绑定 revision 的集合。
///
/// 交给 [`DecorationSet::merge`] 就得到与 [`inline_syntax_decoration_set`]
/// 相同的结果。两条路都留着是有原因的，见 `tests/extension_merge.rs`：
/// 它们**共用同一个 `collect`**，所以「拆开再合并等于不拆」这件事本身几乎
/// 是恒真的，那条测试真正压的是 `merge`——强调与代码的区间在真实文档里是
/// 交错的（`` *`a`* `` 就是 Emphasis/Code/Code/Emphasis），合并要把它们正确
/// 定序并把相邻的隐藏区间接起来。
#[must_use]
pub fn extension_decoration_sets(
    revision: Revision,
    source_len: ByteOffset,
    tree: &Tree,
) -> Vec<DecorationSet> {
    vec![
        DecorationSet::new(revision, source_len, emphasis_decorations(tree)),
        DecorationSet::new(revision, source_len, code_decorations(tree)),
    ]
}

/// 把上面的产出装进一个绑定 revision 的集合。
///
/// # Panics
///
/// 不会 panic：越界的装饰由 [`DecorationSet::new`] 自己丢掉。
#[must_use]
pub fn inline_syntax_decoration_set(
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

/// 哪些节点是「可以整段隐藏的语法字符」。
///
/// 只列强调与代码的定界符。**不含** `HeaderMark` / `QuoteMark` / `ListMark`
/// ——那三种在 v1 里是被替换成语义 marker 而不是简单隐藏的，行为不同，
/// 混进来会让差分比较的是两件事。
const fn hides(kind: NodeKind) -> bool {
    matches!(kind, NodeKind::EmphasisMark | NodeKind::CodeMark)
}

#[cfg(test)]
mod tests {
    use super::inline_syntax_decorations;
    use yu_syntax::parse;

    fn ranges(source: &str) -> Vec<(u64, u64)> {
        let parsed = parse(source).expect("测试文档很短");
        inline_syntax_decorations(parsed.tree())
            .into_iter()
            .map(|entry| (entry.range.start().get(), entry.range.end().get()))
            .collect()
    }

    #[test]
    fn emphasis_and_code_delimiters_are_hidden() {
        assert_eq!(ranges("*a*"), vec![(0, 1), (2, 3)]);
        assert_eq!(ranges("**a**"), vec![(0, 2), (3, 5)]);
        assert_eq!(ranges("`a`"), vec![(0, 1), (2, 3)]);
    }

    /// 不成对的定界符不产生标记节点，因此也不该被隐藏——否则用户打下第一个
    /// `*` 的瞬间它就消失了。
    #[test]
    fn unmatched_delimiters_stay_visible() {
        assert_eq!(ranges("*a"), Vec::<(u64, u64)>::new());
        assert_eq!(ranges("a`b"), Vec::<(u64, u64)>::new());
    }

    /// 块级标记不在这一批里。
    #[test]
    fn block_prefixes_are_not_hidden_here() {
        assert_eq!(ranges("# title\n"), Vec::<(u64, u64)>::new());
        assert_eq!(ranges("> quote\n"), Vec::<(u64, u64)>::new());
        assert_eq!(ranges("- item\n"), Vec::<(u64, u64)>::new());
    }

    /// 产出必须升序且不重叠——`DecorationSet` 的合并依赖这一点，
    /// 而树的遍历顺序是否保证它并不显然。
    #[test]
    fn output_is_ordered_and_disjoint() {
        let found = ranges("*a* **b** `c` *d*\n\n**e `f` g**\n");
        assert!(!found.is_empty());
        for pair in found.windows(2) {
            assert!(pair[0].1 <= pair[1].0, "产出不是升序不重叠的：{found:?}");
        }
    }
}
