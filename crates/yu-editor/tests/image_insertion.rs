use yu_core::ByteOffset;
use yu_editor::{
    CaretAffinity, EditorCommand, EditorDocument, EditorSelection, local_image_markdown,
};

#[test]
fn local_image_insertion_is_one_undo_step_and_keeps_surrounding_bytes() {
    let original = "# 标题\r\n\r\nKEEP\r\n";
    let mut document = EditorDocument::new(original);
    let at = ByteOffset::new(original.find("KEEP").expect("marker") as u64);
    document
        .set_selection(
            EditorSelection::cursor(&document.snapshot(), at, CaretAffinity::Downstream)
                .expect("valid cursor"),
        )
        .expect("caret");
    let image = local_image_markdown("assets/图 空格%[x](1).png", "图 [一]\n尾").expect("image");
    assert_eq!(
        image,
        "![图 \\[一\\] 尾](assets/%E5%9B%BE%20%E7%A9%BA%E6%A0%BC%25%5Bx%5D%281%29.png)"
    );
    document
        .execute(EditorCommand::PasteFragments(vec![image.clone().into()]))
        .expect("insert");
    assert_eq!(
        document.snapshot().as_str(),
        original.replacen("KEEP", &(image + "KEEP"), 1)
    );
    let inserted = document.snapshot().as_str().to_owned();
    document.undo().expect("undo");
    assert_eq!(document.snapshot().as_str(), original);
    document.redo().expect("redo");
    assert_eq!(document.snapshot().as_str(), inserted);
    assert!(document.selection().is_empty());
}

#[test]
fn invalid_local_paths_are_not_serialized() {
    assert!(local_image_markdown("", "alt").is_none());
    assert!(local_image_markdown("bad\0.png", "alt").is_none());
}

fn selection_shape(document: &EditorDocument) -> Vec<(ByteOffset, ByteOffset, CaretAffinity)> {
    document
        .selections()
        .as_slice()
        .iter()
        .map(|selection| (selection.anchor(), selection.focus(), selection.affinity()))
        .collect()
}

#[test]
fn drop_image_is_atomic_at_its_target_and_undo_restores_backward_multi_selection() {
    let original = "🙂 first\r\n\r\nmiddle\r\n\r\nlast";
    let mut document = EditorDocument::new(original);
    let first = original.find("first").expect("first") as u64;
    let last = original.len() as u64;
    document
        .set_selections(
            [
                EditorSelection::range(
                    &document.snapshot(),
                    ByteOffset::new(first + 5),
                    ByteOffset::new(first),
                    CaretAffinity::Upstream,
                )
                .expect("backward range"),
                EditorSelection::cursor(
                    &document.snapshot(),
                    ByteOffset::new(last),
                    CaretAffinity::Downstream,
                )
                .expect("second cursor"),
            ],
            1,
        )
        .expect("multiple selection");
    let before = document.selections().clone();
    let fragment = local_image_markdown("assets/图.png", "图").expect("fragment");
    assert!(
        document
            .insert_local_images(&[("assets/图.png", "图")], Some(ByteOffset::new(1)))
            .is_err()
    );
    assert_eq!(document.selections(), &before);
    assert_eq!(document.snapshot().as_str(), original);
    assert_eq!(document.history_stats().undo_entries(), 0);
    let shape = selection_shape(&document);
    let at = ByteOffset::new(original.find("middle").expect("middle") as u64);
    document
        .insert_local_images(&[("assets/图.png", "图")], Some(at))
        .expect("drop");
    let expected = original.replacen("middle", &(fragment.clone() + "middle"), 1);
    assert_eq!(document.snapshot().as_str(), expected);
    assert_eq!(document.selections().len(), 1);
    assert_eq!(
        document.selection().focus().get(),
        at.get() + fragment.len() as u64
    );
    document.undo().expect("undo drop");
    assert_eq!(document.snapshot().as_str(), original);
    assert_eq!(selection_shape(&document), shape);
    assert_eq!(document.selections().primary_index(), 1);
    document.redo().expect("redo drop");
    assert_eq!(document.snapshot().as_str(), expected);
    assert_eq!(
        document.selection().focus().get(),
        at.get() + fragment.len() as u64
    );
}

#[test]
fn drop_image_in_a_table_preserves_structure_and_restores_grid_selection_on_undo() {
    let original = "| A | B |\r\n| --- | --- |\r\n| one | two |\r\n";
    let mut document = EditorDocument::new(original);
    let one = ByteOffset::new(original.find("one").expect("one") as u64);
    let two = ByteOffset::new(original.find("two").expect("two") as u64);
    document
        .execute(EditorCommand::SelectTableCells {
            anchor: two,
            focus: one,
        })
        .expect("grid");
    let shape = selection_shape(&document);
    let primary = document.selections().primary_index();
    assert_eq!(document.selections().table_columns(), Some(2));
    let fragment = local_image_markdown("assets/a.png", "a|b").expect("fragment");
    document
        .insert_local_images(&[("assets/a.png", "a|b")], Some(one))
        .expect("drop into cell");
    assert_eq!(
        document.snapshot().as_str(),
        original.replacen("one", &(fragment + "one"), 1)
    );
    assert_eq!(document.image_references().expect("images").len(), 1);
    assert_eq!(document.selections().table_columns(), None);
    document.undo().expect("undo grid drop");
    assert_eq!(document.snapshot().as_str(), original);
    assert_eq!(selection_shape(&document), shape);
    assert_eq!(document.selections().primary_index(), primary);
    assert_eq!(document.selections().table_columns(), Some(2));
}

#[test]
fn serialized_image_is_one_real_widget_even_with_markdown_in_the_label() {
    let fragment =
        local_image_markdown("assets/a[b](c)% #图.png", "[*x*] ! & <br>").expect("image");
    let mut document = EditorDocument::new(format!("{fragment}\n"));
    let decorations = document.block_decorations(0).expect("image paragraph");
    assert_eq!(decorations.widgets().len(), 1);
    let yu_editor::BlockWidget::Image(image) = decorations.widgets()[0] else {
        panic!("image widget missing")
    };
    assert_eq!(image.source().len(), fragment.len() as u64);
}

#[test]
fn image_catalog_resolves_references_in_containers_without_layout_cache_work() {
    let source = "`![code](missing.png)`\n\n> ![引用][pic]\n\n- ![inline](assets/a.png)\n\n[pic]: assets/b.png\n";
    let mut document = EditorDocument::new(source);
    let catalog = document.image_references().expect("catalog");
    assert_eq!(
        catalog
            .iter()
            .map(|image| image.destination_text.as_str())
            .collect::<Vec<_>>(),
        vec!["assets/b.png", "assets/a.png"]
    );
    document.set_source_mode(true).expect("source mode");
    assert_eq!(
        document.image_references().expect("source mode catalog"),
        catalog
    );
    assert_eq!(
        document.retained_image_destinations().collect::<Vec<_>>(),
        vec!["assets/a.png", "assets/b.png"]
    );
    document
        .reset_source("new document")
        .expect("reload unrelated document");
    assert_eq!(document.retained_image_destinations().count(), 0);
}

#[test]
fn html_image_dimensions_share_widget_geometry_before_and_after_loading() {
    use yu_editor::{ImageIntrinsicSize, LayoutConfig, LayoutPoint};
    for prefix in ["", "> ", "- ", "before "] {
        let source = format!(
            "{prefix}<img src=\"a&amp;b.png\" alt=\"中文\" width=\"120\" height=\"60\" data-keep=\"yes\">\n"
        );
        let mut document = EditorDocument::new(source);
        let images = document.image_references().expect("image catalog");
        assert_eq!(images.len(), 1, "prefix {prefix}");
        assert_eq!(images[0].destination_text, "a&b.png");
        let range = images[0].source;
        for zoom in [1.0, 1.25, 2.0] {
            let config = LayoutConfig::new(600.0, 16.0 * zoom);
            let mut found = false;
            for index in 0..document.markdown().blocks().len() {
                let pending = document
                    .block_layout_with_images(index, config, &[])
                    .expect("placeholder")
                    .images()
                    .to_vec();
                let sizes = [(range, ImageIntrinsicSize::new(400, 400).expect("intrinsic"))];
                let loaded = document
                    .block_layout_with_images(index, config, &sizes)
                    .expect("loaded");
                for image in loaded.images() {
                    found = true;
                    let bounds = image.bounds();
                    assert!((bounds.width() - 120.0 * zoom).abs() < 0.01, "{bounds:?}");
                    assert!((bounds.height() - 60.0 * zoom).abs() < 0.01, "{bounds:?}");
                    assert_eq!(
                        pending[0].bounds(),
                        bounds,
                        "explicit dimensions must not jump after decode"
                    );
                    let hit = loaded
                        .hit_test(LayoutPoint::new(bounds.x() + 1.0, bounds.y() + 1.0))
                        .expect("image hit");
                    assert_eq!(hit.source(), range.start());
                }
            }
            assert!(found, "image was catalogued but never rendered: {prefix}");
        }
    }
}

#[test]
fn one_declared_dimension_preserves_intrinsic_ratio_and_fits_the_column() {
    use yu_editor::{ImageIntrinsicSize, LayoutConfig};
    for attributes in ["width=200", "height=100"] {
        let mut document = EditorDocument::new(format!("<img src=x {attributes}>\n"));
        let range = document.image_references().expect("catalog")[0].source;
        let sizes = [(range, ImageIntrinsicSize::new(400, 200).expect("intrinsic"))];
        let view = document
            .block_layout_with_images(0, LayoutConfig::new(80.0, 16.0), &sizes)
            .expect("image layout");
        assert_eq!(view.images().len(), 1);
        assert_eq!(view.images()[0].bounds().width(), 80.0);
        assert_eq!(view.images()[0].bounds().height(), 40.0);
    }
}

#[test]
fn code_comments_scripts_and_invalid_image_sizes_remain_editable_source() {
    for source in [
        "`<img src=x width=20>`",
        "```html\n<img src=x width=20>\n```",
        "<!-- <img src=x width=20> -->",
        "<script>\"<img src=x width=20>\"</script>",
        "<img src=x width='100%'>",
        "<img src=x src=y>",
    ] {
        let document = EditorDocument::new(source);
        assert!(
            document.image_references().expect("catalog").is_empty(),
            "{source}"
        );
        assert_eq!(document.snapshot().as_str(), source);
    }
}

#[test]
fn image_inspector_is_revision_bound_and_one_undo_step() {
    let original = "# 保留\r\n\r\n![羽](assets/a.png \"标题 &amp; 说明\")\r\n\r\nTAIL\r\n";
    let mut document = EditorDocument::new(original);
    let range = document.image_references().expect("image property fixture")[0].source;
    let mut properties = document
        .image_properties(range)
        .expect("image property fixture");
    let untouched_revision = document.revision();
    document
        .update_image_properties(&properties)
        .expect("image property fixture");
    assert_eq!(document.revision(), untouched_revision);
    properties.width = Some(240);
    properties.height = Some(120);
    properties.alternative = "替代 <图>".into();
    document
        .update_image_properties(&properties)
        .expect("image property fixture");
    let changed = document.snapshot().as_str().to_owned();
    assert_eq!(
        changed,
        "# 保留\r\n\r\n<img title=\"标题 &amp; 说明\" src=\"assets/a.png\" alt=\"替代 &lt;图&gt;\" width=\"240\" height=\"120\">\r\n\r\nTAIL\r\n"
    );
    assert!(document.update_image_properties(&properties).is_err());
    assert_eq!(document.snapshot().as_str(), changed);
    let new_range = document.image_references().expect("image property fixture")[0].source;
    let restored_properties = document
        .image_properties(new_range)
        .expect("image property fixture");
    assert_eq!(restored_properties.width, Some(240));
    assert_eq!(restored_properties.alternative, "替代 <图>");
    document.undo().expect("undo image property edit");
    assert_eq!(document.snapshot().as_str(), original);
    document.redo().expect("redo image property edit");
    assert_eq!(document.snapshot().as_str(), changed);
}

#[test]
fn reference_image_properties_do_not_modify_shared_definition_or_other_image() {
    let original = "![first][id]\n\n![second][id]\n\n[id]: old.png 'shared title'\n";
    let mut document = EditorDocument::new(original);
    let range = document.image_references().expect("image property fixture")[0].source;
    let mut properties = document
        .image_properties(range)
        .expect("image property fixture");
    properties.destination = "new%20image.png".into();
    properties.width = Some(100);
    document
        .update_image_properties(&properties)
        .expect("image property fixture");
    assert_eq!(
        document.snapshot().as_str(),
        "<img title=\"shared title\" src=\"new%20image.png\" alt=\"first\" width=\"100\">\n\n![second][id]\n\n[id]: old.png 'shared title'\n"
    );
    assert_eq!(
        document.image_references().expect("image property fixture")[1].destination_text,
        "old.png"
    );
    document.undo().expect("undo image property edit");
    assert_eq!(document.snapshot().as_str(), original);
}

#[test]
fn invalid_image_properties_do_not_change_source_or_history() {
    let original = "<img src='a.png' data-private='保留' width=100>\n";
    let mut document = EditorDocument::new(original);
    let range = document.image_references().expect("image property fixture")[0].source;
    let mut properties = document
        .image_properties(range)
        .expect("image property fixture");
    properties.width = Some(0);
    assert!(document.update_image_properties(&properties).is_err());
    assert_eq!(document.snapshot().as_str(), original);
    assert_eq!(document.history_stats().undo_entries(), 0);
    properties.width = None;
    document
        .update_image_properties(&properties)
        .expect("image property fixture");
    assert_eq!(
        document.snapshot().as_str(),
        "<img src='a.png' data-private='保留' alt=\"\">\n"
    );
    document.undo().expect("undo image property edit");
    assert_eq!(document.snapshot().as_str(), original);
}

#[test]
fn unsized_markdown_property_edits_keep_title_and_original_syntax() {
    let original = "before ![原图](<a.png>  'keep title') after\r\n";
    let mut document = EditorDocument::new(original);
    let range = document.image_references().expect("image property fixture")[0].source;
    let mut properties = document
        .image_properties(range)
        .expect("image property fixture");
    assert_eq!(properties.destination, "a.png");
    assert_eq!(
        document.image_references().expect("image property fixture")[0].destination_text,
        "a.png"
    );
    properties.destination = "new%20file&x.png".into();
    properties.alternative = "[新图]".into();
    document
        .update_image_properties(&properties)
        .expect("image property fixture");
    assert_eq!(
        document.snapshot().as_str(),
        "before ![\\[新图\\]](<new%20file\\&x.png>  'keep title') after\r\n"
    );
    document.undo().expect("undo image property edit");
    assert_eq!(document.snapshot().as_str(), original);
}

#[test]
fn image_property_destination_preserves_remote_query_delimiters() {
    let mut document = EditorDocument::new("![图](old.png)\n");
    let range = document.image_references().expect("image property fixture")[0].source;
    let mut properties = document
        .image_properties(range)
        .expect("image property fixture");
    properties.destination = "https://example.test/a(b).png?first=1&copy;=2".into();
    document
        .update_image_properties(&properties)
        .expect("image property fixture");
    assert_eq!(
        document.image_references().expect("image property fixture")[0].destination_text,
        properties.destination
    );
}

#[test]
fn image_batches_preserve_list_quote_and_table_structure() {
    for original in [
        "KEEP\r\n",
        "- KEEP\r\n",
        "> KEEP\r\n",
        "| A | B |\r\n| --- | --- |\r\n| KEEP | RIGHT |\r\n",
        "> | A | B |\r\n> | --- | --- |\r\n> | KEEP | RIGHT |\r\n",
    ] {
        let mut document = EditorDocument::new(original);
        let at = ByteOffset::new(original.find("KEEP").expect("insertion marker") as u64);
        document
            .set_selection(
                EditorSelection::cursor(&document.snapshot(), at, CaretAffinity::Downstream)
                    .expect("source caret"),
            )
            .expect("select insertion site");
        let fragment = [
            local_image_markdown("a|b.png", "图|甲").expect("first image"),
            local_image_markdown("b.png", "乙").expect("second image"),
        ]
        .join(" ");
        document
            .execute(EditorCommand::PasteFragments(vec![fragment.clone().into()]))
            .expect("paste batch");
        assert_eq!(
            document.snapshot().as_str(),
            original.replacen("KEEP", &(fragment + "KEEP"), 1)
        );
        assert_eq!(document.image_references().expect("image catalog").len(), 2);
        document.undo().expect("undo batch");
        assert_eq!(document.snapshot().as_str(), original);
    }
}

#[test]
fn nested_html_images_share_catalog_widgets_and_preserve_source_on_property_undo() {
    use yu_editor::{ImageIntrinsicSize, LayoutConfig};
    let original = "<div><p>before <img src='assets/a&amp;b.png' alt='中文' width='120' height='60' data-private='保留'> after</p><ul><li><img src='second.png' width=40></li></ul></div>\r\n";
    let mut document = EditorDocument::new(original);
    let images = document.image_references().expect("nested images");
    assert_eq!(images.len(), 2);
    assert_eq!(images[0].destination_text, "assets/a&b.png");
    assert_eq!(images[1].destination_text, "second.png");
    let sizes: Vec<_> = images
        .iter()
        .map(|image| {
            (
                image.source,
                ImageIntrinsicSize::new(200, 100).expect("size"),
            )
        })
        .collect();
    let mut painted = Vec::new();
    for block in 0..document.markdown().blocks().len() {
        let view = document
            .block_layout_with_images(block, LayoutConfig::new(600.0, 16.0), &sizes)
            .expect("native geometry");
        painted.extend(
            view.images()
                .iter()
                .map(|image| (image.bounds().width(), image.bounds().height())),
        );
    }
    assert_eq!(painted, [(120.0, 60.0), (40.0, 20.0)]);
    let mut properties = document
        .image_properties(images[0].source)
        .expect("properties");
    properties.width = Some(80);
    document
        .update_image_properties(&properties)
        .expect("edit nested image");
    assert!(document.snapshot().as_str().contains("data-private='保留'"));
    assert!(
        document
            .snapshot()
            .as_str()
            .contains("<ul><li><img src='second.png' width=40></li></ul>")
    );
    document.undo().expect("undo");
    assert_eq!(document.snapshot().as_str(), original);
    document.set_source_mode(true).expect("source mode");
    assert_eq!(document.image_references().expect("source catalog"), images);
}

#[test]
fn unknown_html_ancestors_comments_and_scripts_do_not_schedule_images() {
    for original in [
        "<div class='unknown'><img src='hidden.png'></div>",
        "<custom><img src='hidden.png'></custom>",
        "<div><!-- <img src='hidden.png'> --><script><img src='hidden.png'></script></div>",
        "<div><img src='hidden.png' onclick='run()'></div>",
    ] {
        let document = EditorDocument::new(original);
        assert!(
            document.image_references().expect("catalog").is_empty(),
            "{original}"
        );
        assert_eq!(document.snapshot().as_str(), original);
    }
}

#[test]
fn imported_images_in_html_cells_are_elements_with_atomic_history() {
    let source =
        "\u{feff}KEEP\r\n\r\n<table><tr><td><b>选择</b></td><td>TAIL</td></tr></table>\r\n";
    for drop in [false, true] {
        let mut document = EditorDocument::new(source);
        let start = source.find("选择").expect("HTML image fixture operation");
        document
            .set_selection(
                EditorSelection::range(
                    &document.snapshot(),
                    ByteOffset::new(start as u64),
                    ByteOffset::new((start + "选择".len()) as u64),
                    CaretAffinity::Downstream,
                )
                .expect("HTML image fixture operation"),
            )
            .expect("HTML image fixture operation");
        let selection = selection_shape(&document);
        document
            .insert_local_images(
                &[("assets/图 空格&.png", "图<&\""), ("b.png", "二")],
                drop.then_some(ByteOffset::new(start as u64)),
            )
            .expect("HTML image import");
        let snapshot = document.snapshot();
        let images = yu_markdown::image_spans(document.markdown(), &snapshot, None);
        assert_eq!(images.len(), 2, "{}", snapshot.as_str());
        assert!(images.iter().all(|image| image.is_html()));
        let properties = document
            .image_properties(images[0].source())
            .expect("HTML image fixture operation");
        assert_eq!(properties.alternative, "图<&\"");
        assert_eq!(
            properties.destination,
            yu_editor::local_image_uri("assets/图 空格&.png")
                .expect("HTML image fixture operation")
        );
        let inserted = snapshot.as_str().to_owned();
        assert!(inserted.starts_with("\u{feff}KEEP\r\n\r\n<table><tr><td><b><img"));
        assert!(inserted.ends_with("</b></td><td>TAIL</td></tr></table>\r\n"));
        assert_eq!(inserted.contains("选择"), drop);
        assert!(!inserted.contains("!["));
        let reopened = EditorDocument::new(&inserted);
        assert_eq!(
            yu_markdown::image_spans(reopened.markdown(), &reopened.snapshot(), None).len(),
            2
        );
        document.undo().expect("HTML image fixture operation");
        assert_eq!(document.snapshot().as_str(), source);
        assert_eq!(selection_shape(&document), selection);
        document.redo().expect("HTML image fixture operation");
        assert_eq!(document.snapshot().as_str(), inserted);
    }
}

#[test]
fn imported_images_choose_syntax_per_cursor_and_invalid_batches_are_atomic() {
    let source = "PLAIN\n\n<table><tr><td>HTML</td></tr></table>\n";
    let mut document = EditorDocument::new(source);
    let cursors = [
        0,
        source.find("HTML").expect("HTML image fixture operation"),
    ]
    .map(|at| {
        EditorSelection::cursor(
            &document.snapshot(),
            ByteOffset::new(at as u64),
            CaretAffinity::Downstream,
        )
        .expect("HTML image fixture operation")
    });
    document
        .set_selections(cursors, 1)
        .expect("HTML image fixture operation");
    let before = selection_shape(&document);
    assert!(
        document
            .insert_local_images(&[("good.png", "ok"), ("bad\0.png", "bad")], None)
            .is_err()
    );
    assert_eq!(document.snapshot().as_str(), source);
    assert_eq!(selection_shape(&document), before);
    document
        .insert_local_images(&[("a.png", "图")], None)
        .expect("HTML image fixture operation");
    let snapshot = document.snapshot();
    let images = yu_markdown::image_spans(document.markdown(), &snapshot, None);
    assert_eq!(images.len(), 2);
    assert!(!images[0].is_html());
    assert!(images[1].is_html());
    assert!(snapshot.as_str().starts_with("![图](a.png)PLAIN"));
    document.undo().expect("HTML image fixture operation");
    assert_eq!(document.snapshot().as_str(), source);
    assert_eq!(selection_shape(&document), before);
}

#[test]
fn imported_html_cell_image_has_layout_hit_geometry_and_editable_dimensions() {
    use yu_editor::{ImageIntrinsicSize, LayoutConfig, LayoutPoint};
    let source = "<table><tr><td>内容</td><td>KEEP</td></tr></table>";
    let mut document = EditorDocument::new(source);
    let at = ByteOffset::new(source.find("内容").expect("cell") as u64);
    document
        .insert_local_images(&[("a.png", "图")], Some(at))
        .expect("insert");
    let inserted = document.snapshot().as_str().to_owned();
    let image = document.image_references().expect("catalog")[0].source;
    let mut properties = document.image_properties(image).expect("properties");
    properties.width = Some(80);
    properties.height = Some(40);
    document
        .update_image_properties(&properties)
        .expect("resize");
    let image = document.image_references().expect("resized catalog")[0].source;
    let sizes = [(image, ImageIntrinsicSize::new(400, 200).expect("intrinsic"))];
    let view = document
        .block_layout_with_images(0, LayoutConfig::new(500.0, 16.0), &sizes)
        .expect("cell image geometry");
    assert!(view.table().is_some());
    assert_eq!(view.images().len(), 1);
    let bounds = view.images()[0].bounds();
    assert!((bounds.width() - 80.0).abs() < 0.01);
    assert!((bounds.height() - 40.0).abs() < 0.01);
    assert!(
        view.table().expect("table").cells()[0]
            .bounds()
            .contains(LayoutPoint::new(
                bounds.x() + bounds.width(),
                bounds.y() + bounds.height()
            ))
    );
    let hit = view
        .hit_test(LayoutPoint::new(bounds.x() + 1.0, bounds.y() + 1.0))
        .expect("image hit");
    assert_eq!(hit.source(), image.start());
    document.undo().expect("undo dimensions");
    assert_eq!(document.snapshot().as_str(), inserted);
    document.undo().expect("undo insertion");
    assert_eq!(document.snapshot().as_str(), source);
}

#[test]
fn html_cell_images_are_atomic_editing_objects() {
    let image = "<img src='old.png' alt='旧图'>";
    let source = format!("<table><tr><td><b>前{image}后</b></td><td>KEEP</td></tr></table>\r\n");
    let start = source.find(image).expect("image");
    let end = start + image.len();
    for action in [
        "delete-selection",
        "backspace",
        "forward",
        "replace-text",
        "replace-image",
    ] {
        let mut document = EditorDocument::new(&source);
        let (anchor, focus) = match action {
            "backspace" => (end, end),
            "forward" => (start, start),
            _ => (start, end),
        };
        document
            .set_selection(
                EditorSelection::range(
                    &document.snapshot(),
                    ByteOffset::new(anchor as u64),
                    ByteOffset::new(focus as u64),
                    CaretAffinity::Downstream,
                )
                .expect("image selection"),
            )
            .expect("selection");
        let before = selection_shape(&document);
        match action {
            "backspace" => {
                document
                    .execute(EditorCommand::DeleteBackward)
                    .expect("backspace image");
            }
            "forward" => {
                document
                    .execute(EditorCommand::DeleteForward)
                    .expect("delete image");
            }
            "replace-text" => {
                document
                    .execute(EditorCommand::insert_text("新<&"))
                    .expect("replace image");
            }
            "replace-image" => {
                document
                    .insert_local_images(&[("new.png", "新图")], None)
                    .expect("replace image");
            }
            _ => {
                document
                    .execute(EditorCommand::DeleteSelections)
                    .expect("delete selection");
            }
        }
        let replacement = match action {
            "replace-text" => "新&lt;&amp;",
            "replace-image" => "<img src=\"new.png\" alt=\"新图\">",
            _ => "",
        };
        let expected = source.replace(image, replacement);
        assert_eq!(document.snapshot().as_str(), expected, "{action}");
        let reopened = EditorDocument::new(&expected);
        assert_eq!(
            reopened.image_references().expect("catalog").len(),
            usize::from(action == "replace-image")
        );
        document.undo().expect("undo image edit");
        assert_eq!(document.snapshot().as_str(), source);
        assert_eq!(selection_shape(&document), before);
        document.redo().expect("redo image edit");
        assert_eq!(document.snapshot().as_str(), expected);
    }
}

#[test]
fn html_image_deletion_distinguishes_neighboring_images_text_and_cell_edges() {
    let first = "<img src='a.png'>";
    let second = "<img src='b.png'>";
    let source = format!(
        "<table><tr><td>前<b>{first}</b><i>{second}</i>后🙂</td><td>KEEP</td></tr></table>"
    );
    let cases = [
        (source.find(first).expect("first"), true, first),
        (
            source.find(second).expect("second") + second.len(),
            false,
            second,
        ),
        (source.find("后").expect("text"), true, "后"),
        (source.find("后").expect("text") + "后".len(), false, "后"),
        (source.find("🙂").expect("emoji") + "🙂".len(), false, "🙂"),
        (source.find("前").expect("cell start"), false, ""),
    ];
    for (at, forward, removed) in cases {
        let mut document = EditorDocument::new(&source);
        document
            .set_selection(
                EditorSelection::cursor(
                    &document.snapshot(),
                    ByteOffset::new(at as u64),
                    CaretAffinity::Downstream,
                )
                .expect("caret"),
            )
            .expect("selection");
        document
            .execute(if forward {
                EditorCommand::DeleteForward
            } else {
                EditorCommand::DeleteBackward
            })
            .expect("atomic deletion");
        let expected = if removed.is_empty() {
            source.clone()
        } else {
            source.replacen(removed, "", 1)
        };
        assert_eq!(
            document.snapshot().as_str(),
            expected,
            "at {at}, forward {forward}"
        );
    }
}

#[test]
fn html_cell_image_keyboard_steps_and_shift_selection_share_atomic_ranges() {
    let image = "<img src='a.png' alt='图'>";
    let source = format!("<table><tr><td>前{image}后</td></tr></table>");
    let start = source.find(image).expect("image");
    let end = start + image.len();
    for forward in [false, true] {
        for extend in [false, true] {
            let mut document = EditorDocument::new(&source);
            let from = if forward { start } else { end };
            let to = if forward { end } else { start };
            document
                .set_selection(
                    EditorSelection::cursor(
                        &document.snapshot(),
                        ByteOffset::new(from as u64),
                        CaretAffinity::Downstream,
                    )
                    .expect("caret"),
                )
                .expect("selection");
            let command = if extend {
                EditorCommand::ExtendHorizontal {
                    forward,
                    word: false,
                }
            } else if forward {
                EditorCommand::MoveRight
            } else {
                EditorCommand::MoveLeft
            };
            document.execute(command).expect("image navigation");
            assert_eq!(document.selection().focus().get(), to as u64);
            assert_eq!(document.selection().is_empty(), !extend);
            assert_eq!(document.snapshot().as_str(), source);
            assert_eq!(document.history_stats().undo_entries(), 0);
            if extend {
                document
                    .execute(EditorCommand::DeleteSelections)
                    .expect("delete keyboard selected image");
                assert_eq!(document.snapshot().as_str(), source.replace(image, ""));
                document.undo().expect("undo selected image");
                assert_eq!(document.snapshot().as_str(), source);
                assert_eq!(document.selection().focus().get(), to as u64);
                assert_eq!(document.selection().anchor().get(), from as u64);
            }
        }
    }
}
