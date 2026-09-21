use yu_core::ByteOffset;
use yu_editor::{CaretAffinity, EditorCommand, EditorDocument, EditorSelection};

fn check(before: &str, after: &str, indent: bool) {
    let before = format!("<table><tr><td>{before}</td><td>KEEP</td></tr></table>\r\n");
    let after = format!("<table><tr><td>{after}</td><td>KEEP</td></tr></table>\r\n");
    let at = before.find('|').expect("caret");
    let expected_at = after.find('|').expect("result caret");
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
    assert!(doc.command_available(&if indent {
        EditorCommand::IndentList
    } else {
        EditorCommand::OutdentList
    }));
    if indent {
        doc.indent_list().expect("indent");
    } else {
        doc.outdent_list().expect("outdent");
    }
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
fn indent_moves_whole_item_with_formatting_and_descendants() {
    check(
        "<ul><li>A</li><li id='b'><b>中|文</b><ul><li>child</li></ul></li><li>C</li></ul>",
        "<ul><li>A<ul><li id='b'><b>中|文</b><ul><li>child</li></ul></li></ul></li><li>C</li></ul>",
        true,
    );
    check(
        "<ol start='4'><li>A</li>\r\n<!--keep--><li>B|</li></ol>",
        "<ol start='4'><li>A<ol>\r\n<!--keep--><li>B|</li></ol></li></ol>",
        true,
    );
}

#[test]
fn indent_reuses_previous_items_trailing_matching_list() {
    check(
        "<ul><li>A<ul><li>child</li></ul> </li><li>B|</li></ul>",
        "<ul><li>A<ul><li>child</li><li>B|</li></ul> </li></ul>",
        true,
    );
}

#[test]
fn outdent_keeps_body_children_following_siblings_and_parent_tail() {
    check(
        "<ul><li>A<ol start='3'><li>before</li><li id='b'><b>B|</b><ul><li>child</li></ul></li><li>after</li></ol>tail</li></ul>",
        "<ul><li>A<ol start='3'><li>before</li></ol></li><li id='b'><b>B|</b><ul><li>child</li></ul><ol start='5'><li>after</li></ol>tail</li></ul>",
        false,
    );
}

#[test]
fn root_outdent_preserves_rich_content_and_following_number() {
    check(
        "<ol start='3'><li>A</li><li id='b'><p>B|</p><ul><li>child</li></ul></li><li>C</li></ol>",
        "<ol start='3'><li>A</li></ol><div id='b'><p>B|</p><ul><li>child</li></ul></div><ol start='5'><li>C</li></ol>",
        false,
    );
}

#[test]
fn root_outdent_does_not_wrap_nested_list_in_paragraph() {
    check(
        "<ul><li>B|<ul><li>child</li></ul></li></ul>",
        "B|<ul><li>child</li></ul>",
        false,
    );
}

#[test]
fn first_item_cannot_indent_and_source_mode_keeps_html_literal() {
    let source = "<table><tr><td><ul><li>First</li><li>Second</li></ul></td></tr></table>";
    for (label, source_mode) in [("First", false), ("Second", true)] {
        let mut doc = EditorDocument::new(source);
        doc.set_source_mode(source_mode).expect("mode");
        let at = source.find(label).expect("label");
        doc.set_selection(
            EditorSelection::cursor(
                &doc.snapshot(),
                ByteOffset::new(at as u64),
                CaretAffinity::Downstream,
            )
            .expect("cursor"),
        )
        .expect("select");
        assert!(!doc.command_available(&EditorCommand::IndentList));
        doc.indent_list().expect("no-op");
        assert_eq!(doc.snapshot().as_str(), source);
        assert_eq!(doc.history_stats().undo_entries(), 0);
    }
}

#[test]
fn selected_sibling_items_indent_together_and_keep_selection_direction() {
    let source = "<table><tr><td><ul><li>A</li><li id='b'>中文</li><!--gap--><li>C🙂</li><li>D</li></ul></td></tr></table>\r\n";
    let expected = "<table><tr><td><ul><li>A<ul><li id='b'>中文</li><!--gap--><li>C🙂</li></ul></li><li>D</li></ul></td></tr></table>\r\n";
    for backward in [false, true] {
        let mut doc = EditorDocument::new(source);
        let start = source.find("中文").expect("Chinese selection start");
        let end = source.find("🙂").expect("emoji selection end") + "🙂".len();
        let (anchor, focus) = if backward { (end, start) } else { (start, end) };
        doc.set_selection(
            EditorSelection::range(
                &doc.snapshot(),
                ByteOffset::new(anchor as u64),
                ByteOffset::new(focus as u64),
                CaretAffinity::Downstream,
            )
            .expect("valid selected range"),
        )
        .expect("valid selected range");
        assert!(doc.command_available(&EditorCommand::IndentList));
        let result = doc.indent_list().expect("indent selected items");
        assert_eq!(result.selection(), doc.selection());
        assert_eq!(doc.snapshot().as_str(), expected);
        let new_start = expected.find("中文").expect("Chinese selection start");
        let new_end = expected.find("🙂").expect("emoji selection end") + "🙂".len();
        assert_eq!(
            doc.selection().anchor().get() as usize,
            if backward { new_end } else { new_start }
        );
        assert_eq!(
            doc.selection().focus().get() as usize,
            if backward { new_start } else { new_end }
        );
        doc.undo().expect("undo selected indent");
        assert_eq!(doc.snapshot().as_str(), source);
        assert_eq!(doc.selection().anchor().get() as usize, anchor);
        assert_eq!(doc.selection().focus().get() as usize, focus);
        doc.redo().expect("redo selected indent");
        assert_eq!(doc.snapshot().as_str(), expected);
        assert_eq!(
            doc.selection().anchor().get() as usize,
            if backward { new_end } else { new_start }
        );
    }
}

#[test]
fn selected_nested_items_outdent_as_one_transaction() {
    let source = "<table><tr><td><ul><li>P<ol start='3'><li>A</li><li id='b'>中文</li><!--gap--><li>C🙂</li><li>D</li></ol>tail</li><li>Q</li></ul></td></tr></table>\r\n";
    let expected = "<table><tr><td><ul><li>P<ol start='3'><li>A</li></ol></li><li id='b'>中文</li><!--gap--><li>C🙂<ol start='6'><li>D</li></ol>tail</li><li>Q</li></ul></td></tr></table>\r\n";
    for backward in [false, true] {
        let mut doc = EditorDocument::new(source);
        let start = source.find("中文").expect("start");
        let end = source.find("🙂").expect("end") + "🙂".len();
        let (anchor, focus) = if backward { (end, start) } else { (start, end) };
        doc.set_selection(
            EditorSelection::range(
                &doc.snapshot(),
                ByteOffset::new(anchor as u64),
                ByteOffset::new(focus as u64),
                CaretAffinity::Downstream,
            )
            .expect("range"),
        )
        .expect("select");
        assert!(doc.command_available(&EditorCommand::OutdentList));
        let result = doc.outdent_list().expect("outdent");
        assert_eq!(result.selection(), doc.selection());
        assert_eq!(doc.snapshot().as_str(), expected);
        let a = expected.find("中文").expect("new start");
        let b = expected.find("🙂").expect("new end") + "🙂".len();
        assert_eq!(
            doc.selection().anchor().get() as usize,
            if backward { b } else { a }
        );
        assert_eq!(
            doc.selection().focus().get() as usize,
            if backward { a } else { b }
        );
        doc.undo().expect("undo");
        assert_eq!(doc.snapshot().as_str(), source);
        assert_eq!(doc.selection().anchor().get() as usize, anchor);
        assert_eq!(doc.selection().focus().get() as usize, focus);
        doc.redo().expect("redo");
        assert_eq!(doc.snapshot().as_str(), expected);
        assert_eq!(
            doc.selection().anchor().get() as usize,
            if backward { b } else { a }
        );
    }
}

#[test]
fn selected_root_items_become_paragraphs_with_independent_endpoint_mapping() {
    let source = "<table><tr><td><ol start='3'><li>A</li><li id='b'><b>中文</b></li><!--gap--><li><p>C🙂</p></li><li>D</li></ol></td></tr></table>\r\n";
    let expected = "<table><tr><td><ol start='3'><li>A</li></ol><p id='b'><b>中文</b></p><!--gap--><div><p>C🙂</p></div><ol start='6'><li>D</li></ol></td></tr></table>\r\n";
    for backward in [false, true] {
        let mut doc = EditorDocument::new(source);
        let start = source.find("中文").expect("start");
        let end = source.find("🙂").expect("end") + "🙂".len();
        let (anchor, focus) = if backward { (end, start) } else { (start, end) };
        doc.set_selection(
            EditorSelection::range(
                &doc.snapshot(),
                ByteOffset::new(anchor as u64),
                ByteOffset::new(focus as u64),
                CaretAffinity::Downstream,
            )
            .expect("range"),
        )
        .expect("select");
        assert!(doc.command_available(&EditorCommand::OutdentList));
        let result = doc.outdent_list().expect("outdent");
        assert_eq!(result.selection(), doc.selection());
        assert_eq!(doc.snapshot().as_str(), expected);
        let a = expected.find("中文").expect("new start");
        let b = expected.find("🙂").expect("new end") + "🙂".len();
        assert_eq!(
            doc.selection().anchor().get() as usize,
            if backward { b } else { a }
        );
        assert_eq!(
            doc.selection().focus().get() as usize,
            if backward { a } else { b }
        );
        doc.undo().expect("undo");
        assert_eq!(doc.snapshot().as_str(), source);
        assert_eq!(doc.selection().anchor().get() as usize, anchor);
        assert_eq!(doc.selection().focus().get() as usize, focus);
        doc.redo().expect("redo");
        assert_eq!(doc.snapshot().as_str(), expected);
        assert_eq!(
            doc.selection().anchor().get() as usize,
            if backward { b } else { a }
        );
    }
}

#[test]
fn outdent_promotes_through_div_wrappers_without_unbalancing_them() {
    check(
        "<ul><li>P<div align='right'>prefix<ol start='3'><li>A</li><li>B|</li><li>C</li></ol>tail</div>end</li><li>Q</li></ul>",
        "<ul><li>P<div align='right'>prefix<ol start='3'><li>A</li></ol></div></li><li>B|<div align='right'><ol start='5'><li>C</li></ol>tail</div>end</li><li>Q</li></ul>",
        false,
    );
}

#[test]
fn selected_items_promote_through_nested_divs() {
    let source = "<table><tr><td><ul><li>P<div align='right'><div>prefix<ol start='3'><li>A</li><li id='b'>中文</li><!--gap--><li>C🙂</li><li>D</li></ol>tail</div>end</div></li><li>Q</li></ul></td></tr></table>\r\n";
    let expected = "<table><tr><td><ul><li>P<div align='right'><div>prefix<ol start='3'><li>A</li></ol></div></div></li><li id='b'>中文</li><!--gap--><li>C🙂<div align='right'><div><ol start='6'><li>D</li></ol>tail</div>end</div></li><li>Q</li></ul></td></tr></table>\r\n";
    for backward in [false, true] {
        let mut doc = EditorDocument::new(source);
        let start = source.find("中文").expect("start");
        let end = source.find("🙂").expect("end") + "🙂".len();
        let (anchor, focus) = if backward { (end, start) } else { (start, end) };
        doc.set_selection(
            EditorSelection::range(
                &doc.snapshot(),
                ByteOffset::new(anchor as u64),
                ByteOffset::new(focus as u64),
                CaretAffinity::Downstream,
            )
            .expect("range"),
        )
        .expect("select");
        assert!(doc.command_available(&EditorCommand::OutdentList));
        let result = doc.outdent_list().expect("outdent");
        assert_eq!(result.selection(), doc.selection());
        assert_eq!(doc.snapshot().as_str(), expected);
        let a = expected.find("中文").expect("new start");
        let b = expected.find("🙂").expect("new end") + "🙂".len();
        assert_eq!(
            doc.selection().anchor().get() as usize,
            if backward { b } else { a }
        );
        assert_eq!(
            doc.selection().focus().get() as usize,
            if backward { a } else { b }
        );
        doc.undo().expect("undo");
        assert_eq!(doc.snapshot().as_str(), source);
        assert_eq!(doc.selection().anchor().get() as usize, anchor);
        assert_eq!(doc.selection().focus().get() as usize, focus);
        doc.redo().expect("redo");
        assert_eq!(doc.snapshot().as_str(), expected);
        assert_eq!(
            doc.selection().anchor().get() as usize,
            if backward { b } else { a }
        );
    }
}

#[test]
fn multiple_indent_groups_merge_adjacent_items_without_duplicating_shared_items() {
    let source = "<table><tr><td><ul><li>A</li><li id='b'>中文🙂</li><!--keep--><li>末</li><li>D</li><li>另</li></ul></td><td><ol><li>X</li><li>尾</li></ol></td></tr></table>\r\n";
    let expected = "<table><tr><td><ul><li>A<ul><li id='b'>中文🙂</li><!--keep--><li>末</li></ul></li><li>D<ul><li>另</li></ul></li></ul></td><td><ol><li>X<ol><li>尾</li></ol></li></ol></td></tr></table>\r\n";
    let mut doc = EditorDocument::new(source);
    let needles = ["中文", "🙂", "末", "另", "尾"];
    let cursors = needles.map(|needle| {
        EditorSelection::cursor(
            &doc.snapshot(),
            ByteOffset::new(source.find(needle).expect("caret") as u64),
            CaretAffinity::Downstream,
        )
        .expect("cursor")
    });
    doc.set_selections(cursors, 3).expect("multicarets");
    assert!(doc.command_available(&EditorCommand::IndentList));
    doc.indent_list().expect("indent multiple groups");
    assert_eq!(doc.snapshot().as_str(), expected);
    assert_eq!(doc.selections().len(), 5);
    assert_eq!(doc.selections().primary_index(), 3);
    for (cursor, needle) in doc.selections().as_slice().iter().zip(needles) {
        assert_eq!(
            cursor.focus().get() as usize,
            expected.find(needle).expect("mapped caret")
        );
    }
    doc.undo().expect("undo entire command");
    assert_eq!(doc.snapshot().as_str(), source);
    for (cursor, needle) in doc.selections().as_slice().iter().zip(needles) {
        assert_eq!(
            cursor.focus().get() as usize,
            source.find(needle).expect("original caret")
        );
    }
    doc.redo().expect("redo entire command");
    assert_eq!(doc.snapshot().as_str(), expected);
    assert_eq!(doc.selections().primary_index(), 3);
    for (cursor, needle) in doc.selections().as_slice().iter().zip(needles) {
        assert_eq!(
            cursor.focus().get() as usize,
            expected.find(needle).expect("redo caret")
        );
    }
}

#[test]
fn unindentable_carets_do_not_block_other_html_list_groups() {
    let source =
        "<table><tr><td><ul><li>首</li><li>中文🙂</li></ul></td></tr></table>\r\n\r\n正文尾\r\n";
    let expected = "<table><tr><td><ul><li>首<ul><li>中文🙂</li></ul></li></ul></td></tr></table>\r\n\r\n正文尾\r\n";
    let mut doc = EditorDocument::new(source);
    let needles = ["首", "🙂", "尾"];
    let cursors = needles.map(|needle| {
        EditorSelection::cursor(
            &doc.snapshot(),
            ByteOffset::new(source.find(needle).expect("caret") as u64),
            CaretAffinity::Downstream,
        )
        .expect("cursor")
    });
    doc.set_selections(cursors, 0).expect("mixed carets");
    assert!(doc.command_available(&EditorCommand::IndentList));
    doc.indent_list().expect("indent movable item");
    assert_eq!(doc.snapshot().as_str(), expected);
    assert_eq!(doc.selections().primary_index(), 0);
    for (cursor, needle) in doc.selections().as_slice().iter().zip(needles) {
        assert_eq!(
            cursor.focus().get() as usize,
            expected.find(needle).expect("result")
        );
    }
    doc.undo().expect("undo");
    assert_eq!(doc.snapshot().as_str(), source);
    for (cursor, needle) in doc.selections().as_slice().iter().zip(needles) {
        assert_eq!(
            cursor.focus().get() as usize,
            source.find(needle).expect("original")
        );
    }
    doc.redo().expect("redo");
    assert_eq!(doc.snapshot().as_str(), expected);
    for (cursor, needle) in doc.selections().as_slice().iter().zip(needles) {
        assert_eq!(
            cursor.focus().get() as usize,
            expected.find(needle).expect("redo")
        );
    }
}

#[test]
fn multiple_outdent_carets_exit_shared_and_adjacent_items_once() {
    let source = "<table><tr><td><ol start='4'><li>A</li><li id='b'>中文🙂</li><!--keep--><li>末</li><li>Z</li></ol></td><td><ul><li>尾</li></ul></td></tr></table>\r\n\r\n正文外\r\n";
    let expected = "<table><tr><td><ol start='4'><li>A</li></ol><p id='b'>中文🙂</p><!--keep--><p>末</p><ol start='7'><li>Z</li></ol></td><td><p>尾</p></td></tr></table>\r\n\r\n正文外\r\n";
    let mut doc = EditorDocument::new(source);
    let needles = ["中文", "🙂", "末", "尾", "外"];
    let cursors = needles.map(|needle| {
        EditorSelection::cursor(
            &doc.snapshot(),
            ByteOffset::new(source.find(needle).expect("caret") as u64),
            CaretAffinity::Downstream,
        )
        .expect("cursor")
    });
    doc.set_selections(cursors, 4).expect("select");
    assert!(doc.command_available(&EditorCommand::OutdentList));
    doc.outdent_list().expect("multi outdent");
    assert_eq!(doc.snapshot().as_str(), expected);
    assert_eq!(doc.selections().len(), 5);
    assert_eq!(doc.selections().primary_index(), 4);
    for (cursor, needle) in doc.selections().as_slice().iter().zip(needles) {
        assert_eq!(
            cursor.focus().get() as usize,
            expected.find(needle).expect("new caret")
        );
    }
    doc.undo().expect("undo command");
    assert_eq!(doc.snapshot().as_str(), source);
    for (cursor, needle) in doc.selections().as_slice().iter().zip(needles) {
        assert_eq!(
            cursor.focus().get() as usize,
            source.find(needle).expect("old caret")
        );
    }
    doc.redo().expect("redo command");
    assert_eq!(doc.snapshot().as_str(), expected);
    for (cursor, needle) in doc.selections().as_slice().iter().zip(needles) {
        assert_eq!(
            cursor.focus().get() as usize,
            expected.find(needle).expect("redo caret")
        );
    }
}

#[test]
fn multiple_outdent_preserves_forward_and_backward_ranges() {
    let source = "<table><tr><td><ul><li>A</li><li id='b'><b>中文🙂尾</b></li><li><p>Second</p></li></ul></td></tr></table>\r\n";
    let expected = "<table><tr><td><ul><li>A</li></ul><p id='b'><b>中文🙂尾</b></p><p>Second</p></td></tr></table>\r\n";
    let mut doc = EditorDocument::new(source);
    let first = source.find("中文").expect("first");
    let second = source.find("Second").expect("second");
    let selections = [(first, first + "中文🙂".len()), (second + 6, second)].map(|(a, f)| {
        EditorSelection::range(
            &doc.snapshot(),
            ByteOffset::new(a as u64),
            ByteOffset::new(f as u64),
            CaretAffinity::Downstream,
        )
        .expect("range")
    });
    doc.set_selections(selections, 1).expect("select ranges");
    assert!(doc.command_available(&EditorCommand::OutdentList));
    doc.outdent_list().expect("outdent ranges");
    assert_eq!(doc.snapshot().as_str(), expected);
    let a = expected.find("中文").expect("mapped first");
    let b = expected.find("Second").expect("mapped second");
    let targets = [(a, a + "中文🙂".len()), (b + 6, b)];
    for (selection, (anchor, focus)) in doc.selections().as_slice().iter().zip(targets) {
        assert_eq!(selection.anchor().get() as usize, anchor);
        assert_eq!(selection.focus().get() as usize, focus);
    }
    doc.undo().expect("undo");
    assert_eq!(doc.snapshot().as_str(), source);
    for (actual, original) in doc.selections().as_slice().iter().zip(selections) {
        assert_eq!(actual.anchor(), original.anchor());
        assert_eq!(actual.focus(), original.focus());
    }
    doc.redo().expect("redo");
    assert_eq!(doc.snapshot().as_str(), expected);
    assert_eq!(doc.selections().primary_index(), 1);
    for (selection, (anchor, focus)) in doc.selections().as_slice().iter().zip(targets) {
        assert_eq!(selection.anchor().get() as usize, anchor);
        assert_eq!(selection.focus().get() as usize, focus);
    }
}

#[test]
fn multiple_cross_item_ranges_outdent_root_and_nested_lists_atomically() {
    let source = "<table><tr><td><ol start='3'><li>A</li><li id='b'><b>中文🙂</b></li><!--keep--><li><p>End</p></li><li>Z</li></ol></td><td><ul><li>Parent<ul><li>Second</li><li>Third</li></ul></li></ul></td></tr></table>\r\n";
    let expected = "<table><tr><td><ol start='3'><li>A</li></ol><p id='b'><b>中文🙂</b></p><!--keep--><div><p>End</p></div><ol start='6'><li>Z</li></ol></td><td><ul><li>Parent</li><li>Second</li><li>Third</li></ul></td></tr></table>\r\n";
    for reverse in [false, true] {
        let mut doc = EditorDocument::new(source);
        let endpoints = |text: &str| {
            let first = (
                text.find("中文").expect("cross-item fixture and command"),
                text.find("End").expect("cross-item fixture and command") + 3,
            );
            let second = (
                text.find("Third").expect("cross-item fixture and command") + 5,
                text.find("Second").expect("cross-item fixture and command"),
            );
            [first, second].map(|(a, f)| if reverse { (f, a) } else { (a, f) })
        };
        let originals = endpoints(source).map(|(a, f)| {
            EditorSelection::range(
                &doc.snapshot(),
                ByteOffset::new(a as u64),
                ByteOffset::new(f as u64),
                CaretAffinity::Downstream,
            )
            .expect("cross-item fixture and command")
        });
        doc.set_selections(originals, 1)
            .expect("cross-item fixture and command");
        assert!(doc.command_available(&EditorCommand::OutdentList));
        doc.outdent_list().expect("cross-item fixture and command");
        assert_eq!(doc.snapshot().as_str(), expected);
        let check = |doc: &EditorDocument, text: &str| {
            assert_eq!(doc.selections().primary_index(), 1);
            for (selection, (a, f)) in doc.selections().as_slice().iter().zip(endpoints(text)) {
                assert_eq!(selection.anchor().get() as usize, a);
                assert_eq!(selection.focus().get() as usize, f);
            }
        };
        check(&doc, expected);
        doc.undo().expect("cross-item fixture and command");
        assert_eq!(doc.snapshot().as_str(), source);
        check(&doc, source);
        doc.redo().expect("cross-item fixture and command");
        assert_eq!(doc.snapshot().as_str(), expected);
        check(&doc, expected);
    }
}

#[test]
fn adjacent_nested_outdent_ranges_merge_without_losing_selections() {
    let source = "<table><tr><td><ul><li>Parent<div id='wrap'><ol start='4'><li>A</li><li id='b'>中文🙂</li><!--keep--><li>Second</li><li>Third</li><li>Z</li></ol></div>Tail</li></ul></td></tr></table>\r\n";
    let expected = "<table><tr><td><ul><li>Parent<div id='wrap'><ol start='4'><li>A</li></ol></div></li><li id='b'>中文🙂</li><!--keep--><li>Second</li><li>Third<div ><ol start='8'><li>Z</li></ol></div>Tail</li></ul></td></tr></table>\r\n";
    let mut doc = EditorDocument::new(source);
    let endpoints = |text: &str| {
        let at = |needle| text.find(needle).expect("fixture text");
        [
            (at("中文"), at("🙂") + "🙂".len()),
            (at("Third") + 5, at("Second")),
        ]
    };
    let selections = endpoints(source).map(|(a, f)| {
        EditorSelection::range(
            &doc.snapshot(),
            ByteOffset::new(a as u64),
            ByteOffset::new(f as u64),
            CaretAffinity::Downstream,
        )
        .expect("selection")
    });
    doc.set_selections(selections, 1).expect("set selections");
    assert!(doc.command_available(&EditorCommand::OutdentList));
    doc.outdent_list().expect("outdent");
    assert_eq!(doc.snapshot().as_str(), expected);
    let check = |doc: &EditorDocument, text: &str| {
        assert_eq!(doc.selections().primary_index(), 1);
        for (actual, (a, f)) in doc.selections().as_slice().iter().zip(endpoints(text)) {
            assert_eq!(actual.anchor().get() as usize, a);
            assert_eq!(actual.focus().get() as usize, f);
        }
    };
    check(&doc, expected);
    doc.undo().expect("undo");
    assert_eq!(doc.snapshot().as_str(), source);
    check(&doc, source);
    doc.redo().expect("redo");
    assert_eq!(doc.snapshot().as_str(), expected);
    check(&doc, expected);
}

#[test]
fn nested_outdent_does_not_promote_unselected_gap_items() {
    let source = "<table><tr><td><ul><li>Parent<ul><li>First</li><li>Keep</li><li>Last</li></ul></li></ul></td></tr></table>\n";
    let mut doc = EditorDocument::new(source);
    let cursors = ["First", "Last"].map(|text| {
        EditorSelection::cursor(
            &doc.snapshot(),
            ByteOffset::new(source.find(text).expect("item") as u64),
            CaretAffinity::Downstream,
        )
        .expect("cursor")
    });
    doc.set_selections(cursors, 0).expect("select");
    assert!(doc.command_available(&EditorCommand::OutdentList));
    doc.outdent_list().expect("outdent selected items");
    assert_eq!(
        doc.snapshot().as_str(),
        "<table><tr><td><ul><li>Parent</li><li>First<ul><li>Keep</li></ul></li><li>Last</li></ul></td></tr></table>\n"
    );
}
