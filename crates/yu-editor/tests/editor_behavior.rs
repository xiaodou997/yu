mod support;

use support::EditorScenario;
use yu_core::Revision;

#[test]
fn unicode_grapheme_delete_and_selection_replacement_are_readable() {
    let mut scenario = EditorScenario::new("e\u{301}🙂|x");
    scenario
        .backspace()
        .expect_state("e\u{301}|x")
        .backspace()
        .expect_state("|x");

    let mut selected = EditorScenario::new("hello ⟦世界⟧");
    selected.insert("Yu").expect_state("hello Yu|");
}

#[test]
fn composition_overlay_commits_once_and_cancel_is_zero_mutation() {
    let mut scenario = EditorScenario::new("输入: |");
    scenario
        .begin_composition("にほんご")
        .update_composition("日本語")
        .expect_composition("日本語")
        .expect_source("输入: ")
        .expect_revision(Revision::INITIAL)
        .commit_composition("日本語")
        .expect_state("输入: 日本語|")
        .expect_no_composition()
        .expect_revision(Revision::new(1));

    let mut cancelled = EditorScenario::new("输入: |");
    cancelled
        .begin_composition("にほんご")
        .update_composition("日本語")
        .cancel_composition()
        .expect_state("输入: |")
        .expect_no_composition()
        .expect_revision(Revision::INITIAL);
}

#[test]
fn markdown_list_commands_keep_source_backed_behavior() {
    let mut task = EditorScenario::new("- [x] done|");
    task.enter().expect_state("- [x] done\n- [ ] |");
    task.undo().expect_state("- [x] done|");
    task.redo().expect_state("- [x] done\n- [ ] |");

    let mut toggle = EditorScenario::new("- [ ] item|");
    toggle
        .toggle_task(0)
        .expect_state("- [x] item|")
        .undo()
        .expect_state("- [ ] item|");
}

#[test]
fn vertical_shift_selection_preserves_anchor_and_caret_direction() {
    let mut scenario = EditorScenario::new("|one\ntwo\nthree");
    scenario
        .shift_down()
        .expect_state("⟦one\n⟧two\nthree")
        .shift_down()
        .expect_state("⟦one\ntwo\n⟧three")
        .shift_up()
        .expect_state("⟦one\n⟧two\nthree")
        .shift_up()
        .expect_state("|one\ntwo\nthree");
}
