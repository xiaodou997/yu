use yu_core::{ByteOffset, TextRange};
use yu_projection::{Projection, ProjectionBias, VisualOffset, VisualRunKind};
use yu_text::TextBuffer;

#[test]
fn parser_owned_projection_partitions_source_and_visual_ranges() {
    let source = "before **羽🙂**\n[Yu](https://example.com)\n";
    let snapshot = TextBuffer::new(source).snapshot();
    let range = TextRange::new(ByteOffset::ZERO, snapshot.len_bytes())
        .expect("the complete source range should be ordered");
    let inline = yu_markdown::parse_inline(&snapshot, range).expect("inline CST should parse");
    let projection = Projection::from_inline(&inline).expect("projection should consume the CST");

    let mut source_cursor = range.start();
    let mut visual_cursor = VisualOffset::ZERO;
    let mut visible_text = String::new();
    for run in projection.runs() {
        assert_eq!(
            run.source().start(),
            source_cursor,
            "projection runs must cover source without gaps"
        );
        assert_eq!(
            run.visual().start(),
            visual_cursor,
            "projection runs must cover visual bytes without gaps"
        );
        if run.kind() != VisualRunKind::HiddenSyntax {
            visible_text.push_str(
                &projection
                    .text_for_run(*run)
                    .expect("every visible run should resolve against the source snapshot"),
            );
        }
        source_cursor = run.source().end();
        visual_cursor = run.visual().end();
    }

    assert_eq!(source_cursor, range.end());
    assert_eq!(visual_cursor, projection.visual_len());
    assert_eq!(visible_text, "before 羽🙂\nYu\n");

    let strong_open = ByteOffset::new("before ".len() as u64);
    assert_eq!(
        projection
            .source_to_visual(strong_open, ProjectionBias::Before)
            .expect("hidden delimiter start should map"),
        projection
            .source_to_visual(strong_open, ProjectionBias::After)
            .expect("hidden delimiter end should map")
    );
    assert_eq!(
        projection
            .visual_to_source(
                VisualOffset::new("before ".len() as u64),
                ProjectionBias::Before
            )
            .expect("visual boundary should map to source before delimiter"),
        strong_open
    );
}
