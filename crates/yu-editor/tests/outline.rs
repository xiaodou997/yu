//! 大纲：标题的层级视图。
//!
//! 这份视图与 `AccessibilitySemanticSnapshot` 是**并列**的两个消费者，共用
//! `yu_markdown::heading_content_range` 这一份定义（理由写在
//! `yu-editor/src/outline.rs` 的模块文档里）。下面的用例分两类：
//!
//! - **层级**：只有大纲有，语义树是扁平的，压不住；
//! - **对齐**：大纲与语义树对同一篇文档必须报出同一批标题。这一条**不是完全
//!   自证的**——「正文在哪」两边共用实现（那一半自证），「哪些块是标题、几
//!   级」两边各自 match `BlockKind`，是分开的两段代码。它守的是有人日后
//!   往其中一边加一条过滤。

use yu_editor::{
    AccessibilitySemanticKind, AccessibilitySemanticSnapshot, EditorCommand, EditorDocument,
    OutlineSnapshot,
};
use yu_markdown::BlockKind;

/// 语料只求形状多，不求真实。大纲的失败模式是**层级挂错**与**正文带着语法
/// 出去**，两样都不 panic。
const CORPUS: &[&str] = &[
    "",
    "只有段落\n",
    "# 一级\n",
    "# 一级\n\n## 二级\n\n### 三级\n",
    "# 一级\n\n## 二级\n\n# 另一个一级\n",
    "### 从三级开头\n\n# 后面才是一级\n",
    "# 一级\n\n### 跳到三级\n\n## 再回二级\n",
    "## 同级\n\n## 同级\n\n## 同级\n",
    "标题\n===\n",
    "# ATX\n\nSetext\n---\n",
    "#\n\n## 空标题之后\n",
    "## 收尾井号 ##\n",
    "###\n",
    "# 标题\n\n```\n# 代码里的井号\n```\n\n## 之后\n",
    "> # 引用块里的标题\n",
    "- # 列表项里的标题\n",
    "# 1\n\n## 2\n\n### 3\n\n#### 4\n\n##### 5\n\n###### 6\n",
    "多行\n标题\n===\n",
    "#     多空格\n",
    "   ## 缩进的 ATX\n",
];

fn outline_of(source: &str) -> (EditorDocument, OutlineSnapshot) {
    let document = EditorDocument::new(source);
    let outline = OutlineSnapshot::from_document(&document);
    (document, outline)
}

fn text(source: &str, range: yu_core::TextRange) -> &str {
    let start = usize::try_from(range.start()).expect("源码偏移放得进 usize");
    let end = usize::try_from(range.end()).expect("源码偏移放得进 usize");
    &source[start..end]
}

#[test]
fn nesting_follows_the_nearest_smaller_level() {
    let (_, outline) = outline_of("# a\n\n## b\n\n### c\n\n## d\n\n# e\n");
    let levels_and_parents: Vec<(u8, Option<usize>)> = outline
        .items()
        .iter()
        .map(|item| (item.level(), item.parent()))
        .collect();
    assert_eq!(
        levels_and_parents,
        vec![
            (1, None),    // a
            (2, Some(0)), // b 挂在 a
            (3, Some(1)), // c 挂在 b
            (2, Some(0)), // d 回到 a，不是 c 的孩子
            (1, None),    // e 重新是根
        ]
    );
}

/// 文档以深级标题开头时，它就是根——不给它造一个不存在的父亲。
#[test]
fn a_deeper_first_heading_is_still_a_root() {
    let (_, outline) = outline_of("### deep\n\n# root\n");
    assert_eq!(outline.items()[0].parent(), None);
    assert_eq!(outline.items()[0].level(), 3);
    assert_eq!(outline.items()[1].parent(), None);
}

/// 跳级保留原级别，不压平。大纲报的是文档写成什么样。
#[test]
fn a_skipped_level_keeps_its_level() {
    let (_, outline) = outline_of("# a\n\n### c\n");
    assert_eq!(outline.items()[1].level(), 3);
    assert_eq!(outline.items()[1].parent(), Some(0));
}

/// 正文不带结构标记：`#` 前缀、收尾的 ` ##`、Setext 的下划线都不在里面。
///
/// 这一条第一次跑就红——收尾的 ` ##` 当时会带出去。
#[test]
fn the_label_carries_no_syntax() {
    for (source, expected) in [
        ("# 一级\n", "一级"),
        ("#     多空格\n", "多空格"),
        ("   ## 缩进的 ATX\n", "缩进的 ATX"),
        ("## 收尾井号 ##\n", "收尾井号"),
        ("###\n", ""),
        ("#\n", ""),
        ("标题\n===\n", "标题"),
        ("多行\n标题\n---\n", "多行\n标题"),
    ] {
        let (_, outline) = outline_of(source);
        let item = outline.items().first().copied().expect("至少一条标题");
        assert_eq!(text(source, item.label_range()), expected, "{source:?}");
    }
}

/// 每一条都能回到一个真的标题块，块索引严格递增。导航按 `block` 走，
/// 指错块的后果是「点大纲跳到别处」——不报错。
#[test]
fn every_item_points_at_its_heading_block_across_the_corpus() {
    for source in CORPUS {
        let (document, outline) = outline_of(source);
        let blocks = document.markdown().blocks();
        let mut previous: Option<usize> = None;
        for item in outline.items() {
            let block = blocks.get(item.block()).expect("块索引必须查得到");
            assert_eq!(
                block.kind(),
                BlockKind::Heading {
                    level: item.level()
                },
                "{source:?} 的第 {} 条",
                item.index()
            );
            assert_eq!(item.source_range(), block.range(), "{source:?}");
            assert!(
                previous.is_none_or(|last| last < item.block()),
                "{source:?} 的块索引没有递增"
            );
            previous = Some(item.block());
        }
    }
}

/// 层级本身的自洽：父亲在前、级别更小、序号连续。
#[test]
fn the_hierarchy_is_well_formed_across_the_corpus() {
    for source in CORPUS {
        let (_, outline) = outline_of(source);
        for (position, item) in outline.items().iter().enumerate() {
            assert_eq!(item.index(), position, "{source:?} 的序号不连续");
            let Some(parent) = item.parent() else {
                continue;
            };
            assert!(parent < item.index(), "{source:?} 的父亲不在前面");
            let parent = outline.item(parent).expect("父亲必须查得到");
            assert!(
                parent.level() < item.level(),
                "{source:?}：{} 级挂在了 {} 级下",
                item.level(),
                parent.level()
            );
        }
    }
}

/// 正文区间落在标题块之内。越界的后果是大纲显示邻块的内容。
#[test]
fn the_label_stays_inside_its_block_across_the_corpus() {
    for source in CORPUS {
        let (_, outline) = outline_of(source);
        for item in outline.items() {
            let block = item.source_range();
            assert!(
                item.label_range().start() >= block.start()
                    && item.label_range().end() <= block.end(),
                "{source:?} 的第 {} 条正文越出了它的块",
                item.index()
            );
        }
    }
}

/// 大纲与可访问性语义树报出同一批标题。
///
/// 两条路只在 `heading_content_range` 汇合，所以「正文在哪」这一半是自证的；
/// **「哪些块是标题、几级」不是**——语义树走 `semantic_block_kind`，大纲自己
/// match `BlockKind::Heading`，两段独立的代码。这一条守的就是它们分叉。
#[test]
fn the_outline_and_the_semantic_tree_agree_on_headings() {
    for source in CORPUS {
        let (document, outline) = outline_of(source);
        let semantic =
            AccessibilitySemanticSnapshot::from_document(&document).expect("语义树该建得起来");
        let semantic_headings: Vec<(u8, u64, u64)> = semantic
            .nodes()
            .iter()
            .filter(|node| node.kind() == AccessibilitySemanticKind::Heading)
            .map(|node| {
                (
                    node.level(),
                    node.label_range().range().start().get(),
                    node.label_range().range().end().get(),
                )
            })
            .collect();

        let snapshot = document.snapshot();
        let outline_headings: Vec<(u8, u64, u64)> = outline
            .items()
            .iter()
            .map(|item| {
                let to_utf16 = |offset| {
                    snapshot
                        .utf16_offset(offset)
                        .expect("块内偏移必须换得出 UTF-16")
                        .get()
                };
                (
                    item.level(),
                    to_utf16(item.label_range().start()),
                    to_utf16(item.label_range().end()),
                )
            })
            .collect();

        assert_eq!(outline_headings, semantic_headings, "{source:?}");
    }
}

/// 大纲与 Revision 绑定，跟着编辑走。
#[test]
fn the_outline_is_bound_to_its_revision() {
    let mut document = EditorDocument::new("## b\n");
    let before = OutlineSnapshot::from_document(&document);
    assert_eq!(before.revision(), document.revision());
    assert_eq!(before.len(), 1);
    assert_eq!(before.items()[0].parent(), None);

    // 在开头插一个一级标题：原来那条二级标题要挂到它下面去。
    document
        .execute(EditorCommand::insert_text("# a\n\n"))
        .expect("插入该成功");

    let after = OutlineSnapshot::from_document(&document);
    assert_eq!(after.revision(), document.revision());
    assert_ne!(after.revision(), before.revision());
    assert_eq!(after.len(), 2);
    assert_eq!(after.items()[1].parent(), Some(0));
}

/// 没有标题就是空大纲，不是一条假的根。
#[test]
fn a_document_without_headings_has_an_empty_outline() {
    for source in ["", "段落\n", "- 项目\n", "```\n# 代码\n```\n"] {
        let (_, outline) = outline_of(source);
        assert!(outline.is_empty(), "{source:?}");
    }
}
