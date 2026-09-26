use yu_core::ByteOffset;
use yu_editor::{CaretAffinity, EditorCommand, EditorDocument, EditorSelection, TableEdit};

fn focus(document: &mut EditorDocument, label: &str) {
    let snapshot = document.snapshot();
    let at = snapshot.as_str().find(label).expect("cell label");
    document
        .set_selection(
            EditorSelection::cursor(
                &snapshot,
                ByteOffset::new(at as u64),
                CaretAffinity::Downstream,
            )
            .expect("cursor"),
        )
        .expect("selection");
}

#[test]
fn html_structural_commands_preserve_untouched_tags_and_are_single_undo_steps() {
    let first = "<tr id='keep'><td>A</td><td>B</td></tr>";
    let second = "<tr><th>C</th><td>D</td></tr>";
    let source = format!(
        "\u{feff}KEEP🙂\r\n\r\n<table><tbody>{first}\r\n{second}</tbody></table>\r\n\r\nTAIL"
    );
    for (edit, expected) in [
        (
            TableEdit::InsertColumnBefore,
            source
                .replace("<td>B</td>", "<td></td><td>B</td>")
                .replace("<td>D</td>", "<td></td><td>D</td>"),
        ),
        (
            TableEdit::InsertColumnAfter,
            source
                .replace("<td>B</td>", "<td>B</td><td></td>")
                .replace("<td>D</td>", "<td>D</td><td></td>"),
        ),
        (
            TableEdit::DeleteColumn,
            source.replace("<td>B</td>", "").replace("<td>D</td>", ""),
        ),
        (
            TableEdit::InsertRowBefore,
            source.replace(first, &format!("<tr><td></td><td></td></tr>\r\n{first}")),
        ),
        (
            TableEdit::InsertRowAfter,
            source.replace(first, &format!("{first}\r\n<tr><td></td><td></td></tr>")),
        ),
        (TableEdit::DeleteRow, source.replace(first, "")),
    ] {
        let mut document = EditorDocument::new(&source);
        focus(&mut document, "B</td>");
        let selection = document.selection();
        let command = EditorCommand::EditTable(edit);
        assert!(document.command_available(&command), "{edit:?}");
        document.execute(command).expect("HTML edit");
        assert_eq!(document.snapshot().as_str(), expected, "{edit:?}");
        if matches!(
            edit,
            TableEdit::InsertColumnBefore
                | TableEdit::InsertColumnAfter
                | TableEdit::InsertRowBefore
                | TableEdit::InsertRowAfter
        ) {
            let focus = document.selection().focus().get() as usize;
            assert!(
                document.snapshot().as_str()[focus..].starts_with("</td>"),
                "new empty cell focus for {edit:?}"
            );
        }
        assert_eq!(document.history_stats().undo_entries(), 1);
        document.undo().expect("undo");
        assert_eq!(document.snapshot().as_str(), source);
        assert_eq!(document.selection().anchor(), selection.anchor());
        assert_eq!(document.selection().focus(), selection.focus());
        assert_eq!(document.selection().affinity(), selection.affinity());
        document.redo().expect("redo");
        assert_eq!(document.snapshot().as_str(), expected);
    }
}

#[test]
fn html_column_alignment_preserves_quotes_attributes_and_unquoted_style_validity() {
    let source = "<table><tr><td title='keep' align='left'>A</td></tr><tr><td style=text-align:right>B</td></tr></table>";
    let mut document = EditorDocument::new(source);
    focus(&mut document, "A</td>");
    document
        .execute(EditorCommand::EditTable(TableEdit::AlignCenter))
        .expect("align");
    let expected = source
        .replace("align='left'", "align='center'")
        .replace("style=text-align:right", "style=text-align:center");
    assert_eq!(document.snapshot().as_str(), expected);
    let region = &document.markdown().html_regions().regions[0];
    assert!(
        region
            .model
            .as_ref()
            .expect("model")
            .resolution
            .diagnostics
            .is_empty()
    );
    document.undo().expect("undo");
    assert_eq!(document.snapshot().as_str(), source);
    document.set_source_mode(true).expect("source mode");
    assert!(!document.command_available(&EditorCommand::EditTable(TableEdit::DeleteColumn)));
}

#[test]
fn deleting_the_last_html_row_or_column_removes_only_the_table() {
    for edit in [TableEdit::DeleteRow, TableEdit::DeleteColumn] {
        let mut document =
            EditorDocument::new("KEEP\n\n<table><tr><td>cell</td></tr></table>\n\nTAIL");
        focus(&mut document, "cell");
        document
            .execute(EditorCommand::EditTable(edit))
            .expect("delete table");
        assert_eq!(document.snapshot().as_str(), "KEEP\n\n\n\nTAIL");
        document.undo().expect("undo");
        assert_eq!(
            document.snapshot().as_str(),
            "KEEP\n\n<table><tr><td>cell</td></tr></table>\n\nTAIL"
        );
    }
}

#[test]
fn html_tab_navigation_and_append_preserve_sections_and_history() {
    use yu_editor::{EditorKey, KeyEvent, KeyModifiers};
    let source = "KEEP🙂\r\n\r\n<table><tbody><tr><th>A</th><td>B</td></tr>\r\n<tr><td>C</td><td>D</td></tr></tbody></table>\r\n\r\nTAIL";
    let mut document = EditorDocument::new(source);
    focus(&mut document, "A</th>");
    document
        .route_key(KeyEvent::new(EditorKey::Tab, KeyModifiers::NONE))
        .expect("HTML table navigation contract");
    assert_eq!(
        document.selection().focus().get() as usize,
        source
            .find("B</td>")
            .expect("HTML table navigation contract")
    );
    document
        .route_key(KeyEvent::new(EditorKey::Tab, KeyModifiers::SHIFT))
        .expect("HTML table navigation contract");
    assert_eq!(
        document.selection().focus().get() as usize,
        source
            .find("A</th>")
            .expect("HTML table navigation contract")
    );
    assert_eq!(document.snapshot().as_str(), source);
    assert_eq!(document.history_stats().undo_entries(), 0);
    focus(&mut document, "D</td>");
    let before = document.selection();
    document
        .route_key(KeyEvent::new(EditorKey::Tab, KeyModifiers::NONE))
        .expect("HTML table navigation contract");
    let expected = source.replace("</tbody>", "\r\n<tr><td></td><td></td></tr></tbody>");
    assert_eq!(document.snapshot().as_str(), expected);
    assert_eq!(
        document.selection().focus().get() as usize,
        expected
            .find("</td><td></td>")
            .expect("HTML table navigation contract")
    );
    assert_eq!(document.history_stats().undo_entries(), 1);
    document.undo().expect("HTML table navigation contract");
    assert_eq!(document.snapshot().as_str(), source);
    assert_eq!(document.selection().focus(), before.focus());
    document.redo().expect("HTML table navigation contract");
    assert_eq!(document.snapshot().as_str(), expected);
    document
        .set_source_mode(true)
        .expect("HTML table navigation contract");
    assert!(!document.command_available(&EditorCommand::MoveTableCellNext));
    assert!(!document.command_available(&EditorCommand::MoveTableCellPrevious));
}

#[test]
fn html_rectangular_selection_clears_contents_without_deleting_cell_tags() {
    let source = "\u{feff}KEEP\r\n\r\n<table id='grid'><thead><tr><th>A</th><th>B</th></tr></thead>\r\n<tbody><tr><td><b>中文🙂</b></td><td>&amp;</td></tr></tbody></table>\r\n\r\nTAIL";
    let a = ByteOffset::new(source.find("A</th>").expect("A") as u64);
    let last = ByteOffset::new(source.find("&amp;").expect("entity") as u64);
    for (anchor, focus) in [(a, last), (last, a)] {
        let mut document = EditorDocument::new(source);
        let selection = EditorCommand::SelectTableCells { anchor, focus };
        assert!(document.command_available(&selection));
        document.execute(selection).expect("rectangle");
        assert_eq!(document.selections().table_columns(), Some(2));
        assert_eq!(document.selections().len(), 4);
        let before = document.selections().clone();
        document
            .execute(EditorCommand::DeleteSelections)
            .expect("clear");
        let expected = source
            .replace("A</th>", "</th>")
            .replace("B</th>", "</th>")
            .replace("<b>中文🙂</b>", "")
            .replace("&amp;", "");
        assert_eq!(document.snapshot().as_str(), expected);
        assert_eq!(document.selections().table_columns(), Some(2));
        assert_eq!(document.selections().len(), 4);
        assert!(
            document
                .selections()
                .as_slice()
                .iter()
                .all(|selection| selection.is_empty())
        );
        document.undo().expect("undo clear");
        assert_eq!(document.snapshot().as_str(), source);
        assert_eq!(
            document.selections().primary_index(),
            before.primary_index()
        );
        for (actual, original) in document
            .selections()
            .as_slice()
            .iter()
            .zip(before.as_slice())
        {
            assert_eq!(actual.anchor(), original.anchor());
            assert_eq!(actual.focus(), original.focus());
        }
        document.redo().expect("redo clear");
        assert_eq!(document.snapshot().as_str(), expected);
        document.set_source_mode(true).expect("source mode");
        assert!(
            !document.command_available(&EditorCommand::SelectTableCells {
                anchor: a,
                focus: a
            })
        );
    }
}

#[test]
fn html_grid_paste_grows_inside_sections_and_escapes_literal_payloads() {
    for eol in ["", "\n", "\r\n"] {
        let source = format!(
            "\u{feff}KEEP🙂\r\n\r\n<table id='keep'><thead><tr><th>A</th><th>B</th></tr></thead>{eol}<tbody><tr><td>C</td><td title='keep'>D</td></tr></tbody></table>\r\n\r\nTAIL"
        );
        let mut document = EditorDocument::new(&source);
        focus(&mut document, "D</td>");
        let before = document.selection();
        let command = EditorCommand::PasteTableGrid {
            columns: 2,
            cells: ["<b>&|", "line1\r\nline2", "中文🙂", ""]
                .map(Into::into)
                .to_vec(),
        };
        assert!(document.command_available(&command));
        document.execute(command).expect("HTML grid paste");
        let expected = source.replace("B</th>", "B</th><th></th>")
            .replace("D</td></tr>", &format!("&lt;b&gt;&amp;|</td><td>line1<br>line2</td></tr>{eol}<tr><td></td><td>中文🙂</td><td></td></tr>"));
        assert_eq!(document.snapshot().as_str(), expected);
        assert_eq!(document.selections().table_columns(), Some(2));
        assert_eq!(document.selections().len(), 4);
        assert_eq!(document.history_stats().undo_entries(), 1);
        document.undo().expect("undo paste");
        assert_eq!(document.snapshot().as_str(), source);
        assert_eq!(document.selection().focus(), before.focus());
        document.redo().expect("redo paste");
        assert_eq!(document.snapshot().as_str(), expected);
    }
}

#[test]
fn html_scalar_grid_fill_and_invalid_paste_preserve_source() {
    let source = "<table><tr><td>A</td><td>B</td></tr><tr><td>C</td><td>D</td></tr></table>";
    let mut document = EditorDocument::new(source);
    document
        .execute(EditorCommand::SelectTableCells {
            anchor: ByteOffset::new(source.find("A</td>").expect("A") as u64),
            focus: ByteOffset::new(source.find("D</td>").expect("D") as u64),
        })
        .expect("select");
    for (columns, cells) in [(0, vec!["x".into()]), (2, vec!["x".into()]), (1, vec![])] {
        let command = EditorCommand::PasteTableGrid { columns, cells };
        assert!(!document.command_available(&command));
        assert!(document.execute(command).is_err());
        assert_eq!(document.snapshot().as_str(), source);
        assert_eq!(document.history_stats().undo_entries(), 0);
    }
    document
        .execute(EditorCommand::PasteTableGrid {
            columns: 1,
            cells: vec!["<&\n".into()],
        })
        .expect("fill");
    assert_eq!(
        document.snapshot().as_str(),
        "<table><tr><td>&lt;&amp;<br></td><td>&lt;&amp;<br></td></tr><tr><td>&lt;&amp;<br></td><td>&lt;&amp;<br></td></tr></table>"
    );
    document.undo().expect("undo fill");
    assert_eq!(document.snapshot().as_str(), source);
}

#[test]
fn html_tsv_clipboard_roundtrip_uses_visible_cell_text() {
    let source = "<table><tr><th><b>A&amp;B</b></th><td>one<br>two</td></tr></table>";
    let mut document = EditorDocument::new(source);
    document
        .execute(EditorCommand::SelectTableCells {
            anchor: ByteOffset::new(source.find("A&amp;").expect("A") as u64),
            focus: ByteOffset::new(source.find("one").expect("one") as u64),
        })
        .expect("select");
    let copied = document.copy_table_tsv().expect("copy").expect("grid");
    assert_eq!(copied, "A&B\t\"one\ntwo\"");
    let command = EditorCommand::PasteTsv(copied.into());
    assert!(document.command_available(&command));
    document.execute(command).expect("paste copy");
    assert_eq!(
        document.snapshot().as_str(),
        "<table><tr><th>A&amp;B</th><td>one<br>two</td></tr></table>"
    );
    assert_eq!(
        document
            .copy_table_tsv()
            .expect("copy again")
            .expect("grid"),
        "A&B\t\"one\ntwo\""
    );
    document.undo().expect("undo");
    assert_eq!(document.snapshot().as_str(), source);
}

#[test]
fn html_grapheme_deletion_preserves_tags_and_stops_at_cell_edges() {
    for content in ["<b>中文🙂</b>", "&amp;", "e&#x301;", "<br>", ""] {
        let source = format!("<table><tr><td>{content}</td><td>KEEP</td></tr></table>");
        let start = source.find("<td>").expect("cell") + 4;
        let end = start + content.len();
        for (at, command) in [
            (start, EditorCommand::DeleteBackward),
            (end, EditorCommand::DeleteForward),
        ] {
            let mut document = EditorDocument::new(&source);
            let snapshot = document.snapshot();
            document
                .set_selection(
                    EditorSelection::cursor(
                        &snapshot,
                        ByteOffset::new(at as u64),
                        CaretAffinity::Downstream,
                    )
                    .expect("cursor"),
                )
                .expect("selection");
            document.execute(command).expect("edge deletion");
            assert_eq!(document.snapshot().as_str(), source);
            assert_eq!(document.history_stats().undo_entries(), 0);
        }
    }
    for (content, after, forward) in [
        ("<b>中文🙂</b>", "<b>中文</b>", false),
        ("<b>中文🙂</b>", "<b>文🙂</b>", true),
        ("&amp;", "", false),
        ("&amp;", "", true),
        ("e&#x301;", "", false),
        ("e&#x301;", "", true),
        ("<b>e</b>&#x301;", "<b></b>", false),
        ("<b>e</b>&#x301;", "<b></b>", true),
        ("<br>", "", false),
    ] {
        let source = format!("<table><tr><td>{content}</td><td>KEEP</td></tr></table>");
        let mut document = EditorDocument::new(&source);
        let at = source.find("<td>").expect("cell") + 4 + if forward { 0 } else { content.len() };
        let snapshot = document.snapshot();
        document
            .set_selection(
                EditorSelection::cursor(
                    &snapshot,
                    ByteOffset::new(at as u64),
                    CaretAffinity::Downstream,
                )
                .expect("cursor"),
            )
            .expect("selection");
        document
            .execute(if forward {
                EditorCommand::DeleteForward
            } else {
                EditorCommand::DeleteBackward
            })
            .expect("delete grapheme");
        assert_eq!(
            document.snapshot().as_str(),
            format!("<table><tr><td>{after}</td><td>KEEP</td></tr></table>")
        );
        document.undo().expect("undo");
        assert_eq!(document.snapshot().as_str(), source);
    }
}

#[test]
fn html_cell_typing_and_return_keep_literal_text_and_native_breaks() {
    let source = "KEEP\r\n\r\n<table><tr><td>A</td><td>B</td></tr></table>\r\n\r\nTAIL";
    for command in [
        EditorCommand::insert_text("中文<&🙂"),
        EditorCommand::PasteFragments(vec!["中文<&🙂".into()]),
    ] {
        let mut document = EditorDocument::new(source);
        focus(&mut document, "A</td>");
        document.execute(command).expect("literal HTML input");
        assert_eq!(
            document.snapshot().as_str(),
            source.replace("A</td>", "中文&lt;&amp;🙂A</td>")
        );
        assert_eq!(
            document.selection().focus().get() as usize,
            document.snapshot().as_str().find("A</td>").expect("A")
        );
        document.undo().expect("undo");
        assert_eq!(document.snapshot().as_str(), source);
    }
    let mut document = EditorDocument::new(source);
    focus(&mut document, "A</td>");
    document
        .execute(EditorCommand::InsertNewline)
        .expect("return");
    assert_eq!(
        document.snapshot().as_str(),
        source.replace("A</td>", "<br>A</td>")
    );
    document.undo().expect("undo return");
    assert_eq!(document.snapshot().as_str(), source);
    document.set_source_mode(true).expect("source mode");
    focus(&mut document, "A</td>");
    document
        .execute(EditorCommand::insert_text("<b>"))
        .expect("source input");
    assert_eq!(
        document.snapshot().as_str(),
        source.replace("A</td>", "<b>A</td>")
    );
}

#[test]
fn html_word_deletion_preserves_inline_tags_and_cell_boundaries() {
    let source = "<table><tr><td><b>hello</b> world</td><td>KEEP</td></tr></table>";
    for (forward, at, expected) in [
        (
            false,
            source.find("</td>").expect("cell end"),
            source.replace("world", ""),
        ),
        (
            true,
            source.find("<b>").expect("cell start"),
            source.replace("hello", ""),
        ),
        (
            false,
            source.find("<b>").expect("cell start"),
            source.to_owned(),
        ),
        (
            true,
            source.find("</td>").expect("cell end"),
            source.to_owned(),
        ),
    ] {
        let mut document = EditorDocument::new(source);
        let snapshot = document.snapshot();
        document
            .set_selection(
                EditorSelection::cursor(
                    &snapshot,
                    ByteOffset::new(at as u64),
                    CaretAffinity::Downstream,
                )
                .expect("cursor"),
            )
            .expect("selection");
        document
            .execute(if forward {
                EditorCommand::DeleteWordForward
            } else {
                EditorCommand::DeleteWordBackward
            })
            .expect("word delete");
        assert_eq!(document.snapshot().as_str(), expected);
        if expected != source {
            document.undo().expect("undo");
            assert_eq!(document.snapshot().as_str(), source);
        }
    }
}

#[test]
fn html_linear_selection_replacement_preserves_partial_inline_and_cell_tags() {
    let source = "<table><tr><td><b>AB</b>CD</td><td><i>EF</i>GH</td></tr></table>";
    for (from, to, expected) in [
        (
            "B</b>",
            "D</td>",
            "<table><tr><td><b>A新&lt;</b>D</td><td><i>EF</i>GH</td></tr></table>",
        ),
        (
            "B</b>",
            "F</i>",
            "<table><tr><td><b>A新&lt;</b></td><td><i>F</i>GH</td></tr></table>",
        ),
    ] {
        for reverse in [false, true] {
            let mut document = EditorDocument::new(source);
            let a = ByteOffset::new(source.find(from).expect("from") as u64);
            let b = ByteOffset::new(source.find(to).expect("to") as u64);
            let (anchor, focus) = if reverse { (b, a) } else { (a, b) };
            document
                .set_selection(
                    EditorSelection::range(
                        &document.snapshot(),
                        anchor,
                        focus,
                        CaretAffinity::Downstream,
                    )
                    .expect("range"),
                )
                .expect("select");
            document
                .execute(EditorCommand::insert_text("新<"))
                .expect("replace");
            assert_eq!(document.snapshot().as_str(), expected);
            assert_eq!(
                document.selection().focus().get() as usize,
                expected.find("新&lt;").expect("inserted") + "新&lt;".len()
            );
            document.undo().expect("undo");
            assert_eq!(document.snapshot().as_str(), source);
            assert_eq!(document.selection().anchor(), anchor);
            assert_eq!(document.selection().focus(), focus);
            document
                .execute(EditorCommand::DeleteSelections)
                .expect("delete");
            assert_eq!(document.snapshot().as_str(), expected.replace("新&lt;", ""));
            document.undo().expect("undo delete");
            assert_eq!(document.snapshot().as_str(), source);
        }
    }
}

#[test]
fn justified_html_cells_keep_editing_navigation_and_exact_history() {
    let source = "<table align='justify'><tr><td>中文 words</td><td align='right'>KEEP</td></tr></table>\r\n";
    let mut document = EditorDocument::new(source);
    focus(&mut document, "中文");
    document
        .execute(EditorCommand::insert_text("新<&"))
        .expect("cell input");
    let edited = source.replace("中文", "新&lt;&amp;中文");
    assert_eq!(document.snapshot().as_str(), edited);
    document
        .execute(EditorCommand::MoveTableCellNext)
        .expect("next cell");
    assert_eq!(
        document.selection().focus().get() as usize,
        edited.find("KEEP").expect("next cell")
    );
    document.undo().expect("undo input");
    assert_eq!(document.snapshot().as_str(), source);
    document.redo().expect("redo input");
    assert_eq!(document.snapshot().as_str(), edited);
    let mut reopened = EditorDocument::new(&edited);
    let view = reopened
        .block_layout_for_visual_state(0, yu_editor::LayoutConfig::new(400.0, 16.0))
        .expect("reopen grid");
    assert_eq!(
        view.table().expect("table").cells()[0].alignment(),
        yu_markdown::TableAlignment::Justify
    );
}

#[test]
fn unsupported_html_tables_use_literal_source_editing_without_grid_commands() {
    for source in [
        "<table><tr><td><ul class='unsupported'><li>MARK</li></ul></td></tr></table>",
        "<table><tr><td colspan='0'>MARK</td></tr></table>",
        "<table><tr><td><unknown>MARK</unknown></td></tr></table>",
    ] {
        let mut document = EditorDocument::new(source);
        focus(&mut document, "MARK");
        let view = document
            .block_layout_for_visual_state(0, yu_editor::LayoutConfig::new(500.0, 16.0))
            .expect("source layout");
        assert!(view.table().is_none(), "{source}");
        assert_eq!(view.visual().text(), source);
        for command in [
            EditorCommand::EditTable(TableEdit::DeleteColumn),
            EditorCommand::MoveTableCellNext,
            EditorCommand::MoveTableCellPrevious,
        ] {
            assert!(
                !document.command_available(&command),
                "source mode advertised {command:?}: {source}"
            );
        }
        assert!(
            !document
                .execute(EditorCommand::EditTable(TableEdit::DeleteColumn))
                .expect("unavailable command is a no-op")
                .changed()
        );
        assert_eq!(document.snapshot().as_str(), source);
        assert_eq!(document.history_stats().undo_entries(), 0);
        document
            .execute(EditorCommand::insert_text("<&"))
            .expect("literal source input");
        assert_eq!(
            document.snapshot().as_str(),
            source.replace("MARK", "<&MARK")
        );
        document.undo().expect("undo raw source input");
        assert_eq!(document.snapshot().as_str(), source);
    }
}

#[test]
fn ragged_html_table_navigation_edits_and_paste_keep_exact_source() {
    use yu_editor::{EditorKey, KeyEvent, KeyModifiers};
    let source = "<table><tbody><tr><td>A</td></tr><tr></tr><tr><td>B</td><td>C</td><td>D</td></tr></tbody></table>\r\n";
    let mut doc = EditorDocument::new(source);
    focus(&mut doc, "A</td>");
    doc.route_key(KeyEvent::new(EditorKey::Tab, KeyModifiers::NONE))
        .expect("next real cell");
    assert_eq!(
        doc.selection().focus().get() as usize,
        source.find("B</td>").expect("B cell")
    );
    doc.route_key(KeyEvent::new(EditorKey::Tab, KeyModifiers::SHIFT))
        .expect("previous real cell");
    assert_eq!(
        doc.selection().focus().get() as usize,
        source.find("A</td>").expect("A cell")
    );
    assert_eq!(doc.snapshot().as_str(), source);
    for edit in [
        TableEdit::InsertColumnBefore,
        TableEdit::InsertColumnAfter,
        TableEdit::DeleteColumn,
        TableEdit::AlignRight,
    ] {
        focus(&mut doc, "C</td>");
        assert!(doc.command_available(&EditorCommand::EditTable(edit)));
        doc.execute(EditorCommand::EditTable(edit))
            .expect("sparse edit");
        let edited = doc.snapshot().as_str().to_owned();
        match edit {
            TableEdit::DeleteColumn => assert_eq!(edited, source.replace("<td>C</td>", "")),
            TableEdit::AlignRight => assert_eq!(
                edited,
                source.replace("<td>C</td>", "<td align=\"right\">C</td>")
            ),
            _ => assert!(edited.contains("<td>A</td><td></td>")),
        }
        doc.undo().expect("undo sparse edit");
        assert_eq!(doc.snapshot().as_str(), source);
        doc.redo().expect("redo sparse edit");
        assert_eq!(doc.snapshot().as_str(), edited);
        doc.undo().expect("restore fixture");
    }
    focus(&mut doc, "A</td>");
    doc.execute(EditorCommand::PasteTableGrid {
        columns: 2,
        cells: ["X", "Y", "Z", "W"].map(Into::into).to_vec(),
    })
    .expect("paste materializes only addressed missing cells");
    let expected = source
        .replace("<td>A</td>", "<td>X</td><td>Y</td>")
        .replace("<tr></tr>", "<tr><td>Z</td><td>W</td></tr>");
    assert_eq!(doc.snapshot().as_str(), expected);
    doc.undo().expect("undo paste");
    assert_eq!(doc.snapshot().as_str(), source);
}

#[test]
fn sparse_rectangles_copy_clear_fill_and_restore_holes_in_history() {
    let source = "<table><tr><td>A</td><td>B</td><td>C</td></tr><tr></tr><tr><td>D</td><td>E</td></tr></table>\r\n";
    for reverse in [false, true] {
        let mut doc = EditorDocument::new(source);
        let a = ByteOffset::new(source.find("C</td>").expect("C") as u64);
        let b = ByteOffset::new(source.find("D</td>").expect("D") as u64);
        doc.execute(EditorCommand::SelectTableCells {
            anchor: if reverse { b } else { a },
            focus: if reverse { a } else { b },
        })
        .expect("select sparse rectangle");
        assert_eq!(doc.selections().table_columns(), Some(3));
        assert_eq!(
            doc.selections().table_slots(),
            Some(
                [
                    Some(0),
                    Some(1),
                    Some(2),
                    None,
                    None,
                    None,
                    Some(3),
                    Some(4),
                    None
                ]
                .as_slice()
            )
        );
        assert_eq!(
            doc.copy_table_tsv().expect("copy").as_deref(),
            Some("A\tB\tC\n\t\t\nD\tE\t")
        );
        let selected = doc.selections().clone();
        assert_eq!(doc.snapshot().as_str(), source);
        doc.execute(EditorCommand::DeleteSelections)
            .expect("clear actual cells");
        let cleared = source
            .replace(">A<", "><")
            .replace(">B<", "><")
            .replace(">C<", "><")
            .replace(">D<", "><")
            .replace(">E<", "><");
        assert_eq!(doc.snapshot().as_str(), cleared);
        doc.undo().expect("undo clear");
        assert_eq!(doc.snapshot().as_str(), source);
        assert_eq!(doc.selections().table_slots(), selected.table_slots());
        assert_eq!(
            doc.copy_table_tsv().expect("restored copy").as_deref(),
            Some("A\tB\tC\n\t\t\nD\tE\t")
        );
        doc.execute(EditorCommand::PasteTableGrid {
            columns: 1,
            cells: vec!["<&".into()],
        })
        .expect("fill holes");
        assert_eq!(
            doc.snapshot().as_str(),
            format!(
                "<table>{}</table>\r\n",
                "<tr><td>&lt;&amp;</td><td>&lt;&amp;</td><td>&lt;&amp;</td></tr>".repeat(3)
            )
        );
        doc.undo().expect("undo fill");
        assert_eq!(doc.snapshot().as_str(), source);
        assert_eq!(doc.selections().table_slots(), selected.table_slots());
    }
}

#[test]
fn html_cell_paragraphs_keep_typing_tags_and_clipboard_breaks() {
    let source =
        "<table><tr><td><p>one</p><p align='right'>two</p></td><td>KEEP</td></tr></table>\r\n";
    let mut doc = EditorDocument::new(source);
    focus(&mut doc, "two</p>");
    doc.execute(EditorCommand::InsertText("新<&".into()))
        .expect("paragraph input");
    assert_eq!(
        doc.snapshot().as_str(),
        source.replace("two</p>", "新&lt;&amp;two</p>")
    );
    doc.undo().expect("undo paragraph input");
    assert_eq!(doc.snapshot().as_str(), source);
    doc.execute(EditorCommand::SelectTableCells {
        anchor: ByteOffset::new(source.find("one</p>").expect("one") as u64),
        focus: ByteOffset::new(source.find("KEEP").expect("KEEP") as u64),
    })
    .expect("select paragraphs");
    assert_eq!(
        doc.copy_table_tsv().expect("TSV").as_deref(),
        Some("\"one\ntwo\"\tKEEP")
    );
    doc.execute(EditorCommand::DeleteSelections)
        .expect("clear cells");
    assert_eq!(
        doc.snapshot().as_str(),
        "<table><tr><td></td><td></td></tr></table>\r\n"
    );
    doc.undo().expect("restore paragraphs");
    assert_eq!(doc.snapshot().as_str(), source);
}

#[test]
fn paragraph_boundary_deletion_joins_balanced_tags_and_preserves_comments() {
    for forward in [false, true] {
        let source =
            "<table><tr><td><p>one</p><!--keep--><p align='right'>two</p></td></tr></table>";
        let mut doc = EditorDocument::new(source);
        focus(&mut doc, if forward { "</p><!--" } else { "two</p>" });
        doc.execute(if forward {
            EditorCommand::DeleteForward
        } else {
            EditorCommand::DeleteBackward
        })
        .expect("join paragraphs");
        assert_eq!(
            doc.snapshot().as_str(),
            "<table><tr><td><p>one<!--keep-->two</p></td></tr></table>"
        );
        doc.undo().expect("undo paragraph merge");
        assert_eq!(doc.snapshot().as_str(), source);
    }
}

#[test]
fn typed_html_grid_preserves_format_and_rejects_invalid_payload_atomically() {
    let source = "<table id='keep'><tr><td>A</td><td>B</td></tr></table>\r\n";
    let cells: Vec<std::sync::Arc<str>> = [
        "<p><strong>中文</strong></p><p>second</p>",
        "<a href='https://example.com'>link</a>",
    ]
    .map(Into::into)
    .to_vec();
    let mut doc = EditorDocument::new(source);
    focus(&mut doc, "A</td>");
    let command = EditorCommand::PasteHtmlTableGrid {
        columns: 2,
        cells: cells.clone(),
    };
    assert!(doc.command_available(&command));
    doc.execute(command).expect("typed HTML paste");
    let expected = source
        .replace(">A<", &format!(">{}<", cells[0]))
        .replace(">B<", &format!(">{}<", cells[1]));
    assert_eq!(doc.snapshot().as_str(), expected);
    doc.undo().expect("undo HTML paste");
    assert_eq!(doc.snapshot().as_str(), source);
    doc.redo().expect("redo HTML paste");
    assert_eq!(doc.snapshot().as_str(), expected);
    doc.undo().expect("restore fixture");
    for invalid in [
        "</td></tr></table><p>escape</p><table><tr><td>",
        "<script>bad()</script>",
        "<p>unclosed",
        "<unknown>keep</unknown>",
    ] {
        focus(&mut doc, "A</td>");
        let revision = doc.revision();
        let command = EditorCommand::PasteHtmlTableGrid {
            columns: 2,
            cells: vec!["<b>valid</b>".into(), invalid.into()],
        };
        assert!(!doc.command_available(&command), "{invalid}");
        assert!(doc.execute(command).is_err(), "{invalid}");
        assert_eq!(doc.snapshot().as_str(), source);
        assert_eq!(doc.revision(), revision);
    }
}

#[test]
fn html_cell_heading_edit_clipboard_and_undo_keep_original_markup() {
    let source =
        "<table><tr><td><h2 align='center'>标题</h2><p>body</p></td><td>KEEP</td></tr></table>\r\n";
    let mut doc = EditorDocument::new(source);
    focus(&mut doc, "标题");
    doc.execute(EditorCommand::insert_text("新<&"))
        .expect("heading input");
    assert_eq!(
        doc.snapshot().as_str(),
        source.replace("标题", "新&lt;&amp;标题")
    );
    doc.undo().expect("undo");
    assert_eq!(doc.snapshot().as_str(), source);
    doc.execute(EditorCommand::SelectTableCells {
        anchor: ByteOffset::new(source.find("标题").expect("heading") as u64),
        focus: ByteOffset::new(source.find("KEEP").expect("other cell") as u64),
    })
    .expect("select");
    assert_eq!(
        doc.copy_table_tsv().expect("plain clipboard").as_deref(),
        Some("\"标题\nbody\"\tKEEP")
    );
    doc.execute(EditorCommand::DeleteSelections).expect("clear");
    assert_eq!(
        doc.snapshot().as_str(),
        "<table><tr><td></td><td></td></tr></table>\r\n"
    );
    doc.undo().expect("restore heading");
    assert_eq!(doc.snapshot().as_str(), source);
}

#[test]
fn mixed_html_paragraph_boundaries_keep_the_left_block_and_balanced_tags() {
    for left in ["p", "h1", "h2", "h3", "h4", "h5", "h6"] {
        for right in ["p", "h1", "h2", "h3", "h4", "h5", "h6"] {
            for forward in [false, true] {
                let source = format!(
                    "<table><tr><td><{left} align='right'>one</{left}><!--keep--><{right} id='second'><b>two</b></{right}></td><td>KEEP</td></tr></table>\r\n"
                );
                let expected = format!(
                    "<table><tr><td><{left} align='right'>one<!--keep--><b>two</b></{left}></td><td>KEEP</td></tr></table>\r\n"
                );
                let mut doc = EditorDocument::new(&source);
                let target = if forward {
                    format!("</{left}>")
                } else {
                    "two</b>".to_owned()
                };
                focus(&mut doc, &target);
                doc.execute(if forward {
                    EditorCommand::DeleteForward
                } else {
                    EditorCommand::DeleteBackward
                })
                .expect("merge");
                assert_eq!(
                    doc.snapshot().as_str(),
                    expected,
                    "{left}/{right} forward={forward}"
                );
                assert!(
                    doc.block_layout_for_visual_state(0, yu_editor::LayoutConfig::new(500.0, 16.0))
                        .expect("merged layout")
                        .table()
                        .is_some()
                );
                doc.execute(EditorCommand::insert_text("新"))
                    .expect("type at join");
                assert_eq!(
                    doc.snapshot().as_str(),
                    expected.replace("one<!--", "one新<!--")
                );
                doc.undo().expect("undo typing");
                assert_eq!(doc.snapshot().as_str(), expected);
                doc.undo().expect("undo merge");
                assert_eq!(doc.snapshot().as_str(), source);
                doc.redo().expect("redo merge");
                assert_eq!(doc.snapshot().as_str(), expected);
            }
        }
    }
}

#[test]
fn chained_html_boundary_multicarets_merge_in_one_transaction_and_restore_selections() {
    for forward in [false, true] {
        let source = "<table><tr><td><h2>A</h2><p>B</p><h1>C</h1></td><td><p>X</p><h3>Y</h3></td></tr></table>\r\nTAIL";
        let mut doc = EditorDocument::new(source);
        let snapshot = doc.snapshot();
        let needles = if forward {
            ["</h2>", "</p><h1>", "</p><h3>", "TAIL"]
        } else {
            ["B</p>", "C</h1>", "Y</h3>", "AIL"]
        };
        let original = needles.map(|needle| {
            EditorSelection::cursor(
                &snapshot,
                ByteOffset::new(source.find(needle).expect("caret") as u64),
                CaretAffinity::Downstream,
            )
            .expect("cursor")
        });
        doc.set_selections(original, 2).expect("multicarets");
        let selections = doc.selections().clone();
        let revision = doc.revision();
        doc.execute(if forward {
            EditorCommand::DeleteForward
        } else {
            EditorCommand::DeleteBackward
        })
        .expect("multicaret merge");
        let expected = "<table><tr><td><h2>ABC</h2></td><td><p>XY</p></td></tr></table>\r\nAIL";
        assert_eq!(doc.snapshot().as_str(), expected);
        assert_ne!(doc.revision(), revision);
        assert_eq!(doc.selections().len(), 4);
        assert_eq!(doc.selections().primary_index(), 2);
        for (selection, needle) in doc
            .selections()
            .as_slice()
            .iter()
            .zip(["BC", "C</h2>", "Y</p>", "AIL"])
        {
            assert_eq!(
                selection.focus().get() as usize,
                expected.find(needle).expect("join caret")
            );
        }
        doc.undo().expect("undo entire merge");
        assert_eq!(doc.snapshot().as_str(), source);
        assert_eq!(
            doc.selections()
                .as_slice()
                .iter()
                .map(|s| s.focus())
                .collect::<Vec<_>>(),
            selections
                .as_slice()
                .iter()
                .map(|s| s.focus())
                .collect::<Vec<_>>()
        );
        assert_eq!(doc.selections().primary_index(), 2);
        doc.redo().expect("redo entire merge");
        assert_eq!(doc.snapshot().as_str(), expected);
    }
}

#[test]
fn html_boundary_merge_keeps_a_retyped_closing_tag_inside_another_selection() {
    let source = "<table><tr><td><h2>A</h2><p>BC</p><p>D</p></td></tr></table>";
    let mut doc = EditorDocument::new(source);
    let snapshot = doc.snapshot();
    let start = source
        .find("B</")
        .unwrap_or_else(|| source.find("BC").expect("B"));
    let end = source.find("<p>D").expect("D paragraph");
    doc.set_selections(
        [
            EditorSelection::cursor(
                &snapshot,
                ByteOffset::new(start as u64),
                CaretAffinity::Downstream,
            )
            .expect("join cursor"),
            EditorSelection::range(
                &snapshot,
                ByteOffset::new((start + 1) as u64),
                ByteOffset::new(end as u64),
                CaretAffinity::Downstream,
            )
            .expect("selected suffix and closing tag"),
        ],
        0,
    )
    .expect("selections");
    doc.execute(EditorCommand::DeleteBackward)
        .expect("join plus selection delete");
    assert_eq!(
        doc.snapshot().as_str(),
        "<table><tr><td><h2>AB</h2><p>D</p></td></tr></table>"
    );
    doc.undo().expect("undo");
    assert_eq!(doc.snapshot().as_str(), source);
}

#[test]
fn html_heading_boundary_merge_handles_empty_blocks_and_keeps_source_only_tables_raw() {
    for (left, right) in [("", "中文"), ("中文", ""), ("", "")] {
        for forward in [false, true] {
            let source = format!("<table><tr><td><h1>{left}</h1><p>{right}</p></td></tr></table>");
            let mut doc = EditorDocument::new(&source);
            let at = if forward {
                source.find("</h1>").expect("left end")
            } else {
                source.find("<p>").expect("right start") + 3
            };
            doc.set_selection(
                EditorSelection::cursor(
                    &doc.snapshot(),
                    ByteOffset::new(at as u64),
                    CaretAffinity::Downstream,
                )
                .expect("boundary cursor"),
            )
            .expect("selection");
            doc.execute(if forward {
                EditorCommand::DeleteForward
            } else {
                EditorCommand::DeleteBackward
            })
            .expect("empty boundary merge");
            assert_eq!(
                doc.snapshot().as_str(),
                format!("<table><tr><td><h1>{left}{right}</h1></td></tr></table>")
            );
            doc.undo().expect("restore empty block");
            assert_eq!(doc.snapshot().as_str(), source);
        }
    }
    let source =
        "<table><tr><td><h1>A</h1><p>B</p></td><td><unknown>raw</unknown></td></tr></table>";
    let mut doc = EditorDocument::new(source);
    focus(&mut doc, "B</p>");
    doc.execute(EditorCommand::DeleteBackward)
        .expect("raw deletion");
    assert_eq!(doc.snapshot().as_str(), source.replace("<p>B", "<pB"));
    doc.undo().expect("raw undo");
    assert_eq!(doc.snapshot().as_str(), source);
}

#[test]
fn html_cell_list_content_clipboard_and_history_preserve_structure() {
    let source = "<table><tr><td><ol start='3'><li>one<ul><li>nested</li></ul></li><li>two</li></ol></td><td>KEEP</td></tr></table>\r\n";
    let mut doc = EditorDocument::new(source);
    focus(&mut doc, "nested");
    doc.execute(EditorCommand::insert_text("新<&"))
        .expect("list typing");
    assert_eq!(
        doc.snapshot().as_str(),
        source.replace("nested", "新&lt;&amp;nested")
    );
    doc.undo().expect("undo");
    assert_eq!(doc.snapshot().as_str(), source);
    doc.execute(EditorCommand::SelectTableCells {
        anchor: ByteOffset::new(source.find("one").expect("one") as u64),
        focus: ByteOffset::new(source.find("KEEP").expect("keep") as u64),
    })
    .expect("select");
    assert_eq!(
        doc.copy_table_tsv().expect("copy").as_deref(),
        Some("\"3. one\n  • nested\n4. two\"\tKEEP")
    );
    doc.execute(EditorCommand::DeleteSelections).expect("clear");
    assert_eq!(
        doc.snapshot().as_str(),
        "<table><tr><td></td><td></td></tr></table>\r\n"
    );
    doc.undo().expect("restore");
    assert_eq!(doc.snapshot().as_str(), source);
}

#[test]
fn unified_single_cell_paste_preserves_tags_and_has_no_redundant_revision() {
    let source = "<table><tr><td id='keep' align='right'><b>OLD</b></td><!--gap--><td>OTHER</td></tr></table>\r\n";
    let expected = "<table><tr><td id='keep' align='right'>中文🙂&lt;&amp;<br>尾</td><!--gap--><td>OTHER</td></tr></table>\r\n";
    let mut document = EditorDocument::new(source);
    focus(&mut document, "OLD");
    let command = || EditorCommand::PasteTableGrid {
        columns: 1,
        cells: vec!["中文🙂<&\r\n尾".into()],
    };
    document.execute(command()).expect("single cell paste");
    assert_eq!(document.snapshot().as_str(), expected);
    let revision = document.revision();
    document.execute(command()).expect("same value paste");
    assert_eq!(document.revision(), revision);
    document.undo().expect("undo one paste");
    assert_eq!(document.snapshot().as_str(), source);
    document.redo().expect("redo one paste");
    assert_eq!(document.snapshot().as_str(), expected);
}

#[test]
fn native_merged_table_selection_copy_clear_fill_and_paste_have_atomic_history() {
    let source = "<table><tr><td id='a' colspan='2' rowspan='2'>中文🙂</td><td>B</td></tr><tr><td>C</td></tr></table>\r\n";
    let mut doc = EditorDocument::new(source);
    let a = ByteOffset::new(source.find("中文").expect("A") as u64);
    let c = ByteOffset::new(source.find(">C<").expect("C") as u64 + 1);
    doc.execute(EditorCommand::SelectTableCells {
        anchor: a,
        focus: c,
    })
    .expect("select merged rectangle");
    let selected = doc.selections().clone();
    assert_eq!(selected.as_slice().len(), 3);
    assert_eq!(
        selected.table_slots(),
        Some([Some(0), Some(0), Some(1), Some(0), Some(0), Some(2)].as_slice())
    );
    assert_eq!(
        doc.copy_table_tsv().expect("copy").as_deref(),
        Some("中文🙂\t\tB\n\t\tC")
    );
    doc.execute(EditorCommand::DeleteSelections).expect("clear");
    assert_eq!(
        doc.snapshot().as_str(),
        source
            .replace("中文🙂", "")
            .replace(">B<", "><")
            .replace(">C<", "><")
    );
    assert_eq!(doc.selections().table_slots(), selected.table_slots());
    doc.undo().expect("undo clear");
    assert_eq!(doc.snapshot().as_str(), source);
    doc.execute(EditorCommand::PasteTableGrid {
        columns: 1,
        cells: vec!["新<&".into()],
    })
    .expect("fill");
    let filled = source
        .replace("中文🙂", "新&lt;&amp;")
        .replace(">B<", ">新&lt;&amp;<")
        .replace(">C<", ">新&lt;&amp;<");
    assert_eq!(doc.snapshot().as_str(), filled);
    assert_eq!(doc.selections().table_slots(), selected.table_slots());
    doc.undo().expect("undo fill");
    assert_eq!(doc.snapshot().as_str(), source);
    doc.execute(EditorCommand::PasteTableGrid {
        columns: 3,
        cells: ["a", "b", "c", "d", "e", "f"].map(Into::into).to_vec(),
    })
    .expect("split and paste");
    let pasted = doc.snapshot().as_str().to_owned();
    assert!(!pasted.contains("colspan"));
    assert!(!pasted.contains("rowspan"));
    assert_eq!(pasted.matches("id='a'").count(), 1);
    assert_eq!(
        doc.copy_table_tsv().expect("copy result").as_deref(),
        Some("a\tb\tc\nd\te\tf")
    );
    doc.undo().expect("undo one paste");
    assert_eq!(doc.snapshot().as_str(), source);
    assert_eq!(doc.selections().table_slots(), selected.table_slots());
    doc.redo().expect("redo one paste");
    assert_eq!(doc.snapshot().as_str(), pasted);
}

#[test]
fn native_merged_table_structure_commands_preserve_owners_and_source() {
    let source = "<table><tr><td id='a' colspan='2' rowspan='2'>中文🙂</td><td>B</td></tr><tr><td>C</td></tr></table>\r\n";
    for (target, edit, expected) in [
        (
            "C",
            TableEdit::AlignRight,
            source
                .replace("<td>B", "<td align=\"right\">B")
                .replace("<td>C", "<td align=\"right\">C"),
        ),
        (
            "中文",
            TableEdit::InsertRowAfter,
            source
                .replace("rowspan='2'", "rowspan='3'")
                .replace("</tr><tr><td>C", "</tr><tr><td></td></tr><tr><td>C"),
        ),
        (
            "中文",
            TableEdit::DeleteRow,
            "<table><tr><td id='a' colspan='2' rowspan='1'>中文🙂</td><td>C</td></tr></table>\r\n"
                .to_owned(),
        ),
        (
            "C",
            TableEdit::DeleteColumn,
            source.replace("<td>B</td>", "").replace("<td>C</td>", ""),
        ),
    ] {
        let mut doc = EditorDocument::new(source);
        focus(&mut doc, target);
        assert!(doc.command_available(&EditorCommand::EditTable(edit)));
        doc.execute(EditorCommand::EditTable(edit))
            .expect("structure command");
        assert_eq!(doc.snapshot().as_str(), expected, "{edit:?}");
        doc.undo().expect("undo");
        assert_eq!(doc.snapshot().as_str(), source);
        doc.redo().expect("redo");
        assert_eq!(doc.snapshot().as_str(), expected);
    }
}

#[test]
fn structured_html_table_paste_preserves_spans_and_atomic_history() {
    let payload = "<table><tr><td colspan='2' rowspan='2'><b>新🙂</b></td><td>X</td></tr><tr><td>Y</td></tr></table>";
    let mut empty = EditorDocument::new("");
    empty
        .execute(EditorCommand::PasteHtmlTableSource(payload.into()))
        .expect("new document paste");
    assert_eq!(empty.snapshot().as_str(), payload);
    empty.undo().expect("undo");
    assert_eq!(empty.snapshot().as_str(), "");
    let source = "<table><tr><td>A</td><td>B</td></tr></table>\r\n";
    let mut doc = EditorDocument::new(source);
    doc.execute(EditorCommand::SelectTableCells {
        anchor: ByteOffset::new(source.find(">A<").expect("A") as u64 + 1),
        focus: ByteOffset::new(source.find(">B<").expect("B") as u64 + 1),
    })
    .expect("whole table");
    doc.execute(EditorCommand::PasteHtmlTableSource(payload.into()))
        .expect("replace complete grid");
    assert_eq!(doc.snapshot().as_str(), format!("{payload}\r\n"));
    assert_eq!(doc.selections().as_slice().len(), 3);
    doc.undo().expect("undo replacement");
    assert_eq!(doc.snapshot().as_str(), source);
    doc.redo().expect("redo replacement");
    assert_eq!(doc.snapshot().as_str(), format!("{payload}\r\n"));
    let mut invalid = EditorDocument::new("");
    assert!(
        invalid
            .execute(EditorCommand::PasteHtmlTableSource(
                "<table><tr><td><script>bad()</script></td></tr></table>".into()
            ))
            .is_err()
    );
    assert!(invalid.snapshot().as_str().is_empty());
}

#[test]
fn merged_html_copy_preserves_cell_attributes_and_normalizes_group_spans() {
    let source = "<table><tbody><tr><th id='title' style='text-align:right' colspan='2' rowspan='0'><b>中文🙂</b></th><td title='hint'>B</td></tr><tr><td>C</td></tr></tbody><tbody><tr><td>OUTSIDE</td></tr></tbody></table>\r\n";
    let mut doc = EditorDocument::new(source);
    doc.execute(EditorCommand::SelectTableCells {
        anchor: ByteOffset::new(source.find("中文").expect("A") as u64),
        focus: ByteOffset::new(source.find(">C<").expect("C") as u64 + 1),
    })
    .expect("copy region");
    let copied = doc.copy_html_table_source().expect("copy").expect("html");
    assert_eq!(
        copied,
        "<table><tr><th id='title' style='text-align:right' colspan=\"2\" rowspan=\"2\"><b>中文🙂</b></th><td title='hint'>B</td></tr><tr><td>C</td></tr></table>"
    );
    assert_eq!(doc.snapshot().as_str(), source);
    let mut target = EditorDocument::new("");
    target
        .execute(EditorCommand::PasteHtmlTableSource(copied.clone().into()))
        .expect("structured paste");
    assert_eq!(target.snapshot().as_str(), copied);
    target.undo().expect("undo");
    assert!(target.snapshot().as_str().is_empty());
    target.redo().expect("redo");
    assert_eq!(target.snapshot().as_str(), copied);
}

#[test]
fn html_copy_materializes_inherited_alignment_for_copied_cells() {
    let source =
        "<table align='center'><tr><td colspan='2'>A</td><td align='left'>B</td></tr></table>";
    let mut doc = EditorDocument::new(source);
    doc.execute(EditorCommand::SelectTableCells {
        anchor: ByteOffset::new(source.find(">A<").expect("A") as u64 + 1),
        focus: ByteOffset::new(source.find(">B<").expect("B") as u64 + 1),
    })
    .expect("select");
    assert_eq!(
        doc.copy_html_table_source().expect("copy").as_deref(),
        Some(
            "<table><tr><td align=\"center\" colspan=\"2\">A</td><td align='left'>B</td></tr></table>"
        )
    );
    assert_eq!(doc.snapshot().as_str(), source);
}

#[test]
fn structured_paste_overlays_partial_html_region_and_preserves_neighbors() {
    let source = "prefix\r\n\r\n<table><tr><td>A</td><td>B</td><td>C</td></tr><tr><td>D</td><td>E</td><td>F</td></tr><tr><td>G</td><td>H</td><td>I</td></tr></table>\r\n\r\nsuffix";
    let payload = "<table><tr><th id='new' colspan='2' rowspan='0' align='right'><b>中文🙂</b></th></tr><tr></tr></table>";
    let expected = "prefix\r\n\r\n<table><tr><td>A</td><th id='new' colspan='2' rowspan='2' align='right'><b>中文🙂</b></th></tr><tr><td>D</td></tr><tr><td>G</td><td>H</td><td>I</td></tr></table>\r\n\r\nsuffix";
    let mut doc = EditorDocument::new(source);
    focus(&mut doc, "B</td>");
    let command = EditorCommand::PasteHtmlTableSource(payload.into());
    assert!(doc.command_available(&command));
    doc.execute(command).expect("partial structured paste");
    assert_eq!(doc.snapshot().as_str(), expected);
    assert_eq!(doc.selections().as_slice().len(), 1);
    assert_eq!(doc.selections().table_columns(), Some(2));
    assert_eq!(
        doc.selections().table_slots(),
        Some([Some(0); 4].as_slice())
    );
    doc.undo().expect("undo once");
    assert_eq!(doc.snapshot().as_str(), source);
    doc.redo().expect("redo once");
    assert_eq!(doc.snapshot().as_str(), expected);
}

#[test]
fn structured_partial_paste_rejects_cross_group_span_without_mutation() {
    let source = "<table><tbody><tr><td>A</td><td>B</td></tr></tbody><tbody><tr><td>C</td><td>D</td></tr></tbody></table>";
    let payload = "<table><tr><td rowspan='2'>X</td></tr><tr></tr></table>";
    let mut doc = EditorDocument::new(source);
    focus(&mut doc, "A</td>");
    let revision = doc.revision();
    assert!(
        doc.execute(EditorCommand::PasteHtmlTableSource(payload.into()))
            .is_err()
    );
    assert_eq!(doc.snapshot().as_str(), source);
    assert_eq!(doc.revision(), revision);
}

#[test]
fn structured_paste_promotes_markdown_table_preserving_context_and_single_history() {
    let prefix = "\u{feff}KEEP🙂\r\n\r\n";
    let suffix = "\r\n\r\n[site]: https://example.com \"Reference\"\r\n\r\nTAIL";
    let source = format!(
        "{prefix}| Header | Target | More |\r\n| :--- | ---: | --- |\r\n| [label][site] | old | old2 |\r\n| **bold** | keep | end |{suffix}"
    );
    let payload = "<table><tr><td colspan='2' rowspan='2'>中文🙂</td></tr><tr></tr></table>";
    let mut doc = EditorDocument::new(&source);
    focus(&mut doc, "Target");
    let command = EditorCommand::PasteHtmlTableSource(payload.into());
    assert!(doc.command_available(&command));
    doc.execute(command).expect("promote and paste");
    let actual = doc.snapshot().as_str().to_owned();
    assert!(actual.starts_with(prefix), "{actual}");
    assert!(actual.ends_with(suffix), "{actual}");
    assert!(
        actual.contains("<td colspan='2' rowspan='2'>中文🙂</td>"),
        "{actual}"
    );
    assert!(
        actual.contains("<a href=\"https://example.com\" title=\"Reference\">label</a>"),
        "{actual}"
    );
    assert!(actual.contains("<strong>bold</strong>"), "{actual}");
    assert!(actual.contains("align=\"left\""), "{actual}");
    assert!(!actual.contains("Target"));
    assert!(!actual.contains(">old<"));
    assert!(!actual.contains(">old2<"));
    assert_eq!(doc.selections().table_columns(), Some(2));
    assert_eq!(
        doc.selections().table_slots(),
        Some([Some(0); 4].as_slice())
    );
    doc.undo().expect("one undo");
    assert_eq!(doc.snapshot().as_str(), source);
    doc.redo().expect("one redo");
    assert_eq!(doc.snapshot().as_str(), actual);
}

#[test]
fn markdown_promotion_retains_writing_marks_and_rejects_unsupported_embeds_atomically() {
    let payload = "<table><tr><td colspan='2'>merged</td></tr></table>";
    let source = "| H | TARGET |\n| --- | --- |\n| ==mark== H~2~O x^2^ | keep |\n";
    let mut doc = EditorDocument::new(source);
    focus(&mut doc, "TARGET");
    doc.execute(EditorCommand::PasteHtmlTableSource(payload.into()))
        .expect("marks");
    let actual = doc.snapshot();
    assert!(actual.as_str().contains("<mark>mark</mark>"));
    assert!(actual.as_str().contains("H<sub>2</sub>O"));
    assert!(actual.as_str().contains("x<sup>2</sup>"));
    for content in ["$x^2$", "[^ref]"] {
        let source =
            format!("| H | TARGET |\n| --- | --- |\n| {content} | keep |\n\n[^ref]: note\n");
        let mut doc = EditorDocument::new(&source);
        focus(&mut doc, "TARGET");
        let before = doc.selection();
        doc.execute(EditorCommand::PasteHtmlTableSource(payload.into()))
            .expect("supported source leaf");
        let converted = doc.snapshot().as_str().to_owned();
        assert!(converted.ends_with("[^ref]: note\n"));
        if content.starts_with('$') {
            let spans: Vec<_> = doc
                .markdown()
                .html_regions()
                .regions
                .iter()
                .filter_map(|region| region.model.as_ref().ok())
                .flat_map(|model| model.inline_math_spans())
                .collect();
            assert_eq!(spans.len(), 1);
            assert_eq!(
                doc.markdown().embedded_source(spans[0]).as_deref(),
                Some("x^2")
            );
        } else {
            let index = doc.markdown().footnotes();
            assert_eq!(index.references().len(), 1);
            assert_eq!(index.references()[0].number, Some(1));
            assert_eq!(
                index.navigation_target(index.references()[0].source.start()),
                Some(index.definitions()[0].source)
            );
        }
        doc.undo().expect("replay source-leaf promotion");
        assert_eq!(doc.snapshot().as_str(), source);
        assert_eq!(doc.selection().anchor(), before.anchor());
        assert_eq!(doc.selection().focus(), before.focus());
        assert_eq!(doc.selection().affinity(), before.affinity());
        doc.redo().expect("replay source-leaf promotion");
        assert_eq!(doc.snapshot().as_str(), converted);
    }
    for content in ["<span data-math-style='display'>x^2</span>", "[^missing]"] {
        let source =
            format!("| H | TARGET |\n| --- | --- |\n| {content} | keep |\n\n[^ref]: note\n");
        let mut doc = EditorDocument::new(&source);
        focus(&mut doc, "TARGET");
        let revision = doc.revision();
        assert!(
            doc.execute(EditorCommand::PasteHtmlTableSource(payload.into()))
                .is_err()
        );
        assert_eq!(doc.snapshot().as_str(), source);
        assert_eq!(doc.revision(), revision);
    }
}
