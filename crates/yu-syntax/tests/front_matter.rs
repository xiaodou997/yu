use yu_syntax::{NodeKind, parse};

#[test]
fn metadata_is_opaque_and_preserves_delimiters_and_unicode_ranges() {
    for (prefix, ending) in [("", "---"), ("\u{feff}", "...")] {
        let source = format!(
            "{prefix}---\r\ntitle: 中文\r\n\r\n# not a heading\r\nvalue: '$x^2$ **bold**'\r\n{ending}\r\n\r\n# Body\r\n"
        );
        let parsed = parse(source.as_str()).expect("metadata");
        let (metadata, offset) = parsed.tree().child(0).expect("metadata child");
        assert_eq!(offset, 0);
        assert_eq!(metadata.kind(), NodeKind::FrontMatter);
        let mut markers = Vec::new();
        for i in 0..metadata.child_count() {
            let (child, from) = metadata.child(i).expect("child");
            assert!(matches!(
                child.kind(),
                NodeKind::MetadataMark | NodeKind::MetadataText
            ));
            assert_eq!(child.child_count(), 0);
            let text = &source[from as usize..(from + child.len_bytes()) as usize];
            if child.kind() == NodeKind::MetadataMark {
                markers.push(text);
            }
        }
        assert_eq!(markers, ["---", ending]);
        assert_eq!(
            parsed.tree().child(1).expect("body").0.kind(),
            NodeKind::AtxHeading1
        );
    }
}

#[test]
fn metadata_is_document_initial_and_unclosed_content_remains_opaque() {
    for source in [
        "before\n\n---\nkey: value\n---\n",
        "> ---\n> key: value\n> ---\n",
        " ---\nkey: value\n---\n",
    ] {
        let parsed = parse(source).expect("ordinary Markdown");
        assert!(
            (0..parsed.tree().child_count()).all(|i| parsed
                .tree()
                .child(i)
                .expect("child")
                .0
                .kind()
                != NodeKind::FrontMatter)
        );
    }
    let source = "---\nkey: value\n\n# still metadata\n";
    let parsed = parse(source).expect("unclosed metadata");
    assert_eq!(parsed.tree().child_count(), 1);
    let metadata = parsed.tree().child(0).expect("metadata").0;
    assert_eq!(metadata.kind(), NodeKind::FrontMatter);
    assert_eq!(metadata.len_bytes() as usize, source.len());
}
