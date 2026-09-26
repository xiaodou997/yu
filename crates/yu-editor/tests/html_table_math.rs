//! Inline math promotion: canonical TeX, native widgets, editing and persistence.
//! These are core/geometry tests, not evidence of macOS real-window acceptance.
use yu_core::{ByteOffset, TextRange, VisualOffset};
use yu_editor::{
    BlockView, CaretAffinity, EditorCommand, EditorDocument, EditorDocumentError, EditorSelection,
    LayoutConfig, LayoutPoint, MonospaceMetrics, VisualText,
};
use yu_markdown::{EmbeddedKind, EmbeddedSpan};

const SIMPLE: &str = include_str!("fixtures/group4-paste/math-target.md");
const MIXED: &str = include_str!("fixtures/group4-paste/math-mixed-target.md");
const PAYLOAD: &str = include_str!("fixtures/group4-paste/merged-payload.md");

// Production scheduling calls equation_source, not just embedded_source.
// A decoration-only test can miss an equation-index registration failure.
fn production_tex(document: &mut EditorDocument) -> Vec<String> {
    let mut owned = Vec::new();
    for index in 0..document.markdown().blocks().len() {
        owned.extend(
            document
                .block_decorations(index)
                .expect("production decorations")
                .widgets()
                .iter()
                .filter_map(|widget| match widget {
                    yu_markdown::BlockWidget::Embedded(span) if span.kind == EmbeddedKind::Math => {
                        Some(*span)
                    }
                    _ => None,
                }),
        );
    }
    owned
        .into_iter()
        .map(|span| {
            document
                .markdown()
                .equation_source(span)
                .expect("formula must reach the production renderer, not source fallback")
        })
        .collect()
}

#[test]
fn production_equation_index_retains_promoted_math_through_history_and_reopen() {
    for source in variants(MIXED) {
        let mut document = promote(&source);
        let expected = vec!["h", "x^2", "a+b", "z"];
        assert_eq!(production_tex(&mut document), expected);
        let saved = document.snapshot().as_str().to_owned();
        document.undo().expect("undo promotion");
        assert_eq!(production_tex(&mut document), expected);
        document.redo().expect("redo promotion");
        assert_eq!(production_tex(&mut document), expected);
        let mut reopened = EditorDocument::new(saved);
        assert_eq!(production_tex(&mut reopened), expected);
    }
}

#[test]
fn html_formula_production_index_decodes_once_and_resolves_document_equation_labels() {
    let source = "<table><tr><td><span data-math-style='inline'>a&lt;b &amp; c</span></td><td><span data-math-style='inline'>\\\\eqref{later}</span></td></tr></table>\n\n$$x=1\\\\label{later}$$\n";
    let source = source.replace("\\\\", "\\");
    let mut document = EditorDocument::new(&source);
    let rendered = production_tex(&mut document);
    assert_eq!(rendered[0], "a<b & c");
    assert_eq!(rendered[1], "\\text{(}\\mathrm{1}\\text{)}");
    assert!(rendered[2].ends_with("\\text{(}\\mathrm{1}\\text{)}"));
    assert_eq!(document.snapshot().as_str(), source);
}

fn variants(source: &str) -> Vec<String> {
    ["\n", "\r\n"]
        .into_iter()
        .flat_map(|eol| {
            ["", "\u{feff}"]
                .into_iter()
                .map(move |bom| format!("{bom}{}", source.replace("\r\n", "\n").replace('\n', eol)))
        })
        .collect()
}

fn select(document: &mut EditorDocument, range: TextRange) {
    let snapshot = document.snapshot();
    document
        .set_selection(
            EditorSelection::range(
                &snapshot,
                range.start(),
                range.end(),
                CaretAffinity::Downstream,
            )
            .expect("source selection"),
        )
        .expect("set selection");
}

fn spans(document: &EditorDocument) -> Vec<EmbeddedSpan> {
    document
        .markdown()
        .html_regions()
        .regions
        .iter()
        .filter_map(|region| region.model.as_ref().ok())
        .flat_map(|model| model.inline_math_spans())
        .collect()
}

fn tex(document: &EditorDocument) -> Vec<String> {
    spans(document)
        .into_iter()
        .map(|span| {
            assert_eq!(span.kind, EmbeddedKind::Math);
            assert!(!span.display);
            assert!(span.source.start() < span.content.start());
            assert!(span.content.end() < span.source.end());
            document
                .markdown()
                .embedded_source(span)
                .expect("owned TeX source")
        })
        .collect()
}

fn promote(source: &str) -> EditorDocument {
    let mut document = EditorDocument::new(source);
    let at = source.find("目标中文🙂").expect("paste target");
    select(&mut document, TextRange::empty(ByteOffset::new(at as u64)));
    let command = EditorCommand::PasteHtmlTableSource(PAYLOAD.into());
    assert!(document.command_available(&command), "{source}");
    assert!(document.execute(command).expect("promotion").changed());
    assert_eq!(document.table_target_is_html(), Some(true));
    document
}

fn view(document: &EditorDocument, active: Option<TextRange>) -> BlockView {
    let markdown = document.markdown();
    let index = markdown.html_regions();
    let block = markdown
        .blocks()
        .iter()
        .find(|block| {
            index
                .partition_for(block.range())
                .is_some_and(|part| part.content.kind == yu_markdown::html::HtmlFlowKind::Table)
        })
        .expect("native HTML table block");
    let decorations = index
        .decorate(markdown, block, active)
        .expect("table decorations");
    let visual = VisualText::new(
        &document.snapshot(),
        block.range(),
        decorations.set().clone(),
    )
    .expect("source projection");
    BlockView::build(
        block.kind(),
        &visual,
        &decorations,
        LayoutConfig::new(800.0, 16.0),
        &MonospaceMetrics::new(8.0),
    )
    .expect("native table geometry")
}

#[test]
fn promotion_preserves_multiple_math_leaves_mixed_with_chinese_emoji_and_formatting() {
    for source in variants(MIXED) {
        let mut document = promote(&source);
        assert_eq!(tex(&document), ["h", "x^2", "a+b", "z"]);
        let after = document.snapshot().as_str().to_owned();
        assert!(after.contains("<strong>"));
        assert!(after.contains("<a href=\"https://example.com\">"));
        assert!(after.contains("中文🙂"));
        assert!(!after.contains("<img"));
        assert_eq!(
            after.starts_with('\u{feff}'),
            source.starts_with('\u{feff}')
        );
        if source.contains("\r\n") {
            assert!(!after.replace("\r\n", "").contains('\n'));
        }
        let native = view(&document, None);
        assert_eq!(native.embedded().len(), 4);
        assert!(native.images().is_empty());
        for (span, _) in native.embedded() {
            assert!(native.table().expect("table").cells().iter().any(|cell| {
                cell.source().start() <= span.source.start()
                    && span.source.end() <= cell.source().end()
            }));
        }
        document.undo().expect("one undo");
        assert_eq!(document.snapshot().as_str(), source);
        document.redo().expect("one redo");
        assert_eq!(document.snapshot().as_str(), after);
        assert_eq!(tex(&document), ["h", "x^2", "a+b", "z"]);
    }
}

#[test]
fn promotion_retains_tex_escapes_and_decodes_html_entities_exactly_once() {
    for (cell, expected) in [
        (r"中文🙂$a<b & c>d$尾", vec!["a<b & c>d"]),
        (r"$\frac{1}{2}+\alpha$", vec![r"\frac{1}{2}+\alpha"]),
        (r"$\{x\}+\$$", vec![r"\{x\}+\$"]),
        (r"$a \| b$", vec![r"a \| b"]),
        (r"$&lt; + &amp;$", vec!["&lt; + &amp;"]),
        (r"`$not_math$` and \$literal", vec![]),
        (r"*$a$* 与 [$b$](https://example.com)", vec!["a", "b"]),
    ] {
        for source in variants(&SIMPLE.replace("$x^2$", cell)) {
            let document = promote(&source);
            assert_eq!(tex(&document), expected, "{cell}");
            let reopened = EditorDocument::new(document.snapshot().as_str());
            assert_eq!(tex(&reopened), expected, "reopened {cell}");
        }
    }
}

#[test]
fn formula_clicks_enter_the_body_and_typing_stays_inside_identity_tags() {
    let document = promote(MIXED);
    let native = view(&document, None);
    for (span, placed) in native.embedded() {
        let bounds = placed.bounds();
        for (fraction, expected) in [(0.25, span.content.start()), (0.75, span.content.end())] {
            let hit = native
                .hit_test(LayoutPoint::new(
                    bounds.x() + bounds.width() * fraction,
                    bounds.y() + bounds.height() * 0.5,
                ))
                .expect("formula hit");
            assert_eq!(hit.source(), expected);
            let mut editing = EditorDocument::new(document.snapshot().as_str());
            let before = tex(&editing);
            select(&mut editing, TextRange::empty(hit.source()));
            editing
                .execute(EditorCommand::insert_text("q"))
                .expect("type into formula");
            let after = tex(&editing);
            assert_eq!(after.len(), before.len());
            let ordinal = spans(&document)
                .iter()
                .position(|candidate| candidate == span)
                .expect("valid math integration fixture");
            for (index, (a, b)) in after.iter().zip(&before).enumerate() {
                let expected = if index != ordinal {
                    b.clone()
                } else if fraction < 0.5 {
                    format!("q{b}")
                } else {
                    format!("{b}q")
                };
                assert_eq!(*a, expected);
            }
        }
    }
}

#[test]
fn active_projection_reveals_only_one_formula_and_maps_entities_to_saved_bytes() {
    let source = "<table><tr><td>中文🙂<span data-math-style='inline'>a&lt;b &amp; c</span>尾<span data-math-style='inline'>y^2</span></td></tr></table>\n";
    let document = EditorDocument::new(source);
    let owned = spans(&document);
    assert_eq!(owned.len(), 2);
    let inactive = view(&document, None);
    assert_eq!(inactive.embedded().len(), 2);
    let active = view(&document, Some(TextRange::empty(owned[0].content.start())));
    assert_eq!(active.embedded().len(), 1);
    assert_eq!(active.embedded()[0].0, owned[1]);
    let visual = active.visual();
    assert_eq!(visual.text(), "中文🙂a<b & c尾");
    let at = visual.text().find('<').expect("decoded less-than");
    let coverage = TextRange::new(
        visual
            .visual_to_source(VisualOffset::new(at as u64), yu_editor::Bias::After)
            .expect("valid math integration fixture"),
        visual
            .visual_to_source(VisualOffset::new((at + 1) as u64), yu_editor::Bias::Before)
            .expect("valid math integration fixture"),
    )
    .expect("entity source coverage");
    assert_eq!(
        &source[coverage.start().get() as usize..coverage.end().get() as usize],
        "&lt;"
    );
    assert_eq!(document.snapshot().as_str(), source);
    assert_eq!(tex(&document), ["a<b & c", "y^2"]);
}

#[test]
fn body_editing_escapes_input_and_preserves_other_formulas_and_undo() {
    let mut document = promote(MIXED);
    let before = document.snapshot().as_str().to_owned();
    let body = spans(&document)[1].content;
    select(&mut document, body);
    let replacement = r"\frac{中文🙂}{2}<z & \alpha";
    document
        .execute(EditorCommand::insert_text(replacement))
        .expect("replace formula body");
    assert_eq!(tex(&document), ["h", replacement, "a+b", "z"]);
    let after = document.snapshot().as_str().to_owned();
    assert!(after.contains("&lt;z &amp;"));
    assert_eq!(view(&document, None).embedded().len(), 4);
    document.undo().expect("undo body edit");
    assert_eq!(document.snapshot().as_str(), before);
    document.redo().expect("redo body edit");
    assert_eq!(document.snapshot().as_str(), after);
}

#[test]
fn grapheme_deletion_removes_whole_entity_but_retains_empty_math_identity() {
    for command in [EditorCommand::DeleteBackward, EditorCommand::DeleteForward] {
        let source =
            "<table><tr><td>左<span data-math-style='inline'>&lt;</span>右</td></tr></table>\n";
        let mut document = EditorDocument::new(source);
        let span = spans(&document)[0];
        let at = if command == EditorCommand::DeleteBackward {
            span.content.end()
        } else {
            span.content.start()
        };
        select(&mut document, TextRange::empty(at));
        document
            .execute(command)
            .expect("delete one entity grapheme");
        assert_eq!(tex(&document), [""]);
        assert_eq!(document.snapshot().as_str(), source.replace("&lt;", ""));
        let empty = spans(&document)[0];
        assert!(empty.content.is_empty());
        select(&mut document, empty.content);
        document
            .execute(EditorCommand::insert_text("x"))
            .expect("refill empty formula");
        assert_eq!(tex(&document), ["x"]);
        document.undo().expect("undo refill");
        assert_eq!(tex(&document), [""]);
        document.undo().expect("undo entity deletion");
        assert_eq!(document.snapshot().as_str(), source);
    }
}

#[test]
fn whole_formula_selection_removes_tags_once_without_touching_neighbors() {
    let mut document = promote(MIXED);
    let before = document.snapshot().as_str().to_owned();
    let span = spans(&document)[1];
    let mut expected = before.clone();
    expected.replace_range(
        span.source.start().get() as usize..span.source.end().get() as usize,
        "",
    );
    select(&mut document, span.source);
    document
        .execute(EditorCommand::DeleteSelections)
        .expect("delete complete formula");
    assert_eq!(document.snapshot().as_str(), expected);
    assert_eq!(tex(&document), ["h", "a+b", "z"]);
    document.undo().expect("restore complete formula");
    assert_eq!(document.snapshot().as_str(), before);
}

#[test]
fn malformed_or_unsupported_math_payloads_are_rejected_without_mutation() {
    for body in [
        "<span data-math-style='display'>x</span>",
        "<span data-math-style='inline' data-extra='x'>x</span>",
        "<span data-math-style='inline' data-math-style='inline'>x</span>",
        "<span data-math-style='inline'><b>x</b></span>",
        "<span data-math-style='inline'>x<!--lost--></span>",
        "<span data-math-style='inline'>x&#10;y</span>",
        "<span data-math-style='inline'/>",
        "<div data-math-style='inline'>x</div>",
    ] {
        let mut document = promote(SIMPLE);
        document.undo().expect("seed conversion redo branch");
        let source = document.snapshot().as_str().to_owned();
        let selections = document.selections().clone();
        let history = document.history_stats();
        let revision = document.revision();
        let command = EditorCommand::PasteHtmlTableSource(
            format!("<table><tr><td>{body}</td></tr></table>").into(),
        );
        assert!(!document.command_available(&command));
        assert!(matches!(
            document.execute(command),
            Err(EditorDocumentError::InvalidTablePaste)
        ));
        assert_eq!(document.snapshot().as_str(), source);
        assert_eq!(document.selections(), &selections);
        assert_eq!(document.history_stats(), history);
        assert_eq!(document.revision(), revision);
        document
            .redo()
            .expect("original conversion redo still exists");
        assert_eq!(tex(&document), ["x^2"]);
    }
}

#[test]
fn reconciliation_is_per_cell_and_does_not_silently_accept_a_global_count_match() {
    let source = "| A | B |\n| --- | --- |\n| $x$ | $y$ |\n";
    let document = EditorDocument::new(source);
    let table = document
        .markdown()
        .blocks()
        .iter()
        .find_map(|block| yu_markdown::table_for_block(document.markdown(), block))
        .expect("Markdown table");
    let wrong = "<table><tr><th>A</th><th>B</th></tr><tr><td><span data-math-style='inline'>x</span><span data-math-style='inline'>y</span></td><td></td></tr></table>";
    assert!(
        document
            .markdown()
            .preserve_table_math_source(wrong, &table)
            .is_none()
    );
    let normalized = "<table><tr><th>A</th><th>B</th></tr><tr><td><span data-math-style='inline'>changed</span></td><td><span data-math-style='inline'>changed</span></td></tr></table>";
    let restored = document
        .markdown()
        .preserve_table_math_source(normalized, &table)
        .expect("valid math integration fixture");
    assert_eq!(tex(&EditorDocument::new(restored)), ["x", "y"]);
}

#[test]
fn canonical_file_bytes_reopen_with_the_same_math_semantics_and_remain_editable() {
    use std::io::Write;
    struct Temporary(std::path::PathBuf);
    impl Drop for Temporary {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.0);
        }
    }
    for (index, source) in variants(MIXED).into_iter().enumerate() {
        let document = promote(&source);
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("valid math integration fixture")
            .as_nanos();
        let file = Temporary(std::env::temp_dir().join(format!(
            "yu-table-math-{}-{stamp}-{index}.md",
            std::process::id(),
        )));
        let mut output = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&file.0)
            .expect("valid math integration fixture");
        let saved = document.snapshot().as_str().as_bytes().to_vec();
        output
            .write_all(&saved)
            .expect("valid math integration fixture");
        output.sync_all().expect("valid math integration fixture");
        drop(output);
        let bytes = std::fs::read(&file.0).expect("valid math integration fixture");
        assert_eq!(bytes, saved);
        let mut reopened =
            EditorDocument::new(String::from_utf8(bytes).expect("valid math integration fixture"));
        assert_eq!(tex(&reopened), ["h", "x^2", "a+b", "z"]);
        assert_eq!(view(&reopened, None).embedded().len(), 4);
        let body = spans(&reopened)[1].content;
        select(&mut reopened, body);
        reopened
            .execute(EditorCommand::insert_text("x^3"))
            .expect("edit reopened formula");
        assert_eq!(tex(&reopened), ["h", "x^3", "a+b", "z"]);
    }
}
