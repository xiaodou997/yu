//! 面板那两列文字的性质。
//!
//! 这些断言原来住在 `platform/macos/.../SelfChecks.swift` 里——两条 panel
//! self-check 各自在壳里重证了一遍「树挂对了没有」「标签剥干净了没有」。
//! 第七刀 c 的第三块把逻辑挪进 Rust 之后，它们**自然落到这一层**：壳里再
//! 断言同一件事就成了自证（树的形状现在由 Rust 给）。
//!
//! **判据的分工没变**：「藏对了没有」不在这里证，那是 `yu-decoration` 的线性
//! 参照实现与 `yu-markdown` 那 45 条压住的事。这里压的是别的——身份链把同名
//! 兄弟分不分得开、一次「在文首插入」之后身份还在不在、上下文有没有越出块。

use yu_editor::{EditorDocument, OutlineTree, SearchResults};

/// 语料只求形状多，不求真实。面板这一层的失败模式是**层级挂错**、**标签带
/// 着语法出去**、**身份跟着下标漂**，三样都不 panic。
const CORPUS: &[&str] = &[
    "",
    "只有段落\n",
    "# 一级\n",
    "# 一级\n\n## 二级\n\n### 三级\n",
    "# 一级\n\n## 二级\n\n# 另一个一级\n",
    "### 从三级开头\n\n# 后面才是一级\n",
    "# 一级\n\n### 跳到三级\n\n## 再回二级\n",
    "## 同级\n\n## 同级\n\n## 同级\n",
    "# 带 **行内标记** 的标题\n",
    "# 带 [链接](https://example.com) 的标题\n",
    "## 收尾串 ##\n",
    "多行\n标题\n===\n",
    "# 标题\n\n```\n# 代码里的井号\n```\n\n## 之后\n",
    "> # 引用块里的标题\n",
    "- # 列表项里的标题\n",
    "# 1\n\n## 2\n\n### 3\n\n#### 4\n\n##### 5\n\n###### 6\n",
    "#\n\n## 空标题之后\n",
];

fn outline_of(source: &str) -> (EditorDocument, OutlineTree) {
    let mut document = EditorDocument::new(source);
    let tree = OutlineTree::build(&mut document).expect("大纲");
    (document, tree)
}

fn labels(tree: &OutlineTree) -> Vec<&str> {
    tree.rows().iter().map(|row| row.label()).collect()
}

/// 「前序 + 直接孩子数」还原出来的父子关系，必须与平表的 `parent` 字段
/// 一字不差。
///
/// **这一条正是壳里那次转换的判据搬过来的**：壳原来按 `parent` 查表挂树，
/// self-check 反过来核对树的形状。现在树的形状由这里给，所以判据也得在这里
/// ——两个字段互为对方的参照，挂错父亲、静默把孩子提成根都在这一条下面。
fn parents_from_shape(tree: &OutlineTree) -> Vec<Option<usize>> {
    let mut parents = vec![None; tree.rows().len()];
    // 栈里存 (行号, 还欠几个直接孩子)。
    let mut stack: Vec<(usize, usize)> = Vec::new();
    for (index, row) in tree.rows().iter().enumerate() {
        while stack.last().is_some_and(|(_, remaining)| *remaining == 0) {
            stack.pop();
        }
        if let Some((parent, remaining)) = stack.last_mut() {
            parents[index] = Some(*parent);
            *remaining -= 1;
        }
        stack.push((index, row.child_count()));
    }
    parents
}

#[test]
fn preorder_and_child_counts_rebuild_the_parent_links() {
    for source in CORPUS {
        let (_, tree) = outline_of(source);
        let declared: Vec<Option<usize>> =
            tree.rows().iter().map(|row| row.item().parent()).collect();
        assert_eq!(
            parents_from_shape(&tree),
            declared,
            "还原出来的树与平表的 parent 不一致：{source:?}"
        );
    }
}

#[test]
fn every_row_reports_its_direct_children() {
    let (_, tree) = outline_of("# a\n\n## b\n\n### c\n\n## d\n\n# e\n");
    let counts: Vec<usize> = tree.rows().iter().map(|row| row.child_count()).collect();
    assert_eq!(counts, vec![2, 1, 0, 0, 0]);
}

/// 标签是**正文减掉被藏起来的那几段**：行内标记不进面板。
#[test]
fn labels_drop_the_inline_syntax() {
    let (_, tree) = outline_of("# 带 **行内标记** 的标题\n");
    assert_eq!(labels(&tree), vec!["带 行内标记 的标题"]);

    let (_, tree) = outline_of("# 带 [链接](https://example.com) 的标题\n");
    assert_eq!(labels(&tree), vec!["带 链接 的标题"]);

    // ATX 的收尾串由树剥掉（`heading_content_range`），不是靠装饰。
    let (_, tree) = outline_of("## 收尾串 ##\n");
    assert_eq!(labels(&tree), vec!["收尾串"]);
}

/// Setext 多行标题折成一行。**面板上一行就是一行**——不折的话那一条要么
/// 被截断，要么把整列撑高，两样都不报错。
#[test]
fn a_setext_heading_folds_onto_one_line() {
    let (_, tree) = outline_of("多行\n标题\n===\n");
    assert_eq!(labels(&tree), vec!["多行 标题"]);
}

#[test]
fn no_label_ever_contains_a_line_break() {
    for source in CORPUS {
        let (_, tree) = outline_of(source);
        for row in tree.rows() {
            assert!(
                !row.label().contains(['\n', '\r', '\u{2028}', '\u{2029}']),
                "标签里有换行：{:?}（{source:?}）",
                row.label()
            );
        }
    }
}

/// 标签只允许**删**字节，不允许改写或换顺序（折行那一步只动空白，所以拿
/// 去掉空白之后的两串比）。
#[test]
fn a_label_is_a_subsequence_of_its_source() {
    fn is_subsequence(candidate: &str, source: &str) -> bool {
        let mut remaining = source.chars();
        candidate
            .chars()
            .all(|wanted| remaining.any(|actual| actual == wanted))
    }

    for source in CORPUS {
        let (_, tree) = outline_of(source);
        for row in tree.rows() {
            let start = usize::try_from(row.item().label_range().start()).expect("放得进 usize");
            let end = usize::try_from(row.item().label_range().end()).expect("放得进 usize");
            let raw: String = source[start..end]
                .chars()
                .filter(|character| !character.is_whitespace())
                .collect();
            let label: String = row
                .label()
                .chars()
                .filter(|character| !character.is_whitespace())
                .collect();
            assert!(
                is_subsequence(&label, &raw),
                "「{label}」不是「{raw}」的子序列（{source:?}）"
            );
        }
    }
}

#[test]
fn identities_are_unique_within_one_tree() {
    for source in CORPUS {
        let (_, tree) = outline_of(source);
        let mut seen = std::collections::HashSet::new();
        for row in tree.rows() {
            assert!(
                seen.insert(row.identity()),
                "身份撞了：{:?}（{source:?}）",
                row.identity()
            );
        }
    }
}

/// 同一个父亲下的同名兄弟按出现次序区分，**不同父亲下的同名标题也不能撞**。
#[test]
fn same_named_siblings_get_distinct_identities() {
    let (_, tree) = outline_of("# a\n\n## 同名\n\n## 同名\n\n# b\n\n## 同名\n");
    let identities: Vec<&str> = tree.rows().iter().map(|row| row.identity()).collect();
    assert_eq!(identities.len(), 5);
    let unique: std::collections::HashSet<&&str> = identities.iter().collect();
    assert_eq!(unique.len(), 5, "{identities:?}");
}

/// **在文首插一条标题之后，原有每一条的身份都不变。**
///
/// 这一条压的是「身份不能按下标记」：那次编辑把每一条的 `index` 与 `block`
/// 一起推后一位。按下标记的话展开状态与选中行会整体错位——不报错，只是
/// 展开的变成了别人。在末尾追加是压不住它的，那种编辑谁都活得下来。
#[test]
fn identities_survive_an_insertion_at_the_head_of_the_document() {
    let source = "# 一级\n\n## 二级\n\n### 三级\n\n## 另一个二级\n";
    let (_, before) = outline_of(source);
    let (_, after) = outline_of(&format!("# 新的顶层\n\n{source}"));

    let before_identities: Vec<&str> = before.rows().iter().map(|row| row.identity()).collect();
    let after_identities: Vec<&str> = after
        .rows()
        .iter()
        .skip(1)
        .map(|row| row.identity())
        .collect();
    assert_eq!(before_identities, after_identities);

    // 而下标真的整体推后了——否则上面那一条什么都没压住。
    for (old, new) in before.rows().iter().zip(after.rows().iter().skip(1)) {
        assert_eq!(old.item().index() + 1, new.item().index());
        assert!(new.item().block() > old.item().block());
    }
}

fn results(source: &str, query: &str) -> (EditorDocument, SearchResults) {
    let mut document = EditorDocument::new(source);
    document.set_search_query(query);
    let results = SearchResults::build(&mut document).expect("搜索结果");
    (document, results)
}

const SEARCH_SOURCE: &str = concat!(
    "# 搜索的**标记**测试\n\n",
    "段落里有标记，同一行上还有第二个标记。\n\n",
    "- 列表项里的**标记**\n\n",
    "> 引用块里的*标记*\n",
);

#[test]
fn a_search_row_drops_the_inline_syntax_of_its_line() {
    let (_, results) = results(SEARCH_SOURCE, "标记");
    let labels: Vec<&str> = results.rows().iter().map(|row| row.label()).collect();
    assert_eq!(labels.len(), 5, "{labels:?}");
    assert!(
        labels.iter().all(|label| !label.contains('*')),
        "{labels:?}"
    );
    assert!(labels.contains(&"搜索的标记测试"), "{labels:?}");
    assert!(labels.contains(&"列表项里的标记"), "{labels:?}");
    assert!(labels.contains(&"引用块里的标记"), "{labels:?}");
}

/// 同一行上的两处命中是**两行结果**，显示同一段上下文，但指向不同的位置。
/// 「按行去重」会让第二处点不到——不报错，只是少一条。
#[test]
fn two_hits_on_one_line_are_two_rows() {
    let (_, results) = results(SEARCH_SOURCE, "标记");
    let same_line: Vec<_> = results
        .rows()
        .iter()
        .filter(|row| row.label().contains("第二个标记"))
        .collect();
    assert_eq!(same_line.len(), 2);
    assert_ne!(same_line[0].hit(), same_line[1].hit());
    assert_eq!(same_line[0].context(), same_line[1].context());
}

/// 上下文必须**落在块里**、并且**盖住命中**。
///
/// 前者是这一层唯一会静默做错的事：越出块的请求在拿隐藏区间那一步会被拒，
/// 那一行悄悄带回语法标记。
#[test]
fn every_context_sits_inside_its_block_and_covers_its_hit() {
    for query in ["标记", "里", "\n", "的"] {
        let (_, results) = results(SEARCH_SOURCE, query);
        for row in results.rows() {
            assert!(
                row.context().start() >= row.block_range().start()
                    && row.context().end() <= row.block_range().end(),
                "上下文 {:?} 越出了块 {:?}",
                row.context(),
                row.block_range()
            );
            assert!(
                row.context().start() <= row.hit().start()
                    && row.context().end() >= row.hit().end(),
                "上下文 {:?} 没有盖住命中 {:?}",
                row.context(),
                row.hit()
            );
        }
    }
}

#[test]
fn no_search_label_ever_contains_a_line_break() {
    for query in ["标记", "里", "的", "\n\n"] {
        let (_, results) = results(SEARCH_SOURCE, query);
        for row in results.rows() {
            assert!(
                !row.label().contains(['\n', '\r', '\u{2028}', '\u{2029}']),
                "结果列表上不能出现换行：{:?}",
                row.label()
            );
        }
    }
}

/// 没有搜索、以及查不到东西，都是 0 行，不是错误。
#[test]
fn no_query_and_no_hit_both_mean_no_rows() {
    let mut document = EditorDocument::new(SEARCH_SOURCE);
    let never_searched = SearchResults::build(&mut document).expect("没有搜索也算得出来");
    assert!(never_searched.rows().is_empty());

    let (_, missed) = results(SEARCH_SOURCE, "这四个字一定不在里面");
    assert!(missed.rows().is_empty());
}
