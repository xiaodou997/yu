use yu_core::ByteOffset;
use yu_editor::{CaretAffinity, EditorCommand, EditorDocument, EditorSelection};

#[test]
fn word_navigation_uses_visible_cell_text_and_atomic_source_spelling() {
    for prefix in ["", "> ", "> > "] {
        for eol in ["\n", "\r\n"] {
            let cell = "a\\|b &#32; next<br>last";
            let source = format!(
                "{prefix}| H | B |{eol}{prefix}| --- | --- |{eol}{prefix}| {cell} | stay |{eol}"
            );
            let cell_start = source.find(cell).expect("cell");
            for (from, forward, expected) in [
                (
                    cell.find('\\').expect("escape"),
                    true,
                    cell.find('b').expect("b"),
                ),
                (
                    cell.find('b').expect("b"),
                    false,
                    cell.find('\\').expect("escape"),
                ),
                (
                    cell.find("&#32;").expect("entity"),
                    true,
                    cell.find("<br>").expect("br"),
                ),
                (cell.find("<br>").expect("br"), true, cell.len()),
                (
                    cell.find("last").expect("last"),
                    false,
                    cell.find("next").expect("next"),
                ),
            ] {
                let mut document = EditorDocument::new(source.clone());
                document
                    .set_selection(
                        EditorSelection::cursor(
                            &document.snapshot(),
                            ByteOffset::new((cell_start + from) as u64),
                            CaretAffinity::Downstream,
                        )
                        .expect("cursor"),
                    )
                    .expect("select");
                document
                    .execute(if forward {
                        EditorCommand::MoveWordRight
                    } else {
                        EditorCommand::MoveWordLeft
                    })
                    .expect("move word");
                assert_eq!(
                    document.selection().focus().get(),
                    (cell_start + expected) as u64,
                    "{prefix:?} {eol:?} from={from} forward={forward}"
                );
                assert_eq!(document.snapshot().as_str(), source);
            }
        }
    }
}

#[test]
fn extended_word_delete_preserves_atoms_neighbours_and_undo_source() {
    for (cell, from, forward, removed) in [
        ("a\\|b", 1, true, "\\|"),
        ("a\\|b", 3, false, "\\|"),
        ("next<br>last", 4, true, "<br>last"),
        ("next<br>last", 8, false, "next<br>"),
        ("&#32;word", 0, true, "&#32;word"),
        ("\\*\u{301} word", 0, true, "\\*\u{301}"),
        ("👨‍👩‍👧‍👦 word", 0, true, "👨‍👩‍👧‍👦"),
    ] {
        let source =
            format!("> | H | B |\r\n> | --- | --- |\r\n> | {cell} | untouched |\r\n\r\nTail.\r\n");
        let start = source.find(cell).expect("cell");
        let mut document = EditorDocument::new(source.clone());
        document
            .set_selection(
                EditorSelection::cursor(
                    &document.snapshot(),
                    ByteOffset::new((start + from) as u64),
                    CaretAffinity::Downstream,
                )
                .expect("cursor"),
            )
            .expect("select");
        document
            .execute(EditorCommand::ExtendHorizontal {
                forward,
                word: true,
            })
            .expect("extend");
        let range = document.selection().ordered_range();
        assert_eq!(
            &source[range.start().get() as usize..range.end().get() as usize],
            removed
        );
        document
            .execute(EditorCommand::DeleteSelections)
            .expect("delete");
        let mut expected = source.clone();
        expected.replace_range(range.start().get() as usize..range.end().get() as usize, "");
        assert_eq!(document.snapshot().as_str(), expected);
        document.execute(EditorCommand::Undo).expect("undo");
        assert_eq!(document.snapshot().as_str(), source);
        document.execute(EditorCommand::Redo).expect("redo");
        assert_eq!(document.snapshot().as_str(), expected);
    }
}

#[test]
fn word_extension_at_cell_edges_never_selects_structural_delimiters() {
    let source = "| H | B |\n| --- | --- |\n| value | next |\n";
    let start = source.find("value").expect("cell");
    for (at, forward) in [(start, false), (start + 5, true)] {
        let mut document = EditorDocument::new(source);
        document
            .set_selection(
                EditorSelection::cursor(
                    &document.snapshot(),
                    ByteOffset::new(at as u64),
                    CaretAffinity::Downstream,
                )
                .expect("cursor"),
            )
            .expect("select");
        document
            .execute(EditorCommand::ExtendHorizontal {
                forward,
                word: true,
            })
            .expect("extend");
        assert!(document.selection().is_empty());
        assert_eq!(document.selection().focus().get(), at as u64);
        document
            .execute(EditorCommand::DeleteSelections)
            .expect("delete empty");
        assert_eq!(document.snapshot().as_str(), source);
    }
}

#[test]
fn direct_word_delete_is_atomic_and_protects_cell_edges() {
    for (cell, at, forward, expected) in [
        ("a\\|b", 1, true, "ab"),
        ("a\\|b", 3, false, "ab"),
        ("next<br>last", 4, true, "next"),
        ("next<br>last", 8, false, "last"),
        ("&#32;word", 0, true, ""),
        ("\\*\u{301} word", 0, true, " word"),
        ("value", 0, false, "value"),
        ("value", 5, true, "value"),
    ] {
        let source = format!("| H | B |\n| --- | --- |\n| {cell} | stay |\n");
        let at = source.find(cell).expect("cell") + at;
        let mut document = EditorDocument::new(source.clone());
        document
            .set_selection(
                EditorSelection::cursor(
                    &document.snapshot(),
                    ByteOffset::new(at as u64),
                    CaretAffinity::Downstream,
                )
                .expect("cursor"),
            )
            .expect("select");
        let command = if forward {
            EditorCommand::DeleteWordForward
        } else {
            EditorCommand::DeleteWordBackward
        };
        assert_eq!(document.command_available(&command), cell != expected);
        document.execute(command).expect("delete word");
        let changed = source.replacen(
            &format!("| {cell} | stay"),
            &format!("| {expected} | stay"),
            1,
        );
        assert_eq!(document.snapshot().as_str(), changed);
        if cell != expected {
            assert_eq!(document.history_stats().undo_entries(), 1);
            document.execute(EditorCommand::Undo).expect("undo");
            assert_eq!(document.snapshot().as_str(), source);
            document.execute(EditorCommand::Redo).expect("redo");
            assert_eq!(document.snapshot().as_str(), changed);
        } else {
            assert_eq!(document.history_stats().undo_entries(), 0);
        }
    }
}

#[test]
fn word_delete_merges_overlapping_multi_caret_ranges_and_restores_selection_set() {
    for (ranges, forward, expected) in [
        (vec![(0, 0), (1, 3), (5, 5), (16, 16)], false, " beta "),
        (vec![(0, 0), (2, 3), (4, 4), (11, 13)], true, " beta mma"),
    ] {
        let source = "alpha beta gamma";
        let mut document = EditorDocument::new(source);
        let selections = ranges
            .iter()
            .map(|&(a, b)| {
                EditorSelection::range(
                    &document.snapshot(),
                    ByteOffset::new(a),
                    ByteOffset::new(b),
                    CaretAffinity::Downstream,
                )
                .expect("range")
            })
            .collect::<Vec<_>>();
        document
            .set_selections(selections.clone(), 1)
            .expect("selections");
        document
            .execute(if forward {
                EditorCommand::DeleteWordForward
            } else {
                EditorCommand::DeleteWordBackward
            })
            .expect("delete union");
        assert_eq!(document.snapshot().as_str(), expected);
        assert_eq!(document.history_stats().undo_entries(), 1);
        document.execute(EditorCommand::Undo).expect("undo");
        assert_eq!(document.snapshot().as_str(), source);
        assert_eq!(document.selections().as_slice().len(), selections.len());
        for (actual, &(a, b)) in document.selections().as_slice().iter().zip(&ranges) {
            assert_eq!(actual.anchor().get(), a);
            assert_eq!(actual.focus().get(), b);
        }
        document.execute(EditorCommand::Redo).expect("redo");
        assert_eq!(document.snapshot().as_str(), expected);
    }
}
