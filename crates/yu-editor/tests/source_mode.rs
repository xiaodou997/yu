use yu_core::ByteOffset;
use yu_core::{TextRange, Utf16Offset, Utf16Range};

#[test]
fn composition_blocks_mode_switch_until_commit_or_cancel() {
    let mut doc = EditorDocument::new("# 中文");
    doc.set_source_mode(true).expect("source mode");
    doc.begin_composition(
        TextRange::empty(ByteOffset::new(2)),
        "拼音",
        Utf16Range::empty(Utf16Offset::new(2)),
    )
    .expect("preedit");
    assert!(doc.set_source_mode(false).is_err());
    assert!(doc.source_mode());
    assert_eq!(doc.snapshot().as_str(), "# 中文");
    assert!(doc.cancel_composition());
    doc.set_source_mode(false).expect("switch after cancel");
    assert_eq!(doc.snapshot().as_str(), "# 中文");
}

#[test]
fn source_mode_converts_cell_ranges_to_text_selections() {
    let source = "| H | V |\n| --- | --- |\n| x | y |";
    let mut doc = EditorDocument::new(source);
    let at = ByteOffset::new(source.find('x').expect("cell") as u64);
    doc.execute(EditorCommand::SelectTableCells {
        anchor: at,
        focus: at,
    })
    .expect("select cell");
    let ranges = doc.selections().as_slice().to_vec();
    doc.set_source_mode(true).expect("source mode");
    assert_eq!(doc.selections().table_columns(), None);
    assert_eq!(doc.selections().as_slice(), ranges);
    doc.execute(EditorCommand::insert_text("|"))
        .expect("literal replacement");
    assert_eq!(doc.snapshot().as_str(), source.replace('x', "|"));
    doc.undo().expect("undo cell replacement");
    assert_eq!(doc.snapshot().as_str(), source);
    assert_eq!(doc.selections().table_columns(), None);
}

#[test]
fn outline_labels_stay_semantic_in_source_mode() {
    let mut doc = EditorDocument::new("# **bold** [link](url) &amp;\n");
    let labels = |doc: &mut EditorDocument| {
        yu_editor::OutlineTree::build(doc)
            .expect("outline")
            .rows()
            .iter()
            .map(|row| row.label().to_owned())
            .collect::<Vec<_>>()
    };
    let before = labels(&mut doc);
    doc.set_source_mode(true).expect("source mode");
    assert_eq!(labels(&mut doc), before);
}
use yu_editor::{CaretAffinity, EditorCommand, EditorDocument, EditorSelection, OutlineSnapshot};

#[test]
fn source_mode_is_lossless_and_keeps_selection_outline_and_history() {
    let source = "# 标题\r\n\r\n- **羽** &amp; `code`\r\n\r\n| a | b |\r\n| - | - |\r\n| x | y |\r\n\r\n```rust\r\nfn main() {}\r\n```\r\n";
    let mut doc = EditorDocument::new(source);
    let caret = EditorSelection::cursor(
        &doc.snapshot(),
        ByteOffset::new(2),
        CaretAffinity::Downstream,
    )
    .expect("caret");
    doc.set_selection(caret).expect("selection");
    let history = doc.history_stats();
    let revision = doc.revision();
    let headings = OutlineSnapshot::from_document(&doc).items().len();
    let old_snapshot = doc.capture_render_snapshot();
    doc.set_source_mode(true).expect("source mode");
    assert_eq!(
        doc.visual_text().expect("literal projection").text(),
        source
    );
    assert_eq!(doc.selection(), caret);
    assert_eq!(doc.revision(), revision);
    assert_eq!(doc.history_stats(), history);
    assert_eq!(OutlineSnapshot::from_document(&doc).items().len(), headings);
    assert!(!old_snapshot.can_reuse_layout_context(&doc));
    doc.execute(EditorCommand::insert_text("新")).expect("edit");
    let edited = doc.snapshot().as_str().to_owned();
    assert!(doc.source_mode());
    assert_eq!(doc.visual_text().expect("edited projection").text(), edited);
    doc.set_source_mode(false).expect("preview");
    assert_eq!(doc.snapshot().as_str(), edited);
    doc.undo().expect("undo across mode");
    assert_eq!(doc.snapshot().as_str(), source);
    doc.set_source_mode(true).expect("source mode");
    doc.redo().expect("redo across mode");
    assert_eq!(doc.snapshot().as_str(), edited);
    assert_eq!(doc.visual_text().expect("redo projection").text(), edited);
}

#[test]
fn source_mode_does_not_escape_typed_table_separators_or_continue_lists() {
    for source in ["| a | b |\n| - | - |\n| x | y |", "- item"] {
        let mut doc = EditorDocument::new(source);
        doc.set_source_mode(true).expect("source mode");
        let end = doc.snapshot().len_bytes();
        doc.set_selection(
            EditorSelection::cursor(&doc.snapshot(), end, CaretAffinity::Downstream)
                .expect("caret"),
        )
        .expect("selection");
        doc.execute(EditorCommand::insert_text("|"))
            .expect("literal pipe");
        doc.execute(EditorCommand::insert_newline())
            .expect("literal newline");
        assert_eq!(doc.snapshot().as_str(), format!("{source}|\n"));
        assert!(doc.source_mode());
    }
}
