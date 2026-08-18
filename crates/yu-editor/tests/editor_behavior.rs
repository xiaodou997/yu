mod support;

use support::EditorScenario;
use yu_core::{ByteOffset, Revision, TextRange, Utf16Offset, Utf16Range};
use yu_editor::{CompositionError, EditorCommand, EditorDocument, EditorDocumentError};

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
fn zwj_graphemes_and_backward_selections_follow_source_boundaries() {
    let mut family = EditorScenario::new("e\u{301}👨‍👩‍👧‍👦|x");
    family
        .backspace()
        .expect_state("e\u{301}|x")
        .backspace()
        .expect_state("|x");

    let mut backward = EditorScenario::new("hello ⟧世界⟦");
    backward.insert("Yu").expect_state("hello Yu|");
}

#[test]
fn consecutive_deletions_are_one_undo_group() {
    let mut scenario = EditorScenario::new("abc|");
    scenario
        .backspace()
        .backspace()
        .expect_state("a|")
        .undo()
        .expect_state("abc|")
        .redo()
        .expect_state("a|");
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
fn composition_commit_is_separate_from_typing_and_is_undoable() {
    let mut scenario = EditorScenario::new("前|");
    scenario
        .insert("a")
        .begin_composition("🙂")
        .update_composition("日本語")
        .commit_composition("日本語")
        .expect_state("前a日本語|")
        .undo()
        .expect_state("前a|")
        .undo()
        .expect_state("前|")
        .redo()
        .expect_state("前a|")
        .redo()
        .expect_state("前a日本語|");
}

#[test]
fn composition_rejects_split_utf16_and_permanent_commands() {
    let mut document = EditorDocument::new("🙂");
    let end = ByteOffset::new("🙂".len() as u64);
    let replacement = TextRange::empty(end);
    let split = Utf16Range::empty(Utf16Offset::new(1));
    let error = document
        .begin_composition(replacement, "🙂", split)
        .expect_err("an IME caret cannot split an astral scalar");
    assert!(matches!(
        error,
        EditorDocumentError::Composition(CompositionError::SplitSurrogatePair(_))
    ));
    assert!(document.composition().is_none());
    assert_eq!(document.revision(), Revision::INITIAL);

    document
        .begin_composition(replacement, "🙂", Utf16Range::empty(Utf16Offset::new(2)))
        .expect("a boundary-aligned composition should begin");
    let before = document.snapshot().as_str().to_owned();
    let result = document.execute(EditorCommand::DeleteBackward);
    assert_eq!(result, Err(EditorDocumentError::CompositionActive));
    assert_eq!(document.snapshot().as_str(), before);
    assert_eq!(document.revision(), Revision::INITIAL);
    assert_eq!(document.history_stats().undo_entries(), 0);
    assert!(document.composition().is_some());
}

#[test]
fn invalid_composition_update_keeps_the_previous_overlay() {
    let mut document = EditorDocument::new("text");
    let replacement = TextRange::empty(ByteOffset::new(4));
    document
        .begin_composition(replacement, "🙂", Utf16Range::empty(Utf16Offset::new(2)))
        .expect("composition should begin");
    let before = document.composition().cloned().expect("overlay exists");
    let error = document
        .update_composition("🙂", Utf16Range::empty(Utf16Offset::new(1)))
        .expect_err("split UTF-16 selection must be rejected");
    assert!(matches!(
        error,
        EditorDocumentError::Composition(CompositionError::SplitSurrogatePair(_))
    ));
    assert_eq!(document.composition(), Some(&before));
    assert_eq!(document.snapshot().as_str(), "text");
    assert_eq!(document.revision(), Revision::INITIAL);
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
