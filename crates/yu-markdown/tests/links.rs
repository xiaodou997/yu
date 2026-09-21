use yu_core::ByteOffset;
use yu_text::TextBuffer;

#[test]
fn markdown_and_html_targets_decode_without_mutating_source() {
    for (source, text, target) in [
        (
            "[中文](https://example.com/a\\(b\\)?x=1&amp;y=2)",
            "中文",
            "https://example.com/a(b)?x=1&y=2",
        ),
        (
            "[说明][ref]\n\n[ref]: ../a%20b.md#part\n",
            "说明",
            "../a%20b.md#part",
        ),
        ("<hello@example.com>", "hello", "mailto:hello@example.com"),
        (
            "Before <a href='https://example.com/?x=1&amp;y=2'>中文</a> after",
            "中文",
            "https://example.com/?x=1&y=2",
        ),
        (
            "<div><p><a href='#target'><b>中文</b></a></p></div>",
            "中文",
            "#target",
        ),
    ] {
        let mut document = yu_markdown::parse(&TextBuffer::new(source).snapshot());
        let at = ByteOffset::new(source.find(text).expect("label") as u64);
        let link = document.link_at(at).expect(source);
        assert_eq!(link.destination, target);
        assert_eq!(link.revision, document.revision());
        assert!(link.label.start() <= at && at < link.label.end());
        assert_eq!(document.source().as_str(), source);
        document.set_source_mode(true);
        assert_eq!(document.link_at(at), Some(link));
    }
}

#[test]
fn opaque_unresolved_and_nonlabel_positions_have_no_link_action() {
    for (source, text) in [
        ("`[code](https://example.com)`", "code"),
        ("```\n[a](https://example.com)\n```", "[a]"),
        ("---\nvalue: '[a](https://example.com)'\n---\n", "[a]"),
        ("$[math](https://example.com)$", "math"),
        ("[missing][none]", "missing"),
        ("[label](https://example.com)", "https"),
        ("Before <unknown><a href='x'>label</a></unknown>", "label"),
        ("<div><a href='x' onclick='run()'>label</a></div>", "label"),
        ("Before `<a href='x'>label</a>`", "label"),
    ] {
        let document = yu_markdown::parse(&TextBuffer::new(source).snapshot());
        assert!(
            document
                .link_at(ByteOffset::new(source.find(text).expect("fixture") as u64))
                .is_none(),
            "{source}"
        );
        assert!(
            document
                .link_at(ByteOffset::new(source.len() as u64))
                .is_none()
        );
    }
}
