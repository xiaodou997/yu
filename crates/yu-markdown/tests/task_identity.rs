//! `block_sequence` 与语法树对任务项的两份判断，锁在一起。
//!
//! # 为什么需要这条差分
//!
//! 第九刀把 GFM 的 TaskList 移进了 `yu-syntax`，但 `block_sequence` 那一份
//! 判断**还在**。产品链路上两份都在用：
//!
//! - 隐藏 `[x]` 的是 `extension/task.rs`——**块的身份**问
//!   `BlockKind::TaskListItem`，**标记区间**问树的 `TaskMarker`；
//! - 画复选框的是 `yu-workspace`——两样都问 `block_sequence`
//!   （`BlockKind::TaskListItem` 与 `yu_markdown::task_marker`）。
//!
//! 于是「哪三个字节是复选框」有两个实现，而它们不一致的样子正好是这个项目
//! 最怕的那一类：藏起来的区间与画上去的盒子对不齐，`[x]` 会露出半个，或者
//! 画面上凭空少三个字符。都不 panic、不报错。
//!
//! # 这两条路真的分开吗
//!
//! 分开。`yu-syntax` 的 `starts_task` 判在块解析的 leaf block 上，读的是已经
//! 剥掉容器标记的 `LeafBlock::content`；`parse_task_marker` 判在
//! `block_sequence` 定出来的块的**第一行**上，自己认列表标记、自己数缩进。
//! 两份代码没有共用函数，差分因此不是自证的。
//!
//! # 已登记的不一致：`block_sequence` 不下降到容器里
//!
//! `> - [x] q` 在 `block_sequence` 眼里是一个 `BlockQuote`，在树里是
//! `Blockquote > BulletList > ListItem > Task`。这不是新出现的：v1 就是这样，
//! 引用块里的任务项从来没有复选框。`extension/task.rs` 的定义域按
//! `BlockKind` 取，所以那种块一条装饰都不产——[`a_task_marker_outside_a_task_block_is_left_alone`]
//! 直接压着这个结论。
//!
//! 反过来的方向必须严格成立：**`block_sequence` 说是任务项时，树必须给出
//! 同一段标记**。破了它，`task.rs` 会一个字节都不藏，而复选框照画。
//!
//! # 合并之后这个文件怎么办
//!
//! 删掉。它守的是一段过渡期——`block_sequence` 与语法树的块结构合并之后，
//! 「同一个问题两个实现」这件事本身就没有了。

use yu_core::TextRange;
use yu_markdown::{BlockDecorations, BlockKind, ExtensionSet, TaskState, parse, task_marker};
use yu_syntax::{NodeKind, Tree, parse as parse_syntax};
use yu_text::TextBuffer;

/// 语料。前半段是写得好好的任务项，后半段专挑两边**可能**分道扬镳的地方：
/// 标记后没有内容、列表项的第二个内容块、近似形状、容器嵌套。
const CORPUS: &[&str] = &[
    "- [ ] 待办\n",
    "- [x] 完成\n",
    "- [X] 大写\n",
    "* [ ] 星号\n",
    "+ [ ] 加号\n",
    "1. [ ] 有序\n",
    "1) [x] 圆括号\n",
    "  - [ ] 缩进三格以内\n",
    "- [ ] 一\n- [x] 二\n- 三\n",
    "- [ ] 外层\n  - [x] 内层\n",
    "> - [x] 引用块里\n",
    "- [ ] 带 *强调* 与 [链接](/u)\n",
    // 标记后什么都没有。
    "- [x]\n",
    "- [ ]\n",
    "- [x]\n  续行\n",
    // 列表项的第二个内容块。
    "- 先有内容\n\n  [ ] 这不是任务项\n",
    "- 先有内容\n\n  [x] 这也不是\n",
    // 近似形状。
    "- [x]紧贴着\n",
    "- [y] 不是状态字符\n",
    "- [] 空的\n",
    "- [ x] 多一个空格\n",
    "[ ] 不在列表里\n",
    "- > [ ] 引用块把它挡住了\n",
    // 空白与制表符。
    "-   [ ]   多空格\n",
    "- [ ]\t制表符\n",
    // 混在别的语法中间。
    "# 标题\n\n- [x] 完成\n\n> 引用\n",
    "```\n- [ ] 代码块里的不算\n```\n",
    "    - [ ] 缩进代码块里的也不算\n",
    "",
];

/// 块里第一个 `TaskMarker` 的绝对区间。
///
/// 「块里」按完整包含算，与 `BlockContext::nodes` 同一个口径：半个落在块外
/// 的节点不算这个块的。
fn tree_marker(tree: &Tree, range: TextRange) -> Option<(u64, u64)> {
    fn walk(tree: &Tree, from: u32, range: TextRange, found: &mut Option<(u64, u64)>) {
        let to = from.saturating_add(tree.len_bytes());
        if found.is_none()
            && tree.kind() == NodeKind::TaskMarker
            && range.start().get() <= u64::from(from)
            && u64::from(to) <= range.end().get()
        {
            *found = Some((u64::from(from), u64::from(to)));
        }
        for index in 0..tree.child_count() {
            if let Some((child, offset)) = tree.child(index) {
                walk(child, from.saturating_add(offset), range, found);
            }
        }
    }
    let mut found = None;
    walk(tree, 0, range, &mut found);
    found
}

/// 一份文档逐块跑一遍：`(块, block_sequence 的标记, 树的标记, 装饰)`。
type Row = (
    yu_markdown::Block,
    Option<(u64, u64)>,
    Option<(u64, u64)>,
    BlockDecorations,
);

fn rows(source: &str) -> Vec<Row> {
    let buffer = TextBuffer::new(source.to_owned());
    let snapshot = buffer.snapshot();
    let document = parse(&snapshot);
    let tree = parse_syntax(&snapshot).expect("语料很短").into_tree();
    let extensions = ExtensionSet::markdown();
    document
        .blocks()
        .iter()
        .map(|block| {
            let from_sequence = task_marker(&snapshot, block)
                .map(|marker| (marker.range().start().get(), marker.range().end().get()));
            let from_tree = tree_marker(&tree, block.range());
            let decorations = extensions
                .decorate(&snapshot, &tree, block, None)
                .expect("装饰产出不该失败");
            (block, from_sequence, from_tree, decorations)
        })
        .collect()
}

/// 一段 source 是不是被藏了个干净。
fn is_hidden(decorations: &BlockDecorations, (from, to): (u64, u64)) -> bool {
    decorations.set().all().iter().any(|entry| {
        entry.decoration.hides_source()
            && entry.range.start().get() <= from
            && to <= entry.range.end().get()
    })
}

/// `block_sequence` 说是任务项时，树必须在同一个位置给出同一段标记。
///
/// 这是 `extension/task.rs` 赖以成立的那一条：它的定义域来自
/// `BlockKind`，区间来自树。两边错开一个字节，藏起来的与画上去的就对不齐。
#[test]
fn a_task_list_item_always_has_its_marker_in_the_tree() {
    for source in CORPUS {
        for (block, from_sequence, from_tree, decorations) in rows(source) {
            let Some(expected) = from_sequence else {
                continue;
            };
            assert_eq!(
                Some(expected),
                from_tree,
                "{source:?} 的块 {:?}（{:?}）：block_sequence 找到了标记，树没有",
                block.range(),
                block.kind()
            );
            assert!(
                is_hidden(&decorations, expected),
                "{source:?} 的块 {:?}：复选框会被画出来，而 {expected:?} 没有被藏起来",
                block.range()
            );
        }
    }
}

/// 树里的任务标记落在一个 `block_sequence` 不认的块上时，一条装饰都不许产。
///
/// 唯一走到这里的形状是容器嵌套（`> - [x] q`）：`block_sequence` 不下降到
/// 引用块里，那种任务项从 v1 起就没有复选框。定义域按 `BlockKind` 取，所以
/// `[x]` 原样留着——**要么两样都有，要么两样都没有**，不许只藏不画。
#[test]
fn a_task_marker_outside_a_task_block_is_left_alone() {
    let mut seen = 0_usize;
    for source in CORPUS {
        for (block, from_sequence, from_tree, decorations) in rows(source) {
            let (Some(marker), None) = (from_tree, from_sequence) else {
                continue;
            };
            seen += 1;
            assert!(
                !matches!(block.kind(), BlockKind::TaskListItem { .. }),
                "{source:?} 的块 {:?} 是任务项却问不出标记",
                block.range()
            );
            assert!(
                !is_hidden(&decorations, marker),
                "{source:?} 的块 {:?}（{:?}）藏了 {marker:?}，而没有复选框会画在那里",
                block.range(),
                block.kind()
            );
        }
    }
    assert!(
        seen > 0,
        "语料里没有一个「树认、block_sequence 不认」的块，\
         这条用例什么都没验到——`> - [x] q` 那条被删掉了吗？"
    );
}

/// 勾没勾上也要一致：一边画勾、另一边不画，同样不报错。
#[test]
fn both_parsers_agree_on_whether_the_box_is_checked() {
    let buffer_of = |source: &str| TextBuffer::new(source.to_owned());
    for source in CORPUS {
        let buffer = buffer_of(source);
        let snapshot = buffer.snapshot();
        let text = snapshot.as_str().as_bytes();
        for (block, from_sequence, _, _) in rows(source) {
            let BlockKind::TaskListItem { state, .. } = block.kind() else {
                continue;
            };
            let (from, _) = from_sequence.expect("任务项一定问得出标记");
            let from_tree = if text[usize::try_from(from + 1).expect("偏移")] == b' ' {
                TaskState::Todo
            } else {
                TaskState::Done
            };
            assert_eq!(state, from_tree, "{source:?} 的勾选状态两边不一致");
        }
    }
}
