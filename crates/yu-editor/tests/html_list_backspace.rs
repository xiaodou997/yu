use yu_core::ByteOffset;
use yu_editor::{CaretAffinity, EditorCommand, EditorDocument, EditorSelection};

fn backspace(before: &str, after: &str) {
    let before = format!("<table><tr><td>{before}</td><td>KEEP</td></tr></table>\r\n");
    let after = format!("<table><tr><td>{after}</td><td>KEEP</td></tr></table>\r\n");
    let at = before.find('|').expect("caret");
    let expected_at = after.find('|').expect("new caret");
    let source = before.replace('|', "");
    let expected = after.replace('|', "");
    let mut doc = EditorDocument::new(&source);
    doc.set_selection(
        EditorSelection::cursor(
            &doc.snapshot(),
            ByteOffset::new(at as u64),
            CaretAffinity::Downstream,
        )
        .expect("cursor"),
    )
    .expect("select");
    doc.execute(EditorCommand::DeleteBackward)
        .expect("backspace");
    assert_eq!(doc.snapshot().as_str(), expected);
    assert_eq!(doc.selection().focus().get() as usize, expected_at);
    assert_eq!(doc.table_target_is_html(), Some(true));
    doc.undo().expect("undo");
    assert_eq!(doc.snapshot().as_str(), source);
    assert_eq!(doc.selection().focus().get() as usize, at);
    doc.redo().expect("redo");
    assert_eq!(doc.snapshot().as_str(), expected);
}

#[test]
fn backspace_at_item_start_outdents_without_deleting_previous_text() {
    backspace(
        "<ul><li>KEEP PREVIOUS</li><li>|中文</li></ul>",
        "<ul><li>KEEP PREVIOUS</li></ul><p>|中文</p>",
    );
    backspace(
        "<ul><li>parent<ul><li><p><b>|中文</b></p></li><li>tail</li></ul></li></ul>",
        "<ul><li>parent</li><li><p><b>|中文</b></p><ul><li>tail</li></ul></li></ul>",
    );
    backspace(
        "<ol start='7'><li><!--keep--><b>|</b></li><li>after</li></ol>",
        "<p><!--keep--><b>|</b></p><ol start='8'><li>after</li></ol>",
    );
}

#[test]
fn backspace_inside_item_remains_grapheme_deletion() {
    backspace("<ul><li>中🙂|文</li></ul>", "<ul><li>中|文</li></ul>");
    backspace(
        "<ul><li><p>before</p><p>|after</p></li></ul>",
        "<ul><li><p>before|after</p></li></ul>",
    );
}

#[test]
fn preceding_break_or_empty_paragraph_is_deleted_before_list_level() {
    backspace("<ul><li><br>|text</li></ul>", "<ul><li>|text</li></ul>");
    backspace(
        "<ul><li><p></p><p>|text</p></li></ul>",
        "<ul><li><p>|text</p></li></ul>",
    );
}
