use yu_core::{ByteOffset, TextRange};
use yu_markdown::{parse, parse_incremental};
use yu_text::{Edit, TextBuffer, Transaction};

#[test]
fn toc_uses_semantic_headings_and_excludes_opaque_markers() {
    let source = "---\ntitle: '[toc]'\n# metadata\n---\n\n[TOC]\n\n### 中文🙂 ###\n\n# Root\n\nSetext\n======\n\n#### Child\n\n`[toc]`\n\n```text\n[toc]\n# code\n```\n\n$$\n[toc]\n$$\n\nText [toc]\n\n> [toc]\n";
    let mut document = parse(&TextBuffer::new(source).snapshot());
    let toc = document.table_of_contents().clone();
    assert_eq!(
        toc.headings().iter().map(|h| h.level).collect::<Vec<_>>(),
        [3, 1, 1, 4]
    );
    assert_eq!(
        toc.headings().iter().map(|h| h.parent).collect::<Vec<_>>(),
        [None, None, None, Some(2)]
    );
    assert_eq!(toc.markers().len(), 2);
    assert_eq!(
        toc.headings()
            .iter()
            .map(|h| &source[h.label.start().get() as usize..h.label.end().get() as usize])
            .collect::<Vec<_>>(),
        ["中文🙂", "Root", "Setext", "Child"]
    );
    document.set_source_mode(true);
    assert_eq!(document.table_of_contents(), &toc);
    assert_eq!(document.source().as_str(), source);
}

#[test]
fn toc_revisions_recompute_hierarchy_and_targets_after_source_edits() {
    let mut buffer = TextBuffer::new("[toc]\n\n# A\n\n### B\n\n## C\n");
    let mut document = parse(&buffer.snapshot());
    let old = document.table_of_contents().clone();
    for (needle, replacement) in [
        ("# A", "## 羽🙂"),
        ("### B", "# B"),
        ("[toc]", "[to"),
        ("[to", "[toc]"),
    ] {
        let start = buffer
            .snapshot()
            .as_str()
            .find(needle)
            .expect("edit target");
        let range = TextRange::new(
            ByteOffset::new(start as u64),
            ByteOffset::new((start + needle.len()) as u64),
        )
        .expect("range");
        let applied = buffer
            .apply(&Transaction::new(
                buffer.revision(),
                [Edit::new(range, replacement)],
            ))
            .expect("edit");
        document = parse_incremental(&document, applied.result_snapshot(), applied.change_set())
            .expect("incremental")
            .into_document();
        let full = parse(applied.result_snapshot());
        assert_eq!(document.table_of_contents(), full.table_of_contents());
    }
    assert_eq!(old.headings()[1].parent, Some(0));
    assert_eq!(document.table_of_contents().headings()[1].parent, None);
    assert_ne!(
        old.headings()[2].source,
        document.table_of_contents().headings()[2].source
    );
    assert_eq!(document.table_of_contents().markers().len(), 1);
}
