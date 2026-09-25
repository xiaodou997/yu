//! Rejected structured pastes must preserve a live undo/redo branch, not just text.
//! The same fixtures are prepared for native checks by tools/prepare-group4-paste-checks.py.
use yu_core::ByteOffset;
use yu_editor::{
    CaretAffinity, EditorCommand, EditorDocument, EditorDocumentError, EditorSelection, Selections,
};

const CROSS_GROUPS: &str = include_str!("fixtures/group4-paste/cross-groups.md");
const MATH: &str = include_str!("fixtures/group4-paste/math-target.md");
const FOOTNOTE: &str = include_str!("fixtures/group4-paste/footnote-target.md");
const PAYLOAD: &str = include_str!("fixtures/group4-paste/merged-payload.md");
const TARGET: &str = "目标中文🙂";
const RECT_END: &str = "矩形终点🙂";
const HISTORY_A: &str = " HISTORY-A-中文🙂";
const HISTORY_B: &str = " HISTORY-B-中文🙂";

#[derive(Clone, Copy, Debug)]
enum SelectionKind {
    Caret,
    Forward,
    Backward,
    RectangleForward,
    RectangleBackward,
    Multiple,
}

impl SelectionKind {
    const ALL: [Self; 6] = [
        Self::Caret,
        Self::Forward,
        Self::Backward,
        Self::RectangleForward,
        Self::RectangleBackward,
        Self::Multiple,
    ];
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
        // Undo/redo creates new revisions. Rebind only the revision; preserve
        // both endpoints, affinity, primary index and merged-cell ownership.
        let ranges = self.selections.as_slice().iter().map(|selection| {
            EditorSelection::range(
                &snapshot,
                selection.anchor(),
                selection.focus(),
                selection.affinity(),
            )
            .expect("saved source endpoints remain valid")
        });
        let mut expected = Selections::new(&snapshot, ranges, self.selections.primary_index())
            .expect("selections");
        if let Some(columns) = self.selections.table_columns() {
            expected = expected
                .with_table_slots(
                    columns,
                    self.selections
                        .table_slots()
                        .expect("saved grid slots")
                        .to_vec(),
                )
                .expect("saved grid ownership");
        }
        assert_eq!(document.selections(), &expected);
    }
}

fn source_variants(source: &str) -> [String; 4] {
    let lf = source.replace("\r\n", "\n");
    let crlf = lf.replace('\n', "\r\n");
    [
        lf.clone(),
        format!("\u{feff}{lf}"),
        crlf.clone(),
        format!("\u{feff}{crlf}"),
    ]
}

fn history_depth(document: &EditorDocument, undo: usize, redo: usize) {
    assert_eq!(document.history_stats().undo_entries(), undo);
    assert_eq!(document.history_stats().redo_entries(), redo);
    assert_eq!(document.command_available(&EditorCommand::Undo), undo > 0);
    assert_eq!(document.command_available(&EditorCommand::Redo), redo > 0);
}

fn at_end(document: &mut EditorDocument) {
    let snapshot = document.snapshot();
    document
        .set_selection(
            EditorSelection::cursor(&snapshot, snapshot.len_bytes(), CaretAffinity::Downstream)
                .expect("end cursor"),
        )
        .expect("set end cursor");
}

fn seeded_history(source: &str) -> (EditorDocument, [Frame; 3]) {
    let mut document = EditorDocument::new(source);
    at_end(&mut document);
    let original = Frame::capture(&document);
    assert!(
        document
            .execute(EditorCommand::insert_text(HISTORY_A))
            .expect("first edit")
            .changed()
    );
    let first = Frame::capture(&document);
    // Explicit navigation separates the typing groups; no timing assumption.
    at_end(&mut document);
    assert!(
        document
            .execute(EditorCommand::insert_text(HISTORY_B))
            .expect("second edit")
            .changed()
    );
    let second = Frame::capture(&document);
    assert_eq!(first.source, format!("{source}{HISTORY_A}"));
    assert_eq!(second.source, format!("{source}{HISTORY_A}{HISTORY_B}"));
    history_depth(&document, 2, 0);
    assert!(document.undo().expect("seed redo branch").changed());
    first.assert_replayed(&document);
    history_depth(&document, 1, 1);
    (document, [original, first, second])
}

fn select_target(document: &mut EditorDocument, kind: SelectionKind) {
    let snapshot = document.snapshot();
    let start = snapshot.as_str().find(TARGET).expect("target label");
    let end = start + TARGET.len();
    let other = snapshot.as_str().find(RECT_END).expect("rectangle endpoint");
    let range = |anchor: usize, focus: usize, affinity| {
        EditorSelection::range(
            &snapshot,
            ByteOffset::new(anchor as u64),
            ByteOffset::new(focus as u64),
            affinity,
        )
        .expect("UTF-8 endpoints")
    };
    match kind {
        SelectionKind::Caret | SelectionKind::Forward | SelectionKind::Backward => {
            let (anchor, focus, affinity) = match kind {
                SelectionKind::Caret => (start, start, CaretAffinity::Downstream),
                SelectionKind::Forward => (start, end, CaretAffinity::Downstream),
                _ => (end, start, CaretAffinity::Upstream),
            };
            document
                .set_selection(range(anchor, focus, affinity))
                .expect("text selection");
        }
        SelectionKind::RectangleForward | SelectionKind::RectangleBackward => {
            let (anchor, focus) = if matches!(kind, SelectionKind::RectangleBackward) {
                (other, start)
            } else {
                (start, other)
            };
            document
                .execute(EditorCommand::SelectTableCells {
                    anchor: ByteOffset::new(anchor as u64),
                    focus: ByteOffset::new(focus as u64),
                })
                .expect("native grid selection");
            assert_eq!(document.selections().table_columns(), Some(2));
            if document.table_target_is_html() == Some(true) {
                // Two colspan=2 owners across distinct row groups, not four carets.
                assert_eq!(document.selections().len(), 2);
                assert_eq!(
                    document.selections().table_slots(),
                    Some([Some(0), Some(0), Some(1), Some(1)].as_slice())
                );
            }
        }
        SelectionKind::Multiple => {
            document
                .set_selections(
                    [
                        range(end, start, CaretAffinity::Upstream),
                        range(other, other + RECT_END.len(), CaretAffinity::Downstream),
                    ],
                    1,
                )
                .expect("two nonempty selections");
            assert_eq!(document.selections().len(), 2);
            assert_eq!(document.selections().primary_index(), 1);
            assert_eq!(document.selections().table_columns(), None);
        }
    }
}

fn assert_rejected(document: &mut EditorDocument, command: &EditorCommand) {
    let before = Frame::capture(document);
    let revision = document.revision();
    let history = document.history_stats();
    for attempt in 0..3 {
        assert!(
            !document.command_available(command),
            "availability attempt {attempt}"
        );
        assert_eq!(document.snapshot().as_str(), before.source);
        assert_eq!(document.revision(), revision);
        assert_eq!(document.selections(), &before.selections);
        assert_eq!(document.history_stats(), history);
        let error = document
            .execute(command.clone())
            .expect_err("must reject, not silently paste");
        assert!(
            matches!(&error, EditorDocumentError::InvalidTablePaste),
            "{error:?}"
        );
        assert_eq!(document.snapshot().as_str(), before.source);
        assert_eq!(document.revision(), revision);
        assert_eq!(document.selections(), &before.selections);
        assert_eq!(document.history_stats(), history);
    }
}

fn replay_history(document: &mut EditorDocument, frames: &[Frame; 3]) {
    // Redo first proves the pre-existing branch was not cleared. The complete
    // sequence then checks transaction contents, grouping and saved selections.
    for (command, frame, undo, redo) in [
        (EditorCommand::Redo, 2, 2, 0),
        (EditorCommand::Undo, 1, 1, 1),
        (EditorCommand::Undo, 0, 0, 2),
        (EditorCommand::Redo, 1, 1, 1),
        (EditorCommand::Redo, 2, 2, 0),
    ] {
        assert!(
            document
                .execute(command)
                .expect("replay original history")
                .changed()
        );
        frames[frame].assert_replayed(document);
        history_depth(document, undo, redo);
    }
}

fn rejection_matrix(source: &str) {
    for (encoding, source) in source_variants(source).iter().enumerate() {
        for kind in SelectionKind::ALL {
            eprintln!("encoding={encoding}, selection={kind:?}");
            let (mut document, frames) = seeded_history(source);
            select_target(&mut document, kind);
            history_depth(&document, 1, 1);
            assert_rejected(&mut document, &EditorCommand::PasteHtmlTableSource(PAYLOAD.into()));
            replay_history(&mut document, &frames);
        }
    }
}

#[test]
fn cross_tbody_span_rejection_preserves_history_and_complete_selections() {
    rejection_matrix(CROSS_GROUPS);
}

#[test]
fn cross_head_body_span_rejection_preserves_history_and_complete_selections() {
    let source = CROSS_GROUPS
        .replacen("<tbody>", "<thead>", 1)
        .replacen("</tbody>", "</thead>", 1);
    rejection_matrix(&source);
}

#[test]
fn math_promotion_rejection_preserves_history_and_complete_selections() {
    rejection_matrix(MATH);
}

#[test]
fn footnote_promotion_rejection_preserves_history_and_complete_selections() {
    rejection_matrix(FOOTNOTE);
}

#[test]
fn rejected_span_does_not_poison_a_later_legal_paste() {
    let legal = "<table><tr><td colspan='2'>传入中文🙂</td></tr></table>";
    for source in source_variants(CROSS_GROUPS) {
        let (mut document, _) = seeded_history(&source);
        select_target(&mut document, SelectionKind::RectangleBackward);
        assert_rejected(&mut document, &EditorCommand::PasteHtmlTableSource(PAYLOAD.into()));
        let before = Frame::capture(&document);
        let command = EditorCommand::PasteHtmlTableSource(legal.into());
        assert!(document.command_available(&command));
        assert!(
            document
                .execute(command)
                .expect("legal one-row paste")
                .changed()
        );
        let expected = before.source.replace(
            "<td colspan='2'>目标中文🙂</td>",
            "<td colspan='2'>传入中文🙂</td>",
        );
        assert_ne!(expected, before.source);
        assert_eq!(document.snapshot().as_str(), expected);
        history_depth(&document, 2, 0); // Only a successful edit discards old redo.
        assert!(document.undo().expect("one undo of legal paste").changed());
        before.assert_replayed(&document);
        history_depth(&document, 1, 1);
        assert!(document.redo().expect("redo legal paste").changed());
        assert_eq!(document.snapshot().as_str(), expected);
    }
}

#[test]
fn promotion_controls_accept_the_same_payload_without_unsupported_embeds() {
    for (fixture, unsupported) in [(MATH, "$x^2$"), (FOOTNOTE, "[^note]")] {
        for source in source_variants(&fixture.replacen(unsupported, "**保留中文🙂**", 1)) {
            let (mut document, _) = seeded_history(&source);
            select_target(&mut document, SelectionKind::Backward);
            let before = Frame::capture(&document);
            let command = EditorCommand::PasteHtmlTableSource(PAYLOAD.into());
            assert!(document.command_available(&command));
            assert!(
                document
                    .execute(command)
                    .expect("supported promotion")
                    .changed()
            );
            let after = Frame::capture(&document);
            assert_eq!(document.table_target_is_html(), Some(true));
            assert!(after.source.contains("<strong>保留中文🙂</strong>"));
            assert!(after.source.contains("colspan='2' rowspan='2'>传入中文🙂</td>"));
            let table_start = before.source.find("| H |").expect("table start");
            let table_end = before.source.find("| end |").expect("table end") + "| end |".len();
            assert!(after.source.starts_with(&before.source[..table_start]));
            assert!(after.source.ends_with(&before.source[table_end..]));
            history_depth(&document, 2, 0);
            assert!(document.undo().expect("one undo of promotion").changed());
            before.assert_replayed(&document);
            history_depth(&document, 1, 1);
            assert!(document.redo().expect("redo promotion").changed());
            after.assert_replayed(&document);
        }
    }
}
