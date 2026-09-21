use yu_core::{ByteOffset, TextRange, Utf16Offset, Utf16Range};
use yu_editor::{EditorDocument, EditorDocumentError};

fn range(source: &str, word: &str) -> TextRange {
    let start = source.find(word).expect("fixture");
    TextRange::new(
        ByteOffset::new(start as u64),
        ByteOffset::new((start + word.len()) as u64),
    )
    .expect("range")
}

#[test]
fn spelling_replacement_is_an_isolated_undo_and_preserves_surrounding_source() {
    let original = "# 中文\r\n\r\n**wrld** and [labl](https://exampel.com)\r\n";
    let mut doc = EditorDocument::new(original);
    doc.replace_spelling(range(original, "wrld"), "world")
        .expect("replace");
    let corrected = original.replacen("wrld", "world", 1);
    assert_eq!(doc.snapshot().as_str(), corrected);
    doc.replace_spelling(range(&corrected, "labl"), "label")
        .expect("label");
    doc.undo().expect("undo label");
    assert_eq!(doc.snapshot().as_str(), corrected);
    doc.undo().expect("undo word");
    assert_eq!(doc.snapshot().as_str(), original);
    doc.redo().expect("redo");
    assert_eq!(doc.snapshot().as_str(), corrected);
}

#[test]
fn spelling_rejects_code_addresses_markup_and_preedit_without_source_changes() {
    let original = "wrld `codde` <https://exampel.com>";
    let mut doc = EditorDocument::new(original);
    for word in ["codde", "exampel"] {
        assert!(
            doc.replace_spelling(range(original, word), "correct")
                .is_err()
        );
    }
    doc.begin_composition(
        range(original, "wrld"),
        "你好",
        Utf16Range::new(Utf16Offset::new(2), Utf16Offset::new(2)).expect("IME selection"),
    )
    .expect("composition");
    assert!(matches!(
        doc.replace_spelling(range(original, "wrld"), "world"),
        Err(EditorDocumentError::CompositionActive)
    ));
    assert_eq!(doc.snapshot().as_str(), original);
}

#[test]
fn diagnostics_are_revision_bound_presentation_and_snapshot_state() {
    let original = "wrld and codde";
    let mut doc = EditorDocument::new(original);
    let revision = doc.revision();
    let diagnostic = range(original, "wrld");
    let mut worker = doc.capture_render_snapshot().into_layout_context();
    doc.set_spelling_diagnostics(revision, vec![diagnostic])
        .expect("publish");
    let generation = doc.spelling_generation();
    assert_eq!(doc.revision(), revision);
    assert_eq!(doc.snapshot().as_str(), original);
    let snapshot = doc.capture_render_snapshot();
    assert!(snapshot.can_reuse_layout_context(&worker));
    snapshot.merge_into_layout_context(&mut worker);
    assert_eq!(worker.spelling_diagnostics(), &[diagnostic]);
    doc.set_spelling_diagnostics(revision, vec![diagnostic, diagnostic])
        .expect("dedup");
    assert_eq!(doc.spelling_generation(), generation);
    doc.replace_spelling(diagnostic, "world").expect("edit");
    assert!(doc.spelling_diagnostics().is_empty());
    assert!(
        doc.set_spelling_diagnostics(revision, vec![diagnostic])
            .is_err()
    );
    assert_eq!(
        worker.spelling_diagnostics(),
        &[diagnostic],
        "immutable worker input remains bound to its own revision"
    );
}
