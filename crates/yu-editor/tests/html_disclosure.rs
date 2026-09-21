use yu_core::ByteOffset;
use yu_editor::EditorDocument;

#[test]
fn details_toggle_preserves_nested_body_attributes_and_history() {
    let source = "\u{feff}KEEP\r\n\r\n<details id='outer' open='false'><summary>Outer</summary>\r\n<details><summary>Inner中文</summary><p>BODY🙂</p></details>\r\n</details>\r\n\r\nTAIL";
    let mut document = EditorDocument::new(source);
    let at = ByteOffset::new(source.find("Inner").expect("inner") as u64);
    document.toggle_html_details(at).expect("open inner");
    assert_eq!(
        document.snapshot().as_str(),
        source.replace("<details>", "<details open>")
    );
    assert_eq!(document.history_stats().undo_entries(), 1);
    document.undo().expect("undo");
    assert_eq!(document.snapshot().as_str(), source);
    let outer = ByteOffset::new(source.find("Outer").expect("outer") as u64);
    let details = document.markdown().html_regions().regions[0]
        .model
        .as_ref()
        .expect("model")
        .disclosures();
    assert_eq!(details.len(), 2);
    assert!(
        details[0].open,
        "HTML boolean attributes are true even with false as a string value"
    );
    assert!(!details[1].open);
    let body = details[1].body;
    assert_eq!(
        &source[body.start().get() as usize..body.end().get() as usize],
        "<p>BODY🙂</p>"
    );
    document.toggle_html_details(outer).expect("close outer");
    assert_eq!(
        document.snapshot().as_str(),
        source.replace("open='false'", "")
    );
    document.undo().expect("undo close");
    assert_eq!(document.snapshot().as_str(), source);
    document.set_source_mode(true).expect("source mode");
    document
        .toggle_html_details(outer)
        .expect("no toggle in source mode");
    assert_eq!(document.snapshot().as_str(), source);
}

#[test]
fn malformed_and_unknown_details_are_not_silently_repaired() {
    for source in [
        "<details class='unknown'><summary>Title</summary>Body</details>",
        "<details><summary>One</summary><summary>Two</summary></details>",
        "<details><summary>Unclosed",
    ] {
        let mut document = EditorDocument::new(source);
        document
            .toggle_html_details(ByteOffset::new(1))
            .expect("unsupported details");
        assert_eq!(document.snapshot().as_str(), source);
        assert_eq!(document.history_stats().undo_entries(), 0);
    }
}

#[test]
fn collapsed_details_project_only_summary_and_reopen_without_losing_nested_state() {
    use yu_editor::LayoutConfig;
    fn visible(document: &mut EditorDocument) -> String {
        (0..document.markdown().blocks().len())
            .map(|index| {
                document
                    .block_layout_for_visual_state(index, LayoutConfig::new(500.0, 26.0))
                    .expect("layout")
                    .visual()
                    .text()
                    .to_owned()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }
    let source = "<details><summary>Outer <b>title</b></summary><p>Body</p><details><summary>Inner</summary><p>Hidden</p></details></details>";
    let mut document = EditorDocument::new(source);
    assert_eq!(visible(&mut document), "▸ Outer title");
    document
        .toggle_html_details(ByteOffset::new(1))
        .expect("expand");
    let opened = visible(&mut document);
    assert!(opened.contains("▾ Outer title"));
    assert!(opened.contains("Body"));
    assert!(opened.contains("▸ Inner"));
    assert!(!opened.contains("Hidden"));
    document.undo().expect("undo open");
    assert_eq!(visible(&mut document), "▸ Outer title");
    assert_eq!(document.snapshot().as_str(), source);
    document.set_source_mode(true).expect("source mode");
    assert!(visible(&mut document).contains("<p>Hidden</p>"));
}

#[test]
fn details_without_summary_have_a_visible_default_header() {
    use yu_editor::LayoutConfig;
    let mut document = EditorDocument::new("<details><p>Body</p></details>");
    assert_eq!(
        document
            .block_layout(0, LayoutConfig::new(500.0, 26.0))
            .expect("closed")
            .visual()
            .text(),
        "▸ Details"
    );
    document
        .toggle_html_details(ByteOffset::new(1))
        .expect("open");
    assert_eq!(
        document
            .block_layout(0, LayoutConfig::new(500.0, 26.0))
            .expect("header")
            .visual()
            .text(),
        "▾ Details"
    );
    assert_eq!(
        document
            .block_layout(1, LayoutConfig::new(500.0, 26.0))
            .expect("body")
            .visual()
            .text(),
        "Body"
    );
}

#[test]
fn hidden_disclosure_images_do_not_create_visual_widgets() {
    use yu_editor::DecorationCache;
    let source =
        "<details><summary>Title</summary><p><img src='missing.png' alt='hidden'></p></details>";
    let mut document = EditorDocument::new(source);
    let count_widgets = |document: &EditorDocument| {
        let mut cache = DecorationCache::default();
        document
            .markdown()
            .blocks()
            .iter()
            .map(|block| {
                cache
                    .decorate(document.markdown(), block, None)
                    .expect("decorate")
                    .widgets()
                    .len()
            })
            .sum::<usize>()
    };
    assert_eq!(count_widgets(&document), 0);
    document
        .toggle_html_details(ByteOffset::new(1))
        .expect("expand");
    assert_eq!(count_widgets(&document), 1);
    document.undo().expect("undo");
    assert_eq!(count_widgets(&document), 0);
    assert_eq!(document.snapshot().as_str(), source);
}

#[test]
fn only_visible_summary_headers_offer_disclosure_actions() {
    let source = "<details><summary>Outer</summary><p>Body</p><details><summary>Inner</summary>Nested</details></details>";
    let mut document = EditorDocument::new(source);
    let inner = ByteOffset::new(source.find("Inner").expect("inner") as u64);
    let body = ByteOffset::new(source.find("Body").expect("body") as u64);
    assert!(
        document
            .html_disclosure_header_at(ByteOffset::new(1))
            .is_some()
    );
    assert!(document.html_disclosure_header_at(inner).is_none());
    assert!(document.html_disclosure_header_at(body).is_none());
    document
        .toggle_html_details(inner)
        .expect("hidden header ignored");
    document.toggle_html_details(body).expect("body ignored");
    assert_eq!(document.snapshot().as_str(), source);
    assert_eq!(document.history_stats().undo_entries(), 0);
    document
        .toggle_html_details(ByteOffset::new(1))
        .expect("open parent");
    let current = document.snapshot();
    let inner = ByteOffset::new(current.as_str().find("Inner").expect("inner") as u64);
    let disclosure = document
        .html_disclosure_header_at(inner)
        .expect("visible inner");
    assert!(!disclosure.open);
    assert!(disclosure.summary.is_some());
    document
        .execute(yu_editor::EditorCommand::ToggleHtmlDetails { source: inner })
        .expect("open inner");
    assert!(
        document
            .snapshot()
            .as_str()
            .contains("<details open><summary>Inner")
    );
}

#[test]
fn accessibility_disclosure_hides_body_links_and_tracks_expansion() {
    use yu_editor::{
        AccessibilitySemanticKind as Kind, AccessibilitySemanticSnapshot, AccessibilityTextSnapshot,
    };
    let source = "<details><summary>Title中文</summary><p>Secret <a href='https://example.com'>Hidden link</a></p></details>";
    let mut document = EditorDocument::new(source);
    let snapshot = AccessibilitySemanticSnapshot::from_document(&document).expect("semantics");
    let headers: Vec<_> = snapshot
        .nodes()
        .iter()
        .filter(|node| node.kind() == Kind::Disclosure)
        .collect();
    assert_eq!(headers.len(), 1);
    let text = AccessibilityTextSnapshot::from_document(&document).expect("text");
    assert_eq!(
        text.text_for_range(headers[0].label_range())
            .expect("label"),
        "Title中文"
    );
    assert_eq!(headers[0].flags() & 4, 0);
    assert!(
        !snapshot
            .nodes()
            .iter()
            .any(|node| node.kind() == Kind::Link)
    );
    assert!(
        !text
            .text_for_range(headers[0].source_range())
            .expect("header")
            .contains("Secret")
    );
    document
        .toggle_html_details(ByteOffset::new(0))
        .expect("open");
    let snapshot = AccessibilitySemanticSnapshot::from_document(&document).expect("open semantics");
    let headers: Vec<_> = snapshot
        .nodes()
        .iter()
        .filter(|node| node.kind() == Kind::Disclosure)
        .collect();
    assert_eq!(headers.len(), 1);
    assert_ne!(headers[0].flags() & 4, 0);
    assert!(
        snapshot
            .nodes()
            .iter()
            .any(|node| node.kind() == Kind::Link)
    );
    document.undo().expect("undo");
    assert_eq!(document.snapshot().as_str(), source);
}

#[test]
fn navigation_reveal_expands_only_target_ancestors_without_source_edits() {
    use yu_core::TextRange;
    use yu_editor::{
        AccessibilitySemanticKind as Kind, AccessibilitySemanticSnapshot, LayoutConfig,
    };
    let source = "<details><summary>Outer</summary><details><summary>Inner</summary><p>Target中文</p></details><details><summary>Other</summary><p>Keep hidden</p></details></details>";
    let mut document = EditorDocument::new(source);
    let revision = document.revision();
    let history = document.history_stats().undo_entries();
    let before = document.markdown().clone();
    let at = ByteOffset::new(source.find("Target").expect("target") as u64);
    assert!(
        document
            .reveal_html_range(TextRange::empty(at))
            .expect("reveal")
    );
    assert!(
        !document
            .reveal_html_range(TextRange::empty(at))
            .expect("already visible")
    );
    assert_eq!(document.snapshot().as_str(), source);
    assert_eq!(document.revision(), revision);
    assert_eq!(document.history_stats().undo_entries(), history);
    let old = before.html_regions().regions[0]
        .model
        .as_ref()
        .expect("old model")
        .disclosures();
    assert!(
        old.iter().all(|details| !details.open),
        "Published snapshot must remain immutable"
    );
    let visible: String = (0..document.markdown().blocks().len())
        .map(|index| {
            document
                .block_layout_for_visual_state(index, LayoutConfig::new(500.0, 26.0))
                .expect("layout")
                .visual()
                .text()
                .to_owned()
        })
        .collect();
    assert!(visible.contains("Target中文"), "{visible}");
    assert!(!visible.contains("Keep hidden"), "{visible}");
    let semantic = AccessibilitySemanticSnapshot::from_document(&document).expect("semantics");
    let flags: Vec<_> = semantic
        .nodes()
        .iter()
        .filter(|node| node.kind() == Kind::Disclosure)
        .map(|node| node.flags() & 4)
        .collect();
    assert_eq!(flags, [4, 4, 0]);
    document.set_source_mode(true).expect("literal mode");
    assert!(
        !document
            .reveal_html_range(TextRange::empty(at))
            .expect("no reveal in literal mode")
    );
}

#[test]
fn temporary_reveal_survives_edits_and_closes_without_dirtying_source() {
    use yu_core::TextRange;
    use yu_text::{Edit, Transaction};
    let source = "<details><summary>Title</summary><p>Target</p></details>";
    let mut document = EditorDocument::new(source);
    let target = ByteOffset::new(source.find("Target").expect("target") as u64);
    document
        .reveal_html_range(TextRange::empty(target))
        .expect("reveal");
    document
        .apply_transaction(&Transaction::new(
            document.revision(),
            [
                Edit::new(TextRange::empty(ByteOffset::ZERO), "Prefix\n\n"),
                Edit::new(TextRange::empty(target), "中文"),
            ],
        ))
        .expect("edit");
    let text = document.snapshot().as_str().to_owned();
    let header = ByteOffset::new(text.find("Title").expect("title") as u64);
    assert!(
        document
            .html_disclosure_header_at(header)
            .expect("header")
            .open
    );
    let revision = document.revision();
    let history = document.history_stats().undo_entries();
    let closed = document
        .toggle_html_details(header)
        .expect("close transient reveal");
    assert_eq!(closed.source_sync(), yu_editor::SourceSync::None);
    assert!(
        !document
            .html_disclosure_header_at(header)
            .expect("closed header")
            .open
    );
    assert_eq!(document.revision(), revision);
    assert_eq!(document.snapshot().as_str(), text);
    assert_eq!(document.history_stats().undo_entries(), history);
    document.undo().expect("undo actual edit");
    assert_eq!(document.snapshot().as_str(), source);
}

#[test]
fn folded_headings_keep_toc_outline_and_generated_anchors_stable() {
    use yu_core::TextRange;
    use yu_editor::{OutlineTree, TocProjection, document_anchor_target};
    let source = "[toc]\n\n# Same\n\n<details><summary>Outer</summary><h2>Same</h2><details><summary>Inner</summary><h3>中文 <em>Heading</em></h3></details></details>\n\n# Same\n";
    let mut document = EditorDocument::new(source);
    let headings = document.markdown().table_of_contents().headings().to_vec();
    assert_eq!(headings.len(), 4);
    let toc = TocProjection::build(document.markdown()).expect("toc");
    assert_eq!(&*toc.text, "Same\n  Same\n    中文 Heading\nSame");
    let outline = OutlineTree::build(&mut document).expect("outline");
    assert_eq!(
        outline
            .rows()
            .iter()
            .map(|row| row.label())
            .collect::<Vec<_>>(),
        ["Same", "Same", "中文 Heading", "Same"]
    );
    let target = document_anchor_target(document.markdown(), "#中文-heading")
        .expect("anchor")
        .expect("hidden heading");
    assert_eq!(
        &source[target.start().get() as usize..target.end().get() as usize],
        "中文 <em>Heading</em>"
    );
    let duplicate = document_anchor_target(document.markdown(), "#same-2")
        .expect("duplicate")
        .expect("last heading");
    assert!(duplicate.start().get() > target.start().get());
    document
        .reveal_html_range(TextRange::empty(target.start()))
        .expect("reveal");
    assert_eq!(document.markdown().table_of_contents().headings(), headings);
    assert_eq!(
        TocProjection::build(document.markdown()).expect("open toc"),
        toc
    );
    document
        .toggle_html_details(ByteOffset::new(source.find("Outer").expect("outer") as u64))
        .expect("close");
    assert_eq!(
        TocProjection::build(document.markdown()).expect("closed toc"),
        toc
    );
    document.set_source_mode(true).expect("source mode");
    assert_eq!(
        TocProjection::build(document.markdown()).expect("literal toc"),
        toc
    );
    assert_eq!(document.snapshot().as_str(), source);
    assert_eq!(document.history_stats().undo_entries(), 0);
}

#[test]
fn closing_details_moves_hidden_selection_to_summary_and_restores_history() {
    use yu_core::{CaretAffinity, TextRange};
    use yu_editor::{EditorCommand, EditorSelection};
    for temporary in [false, true] {
        let source = if temporary {
            "<details><summary>Title中文</summary><p>Hidden🙂</p></details>\n\nOutside"
        } else {
            "<details open><summary>Title中文</summary><p>Hidden🙂</p></details>\n\nOutside"
        };
        let mut document = EditorDocument::new(source);
        let hidden = ByteOffset::new(source.find("Hidden").expect("body") as u64);
        if temporary {
            document
                .reveal_html_range(TextRange::empty(hidden))
                .expect("reveal");
        }
        document
            .set_selection(
                EditorSelection::cursor(&document.snapshot(), hidden, CaretAffinity::Downstream)
                    .expect("cursor"),
            )
            .expect("select");
        let history = document.history_stats().undo_entries();
        let header = ByteOffset::new(source.find("Title").expect("header") as u64);
        document.toggle_html_details(header).expect("close");
        let title_end = document
            .snapshot()
            .as_str()
            .find("</summary>")
            .expect("title end") as u64;
        assert_eq!(document.selection().focus(), ByteOffset::new(title_end));
        if temporary {
            assert_eq!(document.snapshot().as_str(), source);
            assert_eq!(document.history_stats().undo_entries(), history);
        } else {
            document.undo().expect("undo close");
            assert_eq!(document.snapshot().as_str(), source);
            assert_eq!(document.selection().focus(), hidden);
            document.redo().expect("redo close");
            assert_eq!(document.selection().focus(), ByteOffset::new(title_end));
        }
        document
            .execute(EditorCommand::insert_text("!"))
            .expect("type after close");
        assert!(
            document
                .snapshot()
                .as_str()
                .contains("Title中文!</summary>")
        );
        assert!(document.snapshot().as_str().contains("<p>Hidden🙂</p>"));
    }
}

#[test]
fn folding_preserves_outside_carets_and_merges_hidden_ranges() {
    use yu_core::CaretAffinity;
    use yu_editor::{EditorCommand, EditorSelection};
    let source = "<details open><summary>Title</summary><p>Hidden body</p></details>\n\nOutside";
    let mut document = EditorDocument::new(source);
    let snapshot = document.snapshot();
    let hidden = ByteOffset::new(source.find("Hidden").expect("hidden") as u64);
    let body = ByteOffset::new(source.find("body").expect("body") as u64);
    document
        .set_selections(
            [
                EditorSelection::range(
                    &snapshot,
                    hidden,
                    ByteOffset::new(hidden.get() + 6),
                    CaretAffinity::Downstream,
                )
                .expect("hidden range"),
                EditorSelection::cursor(&snapshot, body, CaretAffinity::Downstream)
                    .expect("hidden caret"),
                EditorSelection::cursor(&snapshot, snapshot.len_bytes(), CaretAffinity::Downstream)
                    .expect("outside caret"),
            ],
            1,
        )
        .expect("select");
    document
        .toggle_html_details(ByteOffset::new(0))
        .expect("close");
    assert_eq!(document.selections().as_slice().len(), 2);
    let title_end = ByteOffset::new(
        document
            .snapshot()
            .as_str()
            .find("</summary>")
            .expect("summary") as u64,
    );
    assert_eq!(document.selection().focus(), title_end);
    document
        .execute(EditorCommand::insert_text("!"))
        .expect("type");
    assert!(document.snapshot().as_str().contains("Title!</summary>"));
    assert!(document.snapshot().as_str().contains("<p>Hidden body</p>"));
    assert!(document.snapshot().as_str().ends_with("Outside!"));
    document.undo().expect("undo typing");
    document.undo().expect("undo closing");
    assert_eq!(document.snapshot().as_str(), source);
    assert_eq!(document.selections().as_slice().len(), 3);
    assert_eq!(document.selection().focus(), body);
}

#[test]
fn undo_and_redo_reveal_the_restored_hidden_edit_location() {
    use yu_core::{CaretAffinity, TextRange};
    use yu_editor::{EditorCommand, EditorSelection};
    let source = "<details><summary>Title</summary><p>Hidden</p></details>";
    let mut document = EditorDocument::new(source);
    let hidden = ByteOffset::new(source.find("Hidden").expect("body") as u64);
    document
        .reveal_html_range(TextRange::empty(hidden))
        .expect("reveal");
    document
        .set_selection(
            EditorSelection::cursor(&document.snapshot(), hidden, CaretAffinity::Downstream)
                .expect("cursor"),
        )
        .expect("select");
    document
        .execute(EditorCommand::insert_text("X"))
        .expect("edit body");
    document
        .toggle_html_details(ByteOffset::ZERO)
        .expect("close");
    assert!(
        !document
            .html_disclosure_header_at(ByteOffset::ZERO)
            .expect("header")
            .open
    );
    document.undo().expect("undo body edit");
    assert_eq!(document.snapshot().as_str(), source);
    assert_eq!(document.selection().focus(), hidden);
    assert!(
        document
            .html_disclosure_header_at(ByteOffset::ZERO)
            .expect("header")
            .open
    );
    assert_eq!(document.history_stats().redo_entries(), 1);
    document
        .toggle_html_details(ByteOffset::ZERO)
        .expect("close again");
    document.redo().expect("redo body edit");
    assert!(
        document
            .html_disclosure_header_at(ByteOffset::ZERO)
            .expect("header")
            .open
    );
    assert!(document.snapshot().as_str().contains("<p>XHidden</p>"));
    assert!(!document.snapshot().as_str().contains("<details open"));
}

#[test]
fn history_reveals_a_body_end_caret_without_opening_unrelated_details() {
    use yu_core::{CaretAffinity, TextRange};
    use yu_editor::{EditorCommand, EditorSelection};
    let source = "<details><summary>Title</summary>Hidden</details>\n\n<details><summary>Other</summary>Unrelated</details>";
    let mut document = EditorDocument::new(source);
    let end = ByteOffset::new(source.find("</details>").expect("body end") as u64);
    document
        .reveal_html_range(TextRange::empty(end))
        .expect("reveal at end");
    document
        .set_selection(
            EditorSelection::cursor(&document.snapshot(), end, CaretAffinity::Downstream)
                .expect("cursor"),
        )
        .expect("select");
    document
        .execute(EditorCommand::insert_text("X"))
        .expect("edit body end");
    document
        .toggle_html_details(ByteOffset::ZERO)
        .expect("close");
    document.undo().expect("undo");
    assert_eq!(document.snapshot().as_str(), source);
    assert_eq!(document.selection().focus(), end);
    assert!(
        document
            .html_disclosure_header_at(ByteOffset::ZERO)
            .expect("first")
            .open
    );
    let other = ByteOffset::new(source.find("Other").expect("other") as u64);
    assert!(
        !document
            .html_disclosure_header_at(other)
            .expect("other")
            .open
    );
    assert_eq!(document.history_stats().undo_entries(), 0);
    assert_eq!(document.history_stats().redo_entries(), 1);
}

#[test]
fn returning_from_source_mode_reveals_nested_selection_without_source_edits() {
    use yu_core::CaretAffinity;
    use yu_editor::EditorSelection;
    let source = "<details><summary>Outer</summary><details><summary>Inner</summary><p>Hidden中文</p></details></details>\n\n<details><summary>Other</summary>Secret</details>\n\nOutside";
    let mut document = EditorDocument::new(source);
    document.set_source_mode(true).expect("source mode");
    let snapshot = document.snapshot();
    let hidden = ByteOffset::new(source.find("Hidden").expect("hidden") as u64);
    let outside = snapshot.len_bytes();
    document
        .set_selections(
            [
                EditorSelection::cursor(&snapshot, hidden, CaretAffinity::Downstream)
                    .expect("hidden cursor"),
                EditorSelection::cursor(&snapshot, outside, CaretAffinity::Downstream)
                    .expect("outside cursor"),
            ],
            1,
        )
        .expect("selections");
    let selections = document.selections().clone();
    for _ in 0..2 {
        document.set_source_mode(false).expect("preview");
        for label in ["Outer", "Inner"] {
            let at = ByteOffset::new(source.find(label).expect("header") as u64);
            assert!(
                document
                    .html_disclosure_header_at(at)
                    .expect("visible header")
                    .open
            );
        }
        let other = ByteOffset::new(source.find("Other").expect("other") as u64);
        assert!(
            !document
                .html_disclosure_header_at(other)
                .expect("other header")
                .open
        );
        assert_eq!(document.snapshot().as_str(), source);
        assert_eq!(document.revision(), snapshot.revision());
        assert_eq!(document.selections(), &selections);
        assert_eq!(document.history_stats().undo_entries(), 0);
        document.set_source_mode(true).expect("source again");
    }
}
