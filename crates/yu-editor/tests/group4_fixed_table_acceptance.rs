//! Per-ID core contract for the generated 76 Group 4 table inputs.
//! The external acceptance runner supplies a new, hash-checked case manifest.
use std::fs;

use yu_core::ByteOffset;
use yu_editor::{
    CaretAffinity, EditorCommand, EditorDocument, EditorDocumentError, EditorSelection, Selections,
};

const HISTORY_A: &str = " HISTORY-A-中文🙂";
const HISTORY_B: &str = " HISTORY-B-中文🙂";

#[derive(Clone)]
struct Frame {
    source: String,
    selections: Selections,
}

impl Frame {
    fn capture(doc: &EditorDocument) -> Self {
        Self {
            source: doc.snapshot().as_str().to_owned(),
            selections: doc.selections().clone(),
        }
    }

    fn assert_replayed(&self, doc: &EditorDocument) {
        let snapshot = doc.snapshot();
        assert_eq!(snapshot.as_str(), self.source);
        let ranges = self.selections.as_slice().iter().map(|selection| {
            EditorSelection::range(
                &snapshot,
                selection.anchor(),
                selection.focus(),
                selection.affinity(),
            )
            .expect("rebound selection")
        });
        let mut expected =
            Selections::new(&snapshot, ranges, self.selections.primary_index()).expect("set");
        if let Some(columns) = self.selections.table_columns() {
            expected = expected
                .with_table_slots(
                    columns,
                    self.selections.table_slots().expect("slots").to_vec(),
                )
                .expect("owned slots");
        }
        assert_eq!(doc.selections(), &expected);
    }
}

fn cursor_at_end(doc: &mut EditorDocument) {
    let snapshot = doc.snapshot();
    doc.set_selection(
        EditorSelection::cursor(&snapshot, snapshot.len_bytes(), CaretAffinity::Downstream)
            .expect("end cursor"),
    )
    .expect("set cursor");
}

fn seeded(source: &str, expected_a: &str, expected_b: &str) -> (EditorDocument, [Frame; 3]) {
    let mut doc = EditorDocument::new(source);
    cursor_at_end(&mut doc);
    let zero = Frame::capture(&doc);
    assert!(
        doc.execute(EditorCommand::insert_text(HISTORY_A))
            .expect("history A insert")
            .changed()
    );
    let a = Frame::capture(&doc);
    cursor_at_end(&mut doc);
    assert!(
        doc.execute(EditorCommand::insert_text(HISTORY_B))
            .expect("history B insert")
            .changed()
    );
    let b = Frame::capture(&doc);
    assert_eq!(a.source, expected_a);
    assert_eq!(b.source, expected_b);
    assert!(doc.undo().expect("undo B").changed());
    a.assert_replayed(&doc);
    assert_eq!(doc.history_stats().undo_entries(), 1);
    assert_eq!(doc.history_stats().redo_entries(), 1);
    (doc, [zero, a, b])
}

fn select(doc: &mut EditorDocument, rectangle: bool) {
    let snapshot = doc.snapshot();
    let source = snapshot.as_str();
    let start = source.find("目标中文🙂").expect("target label");
    let end = start + "目标中文🙂".len();
    let other = source.find("矩形终点🙂").expect("rectangle end");
    if rectangle {
        assert!(
            !doc.execute(EditorCommand::SelectTableCells {
                anchor: ByteOffset::new(start as u64),
                focus: ByteOffset::new(other as u64),
            })
            .expect("select cells")
            .changed()
        );
        assert!(doc.selections().table_columns().is_some());
    } else {
        doc.set_selection(
            EditorSelection::range(
                &snapshot,
                ByteOffset::new(end as u64),
                ByteOffset::new(start as u64),
                CaretAffinity::Upstream,
            )
            .expect("backward text range"),
        )
        .expect("set range");
    }
}

fn source_context_preserved(before: &str, after: &str, name: &str) {
    assert!(after.starts_with(if before.starts_with('\u{feff}') {
        "\u{feff}KEEP 中文🙂"
    } else {
        "KEEP 中文🙂"
    }));
    assert!(after.contains("TAIL"));
    assert!(after.ends_with(HISTORY_A));
    assert_eq!(
        before.starts_with('\u{feff}'),
        after.starts_with('\u{feff}')
    );
    if before.contains("\r\n") {
        assert!(!after.replace("\r\n", "").contains('\n'));
    }
    if name.starts_with("row-groups-") {
        for token in ["id='head'", "id='body'"] {
            if before.contains(token) {
                assert!(after.contains(token), "row group {token} lost");
            }
        }
        if before.contains("id='empty'") {
            assert!(after.contains("id='empty'"));
        }
        if before.contains("id='row-groups'") {
            assert!(after.contains("id='row-groups'"));
        }
    }
}

fn assert_embeds(doc: &EditorDocument, math: &str, notes: &str) {
    if !math.is_empty() {
        let actual: Vec<_> = doc
            .markdown()
            .html_regions()
            .regions
            .iter()
            .filter_map(|region| region.model.as_ref().ok())
            .flat_map(|model| model.inline_math_spans())
            .filter_map(|span| doc.markdown().embedded_source(span))
            .collect();
        let expected: Vec<_> = math.split('|').map(str::to_owned).collect();
        assert_eq!(actual, expected, "inline formula ownership/order");
    }
    if !notes.is_empty() {
        let actual: Vec<_> = doc
            .markdown()
            .footnotes()
            .references()
            .iter()
            .map(|reference| reference.number)
            .collect();
        let expected: Vec<_> = notes
            .split(',')
            .map(|number| Some(number.parse::<u32>().expect("expected number")))
            .collect();
        assert_eq!(actual, expected, "footnote numbers/order");
    }
}

fn run_case(fields: &[&str]) {
    assert_eq!(fields.len(), 9, "case TSV fields");
    let [
        id,
        source_path,
        a_path,
        b_path,
        payload_path,
        verdict,
        selection,
        math,
        notes,
    ] = <&[&str; 9]>::try_from(fields).expect("nine fields");
    let source = fs::read_to_string(source_path).expect("source file");
    let expected_a = fs::read_to_string(a_path).expect("history A file");
    let expected_b = fs::read_to_string(b_path).expect("history B file");
    let payload = fs::read_to_string(payload_path).expect("payload file");
    let (mut doc, frames) = seeded(&source, &expected_a, &expected_b);
    select(&mut doc, *selection == "rectangle");
    let selected = Frame::capture(&doc);
    let revision = doc.revision();
    let history = doc.history_stats();
    let command = EditorCommand::PasteHtmlTableSource(payload.into());
    if *verdict == "reject" {
        for _ in 0..3 {
            assert!(!doc.command_available(&command), "{id}");
            assert_eq!(doc.revision(), revision);
            assert_eq!(doc.history_stats(), history);
            selected.assert_replayed(&doc);
            assert!(matches!(
                doc.execute(command.clone()),
                Err(EditorDocumentError::InvalidTablePaste)
            ));
            assert_eq!(doc.revision(), revision);
            assert_eq!(doc.history_stats(), history);
            selected.assert_replayed(&doc);
        }
        for (command, frame) in [
            (EditorCommand::Redo, &frames[2]),
            (EditorCommand::Undo, &frames[1]),
            (EditorCommand::Undo, &frames[0]),
            (EditorCommand::Redo, &frames[1]),
            (EditorCommand::Redo, &frames[2]),
        ] {
            assert!(
                doc.execute(command)
                    .expect("rejection history step")
                    .changed()
            );
            assert_eq!(doc.snapshot().as_str(), frame.source);
        }
    } else {
        assert_eq!(*verdict, "accept");
        for _ in 0..3 {
            assert!(doc.command_available(&command), "{id}");
            assert_eq!(doc.revision(), revision);
            assert_eq!(doc.history_stats(), history);
            selected.assert_replayed(&doc);
        }
        assert!(doc.execute(command).expect("accepted paste").changed());
        let after = Frame::capture(&doc);
        assert_ne!(after.source, selected.source);
        source_context_preserved(&selected.source, &after.source, id);
        assert_embeds(&doc, math, notes);
        assert_eq!(doc.history_stats().undo_entries(), 2);
        assert_eq!(doc.history_stats().redo_entries(), 0);
        assert!(doc.undo().expect("undo paste").changed());
        selected.assert_replayed(&doc);
        assert!(doc.redo().expect("redo paste").changed());
        after.assert_replayed(&doc);
        // New document parsing is the core layer's save/reopen contract.
        let reopened = EditorDocument::new(&after.source);
        assert_embeds(&reopened, math, notes);
    }
    println!("G4CASE table/{id} PASS");
}

#[test]
#[ignore = "requires generated cases; run tools/run_group4_fixed_acceptance.py"]
fn generated_table_inputs_have_per_id_core_results() {
    let path = std::env::var("GROUP4_TABLE_CASES").expect("runner case TSV path");
    let cases = fs::read_to_string(path).expect("case TSV");
    let mut count = 0;
    for line in cases.lines() {
        let fields: Vec<_> = line.split('\t').collect();
        eprintln!("G4CASE table/{} START", fields[0]);
        run_case(&fields);
        count += 1;
    }
    assert_eq!(count, 76);
}
