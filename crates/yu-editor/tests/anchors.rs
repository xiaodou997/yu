use yu_editor::{EditorCommand, EditorDocument, document_anchor_target};

#[test]
fn fragments_resolve_visible_heading_text_duplicates_and_html_ids() {
    let source = "[jump](#中文-title)\r\n\r\n# 中文 **Title**!\r\n\r\n## 中文 Title\r\n\r\n# 中文 Title-1\r\n\r\n<div><p id='explicit'>Target</p><h2>HTML <em>Heading</em></h2></div>\r\n";
    let document = EditorDocument::new(source);
    for (fragment, expected) in [
        ("#%E4%B8%AD%E6%96%87-title", "中文 **Title**!"),
        ("#中文-title-1", "中文 Title\r"),
        ("#中文-title-1-1", "中文 Title-1"),
        ("#explicit", "<p id='explicit'>Target</p>"),
        ("#html-heading", "HTML <em>Heading</em>"),
    ] {
        let target = document_anchor_target(document.markdown(), fragment)
            .expect("query")
            .expect(fragment);
        let raw = &source[target.start().get() as usize..target.end().get() as usize];
        assert!(raw.starts_with(expected), "{fragment}: {raw:?}");
    }
    for absent in [
        "https://example.com/#explicit",
        "#missing",
        "#%ff",
        "#%2",
        "#%zz",
    ] {
        assert!(
            document_anchor_target(document.markdown(), absent)
                .expect("query")
                .is_none()
        );
    }
    assert_eq!(
        document_anchor_target(document.markdown(), "#")
            .expect("top")
            .expect("source")
            .start()
            .get(),
        0
    );
}

#[test]
fn anchor_targets_follow_edits_modes_and_undo_without_rewriting_source() {
    let original = "# First\n\n<div class='unknown'><p id='hidden'>No target</p></div>\n";
    let mut document = EditorDocument::new(original);
    assert!(
        document_anchor_target(document.markdown(), "#hidden")
            .expect("query")
            .is_none()
    );
    let before = document_anchor_target(document.markdown(), "#first")
        .expect("query")
        .expect("heading");
    document
        .execute(EditorCommand::insert_text("prefix\n\n"))
        .expect("edit");
    let after = document_anchor_target(document.markdown(), "#first")
        .expect("query")
        .expect("heading");
    assert_eq!(after.start().get(), before.start().get() + 8);
    document.set_source_mode(true).expect("source mode");
    assert_eq!(
        document_anchor_target(document.markdown(), "#first").expect("query"),
        Some(after)
    );
    document.undo().expect("undo");
    assert_eq!(document.markdown().source().as_str(), original);
    assert_eq!(
        document_anchor_target(document.markdown(), "#first").expect("query"),
        Some(before)
    );
}

#[test]
fn inline_ids_use_parser_owned_elements_and_first_source_order() {
    let source = "Before <a id='中文'>first</a>.\n\n<div><p id='中文'>second</p></div>\n\nOther <b><a id='nested'>nested</a></b>.\n";
    let document = EditorDocument::new(source);
    for (fragment, opening) in [
        ("#%E4%B8%AD%E6%96%87", "<a id='中文'>"),
        ("#nested", "<a id='nested'>"),
    ] {
        let target = document_anchor_target(document.markdown(), fragment)
            .expect("query")
            .expect("target");
        assert_eq!(
            target.start().get() as usize,
            source.find(opening).expect("source")
        );
    }
    for source in [
        "Before `<a id='hidden'>code</a>`.",
        "Before $<a id='hidden'>math</a>$.",
        "Before <unknown><a id='hidden'>raw</a></unknown>.",
        "Before <b><a id='hidden'>unclosed parent</a>.",
        "Before <b><a id='hidden'>broken</b></a>.",
    ] {
        let document = EditorDocument::new(source);
        assert!(
            document_anchor_target(document.markdown(), "#hidden")
                .expect("query")
                .is_none(),
            "{source}"
        );
    }
}

#[test]
fn html_links_publish_revision_bound_accessibility_nodes_without_fake_markdown_links() {
    use yu_editor::{AccessibilitySemanticKind, AccessibilitySemanticSnapshot};
    let source = "羽🙂 [earlier](https://first.example) <a href='https://example.com/?a=1&amp;b=2'>行内</a> [later](https://last.example).\n\n<div><p><a href='#part'>块链接</a> [literal](https://wrong.example)</p><p id='part'>Target</p></div>\n";
    let document = EditorDocument::new(source);
    let snapshot = AccessibilitySemanticSnapshot::from_document(&document).expect("AX tree");
    let links: Vec<_> = snapshot
        .nodes()
        .iter()
        .filter(|node| node.kind() == AccessibilitySemanticKind::Link)
        .collect();
    assert_eq!(links.len(), 4);
    for (node, text) in links.iter().zip(["earlier", "行内", "later", "块链接"]) {
        let byte = source.find(text).expect("label");
        assert_eq!(
            node.label_range().revision(),
            document.markdown().revision()
        );
        assert_eq!(
            node.label_range().range().start().get(),
            source[..byte].encode_utf16().count() as u64
        );
        assert_eq!(
            node.label_range().range().end().get(),
            source[..byte + text.len()].encode_utf16().count() as u64
        );
        assert!(node.parent().is_some());
    }
}

#[test]
fn excessive_inline_html_nesting_does_not_publish_partial_anchors() {
    let source = format!(
        "before {}<a id='deep'>body</a>{}",
        "<b>".repeat(150),
        "</b>".repeat(150)
    );
    let document = EditorDocument::new(source);
    assert!(
        document_anchor_target(document.markdown(), "#deep")
            .expect("query")
            .is_none()
    );
}
