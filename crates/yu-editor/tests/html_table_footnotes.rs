//! Footnote table promotion exercises the real command, graph and native layout.
//! File round trips and core geometry do not certify macOS window acceptance.
use yu_core::{ByteOffset, TextRange};
use yu_editor::{
    BlockView, CaretAffinity, EditorCommand, EditorDocument, EditorDocumentError, EditorSelection,
    LayoutConfig, LayoutPoint, MonospaceMetrics, VisualText,
};
use yu_markdown::{BlockWidget, EmbeddedKind};

const SIMPLE: &str = include_str!("fixtures/group4-paste/footnote-target.md");
const PAYLOAD: &str = include_str!("fixtures/group4-paste/merged-payload.md");
const MIXED: &str = include_str!("fixtures/group4-paste/footnote-mixed-target.md");

fn variants(source: &str) -> Vec<String> {
    ["\n", "\r\n"]
        .into_iter()
        .flat_map(|ending| {
            ["", "\u{feff}"].into_iter().map(move |bom| {
                format!(
                    "{bom}{}",
                    source.replace("\r\n", "\n").replace('\n', ending)
                )
            })
        })
        .collect()
}

fn select(document: &mut EditorDocument, range: TextRange) {
    let snapshot = document.snapshot();
    document
        .set_selection(
            EditorSelection::range(
                &snapshot,
                range.start(),
                range.end(),
                CaretAffinity::Downstream,
            )
            .expect("valid footnote integration fixture"),
        )
        .expect("valid footnote integration fixture");
}

fn promote(source: &str) -> EditorDocument {
    let mut document = EditorDocument::new(source);
    let at = ByteOffset::new(
        source
            .find("目标中文🙂")
            .expect("valid footnote integration fixture") as u64,
    );
    select(&mut document, TextRange::empty(at));
    let command = EditorCommand::PasteHtmlTableSource(PAYLOAD.into());
    assert!(
        document.command_available(&command),
        "must promote: {source}"
    );
    assert!(
        document
            .execute(command)
            .expect("valid footnote integration fixture")
            .changed()
    );
    assert_eq!(document.table_target_is_html(), Some(true));
    document
}

fn view(document: &EditorDocument, active: Option<TextRange>) -> BlockView {
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
        .expect("valid footnote integration fixture");
    let decorations = index
        .decorate(markdown, block, active)
        .expect("valid footnote integration fixture");
    let visual = VisualText::new(
        &document.snapshot(),
        block.range(),
        decorations.set().clone(),
    )
    .expect("valid footnote integration fixture");
    BlockView::build(
        block.kind(),
        &visual,
        &decorations,
        LayoutConfig::new(800.0, 16.0),
        &MonospaceMetrics::new(8.0),
    )
    .expect("valid footnote integration fixture")
}

fn numbers(document: &EditorDocument) -> Vec<Option<u32>> {
    document
        .markdown()
        .footnotes()
        .references()
        .iter()
        .map(|reference| reference.number)
        .collect()
}

#[test]
fn promotion_keeps_definitions_repeated_references_math_and_mixed_formatting() {
    for source in variants(MIXED) {
        let mut document = promote(&source);
        assert_eq!(
            numbers(&document),
            [Some(1), Some(2), Some(2), Some(3), Some(2)]
        );
        let index = document.markdown().footnotes();
        assert!(index.diagnostics().is_empty());
        assert_eq!(
            index
                .references()
                .iter()
                .filter(|r| r.content.is_some())
                .count(),
            3
        );
        assert_eq!(index.definitions().len(), 3);
        for reference in index.references() {
            let target = index
                .navigation_target(reference.source.start())
                .expect("valid footnote integration fixture");
            assert!(
                index
                    .definitions()
                    .iter()
                    .any(|definition| definition.source == target)
            );
        }
        let note = index
            .definition_for_marker("[^note]")
            .expect("valid footnote integration fixture");
        assert_eq!(
            index.navigation_target(note.marker.start()),
            Some(index.references()[1].source)
        );
        let after = document.snapshot().as_str().to_owned();
        let before_defs = source
            .find("[^outside]:")
            .expect("valid footnote integration fixture");
        assert!(
            after.ends_with(&source[before_defs..]),
            "definitions stay byte-identical"
        );
        assert_eq!(
            after.starts_with('\u{feff}'),
            source.starts_with('\u{feff}')
        );
        if source.contains("\r\n") {
            assert!(!after.replace("\r\n", "").contains('\n'));
        }
        let native = view(&document, None);
        assert_eq!(native.embedded().len(), 1);
        assert!(!native.visual().text().contains("[^"));
        assert!(!native.visual().text().contains("data-yu-footnote"));
        assert!(after.contains("<strong>"));
        assert!(after.contains("href=\"https://example.com\""));
        assert!(after.contains("colspan='2' rowspan='2'"));
        document.undo().expect("valid footnote integration fixture");
        assert_eq!(document.snapshot().as_str(), source);
        document.redo().expect("valid footnote integration fixture");
        assert_eq!(document.snapshot().as_str(), after);
        assert_eq!(
            numbers(&document),
            [Some(1), Some(2), Some(2), Some(3), Some(2)]
        );
    }
}

#[test]
fn native_number_hit_testing_targets_the_original_definition_on_both_sides() {
    let document = promote(MIXED);
    let index = document.markdown().footnotes();
    let native = view(&document, None);
    for reference in index.references().iter().filter(|r| r.content.is_some()) {
        let cluster = native
            .clusters()
            .iter()
            .find(|cluster| cluster.source() == reference.source)
            .expect("valid footnote integration fixture");
        for fraction in [0.25, 0.75] {
            let hit = native
                .hit_test(LayoutPoint::new(
                    cluster.x() + cluster.width() * fraction,
                    cluster.y() + cluster.line_height() * 0.5,
                ))
                .expect("valid footnote integration fixture");
            let clicked = hit
                .content_source()
                .expect("number has canonical content source");
            assert_eq!(
                index.navigation_target(clicked.start()),
                index.navigation_target(reference.source.start())
            );
        }
    }
}

#[test]
fn changing_definition_and_body_order_rebinds_numbers_without_rewriting_the_table() {
    let mut document = promote(SIMPLE);
    let saved = document.snapshot().as_str().to_owned();
    let before_view = view(&document, None).visual().text().to_owned();
    select(&mut document, TextRange::empty(ByteOffset::ZERO));
    document
        .execute(EditorCommand::insert_text("Earlier[^extra].\n\n"))
        .expect("valid footnote integration fixture");
    let at = document.snapshot().len_bytes();
    select(&mut document, TextRange::empty(at));
    document
        .execute(EditorCommand::insert_text("\n[^extra]: new definition\n"))
        .expect("valid footnote integration fixture");
    assert_eq!(numbers(&document), [Some(1), Some(2)]);
    assert_ne!(view(&document, None).visual().text(), before_view);
    assert!(document.snapshot().as_str().contains(&saved));
    document.undo().expect("valid footnote integration fixture");
    document.undo().expect("valid footnote integration fixture");
    assert_eq!(document.snapshot().as_str(), saved);
    assert_eq!(numbers(&document), [Some(1)]);
    assert_eq!(view(&document, None).visual().text(), before_view);
}

#[test]
fn missing_and_duplicate_definitions_after_conversion_show_diagnostics_and_recover() {
    let document = promote(SIMPLE);
    let saved = document.snapshot().as_str().to_owned();
    for broken in [
        saved.replace("[^note]:", "[^other]:"),
        format!("{saved}\n[^NOTE]: duplicate\n"),
    ] {
        let mut editing = EditorDocument::new(&broken);
        let reference = editing.markdown().footnotes().references()[0].clone();
        assert_eq!(reference.number, None);
        assert!(
            editing
                .markdown()
                .footnotes()
                .diagnostic_at(reference.source.start())
                .is_some()
        );
        assert!(
            editing
                .markdown()
                .footnotes()
                .navigation_target(reference.source.start())
                .is_none()
        );
        assert!(view(&editing, None).visual().text().contains("[^note]"));
        select(
            &mut editing,
            TextRange::new(ByteOffset::ZERO, ByteOffset::new(broken.len() as u64))
                .expect("valid footnote integration fixture"),
        );
        editing
            .set_source_mode(true)
            .expect("valid footnote integration fixture");
        editing
            .execute(EditorCommand::insert_text(saved.as_str()))
            .expect("valid footnote integration fixture");
        editing
            .set_source_mode(false)
            .expect("valid footnote integration fixture");
        assert_eq!(numbers(&editing), [Some(1)]);
        assert!(editing.markdown().footnotes().diagnostics().is_empty());
    }
}

#[test]
fn whole_reference_deletion_keeps_definition_and_restores_on_undo() {
    let mut document = promote(MIXED);
    let saved = document.snapshot().as_str().to_owned();
    let reference = document
        .markdown()
        .footnotes()
        .references()
        .iter()
        .find(|r| r.content.is_some())
        .expect("valid footnote integration fixture")
        .clone();
    let mut expected = saved.clone();
    expected.replace_range(
        reference.source.start().get() as usize..reference.source.end().get() as usize,
        "",
    );
    select(&mut document, reference.source);
    document
        .execute(EditorCommand::DeleteSelections)
        .expect("valid footnote integration fixture");
    assert_eq!(document.snapshot().as_str(), expected);
    assert_eq!(document.markdown().footnotes().definitions().len(), 3);
    assert_eq!(document.markdown().footnotes().references().len(), 4);
    assert!(view(&document, None).table().is_some());
    document.undo().expect("valid footnote integration fixture");
    assert_eq!(document.snapshot().as_str(), saved);
    assert_eq!(
        numbers(&document),
        [Some(1), Some(2), Some(2), Some(3), Some(2)]
    );
}

#[test]
fn label_body_edit_uses_shared_entity_mapping_and_rebinds_the_reference() {
    let mut document = promote(MIXED);
    let before = document.snapshot().as_str().to_owned();
    let reference = document
        .markdown()
        .footnotes()
        .references()
        .iter()
        .find(|r| r.content.is_some())
        .expect("valid footnote integration fixture")
        .clone();
    select(
        &mut document,
        reference
            .content
            .expect("valid footnote integration fixture"),
    );
    document
        .execute(EditorCommand::insert_text("[^二]"))
        .expect("valid footnote integration fixture");
    assert!(document.markdown().footnotes().diagnostics().is_empty());
    let changed = &document.markdown().footnotes().references()[1];
    let expected = document
        .markdown()
        .footnotes()
        .definition_for_marker("[^二]")
        .expect("valid footnote integration fixture");
    assert_eq!(
        document
            .markdown()
            .footnotes()
            .navigation_target(changed.source.start()),
        Some(expected.source)
    );
    document.undo().expect("valid footnote integration fixture");
    assert_eq!(document.snapshot().as_str(), before);
}

#[test]
fn labels_preserve_unicode_casefold_and_html_escaping() {
    for label in ["中文🙂", "ß", "a&b", "a<b"] {
        let source = SIMPLE.replace("[^note]", &format!("[^{label}]"));
        let document = promote(&source);
        let index = document.markdown().footnotes();
        assert!(index.diagnostics().is_empty(), "{label}");
        let reference = &index.references()[0];
        let marker = yu_markdown::html::HtmlFootnoteSpan {
            source: reference.source,
            content: reference
                .content
                .expect("valid footnote integration fixture"),
        }
        .marker_text(document.snapshot().as_str())
        .expect("valid footnote integration fixture");
        assert_eq!(marker, format!("[^{label}]"));
        if label == "ß" {
            assert!(index.definition_for_marker("[^SS]").is_some());
        }
    }
}

#[test]
fn missing_or_ambiguous_promotion_and_foreign_payloads_preserve_live_redo() {
    for source in [
        SIMPLE.replace("[^note]:", "[^different]:"),
        format!("{SIMPLE}\n[^NOTE]: duplicate\n"),
        // The source parser and converter disagree on entity-spelled labels.
        // Preserve this valid-but-unsupported input instead of losing identity.
        SIMPLE.replace("[^note]", "[^a&amp;b]"),
    ] {
        let mut document = EditorDocument::new(&source);
        let end = document.snapshot().len_bytes();
        select(&mut document, TextRange::empty(end));
        document
            .execute(EditorCommand::insert_text("history"))
            .expect("valid footnote integration fixture");
        document.undo().expect("valid footnote integration fixture");
        let at = ByteOffset::new(
            source
                .find("目标中文🙂")
                .expect("valid footnote integration fixture") as u64,
        );
        select(&mut document, TextRange::empty(at));
        let before = document.snapshot();
        let selections = document.selections().clone();
        let history = document.history_stats();
        let command = EditorCommand::PasteHtmlTableSource(PAYLOAD.into());
        for _ in 0..3 {
            assert!(!document.command_available(&command));
            assert!(matches!(
                document.execute(command.clone()),
                Err(EditorDocumentError::InvalidTablePaste)
            ));
            assert_eq!(document.snapshot().as_str(), before.as_str());
            assert_eq!(document.revision(), before.revision());
            assert_eq!(document.selections(), &selections);
            assert_eq!(document.history_stats(), history);
        }
        document.redo().expect("valid footnote integration fixture");
        assert_eq!(document.snapshot().as_str(), format!("{source}history"));
    }
    let mut document = promote(SIMPLE);
    let before = document.snapshot().as_str().to_owned();
    let payload =
        "<table><tr><td><span data-yu-footnote='reference'>[^foreign]</span></td></tr></table>";
    assert!(
        document
            .execute(EditorCommand::PasteHtmlTableSource(payload.into()))
            .is_err()
    );
    assert_eq!(document.snapshot().as_str(), before);
}

#[test]
fn malformed_reference_tags_never_become_native_table_references() {
    for body in [
        "<span data-yu-footnote='reference'>1</span>",
        "<span data-yu-footnote='reference'><b>[^note]</b></span>",
        "<span data-yu-footnote='reference' data-yu-footnote='reference'>[^note]</span>",
        "<span data-yu-footnote='reference' data-math-style='inline'>[^note]</span>",
        "<span data-yu-footnote='reference'>[^no&#10;te]</span>",
        "<span data-yu-footnote='reference'/>",
        "<sup data-yu-footnote='reference'>[^note]</sup>",
    ] {
        let mut document = promote(SIMPLE);
        let before = document.snapshot().as_str().to_owned();
        let payload = format!("<table><tr><td>{body}</td></tr></table>");
        let command = EditorCommand::PasteHtmlTableSource(payload.into());
        assert!(!document.command_available(&command), "{body}");
        assert!(document.execute(command).is_err());
        assert_eq!(document.snapshot().as_str(), before);
    }
}

#[test]
fn reconciliation_rejects_equal_counts_in_the_wrong_cells_or_with_wrong_labels() {
    let source = "| A | B |\n| --- | --- |\n| [^a] | [^b] |\n\n[^a]: A\n[^b]: B\n";
    let document = EditorDocument::new(source);
    let table = document
        .markdown()
        .blocks()
        .iter()
        .find_map(|block| yu_markdown::table_for_block(document.markdown(), block))
        .expect("valid footnote integration fixture");
    let a = "<span data-yu-footnote='reference'>[^a]</span>";
    let b = "<span data-yu-footnote='reference'>[^b]</span>";
    for (left, right) in [
        (format!("{a}{b}"), String::new()),
        (b.to_owned(), a.to_owned()),
        (a.to_owned(), a.to_owned()),
    ] {
        let html = format!(
            "<table><tr><th>A</th><th>B</th></tr><tr><td>{left}</td><td>{right}</td></tr></table>"
        );
        assert!(
            document
                .markdown()
                .preserve_table_footnote_source(&html, &table)
                .is_none()
        );
    }
    let correct =
        format!("<table><tr><th>A</th><th>B</th></tr><tr><td>{a}</td><td>{b}</td></tr></table>");
    assert!(
        document
            .markdown()
            .preserve_table_footnote_source(&correct, &table)
            .is_some()
    );
}

#[test]
fn native_reference_entities_decode_once_and_source_mode_keeps_the_graph() {
    let source = "<table><tr><td><span data-yu-footnote='reference'>[^a&amp;amp;b]</span><span data-yu-footnote='reference'>[^a&amp;b]</span></td></tr></table>\n\n[^a&amp;b]: literal entity label\n[^a&b]: ampersand label\n";
    let mut document = EditorDocument::new(source);
    assert_eq!(numbers(&document), [Some(1), Some(2)]);
    assert!(document.markdown().footnotes().diagnostics().is_empty());
    let refs = document.markdown().footnotes().references().to_vec();
    assert_ne!(
        document
            .markdown()
            .footnotes()
            .navigation_target(refs[0].source.start()),
        document
            .markdown()
            .footnotes()
            .navigation_target(refs[1].source.start())
    );
    assert_eq!(view(&document, None).visual().text(), "12");
    assert_eq!(
        view(
            &document,
            Some(TextRange::empty(
                refs[0]
                    .content
                    .expect("valid footnote integration fixture")
                    .start()
            ))
        )
        .visual()
        .text(),
        "[^a&amp;b]2"
    );
    document
        .set_source_mode(true)
        .expect("valid footnote integration fixture");
    assert_eq!(document.markdown().footnotes().references(), refs);
    document
        .set_source_mode(false)
        .expect("valid footnote integration fixture");
    assert_eq!(numbers(&document), [Some(1), Some(2)]);
}

#[test]
fn closed_details_hides_the_number_without_changing_document_order() {
    let source = "<table><tr><td><details><summary>Summary</summary><p>body<span data-yu-footnote='reference'>[^a]</span></p></details></td></tr></table>\n\nOutside[^b].\n\n[^a]: A\n[^b]: B\n";
    let closed = EditorDocument::new(source);
    assert_eq!(numbers(&closed), [Some(1), Some(2)]);
    assert!(!view(&closed, None).visual().text().contains('1'));
    let opened = EditorDocument::new(source.replace("<details>", "<details open>"));
    assert_eq!(numbers(&opened), numbers(&closed));
    assert!(view(&opened, None).visual().text().contains("body1"));
}

#[test]
fn a_used_table_note_can_reference_other_notes_without_recursing_in_cycles() {
    let source = SIMPLE.replace(
        "[^note]:",
        "[^note]: See [^second].\n\n[^second]: See [^note].\n\n",
    );
    let document = promote(&source);
    assert_eq!(numbers(&document), [Some(1), Some(2), Some(1)]);
    assert!(document.markdown().footnotes().diagnostics().is_empty());
}

#[test]
fn cached_unchanged_table_projection_is_invalidated_by_an_outside_reference() {
    let source = SIMPLE.replace("KEEP 中文🙂", "123456789") + "\n[^n]: other\n";
    let mut document = promote(&source);
    let mut cache = yu_editor::DecorationCache::default();
    let table_block = |document: &EditorDocument| {
        document
            .markdown()
            .blocks()
            .iter()
            .find(|block| {
                document
                    .markdown()
                    .html_regions()
                    .partition_for(block.range())
                    .is_some_and(|part| part.content.kind == yu_markdown::html::HtmlFlowKind::Table)
            })
            .expect("valid footnote integration fixture")
    };
    let block = table_block(&document);
    let before = cache
        .get_or_build_block(document.markdown(), block)
        .expect("valid footnote integration fixture")
        .clone();
    let before_text = VisualText::new(&document.snapshot(), block.range(), before.set().clone())
        .expect("valid footnote integration fixture")
        .text()
        .to_owned();
    let applied = document
        .apply_transaction(&yu_text::Transaction::new(
            document.revision(),
            [yu_text::Edit::new(
                TextRange::new(ByteOffset::ZERO, ByteOffset::new(9))
                    .expect("valid footnote integration fixture"),
                "Text[^n].",
            )],
        ))
        .expect("valid footnote integration fixture");
    cache.shift_through(applied.change_set(), applied.result_snapshot());
    let block_after = table_block(&document);
    assert_eq!(
        block.range(),
        block_after.range(),
        "table bytes did not move"
    );
    let after = cache
        .get_or_build_block(document.markdown(), block_after)
        .expect("valid footnote integration fixture")
        .clone();
    let after_text = VisualText::new(
        &document.snapshot(),
        block_after.range(),
        after.set().clone(),
    )
    .expect("valid footnote integration fixture")
    .text()
    .to_owned();
    assert_ne!(before_text, after_text);
    assert_eq!(numbers(&document), [Some(1), Some(2)]);
}

#[test]
fn native_clipboard_roundtrip_keeps_labels_and_rejects_foreign_grid_references() {
    let mut document = promote(SIMPLE);
    let original = document.snapshot().as_str().to_owned();
    let reference = document.markdown().footnotes().references()[0].clone();
    document
        .execute(EditorCommand::SelectTableCells {
            anchor: reference.source.start(),
            focus: reference.source.start(),
        })
        .expect("valid footnote integration fixture");
    let copied = document
        .copy_html_table_source()
        .expect("valid footnote integration fixture")
        .expect("valid footnote integration fixture");
    assert!(copied.contains("[^note]"));
    let target = ByteOffset::new(
        original
            .find(">H<")
            .expect("valid footnote integration fixture") as u64
            + 1,
    );
    select(&mut document, TextRange::empty(target));
    document
        .execute(EditorCommand::PasteHtmlTableSource(copied.into()))
        .expect("valid footnote integration fixture");
    assert_eq!(numbers(&document), [Some(1), Some(1)]);
    assert_eq!(document.markdown().footnotes().definitions().len(), 1);
    document.undo().expect("valid footnote integration fixture");
    assert_eq!(document.snapshot().as_str(), original);
    select(&mut document, TextRange::empty(target));
    let before = document.snapshot();
    let history = document.history_stats();
    let selected = document.selections().clone();
    let command = EditorCommand::PasteHtmlTableGrid {
        columns: 1,
        cells: vec!["<span data-yu-footnote='reference'>[^foreign]</span>".into()],
    };
    assert!(!document.command_available(&command));
    assert!(document.execute(command).is_err());
    assert_eq!(document.snapshot().as_str(), before.as_str());
    assert_eq!(document.revision(), before.revision());
    assert_eq!(document.history_stats(), history);
    assert_eq!(document.selections(), &selected);
    document.redo().expect("valid footnote integration fixture");
    assert_eq!(numbers(&document), [Some(1), Some(1)]);
}

#[test]
fn saved_utf8_rebuilds_the_same_graph_and_can_be_edited_again() {
    for source in variants(MIXED) {
        let document = promote(&source);
        let bytes = document.snapshot().as_str().as_bytes().to_vec();
        let mut reopened = EditorDocument::new(
            String::from_utf8(bytes.clone()).expect("valid footnote integration fixture"),
        );
        assert_eq!(reopened.snapshot().as_str().as_bytes(), bytes);
        assert_eq!(numbers(&reopened), numbers(&document));
        assert!(reopened.markdown().footnotes().diagnostics().is_empty());
        let native = view(&reopened, None);
        assert!(native.decorations().widgets().iter().any(|widget| {
            matches!(widget, BlockWidget::Embedded(span) if span.kind == EmbeddedKind::Math)
        }));
        let reference = reopened.markdown().footnotes().references()[1].clone();
        select(
            &mut reopened,
            reference
                .content
                .expect("valid footnote integration fixture"),
        );
        reopened
            .execute(EditorCommand::insert_text("[^二]"))
            .expect("valid footnote integration fixture");
        assert!(reopened.markdown().footnotes().diagnostics().is_empty());
    }
}
