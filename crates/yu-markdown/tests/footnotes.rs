use yu_core::{ByteOffset, TextRange};
use yu_markdown::{ExtensionSet, parse};
use yu_text::TextBuffer;

#[test]
fn footnote_numbers_follow_body_order_and_keep_forward_unicode_references() {
    let source = "正文[^二] then[^ONE] repeat[^二].\n\n[^one]: first\n\n[^二]: **second**\n    continuation\n\n    another paragraph\n\nOutside\n";
    let snapshot = TextBuffer::new(source).snapshot();
    let document = parse(&snapshot);
    let index = document.footnotes();
    assert_eq!(
        index
            .references()
            .iter()
            .map(|r| r.number)
            .collect::<Vec<_>>(),
        [Some(1), Some(2), Some(1)]
    );
    assert_eq!(
        index
            .definitions()
            .iter()
            .map(|d| d.number)
            .collect::<Vec<_>>(),
        [Some(2), Some(1)]
    );
    assert!(index.diagnostics().is_empty());
    let second = &index.definitions()[1];
    let definition_text =
        &source[second.source.start().get() as usize..second.source.end().get() as usize];
    assert!(definition_text.contains("another paragraph"));
    assert!(!definition_text.contains("Outside"));
    assert_eq!(
        index.navigation_target(index.references()[0].source.start()),
        Some(second.source)
    );
    assert_eq!(
        index.navigation_target(second.marker.start()),
        Some(index.references()[0].source)
    );
    assert_eq!(document.source().as_str(), source);
}

#[test]
fn nested_refs_are_bounded_and_duplicate_or_missing_labels_stay_diagnostic() {
    let source = "Main[^a] [^missing] [^dup].\n\n[^a]: See [^b].\n\n[^b]: See [^a].\n\n[^dup]: one\n[^DUP]: two\n";
    let snapshot = TextBuffer::new(source).snapshot();
    let document = parse(&snapshot);
    let index = document.footnotes();
    assert_eq!(
        index
            .definitions()
            .iter()
            .map(|d| d.number)
            .collect::<Vec<_>>(),
        [Some(1), Some(2), None, None]
    );
    assert_eq!(index.diagnostics().len(), 4);
    for needle in ["[^missing]", "[^dup]"] {
        let at = ByteOffset::new(source.find(needle).expect("reference") as u64);
        assert!(index.diagnostic_at(at).is_some());
        assert!(index.navigation_target(at).is_none());
    }
}

#[test]
fn footnotes_in_opaque_blocks_do_not_create_references_or_definitions() {
    let source = "---\nvalue: '[^yaml]'\n---\n\n`[^code]` $[^math]$ \\[^escaped]\n\n```text\n[^fence]: value\n```\n";
    let snapshot = TextBuffer::new(source).snapshot();
    let document = parse(&snapshot);
    assert!(document.footnotes().references().is_empty());
    assert!(document.footnotes().definitions().is_empty());
}

#[test]
fn definition_paragraphs_share_the_presentation_tree_and_numeric_substitution() {
    let source = "Body[^note].\n\n[^note]: definition **bold**\n    continuation\n\n    - nested item\n\nAfter\n";
    let snapshot = TextBuffer::new(source).snapshot();
    let document = parse(&snapshot);
    let tree = document.tree().expect("tree");
    let mut substitutions = Vec::new();
    for block in document.blocks().iter() {
        let decorated = ExtensionSet::markdown()
            .decorate(
                &snapshot,
                tree,
                document.reference_definitions(),
                document.presentation(),
                block,
                None,
            )
            .expect("decorate");
        for entry in decorated.set().all() {
            if let yu_decoration::Decoration::Substitute { ref text } = entry.decoration {
                let mut value = String::new();
                text.append_to(&mut value);
                substitutions.push((entry.range, value));
            }
        }
    }
    assert_eq!(substitutions.len(), 2);
    assert!(substitutions.iter().all(|(_, text)| text == "1"));
    let all = TextRange::new(ByteOffset::ZERO, snapshot.len_bytes()).expect("document");
    assert_eq!(all.end().get(), source.len() as u64);
}
