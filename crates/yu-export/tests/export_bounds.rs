//! Fixed public limits. No thresholds are enlarged by these tests.
use yu_export::document::*;
use yu_export::portable::{data_image, validate_svg_resource};
use yu_markdown::EmbeddedSpan;
use yu_text::TextBuffer;

#[derive(Default)]
struct Images {
    calls: usize,
}
impl HtmlResources for Images {
    fn checkpoint(&mut self) -> Result<(), String> {
        Ok(())
    }
    fn image(&mut self, _: &str) -> Result<ExportImage, ResourceError> {
        self.calls += 1;
        Ok(data_image(b"test-only", "image/png", 1, 1))
    }
    fn embedded(
        &mut self,
        _: EmbeddedSpan,
        _: &str,
        _: &HtmlOptions,
    ) -> Result<ExportImage, ResourceError> {
        panic!("no formula in boundary fixture")
    }
}
fn render(source: &str, resources: &mut Images) -> Result<HtmlDocument, String> {
    let document = yu_markdown::parse(&TextBuffer::new(source).snapshot());
    export_html_document(&document, &HtmlOptions::default(), resources)
}
#[test]
fn source_eight_mib_boundary_and_overflow() {
    let source = format!("```\n{}\n```\n", "x".repeat(MAX_SOURCE_BYTES - 9));
    assert_eq!(source.len(), MAX_SOURCE_BYTES);
    let mut resources = Images::default();
    let output = render(&source, &mut resources).expect("exact source limit");
    assert!(output.html.contains("</code></pre>"));
    assert!(output.warnings.is_empty());
    assert!(
        render(&(source + "x"), &mut resources)
            .expect_err("over limit")
            .contains("8 MiB")
    );
    assert_eq!(resources.calls, 0);
}
#[test]
fn resource_occurrence_limit_counts_repeated_references() {
    let mut resources = Images::default();
    let source = "![same](image.png)\n\n".repeat(MAX_RESOURCES);
    let output = render(&source, &mut resources).expect("2048 occurrences");
    assert_eq!(output.image_count, MAX_RESOURCES);
    assert_eq!(resources.calls, MAX_RESOURCES);
    resources.calls = 0;
    assert!(
        render(&(source + "![one more](image.png)\n"), &mut resources)
            .expect_err("2049 refused")
            .contains("2048")
    );
    assert_eq!(
        resources.calls, MAX_RESOURCES,
        "never ask for the excess resource"
    );
}
#[test]
fn svg_byte_budget_exact_and_one_over_is_fatal() {
    let limit = 4 * 1024 * 1024;
    let source = format!("<svg>{}</svg>", " ".repeat(limit - 11));
    assert_eq!(source.len(), limit);
    validate_svg_resource(&source).expect("exact SVG byte limit");
    assert!(
        matches!(validate_svg_resource(&(source + " ")), Err(ResourceError::Fatal(message)) if message.contains("4 MiB"))
    );
}
#[test]
fn svg_node_budget_exact_and_one_over_is_fatal() {
    // roxmltree counts the document root and the svg element as two nodes.
    let source = format!("<svg>{}</svg>", "<g/>".repeat(100_000 - 2));
    validate_svg_resource(&source).expect("exact XML node limit");
    let source = format!("<svg>{}</svg>", "<g/>".repeat(100_000 - 1));
    assert!(
        matches!(validate_svg_resource(&source), Err(ResourceError::Fatal(message)) if message.contains("100000"))
    );
    assert!(matches!(
        validate_svg_resource("<svg><broken></svg>"),
        Err(ResourceError::Warning(_))
    ));
    assert!(matches!(
        validate_svg_resource("<svg><script/></svg>"),
        Err(ResourceError::Warning(_))
    ));
}
#[test]
fn repeated_toc_expansion_is_rejected_before_cloning_large_output() {
    let mut source = "[TOC]\n\n".repeat(1000);
    for n in 0..5000 {
        source.push_str(&format!("# Heading {n}\n\n"));
    }
    assert!(source.len() < MAX_SOURCE_BYTES);
    let error = render(&source, &mut Images::default()).expect_err("TOC expansion refused");
    assert!(error.contains("预算"));
}
