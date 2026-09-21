use yu_core::ByteOffset;
use yu_editor::{CaretAffinity, EditorDocument, EditorSelection};

fn split(before: &str, after: &str) {
    let at = before.find('|').expect("source caret marker");
    let source = before.replace('|', "");
    let expected_at = after.find('|').expect("expected caret marker");
    let expected = after.replace('|', "");
    let mut doc = EditorDocument::new(&source);
    let selection = EditorSelection::cursor(
        &doc.snapshot(),
        ByteOffset::new(at as u64),
        CaretAffinity::Downstream,
    )
    .expect("editor operation");
    doc.set_selection(selection).expect("editor operation");
    doc.insert_newline().expect("editor operation");
    assert_eq!(doc.snapshot().as_str(), expected);
    assert_eq!(doc.selection().focus().get() as usize, expected_at);
    doc.undo().expect("editor operation");
    assert_eq!(doc.snapshot().as_str(), source);
    assert_eq!(doc.selection().focus(), selection.focus());
    assert_eq!(doc.selection().anchor(), selection.anchor());
    doc.redo().expect("editor operation");
    assert_eq!(doc.snapshot().as_str(), expected);
}

#[test]
fn enter_splits_cell_list_preserving_inline_and_nested_markup() {
    split(
        "<table><tr><td><ul><li>中|文🙂<ul><li>child</li></ul></li></ul></td></tr></table>",
        "<table><tr><td><ul><li>中</li><li>|文🙂<ul><li>child</li></ul></li></ul></td></tr></table>",
    );
    split(
        "<table><tr><td><ol start='3'><li><p><b>one|two</b></p></li></ol></td></tr></table>",
        "<table><tr><td><ol start='3'><li><p><b>one</b></p></li><li><p><b>|two</b></p></li></ol></td></tr></table>",
    );
    split(
        "<table><tr><td><ul><li>parent<ul><li>a|b</li></ul>tail</li></ul></td></tr></table>",
        "<table><tr><td><ul><li>parent<ul><li>a</li><li>|b</li></ul>tail</li></ul></td></tr></table>",
    );
}

#[test]
fn enter_does_not_duplicate_anchor_ids() {
    split(
        "<table><tr><td><ul><li id='item'><p id='para'><b id='bold'>a|b</b></p></li></ul></td></tr></table>",
        "<table><tr><td><ul><li id='item'><p id='para'><b id='bold'>a</b></p></li><li ><p ><b >|b</b></p></li></ul></td></tr></table>",
    );
}

#[test]
fn enter_at_content_boundaries_preserves_wrappers() {
    for (before, after) in [
        ("<b>|text</b>", "<b></b></li><li><b>|text</b>"),
        ("<b>text|</b>", "<b>text</b></li><li><b>|</b>"),
        ("text|<!--keep-->", "text</li><li>|<!--keep-->"),
    ] {
        split(
            &format!("<table><tr><td><ul><li>{before}</li></ul></td></tr></table>"),
            &format!("<table><tr><td><ul><li>{after}</li></ul></td></tr></table>"),
        );
    }
}

#[test]
fn source_mode_and_unknown_table_attributes_keep_literal_newlines() {
    for (attr, source_mode) in [("", true), (" class='unknown'", false)] {
        let source = format!("<table{attr}><tr><td><ul><li>ab</li></ul></td></tr></table>");
        let at = source.find(">ab<").expect("item content") + 2;
        let mut doc = EditorDocument::new(&source);
        doc.set_source_mode(source_mode).expect("editor operation");
        doc.set_selection(
            EditorSelection::cursor(
                &doc.snapshot(),
                ByteOffset::new(at as u64),
                CaretAffinity::Downstream,
            )
            .expect("editor operation"),
        )
        .expect("editor operation");
        doc.insert_newline().expect("editor operation");
        assert_eq!(doc.snapshot().as_str(), source.replace(">ab<", ">a\nb<"));
    }
}

#[test]
fn empty_item_enter_exits_and_keeps_following_numbers() {
    for (before, after) in [
        ("<ul><li>|</li></ul>", "<p>|</p>"),
        (
            "<ul><li>before</li><li>|</li></ul>",
            "<ul><li>before</li></ul><p>|</p>",
        ),
        (
            "<ol start='3'><li>before</li><li>|</li><li>after</li></ol>",
            "<ol start='3'><li>before</li></ol><p>|</p><ol start='5'><li>after</li></ol>",
        ),
        (
            "<ol><li>|</li><li>after</li></ol>",
            "<p>|</p><ol start=\"2\"><li>after</li></ol>",
        ),
        (
            "<ul><li><b>|</b><!--keep--></li></ul>",
            "<p><b>|</b><!--keep--></p>",
        ),
        ("<ul><li><p>|</p></li></ul>", "<p>|</p>"),
    ] {
        split(
            &format!("<table><tr><td>{before}</td><td>KEEP</td></tr></table>"),
            &format!("<table><tr><td>{after}</td><td>KEEP</td></tr></table>"),
        );
    }
}

#[test]
fn empty_nested_item_enter_outdents_and_preserves_following_content() {
    split(
        "<table><tr><td><ul><li>parent<ul><li>before</li><li>|</li><li>after</li></ul>tail</li></ul></td></tr></table>",
        "<table><tr><td><ul><li>parent<ul><li>before</li></ul></li><li>|<ul><li>after</li></ul>tail</li></ul></td></tr></table>",
    );
    split(
        "<table><tr><td><ul><li>parent<ul><li>|</li></ul></li></ul></td></tr></table>",
        "<table><tr><td><ul><li>parent</li><li>|</li></ul></td></tr></table>",
    );
}

#[test]
fn empty_exit_keeps_item_anchors_and_creates_geometry_for_empty_containers() {
    split(
        "<table><tr><td><ul><li id='anchor'>|</li></ul></td></tr></table>",
        "<table><tr><td><p id='anchor'>|</p></td></tr></table>",
    );
    split(
        "<table><tr><td><ul><li id='anchor'><p>|</p></li></ul></td></tr></table>",
        "<table><tr><td><div id='anchor'><p>|</p></div></td></tr></table>",
    );
    split(
        "<table><tr><td><ul><li><div>|</div></li></ul></td></tr></table>",
        "<table><tr><td><div></div><p>|</p></td></tr></table>",
    );
}

#[test]
fn split_ordered_list_keeps_one_container_id_and_adjusts_start() {
    split(
        "<table><tr><td><ol id='list' start='8'><li>A</li><li>|</li><li>B</li></ol></td></tr></table>",
        "<table><tr><td><ol id='list' start='8'><li>A</li></ol><p>|</p><ol  start='10'><li>B</li></ol></td></tr></table>",
    );
    split(
        "<table><tr><td><ol id='list' start='8'><li>|</li><li>B</li></ol></td></tr></table>",
        "<table><tr><td><p>|</p><ol id='list' start='9'><li>B</li></ol></td></tr></table>",
    );
}

#[test]
fn multiple_list_carets_split_atomically_and_restore_all_carets() {
    let source = "<table><tr><td><ul><li id='a'><b>中文🙂末</b></li><li>XY</li></ul></td><td><ol><li>AB</li></ol></td></tr></table>\r\n";
    let mut doc = EditorDocument::new(source);
    let offsets = [
        source.find("🙂").expect("emoji"),
        source.find("末").expect("last"),
        source.find("Y</li>").expect("Y"),
        source.find("B</li>").expect("B"),
    ];
    let selections = offsets.map(|at| {
        EditorSelection::cursor(
            &doc.snapshot(),
            ByteOffset::new(at as u64),
            CaretAffinity::Downstream,
        )
        .expect("cursor")
    });
    doc.set_selections(selections, 2).expect("multicarets");
    let before = doc.selections().clone();
    doc.insert_newline().expect("multiple list Enter");
    let expected = "<table><tr><td><ul><li id='a'><b>中文</b></li><li ><b>🙂</b></li><li ><b>末</b></li><li>X</li><li>Y</li></ul></td><td><ol><li>A</li><li>B</li></ol></td></tr></table>\r\n";
    assert_eq!(doc.snapshot().as_str(), expected);
    assert_eq!(doc.selections().len(), 4);
    assert_eq!(doc.selections().primary_index(), 2);
    for (selection, needle) in doc
        .selections()
        .as_slice()
        .iter()
        .zip(["🙂", "末", "Y</li>", "B</li>"])
    {
        assert_eq!(
            selection.focus().get() as usize,
            expected.find(needle).expect("new caret")
        );
    }
    let after = doc.selections().clone();
    doc.undo().expect("undo");
    assert_eq!(doc.snapshot().as_str(), source);
    for (actual, original) in doc.selections().as_slice().iter().zip(before.as_slice()) {
        assert_eq!(actual.focus(), original.focus());
    }
    doc.redo().expect("redo");
    assert_eq!(doc.snapshot().as_str(), expected);
    for (actual, original) in doc.selections().as_slice().iter().zip(after.as_slice()) {
        assert_eq!(actual.focus(), original.focus());
    }
}

#[test]
fn mixed_list_and_plain_carets_keep_individual_newline_rules() {
    let source =
        "<table><tr><td><ul><li>AB</li></ul></td><td>CD</td></tr></table>\r\n\r\n正文🙂尾\r\n";
    let expected = "<table><tr><td><ul><li>A</li><li>B</li></ul></td><td>C<br>D</td></tr></table>\r\n\r\n正文\r\n🙂尾\r\n";
    let mut doc = EditorDocument::new(source);
    let cursors = ["B</li>", "D</td>", "🙂"].map(|needle| {
        EditorSelection::cursor(
            &doc.snapshot(),
            ByteOffset::new(source.find(needle).expect("caret") as u64),
            CaretAffinity::Downstream,
        )
        .expect("cursor")
    });
    doc.set_selections(cursors, 1).expect("mixed carets");
    doc.insert_newline().expect("Enter");
    assert_eq!(doc.snapshot().as_str(), expected);
    assert_eq!(doc.selections().primary_index(), 1);
    for (cursor, needle) in doc
        .selections()
        .as_slice()
        .iter()
        .zip(["B</li>", "D</td>", "🙂"])
    {
        assert_eq!(
            cursor.focus().get() as usize,
            expected.find(needle).expect("result caret")
        );
    }
    doc.undo().expect("undo");
    assert_eq!(doc.snapshot().as_str(), source);
    doc.redo().expect("redo");
    assert_eq!(doc.snapshot().as_str(), expected);
}

#[test]
fn empty_list_exit_and_other_cell_split_share_one_undo() {
    let source = "<table><tr><td><ul><li>A</li><li></li><li>B</li></ul></td><td><ul><li>XY</li></ul></td></tr></table>\r\n";
    let expected = "<table><tr><td><ul><li>A</li></ul><p></p><ul><li>B</li></ul></td><td><ul><li>X</li><li>Y</li></ul></td></tr></table>\r\n";
    let mut doc = EditorDocument::new(source);
    let positions = [
        source.find("<li></li>").expect("empty") + 4,
        source.find("Y</li>").expect("split"),
    ];
    let cursors = positions.map(|at| {
        EditorSelection::cursor(
            &doc.snapshot(),
            ByteOffset::new(at as u64),
            CaretAffinity::Downstream,
        )
        .expect("cursor")
    });
    doc.set_selections(cursors, 1).expect("select");
    doc.insert_newline().expect("mixed empty Enter");
    assert_eq!(doc.snapshot().as_str(), expected);
    assert_eq!(
        doc.selections().as_slice()[0].focus().get() as usize,
        expected.find("</p>").expect("paragraph")
    );
    assert_eq!(
        doc.selections().as_slice()[1].focus().get() as usize,
        expected.find("Y</li>").expect("Y")
    );
    doc.undo().expect("undo");
    assert_eq!(doc.snapshot().as_str(), source);
    for (cursor, at) in doc.selections().as_slice().iter().zip(positions) {
        assert_eq!(cursor.focus().get() as usize, at);
    }
    doc.redo().expect("redo");
    assert_eq!(doc.snapshot().as_str(), expected);
    assert_eq!(doc.selections().primary_index(), 1);
}

#[test]
fn same_list_empty_exit_and_sibling_split_are_both_structural() {
    let source = "<table><tr><td><ul><li></li><li>XY</li></ul></td></tr></table>";
    let mut doc = EditorDocument::new(source);
    let cursors = [
        source.find("</li>").expect("empty"),
        source.find("Y</li>").expect("Y"),
    ]
    .map(|at| {
        EditorSelection::cursor(
            &doc.snapshot(),
            ByteOffset::new(at as u64),
            CaretAffinity::Downstream,
        )
        .expect("cursor")
    });
    doc.set_selections(cursors, 0).expect("select");
    doc.insert_newline().expect("overlap Enter");
    assert_eq!(
        doc.snapshot().as_str(),
        "<table><tr><td><p></p><ul><li>X</li><li>Y</li></ul></td></tr></table>"
    );
    doc.undo().expect("undo");
    assert_eq!(doc.snapshot().as_str(), source);
}

#[test]
fn empty_middle_item_exits_between_two_simultaneous_unicode_splits() {
    let source =
        "<table><tr><td><ul><li>中🙂</li><li></li><li>文末</li></ul></td></tr></table>\r\n";
    let expected = "<table><tr><td><ul><li>中</li><li>🙂</li></ul><p></p><ul><li>文</li><li>末</li></ul></td></tr></table>\r\n";
    let mut doc = EditorDocument::new(source);
    let positions = [
        source.find("🙂").expect("emoji"),
        source.find("<li></li>").expect("empty") + 4,
        source.find("末").expect("last"),
    ];
    let cursors = positions.map(|at| {
        EditorSelection::cursor(
            &doc.snapshot(),
            ByteOffset::new(at as u64),
            CaretAffinity::Downstream,
        )
        .expect("cursor")
    });
    doc.set_selections(cursors, 1).expect("select");
    doc.insert_newline().expect("combined structural Enter");
    assert_eq!(doc.snapshot().as_str(), expected);
    for (selection, needle) in doc.selections().as_slice().iter().zip(["🙂", "</p>", "末"]) {
        assert_eq!(
            selection.focus().get() as usize,
            expected.find(needle).expect("target")
        );
    }
    doc.undo().expect("undo");
    assert_eq!(doc.snapshot().as_str(), source);
    for (selection, at) in doc.selections().as_slice().iter().zip(positions) {
        assert_eq!(selection.focus().get() as usize, at);
    }
    doc.redo().expect("redo");
    assert_eq!(doc.snapshot().as_str(), expected);
    assert_eq!(doc.selections().primary_index(), 1);
}

#[test]
fn separated_empty_items_exit_without_renumbering_remaining_ordered_items() {
    let source = "<table><tr><td><ol start='4'><li>A</li><li></li><li>中文🙂</li><li></li><li>Z</li></ol></td></tr></table>\r\n";
    let expected = "<table><tr><td><ol start='4'><li>A</li></ol><p></p><ol start='6'><li>中文🙂</li></ol><p></p><ol start='8'><li>Z</li></ol></td></tr></table>\r\n";
    let mut doc = EditorDocument::new(source);
    let positions: Vec<_> = source
        .match_indices("<li></li>")
        .map(|(at, _)| at + 4)
        .collect();
    let cursors: Vec<_> = positions
        .iter()
        .map(|at| {
            EditorSelection::cursor(
                &doc.snapshot(),
                ByteOffset::new(*at as u64),
                CaretAffinity::Downstream,
            )
            .expect("cursor")
        })
        .collect();
    doc.set_selections(cursors, 1).expect("select empty items");
    doc.insert_newline().expect("exit both");
    assert_eq!(doc.snapshot().as_str(), expected);
    let targets: Vec<_> = expected.match_indices("</p>").map(|(at, _)| at).collect();
    for (cursor, at) in doc.selections().as_slice().iter().zip(&targets) {
        assert_eq!(cursor.focus().get() as usize, *at);
    }
    doc.undo().expect("undo both");
    assert_eq!(doc.snapshot().as_str(), source);
    for (cursor, at) in doc.selections().as_slice().iter().zip(&positions) {
        assert_eq!(cursor.focus().get() as usize, *at);
    }
    doc.redo().expect("redo both");
    assert_eq!(doc.snapshot().as_str(), expected);
    assert_eq!(doc.selections().primary_index(), 1);
    for (cursor, at) in doc.selections().as_slice().iter().zip(&targets) {
        assert_eq!(cursor.focus().get() as usize, *at);
    }
}

#[test]
fn adjacent_empty_items_exit_without_creating_an_empty_list_between_them() {
    let source = "<table><tr><td><ul><li>A</li><li></li><li></li><li>Z</li></ul></td></tr></table>";
    let mut doc = EditorDocument::new(source);
    let cursors: Vec<_> = source
        .match_indices("<li></li>")
        .map(|(at, _)| {
            EditorSelection::cursor(
                &doc.snapshot(),
                ByteOffset::new((at + 4) as u64),
                CaretAffinity::Downstream,
            )
            .expect("cursor")
        })
        .collect();
    doc.set_selections(cursors, 1).expect("select");
    doc.insert_newline().expect("exit adjacent items");
    assert_eq!(
        doc.snapshot().as_str(),
        "<table><tr><td><ul><li>A</li></ul><p></p><p></p><ul><li>Z</li></ul></td></tr></table>"
    );
    let expected = doc.snapshot().as_str().to_owned();
    let targets: Vec<_> = expected.match_indices("</p>").map(|(at, _)| at).collect();
    for (selection, at) in doc.selections().as_slice().iter().zip(&targets) {
        assert_eq!(selection.focus().get() as usize, *at);
    }
    doc.undo().expect("undo adjacent exits");
    assert_eq!(doc.snapshot().as_str(), source);
    doc.redo().expect("redo adjacent exits");
    assert_eq!(doc.snapshot().as_str(), expected);
    for (selection, at) in doc.selections().as_slice().iter().zip(&targets) {
        assert_eq!(selection.focus().get() as usize, *at);
    }
}

#[test]
fn consecutive_ordered_empty_items_keep_comments_and_all_caret_targets() {
    let source = "<table><tr><td><ol start='8'><li>A</li><li></li>\r\n<!--keep 中文🙂--><li></li>\r\n<li></li><li>Z</li></ol></td></tr></table>\r\n";
    let expected = "<table><tr><td><ol start='8'><li>A</li></ol><p></p>\r\n<!--keep 中文🙂--><p></p>\r\n<p></p><ol start='12'><li>Z</li></ol></td></tr></table>\r\n";
    let mut doc = EditorDocument::new(source);
    let positions: Vec<_> = source
        .match_indices("<li></li>")
        .map(|(at, _)| at + 4)
        .collect();
    let cursors: Vec<_> = positions
        .iter()
        .map(|at| {
            EditorSelection::cursor(
                &doc.snapshot(),
                ByteOffset::new(*at as u64),
                CaretAffinity::Downstream,
            )
            .expect("cursor")
        })
        .collect();
    doc.set_selections(cursors, 1).expect("select");
    doc.insert_newline().expect("exit all");
    assert_eq!(doc.snapshot().as_str(), expected);
    let targets: Vec<_> = expected.match_indices("</p>").map(|(at, _)| at).collect();
    for (cursor, at) in doc.selections().as_slice().iter().zip(&targets) {
        assert_eq!(cursor.focus().get() as usize, *at);
    }
    doc.undo().expect("undo all");
    assert_eq!(doc.snapshot().as_str(), source);
    for (cursor, at) in doc.selections().as_slice().iter().zip(&positions) {
        assert_eq!(cursor.focus().get() as usize, *at);
    }
    doc.redo().expect("redo all");
    assert_eq!(doc.snapshot().as_str(), expected);
    assert_eq!(doc.selections().primary_index(), 1);
    for (cursor, at) in doc.selections().as_slice().iter().zip(&targets) {
        assert_eq!(cursor.focus().get() as usize, *at);
    }
}
