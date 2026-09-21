use yu_editor::{EditorDocument, OutlineTree, TocProjection};

#[test]
fn toc_cache_is_shared_with_workers_and_replaced_on_edit_or_document_reset() {
    use std::sync::Arc;
    use yu_editor::EditorCommand;
    let mut document = EditorDocument::new("[toc]\n\n# First\n");
    let mut worker = document.capture_render_snapshot().into_layout_context();
    let background = worker.toc_projection().expect("cold worker build");
    let live = document.toc_projection().expect("live cache");
    assert!(Arc::ptr_eq(&background, &live));
    for _ in 0..100 {
        assert!(Arc::ptr_eq(
            &live,
            &document.toc_projection().expect("repeat hit")
        ));
    }
    document
        .execute(EditorCommand::insert_text("# Added\n\n"))
        .expect("edit");
    let changed = document.toc_projection().expect("new revision");
    assert!(!Arc::ptr_eq(&live, &changed));
    assert_eq!(&*changed.text, "Added\nFirst");
    assert_eq!(&*background.text, "First");
    let mut new_worker = document.capture_render_snapshot().into_layout_context();
    assert!(Arc::ptr_eq(
        &changed,
        &new_worker.toc_projection().expect("new worker")
    ));
    document
        .reset_source("[toc]\n\n# Different\n")
        .expect("reset revision");
    let reset = document.toc_projection().expect("new document");
    assert_eq!(&*reset.text, "Different");
    assert!(!Arc::ptr_eq(&live, &reset));
    assert_eq!(
        &*worker
            .toc_projection()
            .expect("old worker still immutable")
            .text,
        "First"
    );
}

#[test]
fn wrapped_toc_hits_use_painted_cluster_identity_on_both_edges() {
    use yu_editor::{
        BlockView, DecorationCache, LayoutConfig, LayoutPoint, MonospaceMetrics, VisualText,
    };
    let document =
        EditorDocument::new("[toc]\n\n# A long title with 中文🙂\n\n## שלום child\n\n# Last\n");
    let markdown = document.markdown();
    let projection = TocProjection::build(markdown).expect("projection");
    let metadata = markdown.blocks().get(0).expect("TOC block");
    let marker = markdown.toc_marker_in(metadata.range()).expect("marker");
    let mut cache = DecorationCache::default();
    let decorations = cache
        .get_or_build_block(markdown, metadata)
        .expect("decorations");
    let visual = VisualText::new(
        markdown.source(),
        metadata.range(),
        decorations.set().clone(),
    )
    .expect("visual");
    for width in [40.0, 160.0, 800.0] {
        let config = LayoutConfig::new(width, 20.0).with_default_advance(8.0);
        let block = BlockView::build(
            metadata.kind(),
            &visual,
            decorations,
            config,
            &MonospaceMetrics::new(8.0),
        )
        .expect("layout");
        if width == 40.0 {
            assert!(block.lines().len() > projection.links.len());
        }
        let mut hits = 0;
        for cluster in block
            .clusters()
            .iter()
            .filter(|c| !c.is_line_break() && c.width() > 0.0)
        {
            let expected = projection.target_at(cluster.visual().start().get() as usize);
            for fraction in [0.1, 0.9] {
                let hit = block
                    .hit_test(LayoutPoint::new(
                        cluster.x() + cluster.width() * fraction,
                        cluster.y() + cluster.line_height() / 2.0,
                    ))
                    .expect("hit");
                assert_eq!(projection.target_for_hit(marker, &block, hit), expected);
                hits += usize::from(expected.is_some());
            }
        }
        assert!(hits > 20);
    }
}

#[test]
fn toc_labels_match_the_sidebar_including_entities_and_multiline_headings() {
    let source = "[toc]\n\n# **Bold**  [link](https://example.com) &amp;\n\n### 中文🙂\n\nSetext\nsecond line\n===\n\n#\n";
    let mut document = EditorDocument::new(source);
    let outline = OutlineTree::build(&mut document).expect("outline");
    let toc = TocProjection::build(document.markdown()).expect("TOC");
    assert_eq!(toc.links.len(), outline.rows().len());
    for (link, row) in toc.links.iter().zip(outline.rows()) {
        assert_eq!(
            &toc.text[link.visual.clone()],
            if row.label().is_empty() {
                "未命名标题"
            } else {
                row.label()
            }
        );
        assert_eq!(link.target, row.item().label_range());
    }
    for (position, ch) in toc.text.char_indices() {
        if ch == '\n' {
            assert_eq!(toc.target_at(position), None);
        }
    }
    assert_eq!(toc.target_at(toc.text.len()), None);
    assert_eq!(document.snapshot().as_str(), source);
}

#[test]
fn empty_toc_has_readable_text_without_a_fake_navigation_target() {
    let document = EditorDocument::new("[toc]\n\nordinary text");
    let toc = TocProjection::build(document.markdown()).expect("empty TOC");
    assert_eq!(&*toc.text, "暂无标题");
    assert!(toc.links.is_empty());
    assert_eq!(toc.target_at(0), None);
}
