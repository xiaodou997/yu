use yu_core::{ByteOffset, TextRange};
use yu_editor::{EditorCommand, EditorDocument, LayoutConfig};

fn visual(document: &mut EditorDocument) -> String {
    let view = document
        .block_layout_for_visual_state(0, LayoutConfig::new(500.0, 16.0))
        .expect("cell layout");
    assert!(
        view.table().is_some(),
        "details cell must retain the native grid"
    );
    view.visual().text().to_owned()
}

#[test]
fn cell_details_toggle_summary_and_default_label_without_losing_source() {
    for summary in ["<summary>标题</summary>", ""] {
        let source = format!(
            "<table><tr><td><details>{summary}<p>Hidden body</p></details></td><td>KEEP</td></tr></table>\r\n"
        );
        let mut doc = EditorDocument::new(&source);
        let closed = visual(&mut doc);
        assert!(closed.contains(if summary.is_empty() {
            "▸ Details"
        } else {
            "▸ 标题"
        }));
        assert!(!closed.contains("Hidden body"));
        let at = ByteOffset::new(source.find("<details>").expect("details") as u64);
        doc.toggle_html_details(at).expect("open");
        let opened = visual(&mut doc);
        assert!(opened.contains("▾ ") && opened.contains("Hidden body"));
        assert_eq!(
            doc.snapshot().as_str(),
            source.replacen("<details>", "<details open>", 1)
        );
        doc.undo().expect("undo open");
        assert_eq!(visual(&mut doc), closed);
        assert_eq!(doc.snapshot().as_str(), source);
    }
}

#[test]
fn revealing_a_hidden_cell_target_rebuilds_cell_partitions_without_editing_source() {
    let source = "<table><tr><td><details><summary>Outer</summary><details><summary>Inner</summary><p>Target</p></details></details></td></tr></table>";
    let mut doc = EditorDocument::new(source);
    let at = ByteOffset::new(source.find("Target").expect("target") as u64);
    let revision = doc.revision();
    assert!(!visual(&mut doc).contains("Target"));
    assert!(doc.reveal_html_range(TextRange::empty(at)).expect("reveal"));
    let opened = visual(&mut doc);
    assert!(opened.contains("Outer") && opened.contains("Inner") && opened.contains("Target"));
    assert_eq!(doc.revision(), revision);
    assert_eq!(doc.snapshot().as_str(), source);
    doc.toggle_html_details(ByteOffset::new(
        source.find("<details>").expect("outer") as u64
    ))
    .expect("fold revealed");
    assert!(!visual(&mut doc).contains("Target"));
    assert_eq!(doc.revision(), revision);
    assert_eq!(doc.snapshot().as_str(), source);
}

#[test]
fn editing_a_cell_summary_keeps_hidden_body_and_restores_it_on_undo() {
    let source = "<table><tr><td><details><summary>Title</summary><p>DO NOT CHANGE</p></details></td></tr></table>";
    let mut doc = EditorDocument::new(source);
    visual(&mut doc);
    let at = ByteOffset::new(source.find("Title").expect("summary") as u64);
    doc.set_selection(
        yu_editor::EditorSelection::cursor(
            &doc.snapshot(),
            at,
            yu_editor::CaretAffinity::Downstream,
        )
        .expect("caret"),
    )
    .expect("select");
    doc.execute(EditorCommand::insert_text("中文<&"))
        .expect("summary input");
    assert_eq!(
        doc.snapshot().as_str(),
        source.replace("Title", "中文&lt;&amp;Title")
    );
    assert!(!visual(&mut doc).contains("DO NOT CHANGE"));
    doc.undo().expect("undo");
    assert_eq!(doc.snapshot().as_str(), source);
}

#[test]
fn folded_cell_images_are_not_widgets_and_hidden_headers_are_not_click_targets() {
    let source = "<table><tr><td><details><summary>Outer<img src='summary.png'></summary><p><img src='outer.png'></p><details><summary>Inner</summary><p><img src='inner.png'></p></details></details></td></tr></table>";
    let mut doc = EditorDocument::new(source);
    let count = |doc: &EditorDocument| {
        yu_editor::DecorationCache::default()
            .decorate(
                doc.markdown(),
                doc.markdown().blocks().get(0).expect("table"),
                None,
            )
            .expect("decorations")
            .widgets()
            .len()
    };
    assert_eq!(count(&doc), 1);
    assert_eq!(
        yu_markdown::image_spans(doc.markdown(), &doc.snapshot(), None).len(),
        3
    );
    assert!(
        doc.html_disclosure_header_at(ByteOffset::new(source.find("Inner").expect("inner") as u64))
            .is_none()
    );
    doc.toggle_html_details(ByteOffset::new(source.find("Outer").expect("outer") as u64))
        .expect("outer open");
    assert_eq!(count(&doc), 2);
    let inner = ByteOffset::new(doc.snapshot().as_str().find("Inner").expect("inner") as u64);
    doc.toggle_html_details(inner).expect("inner open");
    assert_eq!(count(&doc), 3);
    let outer = ByteOffset::new(doc.snapshot().as_str().find("Outer").expect("outer") as u64);
    doc.toggle_html_details(outer).expect("outer closed");
    assert_eq!(count(&doc), 1);
    for expected in [3, 2, 1] {
        doc.undo().expect("undo");
        assert_eq!(count(&doc), expected);
    }
    assert_eq!(doc.snapshot().as_str(), source);
}

#[test]
fn source_only_tables_do_not_expose_disclosure_actions() {
    for attrs in [" colspan='0'", " class='unknown'"] {
        let source = format!(
            "<table><tr><td{attrs}><details><summary>Title</summary>Body</details></td></tr></table>"
        );
        let mut doc = EditorDocument::new(&source);
        let at = ByteOffset::new(source.find("Title").expect("title") as u64);
        assert!(doc.html_disclosure_header_at(at).is_none());
        assert!(
            !doc.toggle_html_details(at)
                .expect("raw source no-op")
                .changed()
        );
        assert_eq!(doc.snapshot().as_str(), source);
    }
}

#[test]
fn folding_a_cell_relocates_hidden_caret_and_undo_restores_open_geometry() {
    let source = "<table><tr><td><details open><summary>Title</summary><p>Body</p></details></td><td>KEEP</td></tr></table>";
    let mut doc = EditorDocument::new(source);
    let body = ByteOffset::new(source.find("Body").expect("body") as u64);
    doc.set_selection(
        yu_editor::EditorSelection::cursor(
            &doc.snapshot(),
            body,
            yu_editor::CaretAffinity::Downstream,
        )
        .expect("body cursor"),
    )
    .expect("select body");
    let open_height = doc
        .block_layout_for_visual_state(0, LayoutConfig::new(500.0, 16.0))
        .expect("open")
        .table()
        .expect("table")
        .bounds()
        .height();
    doc.toggle_html_details(ByteOffset::new(source.find("Title").expect("title") as u64))
        .expect("close");
    let expected = doc
        .snapshot()
        .as_str()
        .find("</summary>")
        .expect("summary end") as u64;
    assert_eq!(doc.selection().focus().get(), expected);
    let closed = doc
        .block_layout_for_visual_state(0, LayoutConfig::new(500.0, 16.0))
        .expect("closed");
    assert!(closed.table().expect("table").bounds().height() < open_height);
    doc.undo().expect("undo close");
    assert_eq!(doc.snapshot().as_str(), source);
    assert_eq!(doc.selection().focus(), body);
    assert_eq!(
        doc.block_layout_for_visual_state(0, LayoutConfig::new(500.0, 16.0))
            .expect("restored")
            .table()
            .expect("table")
            .bounds()
            .height(),
        open_height
    );
}
