use serde_json::{Value, json};
use yu_export::document::{
    ExportImage, HtmlOptions, HtmlResources, ResourceError, export_html_document,
};
use yu_export::paged::{PageSettings, prepare_pdf_packet};
use yu_markdown::EmbeddedSpan;
use yu_text::TextBuffer;
struct Resources;
impl HtmlResources for Resources {
    fn checkpoint(&mut self) -> Result<(), String> {
        Ok(())
    }
    fn image(&mut self, _: &str) -> Result<ExportImage, ResourceError> {
        Ok(yu_export::portable::data_image(
            b"test-only",
            "image/png",
            64,
            64,
        ))
    }
    fn embedded(
        &mut self,
        _: EmbeddedSpan,
        _: &str,
        _: &HtmlOptions,
    ) -> Result<ExportImage, ResourceError> {
        Ok(yu_export::portable::data_image(
            b"test-only",
            "image/svg+xml",
            32,
            20,
        ))
    }
}
fn packet(source: &str) -> Value {
    let doc = yu_markdown::parse(&TextBuffer::new(source).snapshot());
    let html =
        export_html_document(&doc, &HtmlOptions::default(), &mut Resources).expect("semantics");
    assert!(
        html.warnings.is_empty(),
        "fixture must be in the supported finite vocabulary: {:?}",
        html.warnings
    );
    serde_json::from_slice(
        &prepare_pdf_packet(
            &html,
            PageSettings::from_config(&json!({})).expect("settings"),
        )
        .expect("packet"),
    )
    .expect("json")
}
#[test]
fn pdf_flow_reuses_resolved_semantics_and_resource_identity() {
    let p = packet(
        "---\ntitle: PDF 中文\nprivate: NEVER-LEAK\n---\n\n[TOC]\n\n# Heading\n\n中文 **bold** $x^2$ [link](https://example.com) ![图](image.png)[^n]\n\n<details><summary>展开</summary><p>SCREEN-OFF</p></details>\n\n[^n]: NOTE-END\n",
    );
    assert_eq!(p["title"], "PDF 中文");
    assert_eq!(p["images"].as_array().expect("images").len(), 2);
    let data = p.to_string();
    assert!(!data.contains("NEVER-LEAK"));
    for marker in [
        "bold",
        "SCREEN-OFF",
        "NOTE-END",
        "yu-heading-0",
        "yu-note-1",
        "yu-ref-0",
        "https://example.com",
    ] {
        assert!(data.contains(marker), "missing {marker}");
    }
    assert!(
        p["blocks"]
            .as_array()
            .expect("blocks")
            .iter()
            .any(|b| b["style"] == "h1")
    );
}
#[test]
fn pdf_table_uses_existing_grid_and_complete_rowspan_bands() {
    let p = packet(
        "<table><thead><tr><th>A</th><th>B</th></tr></thead><tbody><tr><td rowspan=\"2\">ONE</td><td>TWO</td></tr><tr><td>THREE</td></tr></tbody></table>",
    );
    let t = p["blocks"]
        .as_array()
        .expect("blocks")
        .iter()
        .find(|b| b["kind"] == "table")
        .expect("table");
    assert_eq!(t["columns"], 2);
    assert_eq!(t["rows"], 3);
    assert_eq!(t["headers"], 1);
    assert_eq!(t["bands"], json!([[0, 1], [1, 3]]));
    assert_eq!(t["cells"][2]["rows"], 2);
}
#[test]
fn pdf_direct_rows_keep_rowspan_and_cell_paragraph_breaks() {
    let p = packet(
        "<table><tr><td rowspan=\"2\"><p>ONE</p><p>TWO</p></td><td>A</td></tr><tr><td>B</td></tr></table>",
    );
    let t = p["blocks"]
        .as_array()
        .expect("blocks")
        .iter()
        .find(|b| b["kind"] == "table")
        .expect("table");
    assert_eq!(t["bands"], json!([[0, 2]]));
    assert_eq!(t["cells"][0]["rows"], 2);
    let runs = t["cells"][0]["runs"].as_array().expect("runs");
    assert!(runs.iter().any(|run| run["text"] == "\n"));
}
#[test]
fn pdf_preserves_finite_alignment_and_image_size_declarations() {
    let p = packet(
        "<p align=\"center\">CENTER</p>\n\n<img src=\"image.png\" width=\"128\">\n\n<img src=\"image.png\" width=\"50%\">\n",
    );
    assert!(
        p["blocks"]
            .as_array()
            .expect("blocks")
            .iter()
            .any(|b| b["align"] == "center")
    );
    assert_eq!(p["images"][0]["width"], 128.0);
    assert_eq!(p["images"][1]["widthPercent"], 50.0);
}
#[test]
fn pdf_markdown_strike_is_a_styled_text_run_not_literal_delimiters() {
    let p = packet("正文 ~~删除~~ 仍保留。\n");
    let runs = p["blocks"]
        .as_array()
        .expect("blocks")
        .iter()
        .flat_map(|block| block["runs"].as_array().into_iter().flatten());
    assert!(
        runs.into_iter()
            .any(|run| run["text"] == "删除" && run["strike"] == true)
    );
    assert!(!p.to_string().contains("~~"));
}
#[test]
fn pdf_page_options_have_fixed_bounds_and_physical_sizes() {
    let portrait =
        PageSettings::from_config(&json!({"paper":"Letter","margin":36})).expect("letter");
    assert_eq!((portrait.width, portrait.height), (612.0, 792.0));
    let landscape =
        PageSettings::from_config(&json!({"paper":"Letter","landscape":true})).expect("landscape");
    assert_eq!((landscape.width, landscape.height), (792.0, 612.0));
    for options in [
        json!({"paper":"Other"}),
        json!({"margin":17}),
        json!({"margin":145}),
        json!({"margin":"44"}),
    ] {
        assert!(PageSettings::from_config(&options).is_err());
    }
}
#[test]
fn pdf_flow_keeps_code_breaks_and_nested_list_prefixes() {
    let p = packet("- one\n  - child\n- two\n\n```rust\nlet a = 1;\nlet b = 2;\n```\n");
    let blocks = p["blocks"].as_array().expect("blocks");
    assert!(blocks.iter().any(|b| b["indent"] == 2));
    let code = blocks.iter().find(|b| b["style"] == "code").expect("code");
    assert!(code["runs"].to_string().contains("let a = 1;\\nlet b = 2;"));
}
