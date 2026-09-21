use yu_syntax::{NodeKind, Tree, parse};

fn ranges(source: &str, kind: NodeKind) -> Vec<String> {
    fn walk(tree: &Tree, offset: usize, source: &str, kind: NodeKind, out: &mut Vec<String>) {
        if tree.kind() == kind {
            out.push(source[offset..offset + tree.len_bytes() as usize].to_owned());
        }
        for index in 0..tree.child_count() {
            let (child, position) = tree.child(index).expect("valid fixture syntax");
            walk(child, offset + position as usize, source, kind, out);
        }
    }
    let parsed = parse(source).expect("valid fixture syntax");
    let mut out = Vec::new();
    walk(parsed.tree(), 0, source, kind, &mut out);
    out
}

#[test]
fn writing_marks_keep_unicode_ranges_and_nested_semantics() {
    let source = "==中文 **bold** and `code`== H~2~O x^n+1^";
    assert_eq!(
        ranges(source, NodeKind::Highlight),
        ["==中文 **bold** and `code`=="]
    );
    assert_eq!(ranges(source, NodeKind::StrongEmphasis), ["**bold**"]);
    assert_eq!(ranges(source, NodeKind::InlineCode), ["`code`"]);
    assert_eq!(ranges(source, NodeKind::Subscript), ["~2~"]);
    assert_eq!(ranges(source, NodeKind::Superscript), ["^n+1^"]);
    assert_eq!(ranges(source, NodeKind::HighlightMark), ["==", "=="]);
    assert_eq!(ranges(source, NodeKind::ScriptMark), ["~", "~", "^", "^"]);
}

#[test]
fn opaque_and_unclosed_content_stays_literal() {
    for source in [
        "`==code== x^2^ H~2~O`",
        "$x^2^ + a~b~ + ==c==$",
        "\\==escaped==",
        "\\^escaped^",
        "\\~escaped~",
        "==open",
        "x^open",
        "H~open",
        "===long===",
        "~~strike~~",
        "^two words^",
        "~two\nwords~",
        "== leading==",
        "==trailing ==",
    ] {
        for kind in [
            NodeKind::Highlight,
            NodeKind::Superscript,
            NodeKind::Subscript,
        ] {
            assert!(ranges(source, kind).is_empty(), "{source:?}: {kind:?}");
        }
    }
}

#[test]
fn link_labels_allow_marks_but_destinations_do_not() {
    let source = "[==label==](https://example.com/x^2^) ![x~2~](image==path==.png)";
    assert_eq!(ranges(source, NodeKind::Highlight), ["==label=="]);
    assert!(ranges(source, NodeKind::Superscript).is_empty());
    assert_eq!(ranges(source, NodeKind::Subscript), ["~2~"]);
}
