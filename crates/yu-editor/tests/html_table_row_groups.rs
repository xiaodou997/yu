//! Row-group overlay contracts. Core/geometry checks are not native window acceptance.
use yu_core::{ByteOffset, TextRange};
use yu_editor::{
    CaretAffinity, EditorCommand, EditorDocument, EditorDocumentError, EditorSelection, Selections,
};
use yu_markdown::html::{HtmlNodeKind, HtmlTable, HtmlTableError};

fn variants(source: &str) -> Vec<String> {
    ["\n", "\r\n"]
        .into_iter()
        .flat_map(|eol| {
            ["", "\u{feff}"]
                .into_iter()
                .map(move |bom| format!("{bom}{}", source.replace("\r\n", "\n").replace('\n', eol)))
        })
        .collect()
}

fn table(document: &EditorDocument) -> HtmlTable {
    document
        .markdown()
        .html_regions()
        .regions
        .iter()
        .filter_map(|r| r.model.as_ref().ok())
        .find_map(|m| {
            m.partitions
                .iter()
                .filter_map(|p| p.content.owner)
                .find_map(|owner| m.native_table(owner).ok())
        })
        .expect("native table")
}

fn at(document: &EditorDocument, row: usize, column: usize) -> ByteOffset {
    let table = table(document);
    let owner = table.grid.cells[table.grid.slots[row * table.columns + column].expect("slot")];
    table.rows[owner.source_row].cells[owner.source_cell]
        .content
        .start()
}

fn cursor(document: &mut EditorDocument, position: ByteOffset) {
    document
        .set_selection(
            EditorSelection::cursor(&document.snapshot(), position, CaretAffinity::Downstream)
                .expect("cursor"),
        )
        .expect("set cursor");
}

fn rectangle(document: &mut EditorDocument, first: (usize, usize), last: (usize, usize)) {
    let anchor = at(document, first.0, first.1);
    let focus = at(document, last.0, last.1);
    document
        .execute(EditorCommand::SelectTableCells { anchor, focus })
        .expect("rectangle");
}

struct Frame {
    source: String,
    selections: Selections,
}
impl Frame {
    fn capture(document: &EditorDocument) -> Self {
        Self {
            source: document.snapshot().as_str().to_owned(),
            selections: document.selections().clone(),
        }
    }
    fn assert_replayed(&self, document: &EditorDocument) {
        let snapshot = document.snapshot();
        assert_eq!(snapshot.as_str(), self.source);
        let ranges = self.selections.as_slice().iter().map(|s| {
            EditorSelection::range(&snapshot, s.anchor(), s.focus(), s.affinity())
                .expect("saved range")
        });
        let mut expected = Selections::new(&snapshot, ranges, self.selections.primary_index())
            .expect("selections");
        if let Some(columns) = self.selections.table_columns() {
            expected = expected
                .with_table_slots(
                    columns,
                    self.selections.table_slots().expect("slots").to_vec(),
                )
                .expect("grid ownership");
        }
        assert_eq!(document.selections(), &expected);
    }
}

fn seeded(source: &str) -> (EditorDocument, [Frame; 3]) {
    let mut document = EditorDocument::new(source);
    let end = document.snapshot().len_bytes();
    cursor(&mut document, end);
    let original = Frame::capture(&document);
    document
        .execute(EditorCommand::insert_text(" HISTORY-A中文🙂"))
        .expect("first edit");
    let first = Frame::capture(&document);
    let end = document.snapshot().len_bytes();
    cursor(&mut document, end);
    document
        .execute(EditorCommand::insert_text(" HISTORY-B中文🙂"))
        .expect("second edit");
    let second = Frame::capture(&document);
    document.undo().expect("live redo");
    assert_eq!(document.history_stats().undo_entries(), 1);
    assert_eq!(document.history_stats().redo_entries(), 1);
    (document, [original, first, second])
}

fn reject(document: &mut EditorDocument, command: EditorCommand) {
    let before = Frame::capture(document);
    let revision = document.revision();
    let history = document.history_stats();
    for _ in 0..3 {
        assert!(!document.command_available(&command));
        assert_eq!(document.revision(), revision);
        assert_eq!(document.history_stats(), history);
        before.assert_replayed(document);
        assert!(matches!(
            document.execute(command.clone()),
            Err(EditorDocumentError::InvalidTablePaste)
        ));
        assert_eq!(document.revision(), revision);
        assert_eq!(document.history_stats(), history);
        assert_eq!(document.selections(), &before.selections);
        before.assert_replayed(document);
    }
}

fn replay_original(document: &mut EditorDocument, frames: &[Frame; 3]) {
    for (command, index, undo, redo) in [
        (EditorCommand::Redo, 2, 2, 0),
        (EditorCommand::Undo, 1, 1, 1),
        (EditorCommand::Undo, 0, 0, 2),
        (EditorCommand::Redo, 1, 1, 1),
        (EditorCommand::Redo, 2, 2, 0),
    ] {
        assert!(document.execute(command).expect("history replay").changed());
        frames[index].assert_replayed(document);
        assert_eq!(document.history_stats().undo_entries(), undo);
        assert_eq!(document.history_stats().redo_entries(), redo);
    }
}

fn paste(document: &mut EditorDocument, payload: &str) -> (Frame, Frame) {
    let before = Frame::capture(document);
    let revision = document.revision();
    let history = document.history_stats();
    let command = EditorCommand::PasteHtmlTableSource(payload.into());
    for _ in 0..3 {
        assert!(
            document.command_available(&command),
            "{payload}\n{}",
            before.source
        );
        assert_eq!(document.revision(), revision);
        assert_eq!(document.history_stats(), history);
        before.assert_replayed(document);
    }
    assert!(document.execute(command).expect("legal overlay").changed());
    assert_eq!(
        document.history_stats().undo_entries(),
        history.undo_entries() + 1
    );
    assert_eq!(document.history_stats().redo_entries(), 0);
    (before, Frame::capture(document))
}

fn replay_paste(document: &mut EditorDocument, before: &Frame, after: &Frame) {
    assert!(document.undo().expect("one undo").changed());
    before.assert_replayed(document);
    assert!(document.redo().expect("one redo").changed());
    after.assert_replayed(document);
}

fn source(groups: &[(&str, usize)], columns: usize) -> String {
    let mut text = String::from("KEEP中文🙂\n\n<table id='target'>\n");
    let mut row = 0;
    for (group, &(tag, count)) in groups.iter().enumerate() {
        text.push_str(&format!("<{tag} id='group-{group}'>\n"));
        for _ in 0..count {
            text.push_str(&format!("<tr title='row-{row}'>"));
            for column in 0..columns {
                text.push_str(&format!("<td>r{row}c{column}中文🙂</td>"));
            }
            text.push_str("</tr>\n");
            row += 1;
        }
        text.push_str(&format!("</{tag}>\n"));
    }
    text.push_str("</table>\n\nTAIL");
    text
}

fn skeleton(document: &EditorDocument) -> Vec<String> {
    let snapshot = document.snapshot();
    document
        .markdown()
        .html_regions()
        .regions
        .iter()
        .filter_map(|r| r.model.as_ref().ok())
        .flat_map(|m| &m.fragment.nodes)
        .filter_map(|node| match &node.kind {
            HtmlNodeKind::Element { opening, closing }
                if matches!(
                    opening.name.as_str(),
                    "table" | "thead" | "tbody" | "tfoot" | "tr"
                ) =>
            {
                let slice = |r: TextRange| {
                    &snapshot.as_str()[r.start().get() as usize..r.end().get() as usize]
                };
                Some(format!(
                    "{}{}",
                    slice(opening.source),
                    closing.as_ref().map_or("", |c| slice(c.source))
                ))
            }
            _ => None,
        })
        .collect()
}

fn assert_overlay(document: &EditorDocument, start: (usize, usize), payload: &str) {
    let incoming = table(&EditorDocument::new(payload));
    let result = table(document);
    let snapshot = document.snapshot();
    for expected in &incoming.grid.cells {
        let row = start.0 + expected.source_row;
        let column = start.1 + expected.column;
        let actual = result.grid.cells
            [result.grid.slots[row * result.columns + column].expect("pasted owner")];
        assert_eq!(
            (
                actual.source_row,
                actual.column,
                actual.rows,
                actual.columns
            ),
            (row, column, expected.rows, expected.columns)
        );
        let cell = &result.rows[actual.source_row].cells[actual.source_cell];
        let original = &incoming.rows[expected.source_row].cells[expected.source_cell];
        assert_eq!(cell.header, original.header);
        assert_eq!(
            &snapshot.as_str()
                [cell.content.start().get() as usize..cell.content.end().get() as usize],
            &payload
                [original.content.start().get() as usize..original.content.end().get() as usize]
        );
    }
}

#[test]
fn legal_group_pairs_and_triples_preserve_wrappers_and_owner_geometry() {
    for groups in [
        vec![("thead", 1), ("tbody", 1)],
        vec![("tbody", 1), ("tbody", 1)],
        vec![("tbody", 1), ("tfoot", 1)],
        vec![("thead", 1), ("tbody", 1), ("tfoot", 1)],
    ] {
        let payload = format!(
            "<table>{}</table>",
            (0..groups.len())
                .map(|row| format!("<tr><th colspan='2'><b>新{row}中文🙂</b></th></tr>"))
                .collect::<String>()
        );
        for source in variants(&source(&groups, 3)) {
            for backward in [false, true] {
                let (mut document, _) = seeded(&source);
                let last = (groups.len() - 1, 1);
                rectangle(
                    &mut document,
                    if backward { last } else { (0, 0) },
                    if backward { (0, 0) } else { last },
                );
                let structure = skeleton(&document);
                let (before, after) = paste(&mut document, &payload);
                assert_eq!(skeleton(&document), structure);
                assert_overlay(&document, (0, 0), &payload);
                for row in 0..groups.len() {
                    assert!(after.source.contains(&format!("r{row}c2中文🙂")));
                }
                assert_eq!(document.selections().table_columns(), Some(2));
                let expected_slots: Vec<_> =
                    (0..groups.len()).flat_map(|row| [Some(row); 2]).collect();
                assert_eq!(
                    document.selections().table_slots(),
                    Some(expected_slots.as_slice())
                );
                replay_paste(&mut document, &before, &after);
            }
        }
    }
}

#[test]
fn separate_rowspans_can_cross_the_region_but_not_a_target_group_boundary() {
    let payload = "<table><tbody id='clipboard'><tr><td rowspan='2' colspan='2'>甲</td></tr><tr></tr><tr><td rowspan='2' colspan='2'>乙</td></tr><tr></tr></tbody></table>";
    for groups in [
        vec![("thead", 2), ("tbody", 2)],
        vec![("tbody", 2), ("tbody", 2)],
        vec![("tbody", 2), ("tfoot", 2)],
    ] {
        let mut document = EditorDocument::new(source(&groups, 3));
        let pos = at(&document, 0, 0);
        cursor(&mut document, pos);
        let structure = skeleton(&document);
        let (before, after) = paste(&mut document, payload);
        assert_overlay(&document, (0, 0), payload);
        assert_eq!(skeleton(&document), structure);
        assert!(!after.source.contains("clipboard"));
        assert_eq!(
            document.selections().table_slots(),
            Some(
                [
                    Some(0),
                    Some(0),
                    Some(0),
                    Some(0),
                    Some(1),
                    Some(1),
                    Some(1),
                    Some(1)
                ]
                .as_slice()
            )
        );
        replay_paste(&mut document, &before, &after);
    }
}

#[test]
fn grouped_whole_grid_selection_keeps_groups_empty_groups_and_uncovered_rows() {
    for backward in [false, true] {
        let source = source(&[("thead", 1), ("tbody", 0), ("tbody", 1), ("tfoot", 0)], 2);
        let mut document = EditorDocument::new(&source);
        rectangle(
            &mut document,
            if backward { (1, 1) } else { (0, 0) },
            if backward { (0, 0) } else { (1, 1) },
        );
        let structure = skeleton(&document);
        let payload = "<table id='foreign'><tr><td colspan='2'>new</td></tr></table>";
        let (before, after) = paste(&mut document, payload);
        assert_eq!(skeleton(&document), structure);
        assert_overlay(&document, (0, 0), payload);
        assert!(after.source.contains("r1c0中文🙂"));
        assert!(!after.source.contains("foreign"));
        replay_paste(&mut document, &before, &after);
    }
}

#[test]
fn full_or_partial_grid_selection_cannot_bypass_cross_group_rejection_and_history() {
    let payload = "<table><tr><td colspan='2' rowspan='2'>不能跨组</td></tr><tr></tr></table>";
    for columns in [2, 3] {
        for source in variants(&source(&[("thead", 1), ("tbody", 1)], columns)) {
            for kind in 0..6 {
                let (mut document, frames) = seeded(&source);
                let a = at(&document, 0, 0);
                let b = at(&document, 1, 1);
                match kind {
                    0 => cursor(&mut document, a),
                    1 | 2 => {
                        let end = ByteOffset::new(a.get() + 4);
                        let (anchor, focus) = if kind == 1 { (a, end) } else { (end, a) };
                        document
                            .set_selection(
                                EditorSelection::range(
                                    &document.snapshot(),
                                    anchor,
                                    focus,
                                    if kind == 1 {
                                        CaretAffinity::Downstream
                                    } else {
                                        CaretAffinity::Upstream
                                    },
                                )
                                .expect("range"),
                            )
                            .expect("selection");
                    }
                    3 => rectangle(&mut document, (0, 0), (1, 1)),
                    4 => rectangle(&mut document, (1, 1), (0, 0)),
                    _ => {
                        let snapshot = document.snapshot();
                        document
                            .set_selections(
                                [
                                    EditorSelection::cursor(&snapshot, a, CaretAffinity::Upstream)
                                        .expect("a"),
                                    EditorSelection::cursor(
                                        &snapshot,
                                        b,
                                        CaretAffinity::Downstream,
                                    )
                                    .expect("b"),
                                ],
                                1,
                            )
                            .expect("multicaret");
                    }
                }
                reject(
                    &mut document,
                    EditorCommand::PasteHtmlTableSource(payload.into()),
                );
                replay_original(&mut document, &frames);
            }
        }
    }
}

#[test]
fn rowspan_acceptance_matches_target_group_extent_including_final_group_growth() {
    for first_rows in 1..=3 {
        for last_rows in 1..=3 {
            let source = source(&[("tbody", first_rows), ("tbody", last_rows)], 2);
            for start in 0..first_rows + last_rows {
                for height in 1..=4 {
                    let payload = format!(
                        "<table><tr><td rowspan='{height}'>X</td></tr>{}</table>",
                        "<tr></tr>".repeat(height - 1)
                    );
                    let mut document = EditorDocument::new(&source);
                    let pos = at(&document, start, 0);
                    cursor(&mut document, pos);
                    let legal = start >= first_rows || start + height <= first_rows;
                    let command = EditorCommand::PasteHtmlTableSource(payload.clone().into());
                    assert_eq!(
                        document.command_available(&command),
                        legal,
                        "{first_rows}/{last_rows} start={start}, height={height}"
                    );
                    if legal {
                        let (before, after) = paste(&mut document, &payload);
                        assert_overlay(&document, (start, 0), &payload);
                        replay_paste(&mut document, &before, &after);
                    } else {
                        let index = document.markdown().html_regions();
                        let model = index.regions[0].model.as_ref().expect("model");
                        let owner = model.partitions[0].content.owner.expect("owner");
                        assert_eq!(
                            model.paste_table_edit(&source, owner, (start, 0), &payload),
                            Err(HtmlTableError::CrossRowGroupSpan)
                        );
                        reject(&mut document, command);
                    }
                }
            }
        }
    }
}

#[test]
fn zero_and_oversized_incoming_spans_are_finite_without_being_split() {
    for declared in [0, 65534] {
        let payload = format!(
            "<table><thead><tr><th colspan='2' rowspan='{declared}'>上</th></tr><tr></tr></thead><tbody><tr><td colspan='2' rowspan='{declared}'>下</td></tr><tr></tr></tbody></table>"
        );
        let mut document = EditorDocument::new(source(&[("tbody", 2), ("tfoot", 3)], 3));
        let pos = at(&document, 0, 0);
        cursor(&mut document, pos);
        let structure = skeleton(&document);
        let (before, after) = paste(&mut document, &payload);
        assert_overlay(&document, (0, 0), &payload);
        assert_eq!(after.source.matches("rowspan='2'").count(), 2);
        assert_eq!(skeleton(&document), structure);
        let reopened = EditorDocument::new(after.source.clone());
        assert_overlay(&reopened, (0, 0), &payload);
        replay_paste(&mut document, &before, &after);
    }
}

#[test]
fn cross_group_growth_keeps_old_boundaries_and_appends_only_to_the_final_group() {
    let source = source(&[("thead", 1), ("tbody", 1), ("tfoot", 1)], 2);
    let payload = format!(
        "<table>{}</table>",
        (0..4)
            .map(|row| format!("<tr><td colspan='2'>新增{row}</td></tr>"))
            .collect::<String>()
    );
    let mut document = EditorDocument::new(&source);
    let pos = at(&document, 0, 1);
    cursor(&mut document, pos);
    let (before, after) = paste(&mut document, &payload);
    let result = table(&document);
    assert_eq!((result.rows.len(), result.columns), (4, 3));
    assert_ne!(result.rows[0].group, result.rows[1].group);
    assert_ne!(result.rows[1].group, result.rows[2].group);
    assert_eq!(result.rows[2].group, result.rows[3].group);
    assert_overlay(&document, (0, 1), &payload);
    for row in 0..3 {
        assert!(after.source.contains(&format!("r{row}c0中文🙂")));
    }
    replay_paste(&mut document, &before, &after);
}

#[test]
fn arbitrary_multicarets_and_ranges_through_group_tags_are_not_table_rectangles() {
    for across_tags in [false, true] {
        let source = source(&[("tbody", 1), ("tbody", 1)], 3);
        let (mut document, frames) = seeded(&source);
        let a = at(&document, 0, 0);
        let b = at(&document, 1, 0);
        let snapshot = document.snapshot();
        if across_tags {
            document
                .set_selection(
                    EditorSelection::range(&snapshot, b, a, CaretAffinity::Upstream)
                        .expect("raw range"),
                )
                .expect("set range");
        } else {
            document
                .set_selections(
                    [
                        EditorSelection::cursor(&snapshot, a, CaretAffinity::Upstream).expect("a"),
                        EditorSelection::cursor(&snapshot, b, CaretAffinity::Downstream)
                            .expect("b"),
                    ],
                    1,
                )
                .expect("multicaret");
        }
        reject(
            &mut document,
            EditorCommand::PasteHtmlTableSource(
                "<table><tr><td>X</td></tr><tr><td>Y</td></tr></table>".into(),
            ),
        );
        replay_original(&mut document, &frames);
    }
}

#[test]
fn identical_grouped_overlay_is_a_noop_and_keeps_the_old_redo_branch() {
    let source = "<table id='keep'><thead><tr><td>A</td></tr></thead><tbody><tr><td>B</td></tr></tbody></table>\n\nTAIL";
    let (mut document, frames) = seeded(source);
    rectangle(&mut document, (0, 0), (1, 0));
    let before = Frame::capture(&document);
    let history = document.history_stats();
    let revision = document.revision();
    let command = EditorCommand::PasteHtmlTableSource(
        "<table><tr><td>A</td></tr><tr><td>B</td></tr></table>".into(),
    );
    assert!(document.command_available(&command));
    assert!(
        !document
            .execute(command)
            .expect("identical overlay")
            .changed()
    );
    before.assert_replayed(&document);
    assert_eq!(document.history_stats(), history);
    assert_eq!(document.revision(), revision);
    replay_original(&mut document, &frames);
}

#[test]
fn copied_cross_group_merges_reopen_with_formulas_and_footnote_relationships() {
    let source =
        source(&[("thead", 1), ("tbody", 2)], 3).replace("TAIL", "TAIL\n\n[^note]: 外部定义。\n");
    let payload = "<table><tr><th colspan='2'>中文🙂<span data-yu-footnote='reference'>[^note]</span></th></tr><tr><td rowspan='2' colspan='2'><span data-math-style='inline'>x^2 &lt; y</span></td></tr><tr></tr></table>";
    for source in variants(&source) {
        let mut document = EditorDocument::new(&source);
        let pos = at(&document, 0, 0);
        cursor(&mut document, pos);
        let (before, after) = paste(&mut document, payload);
        assert_overlay(&document, (0, 0), payload);
        assert_eq!(
            document.markdown().footnotes().references()[0].number,
            Some(1)
        );
        assert!(
            document
                .markdown()
                .footnotes()
                .navigation_target(
                    document.markdown().footnotes().references()[0]
                        .source
                        .start()
                )
                .is_some()
        );
        let copied = document
            .copy_html_table_source()
            .expect("copy")
            .expect("HTML payload");
        let mut reopened = EditorDocument::new(
            String::from_utf8(after.source.as_bytes().to_vec()).expect("UTF-8 save representation"),
        );
        assert_eq!(skeleton(&reopened), skeleton(&document));
        let pos = at(&reopened, 0, 0);
        cursor(&mut reopened, pos);
        assert!(reopened.command_available(&EditorCommand::PasteHtmlTableSource(copied.into())));
        let spans: Vec<_> = reopened
            .markdown()
            .html_regions()
            .regions
            .iter()
            .filter_map(|r| r.model.as_ref().ok())
            .flat_map(|m| m.inline_math_spans())
            .collect();
        assert_eq!(spans.len(), 1);
        assert_eq!(
            reopened.markdown().embedded_source(spans[0]).as_deref(),
            Some("x^2 < y")
        );
        replay_paste(&mut document, &before, &after);
    }
}

#[test]
fn fixed_native_samples_share_the_same_success_and_rejection_contract() {
    let base = include_str!("fixtures/group4-paste/row-groups-target.md");
    let whole = include_str!("fixtures/group4-paste/row-groups-whole-target.md");
    let legal = include_str!("fixtures/group4-paste/row-groups-merged-payload.md");
    let conflict = include_str!("fixtures/group4-paste/row-groups-conflict-payload.md");
    let rows = include_str!("fixtures/group4-paste/row-groups-rows-payload.md");
    let crossing = include_str!("fixtures/group4-paste/merged-payload.md");
    for (source, payload, accept, whole_grid) in [
        (base.to_owned(), legal, true, false),
        (base.replace("thead", "tbody"), legal, true, false),
        (
            base.replace("tbody", "tfoot").replace("thead", "tbody"),
            legal,
            true,
            false,
        ),
        (base.to_owned(), conflict, false, false),
        (whole.to_owned(), rows, true, true),
        (whole.to_owned(), crossing, false, true),
    ] {
        for source in variants(&source) {
            let (mut document, frames) = seeded(&source);
            let target = table(&document);
            rectangle(&mut document, (target.rows.len() - 1, 1), (0, 0));
            if whole_grid {
                assert_eq!(document.selections().len(), target.grid.cells.len());
            }
            if accept {
                let structure = skeleton(&document);
                let (before, after) = paste(&mut document, payload);
                assert_eq!(skeleton(&document), structure);
                assert_overlay(&document, (0, 0), payload);
                if !whole_grid {
                    for text in [
                        "保留甲",
                        "保留乙",
                        "保留丙",
                        "保留丁",
                        "[^note]: 表格外定义不变。",
                    ] {
                        assert!(after.source.contains(text));
                    }
                    assert_eq!(
                        document.markdown().footnotes().references()[0].number,
                        Some(1)
                    );
                }
                assert_eq!(
                    after.source.starts_with('\u{feff}'),
                    source.starts_with('\u{feff}')
                );
                if source.contains("\r\n") {
                    assert!(!after.source.replace("\r\n", "").contains('\n'));
                }
                replay_paste(&mut document, &before, &after);
            } else {
                reject(
                    &mut document,
                    EditorCommand::PasteHtmlTableSource(payload.into()),
                );
                replay_original(&mut document, &frames);
            }
        }
    }
}

#[test]
fn rejected_span_does_not_poison_a_later_legal_cross_group_overlay() {
    for source in variants(&source(&[("thead", 1), ("tbody", 1)], 2)) {
        let (mut document, _) = seeded(&source);
        rectangle(&mut document, (1, 1), (0, 0));
        reject(
            &mut document,
            EditorCommand::PasteHtmlTableSource(
                "<table><tr><td colspan='2' rowspan='2'>bad</td></tr><tr></tr></table>".into(),
            ),
        );
        let legal =
            "<table><tr><td colspan='2'>good1</td></tr><tr><td colspan='2'>good2</td></tr></table>";
        let (before, after) = paste(&mut document, legal);
        assert_overlay(&document, (0, 0), legal);
        replay_paste(&mut document, &before, &after);
    }
}

#[test]
fn final_group_growth_keeps_unselected_zero_span_and_trailing_empty_group() {
    let source = "<table><thead><tr><th>H</th><th>h</th></tr></thead><tbody><tr><td rowspan='0'>keep</td><td>target</td></tr></tbody><tfoot id='empty'></tfoot></table>\n";
    let mut document = EditorDocument::new(source);
    let pos = at(&document, 1, 1);
    cursor(&mut document, pos);
    let payload = "<table><tr><td rowspan='3'>new</td></tr><tr></tr><tr></tr></table>";
    let (before, after) = paste(&mut document, payload);
    assert_overlay(&document, (1, 1), payload);
    let target = table(&document);
    let keep = target.grid.cells[target.grid.slots[target.columns].expect("unselected owner")];
    assert_eq!(
        (keep.source_row, keep.column, keep.rows, keep.columns),
        (1, 0, 3, 1)
    );
    assert!(after.source.contains("<td rowspan='0'>keep</td>"));
    assert!(after.source.contains("<tfoot id='empty'></tfoot>"));
    assert_eq!(target.rows[1].group, target.rows[3].group);
    replay_paste(&mut document, &before, &after);
}

#[test]
fn cross_group_native_cell_hits_keep_utf8_source_and_embedded_identity() {
    use yu_editor::{BlockView, LayoutConfig, LayoutPoint, MonospaceMetrics, VisualText};
    let source = include_str!("fixtures/group4-paste/row-groups-target.md");
    let payload = include_str!("fixtures/group4-paste/row-groups-merged-payload.md");
    let mut document = EditorDocument::new(source);
    let pos = at(&document, 0, 0);
    cursor(&mut document, pos);
    paste(&mut document, payload);
    let markdown = document.markdown();
    let index = markdown.html_regions();
    let block = markdown
        .blocks()
        .iter()
        .find(|block| {
            index
                .partition_for(block.range())
                .is_some_and(|part| part.content.kind == yu_markdown::html::HtmlFlowKind::Table)
        })
        .expect("HTML block");
    let decorations = index.decorate(markdown, block, None).expect("decorations");
    let snapshot = document.snapshot();
    let visual = VisualText::new(&snapshot, block.range(), decorations.set().clone())
        .expect("visual mapping");
    let view = BlockView::build(
        block.kind(),
        &visual,
        &decorations,
        LayoutConfig::new(700.0, 16.0),
        &MonospaceMetrics::new(8.0),
    )
    .expect("native geometry");
    assert_eq!(view.embedded().len(), 1);
    assert_eq!(
        markdown.embedded_source(view.embedded()[0].0).as_deref(),
        Some("x^2")
    );
    assert_eq!(view.table().expect("grid").cells().len(), 6);
    for cell in view.table().expect("grid").cells() {
        let bounds = cell.bounds();
        let hit = view
            .hit_test(LayoutPoint::new(
                bounds.x() + bounds.width() * 0.5,
                bounds.y() + bounds.height() * 0.5,
            ))
            .expect("cell hit");
        assert!(cell.source().start() <= hit.source() && hit.source() <= cell.source().end());
        assert!(
            snapshot
                .as_str()
                .is_char_boundary(hit.source().get() as usize)
        );
    }
}
