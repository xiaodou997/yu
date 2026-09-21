use yu_editor::{
    BlockView, DecorationCache, LayoutConfig, LayoutPoint, MonospaceMetrics, VisualText,
};
use yu_text::TextBuffer;

#[test]
fn html_alignment_inherits_and_overrides_while_moving_carets_and_hits() {
    let source = "<div align='center'><p>ABC</p><p style='text-align:right'>DEF</p><p align='left'>GHI</p></div>";
    let snapshot = TextBuffer::new(source).snapshot();
    let document = yu_markdown::parse(&snapshot);
    let mut cache = DecorationCache::default();
    for (block, expected_x) in document.blocks().iter().zip([88.0, 176.0, 0.0]) {
        let decorations = cache.decorate(&document, block, None).expect("decoration");
        let visual = VisualText::new(&snapshot, block.range(), decorations.set().clone())
            .expect("projection");
        let view = BlockView::build(
            block.kind(),
            &visual,
            &decorations,
            LayoutConfig::new(200.0, 16.0),
            &MonospaceMetrics::new(8.0),
        )
        .expect("layout");
        let cluster = view.clusters().first().expect("text");
        assert!(
            (cluster.x() - expected_x).abs() < 0.01,
            "{} != {expected_x}",
            cluster.x()
        );
        let caret = view
            .caret_for_source(cluster.source().start(), yu_decoration::Bias::After)
            .expect("caret");
        assert!((caret.point().x() - cluster.x()).abs() < 0.01);
        let hit = view
            .hit_test(LayoutPoint::new(
                cluster.x() + 1.0,
                cluster.y() + cluster.line_height() / 2.0,
            ))
            .expect("hit");
        assert_eq!(hit.content_source(), Some(cluster.source()));
    }
    assert_eq!(document.source().as_str(), source);
}

#[test]
fn justification_projects_text_and_emits_paragraph_layout_intent() {
    let source = "<div align='justify'><p>one two three</p></div>";
    let snapshot = TextBuffer::new(source).snapshot();
    let document = yu_markdown::parse(&snapshot);
    let block = document.blocks().get(0).expect("paragraph");
    let decorations = DecorationCache::default()
        .decorate(&document, block, None)
        .expect("decorations");
    assert_eq!(
        VisualText::new(&snapshot, block.range(), decorations.set().clone())
            .expect("projection")
            .text(),
        "one two three"
    );
    assert!(decorations.line_ornaments().iter().any(|(_, o)| matches!(
        o,
        yu_markdown::BlockOrnament::Alignment {
            alignment: yu_markdown::html::HtmlAlignment::Justify
        }
    )));
}

#[test]
fn each_wrapped_line_aligns_independently_and_active_source_removes_alignment() {
    for alignment in ["center", "right"] {
        let source = format!("<p align='{alignment}'>one two three four five six<br>last</p>");
        let snapshot = TextBuffer::new(&source).snapshot();
        let document = yu_markdown::parse(&snapshot);
        let block = document.blocks().get(0).expect("paragraph");
        let mut cache = DecorationCache::default();
        let decorations = cache.decorate(&document, block, None).expect("decorations");
        let visual =
            VisualText::new(&snapshot, block.range(), decorations.set().clone()).expect("visual");
        let view = BlockView::build(
            block.kind(),
            &visual,
            &decorations,
            LayoutConfig::new(80.0, 16.0),
            &MonospaceMetrics::new(8.0),
        )
        .expect("layout");
        assert!(view.lines().len() >= 3);
        for line in view.lines() {
            let clusters: Vec<_> = view
                .clusters()
                .iter()
                .filter(|cluster| cluster.line() == line.index() && !cluster.is_line_break())
                .collect();
            if clusters.is_empty() {
                continue;
            }
            let first = clusters.first().expect("first");
            let last = clusters
                .iter()
                .rev()
                .find(|cluster| {
                    !visual.text()[cluster.visual().start().get() as usize
                        ..cluster.visual().end().get() as usize]
                        .chars()
                        .all(char::is_whitespace)
                })
                .expect("visible final cluster");
            let right = last.x() + last.width();
            if alignment == "right" {
                assert!((right - 80.0).abs() < 0.01);
            } else {
                assert!(
                    (first.x() - (80.0 - right)).abs() < 0.01,
                    "line {} first={} right={} clusters={:?}",
                    line.index(),
                    first.x(),
                    right,
                    clusters
                );
            }
            for cluster in clusters {
                let hit = view
                    .hit_test(LayoutPoint::new(
                        cluster.x() + 1.0,
                        cluster.y() + cluster.line_height() / 2.0,
                    ))
                    .expect("hit");
                assert_eq!(hit.content_source(), Some(cluster.source()));
                let caret = view
                    .caret_for_source(cluster.source().start(), yu_decoration::Bias::After)
                    .expect("caret");
                assert!((caret.point().x() - cluster.x()).abs() < 0.01);
            }
        }
        let active = yu_core::TextRange::empty(yu_core::ByteOffset::new(
            source.find("one").expect("text") as u64,
        ));
        let decorations = cache
            .decorate(&document, block, Some(active))
            .expect("active");
        assert!(
            !decorations
                .line_ornaments()
                .iter()
                .any(|(_, o)| matches!(o, yu_markdown::BlockOrnament::Alignment { .. }))
        );
        assert_eq!(
            VisualText::new(&snapshot, block.range(), decorations.set().clone())
                .expect("active text")
                .text(),
            source
        );
    }
}
