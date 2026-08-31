//! 多光标：一组选区在编辑器命令层的行为。
//!
//! # 判据落在哪
//!
//! - **「N 条 edit 真的都生效了」的判据是 canonical source**（`expect_source`），
//!   不是选区。两条路分开：选区是命令的输出，源码是 `TextBuffer` 的输出。
//! - **「另外几个光标也动了」的判据是 `expect_selections`**，不是
//!   `expect_state`——后者只看主选区，会让「除了 primary 谁都没动」静默通过。
//! - **不变式（有序、不重叠、至少一个）的判据在 `yu-state` 的单元用例上**，
//!   这里不重证一遍；这里证的是命令层有没有把不变式喂坏。

mod support;

use support::EditorScenario;
use yu_core::Revision;
use yu_editor::{EditorCommand, EditorDocument};

#[test]
fn typing_at_three_carets_edits_all_three() {
    let mut scenario = EditorScenario::from_source("a.b.c");
    scenario
        .set_carets(&[1, 3, 5], 0)
        .insert("X")
        .expect_source("aX.bX.cX")
        // 每一条落在自己插进去的那个字后面，不是全都跑到一处。
        .expect_selections(&[(2, 2), (5, 5), (8, 8)]);
}

/// **一条命令一个 Transaction。** 三个光标同时打一个字仍然只进一条 history
/// entry，所以一次 undo 要把三处一起收回去。
///
/// 反过来说，如果哪一天改成「一个光标一个 Transaction」，这条会红：那时一次
/// undo 只收回一处，源码变成 `aX.bX.c`——不报错，只是撤销撤不干净。
#[test]
fn one_keystroke_at_three_carets_is_one_undo_step() {
    let mut scenario = EditorScenario::from_source("a.b.c");
    scenario
        .set_carets(&[1, 3, 5], 0)
        .insert("X")
        .expect_source("aX.bX.cX")
        .undo()
        .expect_source("a.b.c")
        .redo()
        .expect_source("aX.bX.cX");
}

/// 连续输入仍然并进同一个 undo group——分组归 `HistoryGroup::Typing`，与光标
/// 有几个无关。
#[test]
fn consecutive_multi_caret_typing_is_one_undo_group() {
    let mut scenario = EditorScenario::from_source("a.b");
    scenario
        .set_carets(&[1, 3], 0)
        .insert("X")
        .insert("Y")
        .expect_source("aXY.bXY")
        .undo()
        .expect_source("a.b");
}

#[test]
fn backspace_at_three_carets_deletes_three_graphemes() {
    let mut scenario = EditorScenario::from_source("aX.bX.cX");
    scenario
        .set_carets(&[2, 5, 8], 0)
        .backspace()
        .expect_source("a.b.c")
        .expect_selections(&[(1, 1), (3, 3), (5, 5)]);
}

/// **文档开头的那个光标产出一个空区间，必须先滤掉。**
///
/// 不滤的表现不是「它自己删不动」——`yu_text::validate_edits` 会因为两个空
/// edit 落在同一偏移（或者一个越界的空 edit）拒掉**整条** Transaction，于是
/// 另外几个光标也删不动，而且不报错。
#[test]
fn a_caret_at_the_document_start_does_not_block_the_other_carets() {
    let mut scenario = EditorScenario::from_source("abc");
    scenario
        .set_carets(&[0, 2], 1)
        .backspace()
        .expect_source("ac")
        .expect_selections(&[(0, 0), (1, 1)]);
}

/// **空区间不进 Transaction，而「不进」的判据是 Revision。**
///
/// 一条零效果的 edit（`[0,0)` 换成 `""`）**照样推进 Revision**——`TextBuffer::apply`
/// 不认「这条 edit 什么都没改」。于是不滤掉的后果是：在文首按一次退格，文档变脏、
/// 压进一条什么都不做的 undo，而源码一个字节都没变。**不报错、不 panic**，
/// 上一条用例（只断源码与选区）完全看不见它。
///
/// 这个契约在多光标之前就有（`delete_range` 开头那句 `if range.is_empty()`），
/// **一直没有断言**。这是「常数/契约没有断言就等于没有约定」的又一个实例。
#[test]
fn a_command_that_changes_nothing_does_not_advance_the_revision() {
    let mut single = EditorScenario::from_source("abc");
    single
        .set_caret(0)
        .expect_revision(Revision::INITIAL)
        .backspace()
        .expect_source("abc")
        .expect_revision(Revision::INITIAL);

    let mut trailing = EditorScenario::from_source("abc");
    trailing
        .set_caret(3)
        .delete_forward()
        .expect_source("abc")
        .expect_revision(Revision::INITIAL);

    // 多光标下同样：文首那一根产出空区间，其余照删，而 Revision 只推进一次。
    let mut multiple = EditorScenario::from_source("abc");
    multiple
        .set_carets(&[0, 2], 1)
        .backspace()
        .expect_source("ac")
        .expect_revision(Revision::new(1));
}

/// **删掉两个光标之间的字，它们会落到同一个偏移。**
///
/// 收敛点是 `Selections::map_through`。不合并的话下一次插入被
/// `validate_edits` 拒掉，表现是「打字突然没反应」。
#[test]
fn carets_that_collide_after_a_deletion_merge_into_one() {
    let mut scenario = EditorScenario::from_source("a-b");
    scenario
        .set_carets(&[1, 2], 0)
        .delete_forward()
        .expect_source("a")
        .expect_selections(&[(1, 1)]);
}

/// 相邻的两段**非空**选区不能被并掉：`aa` 在 `aaaa` 里的两处匹配就是这个
/// 形状，并掉等于把「选中全部匹配」变成「全选」。
#[test]
fn adjacent_non_empty_selections_stay_separate_through_a_command() {
    let mut scenario = EditorScenario::from_source("aaaa");
    scenario
        .set_selections(&[(0, 2), (2, 4)], 0)
        .expect_selections(&[(0, 2), (2, 4)])
        .insert("Z")
        .expect_source("ZZ")
        .expect_selections(&[(1, 1), (2, 2)]);
}

#[test]
fn horizontal_movement_moves_every_caret() {
    let mut scenario = EditorScenario::from_source("abcdef");
    scenario
        .set_carets(&[1, 4], 0)
        .right()
        .expect_selections(&[(2, 2), (5, 5)])
        .left()
        .left()
        .expect_selections(&[(0, 0), (3, 3)]);
}

/// 非空选区先塌到那一头——每一条各塌各的，不是只塌 primary。
#[test]
fn horizontal_movement_collapses_every_selection() {
    let mut scenario = EditorScenario::from_source("abcdef");
    scenario
        .set_selections(&[(0, 2), (3, 5)], 0)
        .left()
        .expect_selections(&[(0, 0), (3, 3)]);

    let mut forward = EditorScenario::from_source("abcdef");
    forward
        .set_selections(&[(0, 2), (3, 5)], 0)
        .right()
        .expect_selections(&[(2, 2), (5, 5)]);
}

#[test]
fn word_movement_moves_every_caret() {
    let mut scenario = EditorScenario::from_source("alpha beta gamma");
    scenario
        .set_carets(&[5, 10], 0)
        .word_left()
        .expect_selections(&[(0, 0), (6, 6)])
        .word_right()
        .expect_selections(&[(5, 5), (10, 10)]);
}

/// 纵向移动：两个光标各走各的一行。
#[test]
fn vertical_movement_moves_every_caret() {
    let mut scenario = EditorScenario::from_source("aaaa\nbbbb\ncccc\n");
    scenario
        .set_carets(&[1, 6], 0)
        .down()
        .expect_selections(&[(6, 6), (11, 11)]);
}

/// **走不动的那一条留在原地，不能把别的光标一起拖住。**
///
/// 把「一条走不动就整条命令返回 false」留着的表现是：最后一行上有一个光标时，
/// 按 ↓ 谁都不动——不报错。
#[test]
fn a_caret_that_cannot_move_down_does_not_block_the_others() {
    let mut scenario = EditorScenario::from_source("aaaa\nbbbb");
    scenario
        .set_carets(&[1, 6], 0)
        .down()
        // 第二个光标已经在最后一行，留在原地；第一个照走。
        .expect_selections(&[(6, 6)]);
}

/// **IME 组字塌回一条。** `CompositionOverlay` 是一个 preedit 覆盖一个区间，
/// 留着 N 条选区会在屏幕上留下几根不动的假光标，提交之后还会被映射到莫名其
/// 妙的位置——不报错、不 panic。降级的理由与还债条件写在
/// `EditorDocument::begin_composition` 上。
#[test]
fn beginning_a_composition_collapses_to_the_primary_caret() {
    let mut scenario = EditorScenario::from_source("a.b.c");
    scenario.set_carets(&[1, 3, 5], 1);
    assert_eq!(scenario.document().selections().len(), 3);

    scenario.begin_composition("にほんご");
    assert_eq!(
        scenario.document().selections().len(),
        1,
        "组字期间必须只剩一条选区"
    );
    assert_eq!(
        scenario.selection_ranges(),
        vec![(3, 3)],
        "留下的必须是 primary 那一条"
    );

    scenario
        .commit_composition("日本語")
        .expect_source("a.b日本語.c");
}

/// **N>1 时 Enter 不续列表。** 这是一笔登记在案的降级（理由在
/// `insert_plain_newlines` 上：列表类编辑改的是整行，两个光标可以停在同一
/// 行上）。写这条断言是为了让「顺手把它改回去」变红——降级要写在代码上，
/// 也要写在用例上。
#[test]
fn multi_caret_enter_inserts_plain_newlines_without_continuing_the_list() {
    let mut single = EditorScenario::from_source("- one\n- two");
    single
        .set_caret(5)
        .enter()
        .expect_source("- one\n- \n- two");

    let mut multiple = EditorScenario::from_source("- one\n- two");
    multiple
        .set_carets(&[5, 11], 0)
        .enter()
        // 两处都只插了换行，没有续出 `- ` 前缀。
        .expect_source("- one\n\n- two\n")
        .expect_selections(&[(6, 6), (13, 13)]);
}

/// **命令可用性的判据是「有没有哪一条动得了」，不是 primary 动不动得了。**
///
/// 按 primary 判的表现：primary 停在文档开头时退格菜单项整个变灰，而另外那个
/// 光标明明删得动。
#[test]
fn a_command_is_available_when_any_selection_can_run_it() {
    let mut document = EditorDocument::new("abc");
    let snapshot = document.snapshot();
    let carets: Vec<_> = [0_u64, 2]
        .iter()
        .map(|offset| {
            yu_editor::EditorSelection::cursor(
                &snapshot,
                yu_core::ByteOffset::new(*offset),
                yu_editor::CaretAffinity::Downstream,
            )
            .expect("valid caret")
        })
        .collect();
    // primary 是开头那一个，它自己退不动。
    document
        .set_selections(carets, 0)
        .expect("selections should normalize");

    assert!(
        document.command_available(&EditorCommand::DeleteBackward),
        "另一个光标删得动，退格就该是可用的"
    );
}

/// 编辑之后 revision 只推进一次，无论有几个光标。
#[test]
fn one_multi_caret_command_advances_the_revision_once() {
    let mut scenario = EditorScenario::from_source("a.b.c");
    scenario
        .set_carets(&[1, 3, 5], 0)
        .expect_revision(Revision::INITIAL)
        .insert("X")
        .expect_revision(Revision::new(1));
}
