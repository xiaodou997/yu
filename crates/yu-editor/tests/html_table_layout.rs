use yu_editor::{BlockView, LayoutConfig, LayoutPoint, MonospaceMetrics, VisualText};
use yu_markdown::{
    BlockDecorations, ExtensionOutput, TableAlignment, TableBlock, TableCellAddress,
};
use yu_text::TextBuffer;

#[test]
fn html_grid_reuses_cell_geometry_without_a_fake_header_or_delimiter_row() {
    let source = "<table><tr><td><b>A</b>&amp;</td><td align='right'>B</td></tr><tr><th align='center'>C</th><td>D</td></tr></table>";
    let snapshot = TextBuffer::new(source).snapshot();
    let document = yu_markdown::parse(&snapshot);
    let block = document.blocks().get(0).expect("html block");
    let model = document.html_regions().regions[0]
        .model
        .as_ref()
        .expect("model");
    let html = model
        .table(model.partitions[0].content.owner.expect("table owner"))
        .expect("table");
    let grid = TableBlock::from_html(&html).expect("grid adapter");
    assert!(grid.is_html());
    assert!(grid.delimiter().is_empty());
    assert_eq!(grid.delimiter_source_range(), None);
    assert_eq!(grid.visible_row_count(), 2);
    assert_eq!(grid.row_ranges().len(), 2);
    assert!(!grid.cell_is_header(TableCellAddress::new(0, 0)));
    assert!(grid.cell_is_header(TableCellAddress::new(1, 0)));
    assert_eq!(
        grid.cell_alignment(TableCellAddress::new(0, 1)),
        TableAlignment::Right
    );
    let moved = grid.clone().shifted(7).expect("shift grid");
    assert!(moved.is_html());
    assert!(moved.cell_is_header(TableCellAddress::new(1, 0)));
    assert_eq!(
        moved
            .visible_cell(TableCellAddress::new(1, 0))
            .expect("cell")
            .start(),
        grid.visible_cell(TableCellAddress::new(1, 0))
            .expect("cell")
            .start()
            + 7
    );
    // Use the real HTML cell projection producer; production entry and HTML
    // editing commands are still separate integration work.
    let mut output = ExtensionOutput::default();
    assert!(model.decorate_table(source, &model.partitions[0], &mut output));
    let decorations = BlockDecorations::from_output(&snapshot, block.range(), output);
    let visual =
        VisualText::new(&snapshot, block.range(), decorations.set().clone()).expect("visual");
    assert_eq!(visual.text(), "A&BCD");
    let view = BlockView::build(
        block.kind(),
        &visual,
        &decorations,
        LayoutConfig::new(400.0, 16.0),
        &MonospaceMetrics::new(8.0),
    )
    .expect("layout");
    let layout = view.table().expect("native grid");
    assert_eq!(layout.cells().len(), 4);
    assert_eq!(
        layout
            .cells()
            .iter()
            .map(|cell| cell.is_header())
            .collect::<Vec<_>>(),
        [false, false, true, false]
    );
    assert_eq!(
        layout
            .width_anchor_source()
            .expect("HTML width anchor")
            .end()
            .get() as usize,
        source.find("<tr>").expect("first row")
    );
    assert_eq!(
        layout.row_sources(),
        html.rows.iter().map(|row| row.source).collect::<Vec<_>>()
    );
    for (cell, original) in layout
        .cells()
        .iter()
        .zip(html.rows.iter().flat_map(|row| &row.cells))
    {
        assert_eq!(cell.source(), original.content);
        let bounds = cell.bounds();
        let hit = view
            .hit_test(LayoutPoint::new(
                bounds.x() + bounds.width() / 2.0,
                bounds.y() + bounds.height() / 2.0,
            ))
            .expect("hit");
        assert!(original.content.start() <= hit.source() && hit.source() <= original.content.end());
    }
}

#[test]
fn html_table_projection_preserves_empty_cells_and_rejects_unlaid_block_content() {
    for (source, supported) in [
        (
            "<table><tr><td></td><td>  中文 &amp; text<br>next  </td></tr></table>",
            true,
        ),
        (
            "<table><tr><td><p>one</p><p>two</p></td></tr></table>",
            true,
        ),
        (
            "<table><tr><td>one<div>two</div>three</td></tr></table>",
            true,
        ),
    ] {
        let snapshot = TextBuffer::new(source).snapshot();
        let document = yu_markdown::parse(&snapshot);
        let block = document.blocks().get(0).expect("block");
        let model = document.html_regions().regions[0]
            .model
            .as_ref()
            .expect("model");
        let mut output = ExtensionOutput::default();
        assert_eq!(
            model.decorate_table(source, &model.partitions[0], &mut output),
            supported
        );
        let decorations = BlockDecorations::from_output(&snapshot, block.range(), output);
        let visual =
            VisualText::new(&snapshot, block.range(), decorations.set().clone()).expect("visual");
        if supported {
            let expected = if source.contains("<p>") {
                "one\ntwo"
            } else if source.contains("<div>") {
                "one\ntwo\nthree"
            } else {
                "中文 & text\nnext"
            };
            assert_eq!(visual.text(), expected);
            let view = BlockView::build(
                block.kind(),
                &visual,
                &decorations,
                LayoutConfig::new(400.0, 16.0),
                &MonospaceMetrics::new(8.0),
            )
            .expect("layout");
            let cells = view.table().expect("table").cells();
            assert_eq!(cells[0].source().is_empty(), source.contains("<td></td>"));
            assert!(cells[0].bounds().width() > 0.0);
            assert!(cells[0].bounds().height() > 0.0);
        } else {
            assert_eq!(visual.text(), source);
        }
        assert_eq!(document.source().as_str(), source);
    }
}

#[test]
fn html_column_widths_follow_structure_without_a_markdown_delimiter() {
    use yu_core::ByteOffset;
    use yu_editor::{CaretAffinity, EditorCommand, EditorDocument, EditorSelection, TableEdit};
    let source =
        "<table><tbody><tr><td>A</td><td>B</td></tr><tr><td>C</td><td>D</td></tr></tbody></table>";
    let mut document = EditorDocument::new(source);
    let snapshot = document.snapshot();
    let block = document.markdown().blocks().get(0).expect("table");
    let model = document.markdown().html_regions().regions[0]
        .model
        .as_ref()
        .expect("model");
    let mut output = ExtensionOutput::default();
    assert!(model.decorate_table(source, &model.partitions[0], &mut output));
    let decorations = BlockDecorations::from_output(&snapshot, block.range(), output);
    let visual =
        VisualText::new(&snapshot, block.range(), decorations.set().clone()).expect("visual");
    let mut view = BlockView::build(
        block.kind(),
        &visual,
        &decorations,
        LayoutConfig::new(500.0, 16.0),
        &MonospaceMetrics::new(8.0),
    )
    .expect("layout");
    view.apply_table_column_resize(0, 55.0).expect("resize");
    document
        .confirm_table_column_widths(0, view.table().expect("table"))
        .expect("confirm");
    assert_eq!(document.snapshot().as_str(), source);
    let records = document.table_column_width_records();
    assert_eq!(records.len(), 1);
    let anchor = records[0].delimiter;
    assert_eq!(
        &source[anchor.start().get() as usize..anchor.end().get() as usize],
        "<table><tbody>"
    );
    assert!(records[0].proportions[0] > records[0].proportions[1]);
    let mut reopened = EditorDocument::new(source);
    reopened
        .restore_table_column_width_records(&records)
        .expect("restore");
    assert_eq!(reopened.table_column_width_records(), records);

    let at = ByteOffset::new(source.find("A</td>").expect("A") as u64);
    document
        .set_selection(
            EditorSelection::cursor(&snapshot, at, CaretAffinity::Downstream).expect("cursor"),
        )
        .expect("selection");
    document
        .execute(EditorCommand::insert_text("中文"))
        .expect("edit cell");
    assert_eq!(document.table_column_width_records(), records);
    assert!(
        document
            .confirm_table_column_widths(0, view.table().expect("old geometry"))
            .is_err()
    );
    document
        .execute(EditorCommand::EditTable(TableEdit::InsertColumnAfter))
        .expect("add column");
    assert!(document.table_column_width_records().is_empty());
    document.undo().expect("undo column");
    assert_eq!(document.table_column_width_records(), records);
    document.undo().expect("undo typing");
    assert_eq!(document.snapshot().as_str(), source);
    let snapshot = document.snapshot();
    document
        .set_selection(
            EditorSelection::cursor(&snapshot, ByteOffset::ZERO, CaretAffinity::Downstream)
                .expect("start cursor"),
        )
        .expect("selection");
    document
        .execute(EditorCommand::insert_text("前言\n\n"))
        .expect("prefix");
    let shifted = document.table_column_width_records();
    assert_eq!(shifted.len(), 1);
    assert_eq!(shifted[0].proportions, records[0].proportions);
    assert_eq!(shifted[0].delimiter.start().get(), "前言\n\n".len() as u64);
}

#[test]
fn html_table_production_layout_survives_editing_resize_and_source_mode() {
    use yu_core::ByteOffset;
    use yu_editor::{CaretAffinity, EditorCommand, EditorDocument, EditorSelection};
    let source =
        "<table><tr><th>A&amp;B</th><th>标题</th></tr><tr><td>中文🙂</td><td></td></tr></table>";
    let config = LayoutConfig::new(500.0, 16.0);
    let mut document = EditorDocument::new(source);
    let mut layout = document
        .block_layout_for_visual_state(0, config)
        .expect("production layout");
    assert_eq!(layout.visual().text(), "A&B标题中文🙂");
    assert_eq!(layout.table().expect("table").cells().len(), 4);
    for cell in layout.table().expect("table").cells() {
        let bounds = cell.bounds();
        let hit = layout
            .hit_test(LayoutPoint::new(
                bounds.x() + bounds.width() / 2.0,
                bounds.y() + bounds.height() / 2.0,
            ))
            .expect("cell hit");
        assert!(cell.source().start() <= hit.source() && hit.source() <= cell.source().end());
    }
    layout.apply_table_column_resize(0, 40.0).expect("resize");
    document
        .confirm_table_column_widths(0, layout.table().expect("table"))
        .expect("confirm");
    let widths = layout.table().expect("table").column_widths().to_vec();
    let at = ByteOffset::new(source.find("中文").expect("cell") as u64);
    document
        .set_selection(
            EditorSelection::cursor(&document.snapshot(), at, CaretAffinity::Downstream)
                .expect("cursor"),
        )
        .expect("selection");
    document
        .execute(EditorCommand::insert_text("新<&"))
        .expect("input");
    let current = document
        .block_layout_for_visual_state(0, config)
        .expect("edited layout");
    assert_eq!(current.visual().text(), "A&B标题新<&中文🙂");
    assert_eq!(
        current.table().expect("active table").column_widths(),
        widths
    );
    let caret = current
        .caret_for_source(document.selection().focus(), yu_decoration::Bias::After)
        .expect("caret");
    assert!(
        current.table().expect("table").cells()[2]
            .bounds()
            .contains(caret.point())
    );
    document.undo().expect("undo");
    assert_eq!(document.snapshot().as_str(), source);
    document.set_source_mode(true).expect("source");
    let raw = document
        .block_layout_for_visual_state(0, config)
        .expect("raw");
    assert!(raw.table().is_none());
    assert_eq!(raw.visual().text(), source);
    document.set_source_mode(false).expect("preview");
    assert!(
        document
            .block_layout_for_visual_state(0, config)
            .expect("preview layout")
            .table()
            .is_some()
    );
}

#[test]
fn html_table_preedit_uses_production_cell_geometry_without_mutating_source() {
    use yu_core::{ByteOffset, TextRange, Utf16Offset, Utf16Range};
    use yu_editor::EditorDocument;
    let source = "<table><tr><td>A</td><td></td></tr></table>";
    for target in ["A</td>", "</td></tr>"] {
        let mut document = EditorDocument::new(source);
        let at = ByteOffset::new(source.find(target).expect("target") as u64);
        document
            .begin_composition(
                TextRange::empty(at),
                "中文",
                Utf16Range::empty(Utf16Offset::new(2)),
            )
            .expect("preedit");
        let layout = document
            .block_layout_for_visual_state(0, LayoutConfig::new(400.0, 16.0))
            .expect("marked layout");
        assert!(layout.table().is_some());
        assert!(layout.visual().text().contains("中文"));
        assert_eq!(document.snapshot().as_str(), source);
        assert!(document.cancel_composition());
        assert_eq!(document.snapshot().as_str(), source);
        document
            .begin_composition(
                TextRange::empty(at),
                "中文",
                Utf16Range::empty(Utf16Offset::new(2)),
            )
            .expect("preedit again");
        document.commit_composition("中文<&").expect("commit");
        assert_eq!(
            document.snapshot().as_str(),
            format!(
                "{}中文&lt;&amp;{}",
                &source[..at.get() as usize],
                &source[at.get() as usize..]
            )
        );
        assert!(
            document
                .block_layout_for_visual_state(0, LayoutConfig::new(400.0, 16.0))
                .expect("committed layout")
                .table()
                .is_some()
        );
        document.undo().expect("undo commit");
        assert_eq!(document.snapshot().as_str(), source);
    }
}

#[test]
fn ragged_html_layout_has_only_real_cells_and_keeps_empty_rows() {
    let source = "<table><tr></tr><tr><td>A</td></tr><tr><td>B</td><td>C</td></tr></table>";
    let mut doc = yu_editor::EditorDocument::new(source);
    let view = doc
        .block_layout_for_visual_state(0, LayoutConfig::new(400.0, 16.0))
        .expect("sparse layout");
    let table = view.table().expect("native table");
    assert_eq!(table.cells().len(), 3);
    assert_eq!(
        table
            .cells()
            .iter()
            .map(|cell| (cell.row(), cell.column()))
            .collect::<Vec<_>>(),
        [(1, 0), (2, 0), (2, 1)]
    );
    assert_eq!(table.cells()[0].bounds().x(), table.cells()[1].bounds().x());
    assert!(table.cells()[0].bounds().y() > 0.0);
    let hit = view
        .hit_test(LayoutPoint::new(table.cells()[0].bounds().x() + 1.0, 1.0))
        .expect("empty row hit");
    assert!(
        table.cells()[0].source().start() <= hit.source()
            && hit.source() <= table.cells()[0].source().end()
    );
    assert_eq!(doc.snapshot().as_str(), source);
}

#[test]
fn html_cell_paragraphs_have_independent_alignment_and_gap_geometry() {
    let source = "<table><tr><td><p>one</p><p align='right'>two</p></td><td>KEEP</td></tr></table>";
    let mut doc = yu_editor::EditorDocument::new(source);
    let view = doc
        .block_layout_for_visual_state(0, LayoutConfig::new(400.0, 16.0))
        .expect("paragraph cell");
    let table = view.table().expect("table");
    let layout = &table.cell_layouts()[0];
    assert_eq!(layout.lines().len(), 2);
    assert!(layout.lines()[1].y() > layout.lines()[0].y() + layout.lines()[0].height());
    assert!(layout.lines()[1].indent() > layout.lines()[0].indent());
    let source_at = yu_core::ByteOffset::new(source.find("two</p>").expect("two") as u64);
    let caret = view
        .caret_for_source(source_at, yu_editor::Bias::After)
        .expect("paragraph caret");
    let hit = view
        .hit_test(LayoutPoint::new(
            caret.point().x() + 1.0,
            caret.point().y() + 1.0,
        ))
        .expect("paragraph hit");
    assert!(hit.source().get() >= source_at.get());
    assert_eq!(doc.snapshot().as_str(), source);
}

#[test]
fn empty_html_cell_paragraphs_have_distinct_caret_rows_and_preedit() {
    use yu_core::{ByteOffset, TextRange, Utf16Offset, Utf16Range};
    let source = "<table><tr><td><p></p><p></p><p>LAST</p></td></tr></table>";
    let mut doc = yu_editor::EditorDocument::new(source);
    let offsets: Vec<_> = source
        .match_indices("</p>")
        .take(2)
        .map(|(i, _)| ByteOffset::new(i as u64))
        .collect();
    let view = doc
        .block_layout_for_visual_state(0, LayoutConfig::new(400.0, 16.0))
        .expect("empty paragraphs");
    let carets = offsets
        .iter()
        .map(|at| {
            view.caret_for_source(*at, yu_editor::Bias::After)
                .expect("empty caret")
                .point()
                .y()
        })
        .collect::<Vec<_>>();
    assert!(
        carets[1] > carets[0],
        "empty paragraphs share caret: {carets:?}"
    );
    doc.begin_composition(
        TextRange::empty(offsets[1]),
        "中文",
        Utf16Range::empty(Utf16Offset::new(2)),
    )
    .expect("preedit");
    let marked = doc
        .block_layout_for_visual_state(0, LayoutConfig::new(400.0, 16.0))
        .expect("marked paragraphs");
    assert!(marked.visual().text().contains("中文"));
    assert!(marked.table().is_some());
    assert_eq!(doc.snapshot().as_str(), source);
    assert!(doc.cancel_composition());
    assert_eq!(doc.snapshot().as_str(), source);
}

#[test]
fn html_cell_headings_keep_paragraph_geometry_and_empty_input_positions() {
    use yu_core::{ByteOffset, TextRange, Utf16Offset, Utf16Range};
    for level in 1..=6 {
        let source = format!(
            "<table><tr><td><h{level} align='right'>标题</h{level}><p>body</p><h{level}></h{level}></td><td>KEEP</td></tr></table>"
        );
        let mut doc = yu_editor::EditorDocument::new(&source);
        let config = LayoutConfig::new(500.0, 16.0);
        let view = doc
            .block_layout_for_visual_state(0, config)
            .expect("heading table");
        assert_eq!(view.visual().text(), "标题\nbody\nKEEP");
        let table = view.table().expect("native table");
        let lines = table.cell_layouts()[0].lines();
        assert_eq!(lines.len(), 3);
        assert!(lines[0].indent() > 0.0);
        assert!(lines[1].y() >= lines[0].y() + lines[0].height());
        assert!(lines[2].y() >= lines[1].y() + lines[1].height());
        let at = ByteOffset::new(
            source
                .find(&format!("<h{level}></h{level}>"))
                .expect("empty heading") as u64
                + 4,
        );
        doc.begin_composition(
            TextRange::empty(at),
            "中文",
            Utf16Range::empty(Utf16Offset::new(2)),
        )
        .expect("preedit");
        let marked = doc
            .block_layout_for_visual_state(0, config)
            .expect("heading preedit");
        assert!(marked.table().is_some());
        assert!(marked.visual().text().contains("中文"));
        assert_eq!(doc.snapshot().as_str(), source);
        assert!(doc.cancel_composition());
        doc.set_source_mode(true).expect("source mode");
        let raw = doc.block_layout_for_visual_state(0, config).expect("raw");
        assert!(raw.table().is_none());
        assert_eq!(raw.visual().text(), source);
    }
}

#[test]
fn html_cell_lists_wrap_inside_nested_gutters_and_keep_empty_item_hits() {
    let source = "<table><tr><td><ol start='99'><li>outer words words words words<ul><li>nested words words words words</li><li></li></ul>continuation</li><li>second</li></ol></td><td>KEEP</td></tr></table>";
    let mut doc = yu_editor::EditorDocument::new(source);
    let view = doc
        .block_layout_for_visual_state(0, LayoutConfig::new(300.0, 16.0))
        .expect("list layout");
    let table = view.table().expect("native list table");
    let cell = table.cells()[0];
    let rows = table.cell_layouts()[0].lines();
    assert!(rows.len() >= 5);
    let outer = rows[0].indent();
    assert!(outer > 0.0);
    assert!(rows.iter().any(|line| line.indent() > outer));
    let at = yu_core::ByteOffset::new(source.find("<li></li>").expect("empty") as u64 + 4);
    let caret = view
        .caret_for_source(at, yu_editor::Bias::After)
        .expect("empty caret");
    let hit = view
        .hit_test(LayoutPoint::new(caret.point().x(), caret.point().y() + 1.0))
        .expect("empty hit");
    assert_eq!(hit.source(), at);
    assert!(cell.bounds().contains(caret.point()));
    for word in ["outer", "nested", "continuation", "second"] {
        let at = yu_core::ByteOffset::new(source.find(word).expect("word") as u64);
        let caret = view
            .caret_for_source(at, yu_editor::Bias::After)
            .expect("word caret");
        let hit = view
            .hit_test(LayoutPoint::new(caret.point().x(), caret.point().y() + 1.0))
            .expect("word hit");
        assert_eq!(hit.source(), at, "{word}");
    }
    assert_eq!(doc.snapshot().as_str(), source);
}

#[test]
fn html_cell_loose_list_paragraphs_have_gaps_and_consistent_number_gutters() {
    let source = "<table><tr><td><ol start='9999'><li><p>first</p><p>continued</p></li><li><p>second</p></li></ol></td></tr></table>";
    let mut doc = yu_editor::EditorDocument::new(source);
    let view = doc
        .block_layout_for_visual_state(0, LayoutConfig::new(500.0, 16.0))
        .expect("loose list");
    let lines = view.table().expect("table").cell_layouts()[0].lines();
    assert_eq!(lines.len(), 3);
    assert_eq!(lines[0].indent(), lines[1].indent());
    assert_eq!(lines[0].indent(), lines[2].indent());
    assert!(lines[1].y() > lines[0].y() + lines[0].height());
}

#[test]
fn separate_html_cell_lists_keep_a_block_gap() {
    let source = "<table><tr><td><ol><li>first</li></ol><ul><li>second</li></ul></td></tr></table>";
    let mut doc = yu_editor::EditorDocument::new(source);
    let view = doc
        .block_layout_for_visual_state(0, LayoutConfig::new(500.0, 16.0))
        .expect("separate lists");
    let lines = view.table().expect("table").cell_layouts()[0].lines();
    assert_eq!(lines.len(), 2);
    assert!(lines[1].y() > lines[0].y() + lines[0].height());
}

#[test]
fn mixed_spans_share_width_height_and_hit_owner_without_duplicate_cells() {
    let source = "<table><tr><td colspan='2' rowspan='2'>中文🙂 long text wraps across the merged cell several times</td><td>B</td></tr><tr><td>C</td></tr><tr><td>D</td><td>E</td><td>F</td></tr></table>";
    let snapshot = TextBuffer::new(source).snapshot();
    let document = yu_markdown::parse(&snapshot);
    let block = document.blocks().get(0).expect("block");
    let index = document.html_regions();
    let model = index.regions[0].model.as_ref().expect("model");
    let mut output = ExtensionOutput::default();
    assert!(model.decorate_table(source, &model.partitions[0], &mut output));
    let decorations = BlockDecorations::from_output(&snapshot, block.range(), output);
    let visual =
        VisualText::new(&snapshot, block.range(), decorations.set().clone()).expect("visual");
    for width in [160.0, 420.0] {
        let view = BlockView::build(
            block.kind(),
            &visual,
            &decorations,
            LayoutConfig::new(width, 16.0),
            &MonospaceMetrics::new(8.0),
        )
        .expect("layout");
        let layout = view.table().expect("table geometry");
        assert_eq!(layout.cells().len(), 6);
        let cells = layout.cells();
        let merged = cells[0];
        let b = cells[1];
        let c = cells[2];
        let d = cells[3];
        let e = cells[4];
        assert_eq!((c.row(), c.column()), (1, 2));
        assert!((merged.bounds().width() - d.bounds().width() - e.bounds().width()).abs() < 0.001);
        assert!(
            (merged.bounds().height() - b.bounds().height() - c.bounds().height()).abs() < 0.001
        );
        assert!((c.bounds().x() - b.bounds().x()).abs() < 0.001);
        assert!((d.bounds().y() - merged.bounds().height()).abs() < 0.001);
        let inner_x = d.bounds().x() + d.bounds().width();
        let inner_y = b.bounds().y() + b.bounds().height();
        assert!(
            layout
                .resize_hit_test(
                    LayoutPoint::new(inner_x, merged.bounds().height() * 0.25),
                    0.1
                )
                .expect("hidden vertical divider")
                .is_none()
        );
        assert!(
            layout
                .resize_hit_test(LayoutPoint::new(merged.bounds().x() + 2.0, inner_y), 0.1)
                .expect("hidden horizontal divider")
                .is_none()
        );
        assert!(
            layout
                .resize_hit_test(
                    LayoutPoint::new(inner_x, d.bounds().y() + d.bounds().height() * 0.5),
                    0.1
                )
                .expect("visible vertical divider")
                .is_some()
        );
        assert!(
            layout
                .resize_hit_test(
                    LayoutPoint::new(b.bounds().x() + b.bounds().width() * 0.5, inner_y),
                    0.1
                )
                .expect("visible horizontal divider")
                .is_some()
        );
        let borders = layout.border_rects().expect("border rectangles");
        for point in [
            LayoutPoint::new(inner_x, merged.bounds().height() * 0.25),
            LayoutPoint::new(merged.bounds().x() + 2.0, inner_y),
        ] {
            assert!(!borders.iter().any(|rect| point.x() >= rect.x()
                && point.x() < rect.x() + rect.width()
                && point.y() >= rect.y()
                && point.y() < rect.y() + rect.height()));
        }
        for fraction in [0.1, 0.5, 0.9] {
            let hit = layout
                .hit_test(LayoutPoint::new(
                    merged.bounds().x() + merged.bounds().width() * 0.5,
                    merged.bounds().y() + merged.bounds().height() * fraction,
                ))
                .expect("hit")
                .expect("cell");
            assert_eq!((hit.row(), hit.column()), (0, 0));
            assert_eq!(hit.source(), merged.source());
        }
        let hit = layout
            .hit_test(LayoutPoint::new(c.bounds().x() + 1.0, c.bounds().y() + 1.0))
            .expect("hit")
            .expect("cell");
        assert_eq!((hit.row(), hit.column()), (1, 2));
    }
    assert_eq!(snapshot.as_str(), source);
}
