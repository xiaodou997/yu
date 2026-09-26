#![forbid(unsafe_code)]
//! Native vector rendering. This crate belongs in the helper, not the app host.

mod mermaid_validation;

use serde::{Deserialize, Serialize};
use std::sync::OnceLock;
use typst::{
    Library, LibraryExt, World,
    diag::{FileError, FileResult},
    foundations::{Bytes, Datetime, Duration, Repr, Str},
    syntax::{FileId, Source},
    text::{Font, FontBook},
    utils::LazyHash,
};
use typst_layout::PagedDocument;

pub const MAX_SOURCE_BYTES: usize = 64 * 1024;
pub const MAX_SVG_BYTES: usize = yu_assets::EMBEDDED_SVG_MAX_MARKUP_BYTES;

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    Math,
    Mermaid,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct VectorOutput {
    pub width: u32,
    pub height: u32,
    pub svg: String,
    pub baseline_milli: u32,
}

fn output(width: f64, height: f64, svg: String) -> Result<VectorOutput, String> {
    if !width.is_finite()
        || !height.is_finite()
        || width <= 0.0
        || height <= 0.0
        || width > f64::from(yu_assets::EMBEDDED_SVG_MAX_DIMENSION)
        || height > f64::from(yu_assets::EMBEDDED_SVG_MAX_DIMENSION)
        || svg.len() > MAX_SVG_BYTES
    {
        return Err("Rendered resource exceeds geometry or output limits".into());
    }
    Ok(VectorOutput {
        width: width.ceil() as u32,
        height: height.ceil() as u32,
        svg,
        baseline_milli: (height * 1000.0).round() as u32,
    })
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RenderStyle {
    pub font_milli: u32,
    pub foreground: u32,
    pub dark: bool,
    pub display: bool,
    #[serde(default)]
    pub reference_day: Option<i32>,
}
impl Default for RenderStyle {
    fn default() -> Self {
        Self {
            font_milli: 24_000,
            foreground: 0x000000ff,
            dark: false,
            display: true,
            reference_day: None,
        }
    }
}

pub fn render(kind: Kind, source: &str) -> Result<VectorOutput, String> {
    render_styled(kind, source, RenderStyle::default())
}

pub fn render_styled(kind: Kind, source: &str, style: RenderStyle) -> Result<VectorOutput, String> {
    if !(1000..=256_000).contains(&style.font_milli) {
        return Err("Invalid render font size".into());
    }
    if style
        .reference_day
        .is_some_and(|day| !(-719162..=2932896).contains(&day))
    {
        return Err("Invalid render calendar day".into());
    }
    if source.trim().is_empty() || source.len() > MAX_SOURCE_BYTES {
        return Err("Empty or oversized embedded source".into());
    }
    match kind {
        Kind::Math => render_math(source, style),
        Kind::Mermaid => {
            mermaid_validation::validate(source)?;
            // The pinned parser checks statement consumption for the audited families.
            // Host checks also diagnose unsupported native interactions and pie data.
            let parsed = mermaid_rs_renderer::parse_mermaid_strict_at(source, style.reference_day)
                .map_err(|e| e.to_string())?;
            let mut options = mermaid_rs_renderer::RenderOptions::default();
            if style.dark {
                let font_family = options.theme.font_family.clone();
                options.theme = mermaid_rs_renderer::Theme::dark();
                options.theme.font_family = font_family;
            }
            options.theme.background = "none".into();
            options.theme.font_size = style.font_milli as f32 / 1000.0;
            let foreground = format!("#{:06x}", style.foreground >> 8);
            options.theme.text_color = foreground.clone();
            options.theme.primary_text_color = foreground;
            options.theme.pie_stroke_color = options.theme.primary_border_color.clone();
            options.theme.pie_outer_stroke_color = options.theme.primary_border_color.clone();

            // Specialized diagrams carry their colors in layout configuration.
            // Resolve those defaults from the same reading theme as ordinary edges.
            options.layout.c4.boundary_stroke = options.theme.primary_text_color.clone();
            options.layout.requirement.fill = options.theme.primary_color.clone();
            options.layout.requirement.box_stroke = options.theme.primary_border_color.clone();
            options.layout.requirement.stroke = options.theme.primary_border_color.clone();
            options.layout.requirement.divider_color = options.theme.primary_border_color.clone();
            options.layout.requirement.label_color = options.theme.primary_text_color.clone();
            options.layout.requirement.edge_stroke = options.theme.line_color.clone();
            options.layout.requirement.edge_label_color = options.theme.primary_text_color.clone();
            options.layout.requirement.edge_label_background = options.theme.primary_color.clone();
            parsed
                .apply_source_layout(&mut options.layout)
                .map_err(|error| error.to_string())?;
            let layout =
                mermaid_rs_renderer::compute_layout(&parsed.graph, &options.theme, &options.layout);
            if let mermaid_rs_renderer::layout::DiagramData::Gantt(gantt) = &layout.diagram
                && gantt.ticks.len() > 2048
            {
                return Err(
                    "Gantt tickInterval exceeds the native 2048-tick resource limit".into(),
                );
            }
            let size = mermaid_rs_renderer::measure_svg_dimensions(&layout, &options.layout, None);
            let svg = mermaid_rs_renderer::render_svg(&layout, &options.theme, &options.layout);
            output(size.width.into(), size.height.into(), svg)
        }
    }
}

struct Resources {
    library: LazyHash<Library>,
    book: LazyHash<FontBook>,
    fonts: Vec<Font>,
}
fn resources() -> &'static Resources {
    static RESOURCES: OnceLock<Resources> = OnceLock::new();
    RESOURCES.get_or_init(|| {
        let mut fonts: Vec<_> = typst_assets::fonts()
            .flat_map(|data| Font::iter(Bytes::new(data)))
            .collect();
        fonts.extend(Font::iter(Bytes::new(
            include_bytes!("../vendor/noto/NotoSerifCJKsc-Regular.otf").as_slice(),
        )));
        Resources {
            library: LazyHash::new(Library::default()),
            book: LazyHash::new(FontBook::from_fonts(&fonts)),
            fonts,
        }
    })
}
struct MathWorld {
    source: Source,
}
impl World for MathWorld {
    fn library(&self) -> &LazyHash<Library> {
        &resources().library
    }
    fn book(&self) -> &LazyHash<FontBook> {
        &resources().book
    }
    fn main(&self) -> FileId {
        self.source.id()
    }
    fn source(&self, id: FileId) -> FileResult<Source> {
        if id == self.main() {
            return Ok(self.source.clone());
        }
        for (name, text) in [
            ("xarrow/lib.typ", include_str!("../vendor/xarrow/lib.typ")),
            (
                "mitex/specs/mod.typ",
                include_str!("../vendor/mitex/specs/mod.typ"),
            ),
            (
                "mitex/specs/prelude.typ",
                include_str!("../vendor/mitex/specs/prelude.typ"),
            ),
            (
                "mitex/specs/latex/standard.typ",
                include_str!("../vendor/mitex/specs/latex/standard.typ"),
            ),
        ] {
            let allowed = typst::syntax::RootedPath::new(
                typst::syntax::VirtualRoot::Project,
                typst::syntax::VirtualPath::new(name).expect("bundled path"),
            )
            .intern();
            if id == allowed {
                return Ok(Source::new(id, text.into()));
            }
        }
        Err(FileError::AccessDenied)
    }
    fn file(&self, id: FileId) -> FileResult<Bytes> {
        self.source(id)
            .map(|source| Bytes::from_string(source.text().to_owned()))
    }
    fn font(&self, index: usize) -> Option<Font> {
        resources().fonts.get(index).cloned()
    }
    fn today(&self, _: Option<Duration>) -> Option<Datetime> {
        None
    }
}
fn render_math(source: &str, style: RenderStyle) -> Result<VectorOutput, String> {
    if !yu_tex::parse_document_controls(source)?.is_empty() {
        return Err("Equation labels, tags and references must be resolved by the document before rendering".into());
    }
    let font_size = style.font_milli as f64 / 1000.0;
    let foreground = format!("#{:08x}", style.foreground);
    let converted = mitex::convert_math(source, None)?;
    // Only generated MiTeX input is compiled. The World exposes no files or packages.
    let expression = Str::from(if style.display {
        format!("$ {converted} $")
    } else {
        format!("${converted}$")
    })
    .repr();
    let world = MathWorld {
        source: Source::detached(format!(
            "#set page(width: auto, height: auto, margin: 4pt, fill: none)\n#set text(size: {font_size}pt, fill: rgb(\"{foreground}\"))\n#import \"mitex/specs/mod.typ\": mitex-scope\n#box[#eval({expression}, scope: mitex-scope)]<yu-formula>"
        )),
    };
    let document = typst::compile::<PagedDocument>(&world)
        .output
        .map_err(|errors| {
            errors
                .iter()
                .map(|e| e.message.to_string())
                .collect::<Vec<_>>()
                .join("; ")
        })?;
    if document.pages().len() != 1 {
        return Err("Formula did not produce a single vector frame".into());
    }
    let page = &document.pages()[0];
    fn missing_glyphs(
        frame: &typst::layout::Frame,
        missing: &mut std::collections::BTreeSet<char>,
    ) {
        for (_, item) in frame.items() {
            match item {
                typst::layout::FrameItem::Group(group) => missing_glyphs(&group.frame, missing),
                typst::layout::FrameItem::Text(text) => {
                    for glyph in &text.glyphs {
                        if glyph.id == 0
                            && let Some(value) = text.text.get(glyph.range())
                        {
                            missing.extend(value.chars());
                        }
                    }
                }
                _ => {}
            }
        }
    }
    let mut missing = std::collections::BTreeSet::new();
    missing_glyphs(&page.frame, &mut missing);
    if !missing.is_empty() {
        return Err(format!(
            "Formula font coverage missing for: {}",
            missing.into_iter().collect::<String>()
        ));
    }
    let size = page.frame.size();
    let mut vector = output(
        size.x.to_pt(),
        size.y.to_pt(),
        typst_svg::svg(page, &typst_svg::SvgOptions::default()),
    )?;
    fn formula_baseline(frame: &typst::layout::Frame, y: f64) -> Option<f64> {
        for (position, item) in frame.items() {
            if let typst::layout::FrameItem::Group(group) = item {
                let y = y + position.y.to_pt();
                if group
                    .label
                    .is_some_and(|label| label.resolve().as_str() == "yu-formula")
                {
                    return group
                        .frame
                        .has_baseline()
                        .then(|| y + group.frame.baseline().to_pt());
                }
                if let Some(baseline) = formula_baseline(&group.frame, y) {
                    return Some(baseline);
                }
            }
        }
        None
    }
    let baseline =
        formula_baseline(&page.frame, 0.0).ok_or("Formula baseline missing from native layout")?;
    if !baseline.is_finite() || baseline < 0.0 || baseline > size.y.to_pt() {
        return Err("Invalid formula baseline".into());
    }
    vector.baseline_milli = (baseline * 1000.0).round() as u32;
    Ok(vector)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn foreground_changes_preserve_geometry_and_font_size_reflows_vectors() {
        let small = RenderStyle {
            font_milli: 16_000,
            foreground: 0x202020ff,
            dark: false,
            display: true,
            reference_day: None,
        };
        let dark = RenderStyle {
            foreground: 0xf2f2f2ff,
            dark: true,
            ..small
        };
        let light = render_styled(Kind::Math, r"\frac{a}{b}", small).expect("light");
        let night = render_styled(Kind::Math, r"\frac{a}{b}", dark).expect("dark");
        assert_eq!((light.width, light.height), (night.width, night.height));
        assert!(night.svg.contains("#f2f2f2"));
        assert_ne!(light.svg, night.svg);
        let large = render_styled(
            Kind::Math,
            r"\frac{a}{b}",
            RenderStyle {
                font_milli: 32_000,
                ..small
            },
        )
        .expect("large");
        assert!(large.width > light.width && large.height > light.height);
        let day_graph =
            render_styled(Kind::Mermaid, "flowchart LR\nA-->B", small).expect("day graph");
        let night_graph =
            render_styled(Kind::Mermaid, "flowchart LR\nA-->B", dark).expect("night graph");
        assert_eq!(
            (day_graph.width, day_graph.height),
            (night_graph.width, night_graph.height)
        );
        assert_ne!(day_graph.svg, night_graph.svg);
    }

    #[test]
    fn inline_formula_keeps_native_baseline_and_text_style() {
        let inline = RenderStyle {
            font_milli: 16_000,
            display: false,
            ..RenderStyle::default()
        };
        for source in ["x", "x_2", r"\frac{a}{b}", r"\sqrt{x}"] {
            let output = render_styled(Kind::Math, source, inline).expect(source);
            assert!(output.baseline_milli > 0);
            assert!(
                output.baseline_milli < output.height * 1000,
                "baseline must include descender/margin: {source}"
            );
        }
        let small = render_styled(Kind::Math, r"\frac{a}{b}", inline).expect("inline");
        let display = render_styled(
            Kind::Math,
            r"\frac{a}{b}",
            RenderStyle {
                display: true,
                ..inline
            },
        )
        .expect("display");
        assert!(
            display.height > small.height,
            "display and text math must use native typesetting styles"
        );
    }

    #[test]
    fn resolved_equation_numbers_render_and_unresolved_metadata_fails() {
        let plain = render(Kind::Math, "x=1").expect("plain");
        let numbered =
            render(Kind::Math, r"x=1 \qquad \text{(}\mathrm{A}\text{)}").expect("number");
        assert!(numbered.width > plain.width);
        assert!(render(Kind::Math, r"\text{(}\mathrm{12}\text{)}").is_ok());
        for source in [r"x\label{a}", r"x\tag{A}", r"\eqref{undefined}"] {
            assert!(
                render(Kind::Math, source).is_err(),
                "must not silently ignore {source}"
            );
        }
    }

    #[test]
    fn ignored_vendor_commands_have_explicit_document_policy() {
        let scope = include_str!("../vendor/mitex/specs/latex/standard.typ");
        for line in scope
            .lines()
            .filter(|line| line.contains("ignore-sym") || line.contains("handle: ignore-me"))
        {
            let Some((name, _)) = line.split_once(':') else {
                continue;
            };
            let name = name.trim().trim_matches('"').trim_end_matches('*');
            if name == "relax" {
                continue;
            } // Semantically a no-op in a standalone expression.
            let result = yu_tex::parse_document_controls(&format!("\\{name}{{x}}"));
            assert!(
                result.is_err() || result.is_ok_and(|controls| !controls.is_empty()),
                "Ignored vendor command must be resolved or rejected: {name}"
            );
        }
    }

    #[test]
    fn vector_limits_match_native_transport() {
        let limit = f64::from(yu_assets::EMBEDDED_SVG_MAX_DIMENSION);
        assert!(output(limit, 1.0, "<svg/>".into()).is_ok());
        assert!(output(limit + 0.1, 1.0, "<svg/>".into()).is_err());
        assert!(output(1.0, limit + 1.0, "<svg/>".into()).is_err());
        assert!(output(1.0, 1.0, "x".repeat(MAX_SVG_BYTES + 1)).is_err());
    }

    #[test]
    fn missing_formula_glyphs_are_diagnostics_instead_of_tofu_success() {
        for formula in [r"\text{🙂}", r"x+\frac{1}{\text{🙂}}"] {
            let error = render(Kind::Math, formula)
                .expect_err("unavailable glyph must not be a successful vector");
            assert!(error.contains("font coverage missing"), "{error}");
        }
        assert!(render(Kind::Math, r"\text{English}+\alpha+\sum_{i=1}^{n}i").is_ok());
    }

    #[test]
    fn bundled_cjk_font_renders_chinese_formula_text_as_real_glyphs() {
        for source in [
            r"\text{中文}",
            r"\frac{\text{收益}}{\text{成本}}",
            r"\text{繁體中文 日本語 한국어}",
        ] {
            let output = render(Kind::Math, source).expect("bundled CJK coverage");
            assert!(output.svg.contains("<path"));
            assert!(
                output.svg.len() > 1000,
                "CJK text must contain actual outlines"
            );
        }
    }

    #[test]
    fn real_math_vectors_cover_structures() {
        for formula in [
            r"x^2+\frac{a}{b}",
            r"\sqrt{1+x^2}",
            r"\begin{pmatrix}a&b\\c&d\end{pmatrix}",
            r"\begin{aligned}x&=1\\y&=2\end{aligned}",
        ] {
            let result = render(Kind::Math, formula).expect(formula);
            assert!(result.width > 8 && result.height > 8);
            assert!(
                result.svg.contains("<path"),
                "Math must contain outlined glyphs"
            );
        }
    }
    #[test]
    fn cases_columns_have_one_quad_separation_at_each_font_size() {
        for environment in ["cases", "rcases"] {
            for font_milli in [16_000, 24_000, 32_000] {
                let source = format!(r"\begin{{{environment}}}x&x>0\\x&x<0\end{{{environment}}}");
                let spaced = source.replace('&', r"\quad ");
                let joined = source.replace('&', "");
                let style = RenderStyle {
                    font_milli,
                    ..RenderStyle::default()
                };
                let actual =
                    render_styled(Kind::Math, &source, style).expect("valid cases spacing fixture");
                let expected =
                    render_styled(Kind::Math, &spaced, style).expect("valid cases spacing fixture");
                let collapsed =
                    render_styled(Kind::Math, &joined, style).expect("valid cases spacing fixture");
                assert_eq!(
                    (actual.width, actual.height),
                    (expected.width, expected.height)
                );
                assert!(actual.width >= collapsed.width + font_milli / 1000 - 1);
            }
        }
    }

    #[test]
    fn cases_spacing_does_not_change_nested_alignment() {
        let source = r"\begin{cases}\begin{aligned}a&=b\\c&=d\end{aligned}&x>0\end{cases}";
        let reference = source.replace("&x>0", r"\quad x>0");
        let actual = render(Kind::Math, source).expect("valid cases spacing fixture");
        let expected = render(Kind::Math, &reference).expect("valid cases spacing fixture");
        assert_eq!(
            (actual.width, actual.height),
            (expected.width, expected.height)
        );
    }

    #[test]
    fn multiline_aligned_source_preserves_rows_across_line_endings() {
        let compact = r"\begin{aligned}x&=1+2\\y&=3+4\\z&=5+6\end{aligned}";
        let expanded = compact.replace(r"\\", "\\\\\n");
        let reference = render(Kind::Math, compact).expect("three aligned rows");
        let single =
            render(Kind::Math, r"\begin{aligned}x&=1+2\end{aligned}").expect("one aligned row");
        assert!(reference.height > single.height * 2);
        for source in [expanded.clone(), expanded.replace('\n', "\r\n")] {
            let output = render(Kind::Math, &source).expect("multiline aligned source");
            assert_eq!(
                (output.width, output.height),
                (reference.width, reference.height)
            );
            assert_eq!(output.svg, reference.svg);
        }
    }

    #[test]
    fn extended_math_corpus_keeps_geometry_baselines_and_theme_parity() {
        for formula in [
            r"\int_0^\infty e^{-x}\,dx",
            r"\lim_{x\to0}\frac{\sin x}{x}=1",
            r"\sum_{i=1}^{n}i=\frac{n(n+1)}{2}",
            r"\begin{cases}x&x>0\\-x&x<0\end{cases}",
            r"\left(\frac{a}{b}\right)^2+\sqrt[3]{x}",
            r"\hat{x}+\bar{y}+\vec{v}+\overline{AB}",
            r"\begin{aligned}\text{收益}&=a+b\\\text{成本}&=c+d\end{aligned}",
            r"\begin{bmatrix}1&0\\0&1\end{bmatrix}",
        ] {
            for display in [false, true] {
                let style = RenderStyle {
                    display,
                    font_milli: 16_000,
                    ..RenderStyle::default()
                };
                let light = render_styled(Kind::Math, formula, style).expect(formula);
                let dark = render_styled(
                    Kind::Math,
                    formula,
                    RenderStyle {
                        dark: true,
                        foreground: 0xffffffff,
                        ..style
                    },
                )
                .expect(formula);
                assert_eq!(
                    (light.width, light.height, light.baseline_milli),
                    (dark.width, dark.height, dark.baseline_milli),
                    "{formula}"
                );
                assert!(
                    light.width > 0
                        && light.height > 0
                        && light.baseline_milli <= light.height * 1000,
                    "{formula}"
                );
                assert!(
                    light.svg.contains("<path") && dark.svg.contains("<path"),
                    "{formula}"
                );
                assert_ne!(
                    light.svg, dark.svg,
                    "Theme must change foreground: {formula}"
                );
            }
        }
    }

    #[test]
    fn native_diagram_families_produce_vectors() {
        for diagram in [
            "flowchart LR\nA-->B",
            "sequenceDiagram\nAlice->>Bob: Hello",
            "classDiagram\nAnimal <|-- Dog",
            "stateDiagram-v2\n[*] --> Ready",
            "erDiagram\nUSER ||--o{ POST : writes",
            "gantt\n dateFormat YYYY-MM-DD\n section Work\n Task :2026-01-01, 2d",
            "pie\n\"A\" : 30\n\"B\" : 70",
        ] {
            let result = render(Kind::Mermaid, diagram).expect(diagram);
            assert!(result.width > 8 && result.height > 8);
            assert!(result.svg.contains("<svg"));
        }
    }
    #[test]
    fn structured_diagram_corpus_preserves_labels_in_both_themes() {
        let cases: &[(&str, &[&str])] = &[
            (
                "flowchart LR\nsubgraph Work\nA[Input] --> B{Valid}\nend\nB -->|Yes| C[Save]\nB -->|No| D[Retry]",
                &["Input", "Valid", "Save", "Retry", "Work"],
            ),
            (
                "sequenceDiagram\nparticipant A as Alice\nparticipant B as Bob\nloop Each item\nA->>B: Request\nalt Accepted\nB-->>A: Done\nelse Rejected\nB-->>A: Retry\nend\nend",
                &["Alice", "Bob", "Request", "Accepted", "Rejected", "Retry"],
            ),
            (
                "classDiagram\nclass Ledger {\n+int count\n+save()\n}\nUser --> Ledger : owns",
                &["Ledger", "count", "save", "User", "owns"],
            ),
            (
                "stateDiagram-v2\n[*] --> Ready\nReady --> Working : start\nWorking --> Ready : finish\nReady --> [*]",
                &["Ready", "Working", "start", "finish"],
            ),
            (
                "erDiagram\nUSER ||--o{ POST : writes\nUSER {\nint id PK\nstring name\n}\nPOST {\nint id PK\nint user_id FK\n}",
                &["USER", "POST", "writes", "name", "user_id"],
            ),
            (
                "gantt\ndateFormat YYYY-MM-DD\nsection Delivery\nDesign :a, 2026-01-01, 2d\nBuild :after a, 3d",
                &["Delivery", "Design", "Build"],
            ),
            (
                "pie showData\ntitle Allocation\n\"Writing\" : 60\n\"Review\" : 40",
                &["Allocation", "Writing", "Review"],
            ),
        ];
        for &(source, labels) in cases {
            for dark in [false, true] {
                let vector = render_styled(
                    Kind::Mermaid,
                    source,
                    RenderStyle {
                        dark,
                        ..RenderStyle::default()
                    },
                )
                .expect(source);
                assert!(vector.width > 8 && vector.height > 8);
                for label in labels {
                    assert!(
                        vector.svg.contains(label),
                        "Missing {label} in diagram: {source}"
                    );
                }
            }
        }
    }

    #[test]
    fn native_diagrams_diagnose_unsupported_click_actions() {
        for source in [
            "flowchart LR\nA-->B\nclick A call external()",
            "graph TD\nA-->B\nclick A \"https://example.com\"",
        ] {
            let error = render(Kind::Mermaid, source)
                .expect_err("unsupported interaction must be explicit");
            assert!(error.contains("click directives"), "{error}");
        }
    }

    #[test]
    fn pie_labels_and_comments_preserve_every_slice() {
        let source = r#"pie showData
"Revenue: East" : 10 %% note: keep slice
"Cost %% label" : 20
"Say \"yes%%label\"" : 30
"Other" : 40"#;
        let parsed = mermaid_rs_renderer::parse_mermaid_strict(source).expect("pie");
        assert_eq!(
            parsed
                .graph
                .pie_slices
                .iter()
                .map(|s| (s.label.as_str(), s.value))
                .collect::<Vec<_>>(),
            [
                ("Revenue: East", 10.0),
                ("Cost %% label", 20.0),
                ("Say \"yes%%label\"", 30.0),
                ("Other", 40.0)
            ]
        );
        for dark in [false, true] {
            let vector = render_styled(
                Kind::Mermaid,
                source,
                RenderStyle {
                    dark,
                    ..RenderStyle::default()
                },
            )
            .expect("all pie slices");
            assert!(vector.svg.contains("Revenue: East") && vector.svg.contains("Cost %% label"));
            let border = if dark { "#cccccc" } else { "#94A3B8" };
            // Four slice boundaries plus the outer circle must survive the
            // transparent helper canvas used by the native image decoder.
            assert!(vector.svg.matches(&format!("stroke=\"{border}\"")).count() >= 5);
        }
        assert!(render(Kind::Mermaid, "pie\n\"overflow\" : 1e100").is_err());
    }

    #[test]
    fn gantt_task_timing_preserves_sequence_case_and_fractional_units() {
        let source = "gantt\nA :Build-A,2026-01-01,1.5d\nB :after Build-A,30m\nC :2h\nD :d,2026-01-01,1d\nE :1d\nMilestone :milestone,m,after d,0d";
        let parsed = mermaid_rs_renderer::parse_mermaid_strict(source).expect("timed tasks");
        assert_eq!(parsed.graph.gantt_tasks[0].id, "Build-A");
        assert_eq!(
            parsed.graph.gantt_tasks[0].duration.as_deref(),
            Some("1.5d")
        );
        assert_eq!(
            parsed.graph.gantt_tasks[1].after.as_deref(),
            Some("Build-A")
        );
        let layout = mermaid_rs_renderer::compute_layout(
            &parsed.graph,
            &mermaid_rs_renderer::Theme::modern(),
            &mermaid_rs_renderer::LayoutConfig::default(),
        );
        let mermaid_rs_renderer::layout::DiagramData::Gantt(gantt) = layout.diagram else {
            panic!("gantt")
        };
        let tasks = &gantt.tasks;
        assert!((tasks[0].duration - 1.5).abs() < 1e-8);
        assert!((tasks[1].duration - 1.0 / 48.0).abs() < 1e-8);
        assert!((tasks[1].start - tasks[0].start - 1.5).abs() < 1e-8);
        assert!((tasks[2].start - tasks[1].start - tasks[1].duration).abs() < 1e-8);
        assert!((tasks[2].duration - 1.0 / 12.0).abs() < 1e-8);
        assert_eq!(tasks[4].start, tasks[3].start + 1.0);
        assert_eq!(tasks[5].duration, 0.0);
        assert_eq!(tasks[5].start, tasks[3].start + 1.0);
        let scale = f64::from(gantt.chart_width) / (gantt.time_end - gantt.time_start);
        assert!((f64::from(tasks[1].width) - tasks[1].duration * scale).abs() < 1e-4);
        for (token, expected) in [
            ("500ms", 0.5 / 86400.0),
            ("30s", 30.0 / 86400.0),
            ("30m", 30.0 / 1440.0),
            ("2h", 2.0 / 24.0),
            ("1.5d", 1.5),
            ("2w", 14.0),
        ] {
            let source = format!("gantt\nA :a,2026-01-01,{token}\nB :after a,{token}");
            let parsed = mermaid_rs_renderer::parse_mermaid_strict(&source).expect("duration");
            let layout = mermaid_rs_renderer::compute_layout(
                &parsed.graph,
                &mermaid_rs_renderer::Theme::modern(),
                &mermaid_rs_renderer::LayoutConfig::default(),
            );
            let mermaid_rs_renderer::layout::DiagramData::Gantt(gantt) = layout.diagram else {
                panic!("gantt")
            };
            assert!(
                (gantt.tasks[0].duration - expected).abs() < 1e-10,
                "{token}"
            );
            assert!(
                (gantt.tasks[1].start - gantt.tasks[0].start - expected).abs() < 1e-8,
                "{token}"
            );
            assert!(
                (gantt.tasks[0].width / gantt.chart_width - 0.5).abs() < 1e-5,
                "{token}"
            );
        }
    }

    #[test]
    fn gantt_until_uses_earliest_of_all_referenced_starts() {
        let body = "Window :w,2026-01-01,until later earlier\nLater :later,2026-01-05,1d\nEarlier :earlier,2026-01-03,1d\nAfter window :after w,1d";
        let graph = mermaid_rs_renderer::parse_mermaid_strict(&format!("gantt\n{body}"))
            .expect("multiple forward references")
            .graph;
        assert_eq!(graph.gantt_schedule[0].1, graph.gantt_schedule[2].0);
        assert_eq!(graph.gantt_schedule[3].0, graph.gantt_schedule[2].0);
        for refs in ["earlier later", "later earlier earlier"] {
            let reordered = body.replace("later earlier", refs);
            let other = mermaid_rs_renderer::parse_mermaid_strict(&format!("gantt\n{reordered}"))
                .expect("order and duplicates");
            assert_eq!(other.graph.gantt_schedule, graph.gantt_schedule);
        }
        for body in [
            "A :a,2026-01-01,until b missing\nB :b,2026-01-02,1d",
            "A :a,2026-01-01,until b c\nB :b,after a,1d\nC :c,2026-01-03,1d",
            "A :a,2026-01-05,until b c\nB :b,2026-01-02,1d\nC :c,2026-01-03,1d",
        ] {
            assert!(render(Kind::Mermaid, &format!("gantt\n{body}")).is_err());
        }
    }

    #[test]
    fn gantt_milestone_uses_schedule_midpoint_not_trimmed_bar_endpoint() {
        let source =
            "gantt\nexcludes weekends\nM :milestone,m,2026-01-02,1d\nBar :bar,2026-01-02,1d";
        let graph = mermaid_rs_renderer::parse_mermaid_strict(source)
            .expect("calendar milestones")
            .graph;
        let (start, end) = graph.gantt_schedule[0];
        assert!(
            graph.gantt_render_ends[0] < end,
            "fixture needs trailing excluded dates"
        );
        let layout = mermaid_rs_renderer::compute_layout(
            &graph,
            &mermaid_rs_renderer::Theme::modern(),
            &mermaid_rs_renderer::LayoutConfig::default(),
        );
        let mermaid_rs_renderer::layout::DiagramData::Gantt(gantt) = layout.diagram else {
            panic!("gantt")
        };
        let expected = gantt.chart_x
            + (((start + end) * 0.5 - gantt.time_start) / (gantt.time_end - gantt.time_start))
                as f32
                * gantt.chart_width;
        assert!(
            (gantt.tasks[0].x - expected).abs() < 0.01,
            "milestone misplaced: {} != {expected}",
            gantt.tasks[0].x
        );
        let bar_end = gantt.chart_x
            + ((graph.gantt_render_ends[1] - gantt.time_start)
                / (gantt.time_end - gantt.time_start)) as f32
                * gantt.chart_width;
        assert!((gantt.tasks[1].x + gantt.tasks[1].width - bar_end).abs() < 0.01);
    }

    #[test]
    fn gantt_inclusive_dates_update_dependencies_but_not_durations_or_until() {
        let body = "A :a,2026-01-01,2026-01-02\nB :b,after a,1d\nC :c,2026-01-01,until b\nD :d,2026-01-01,1d";
        let plain = mermaid_rs_renderer::parse_mermaid_strict(&format!("gantt\n{body}"))
            .expect("exclusive");
        let inclusive =
            mermaid_rs_renderer::parse_mermaid_strict(&format!("gantt\n{body}\ninclusiveEndDates"))
                .expect("inclusive");
        let old = plain.graph.gantt_schedule;
        let new = inclusive.graph.gantt_schedule;
        assert_eq!(new[0], (old[0].0, old[0].1 + 1.0));
        assert_eq!(new[1], (old[1].0 + 1.0, old[1].1 + 1.0));
        assert_eq!(new[2], (old[2].0, new[1].0));
        assert_eq!(new[3], old[3]);
        let same_day = "gantt\ninclusiveEndDates\nexcludes weekends\nA :a,2026-01-03,2026-01-03";
        let graph = mermaid_rs_renderer::parse_mermaid_strict(same_day)
            .expect("same day")
            .graph;
        assert_eq!(graph.gantt_schedule[0].1 - graph.gantt_schedule[0].0, 1.0);
        assert_eq!(graph.gantt_render_ends[0], graph.gantt_schedule[0].1);
    }

    #[test]
    fn gantt_top_axis_keeps_bottom_ticks_and_title_clear() {
        for title in ["", "title 中文 title\n"] {
            let body =
                format!("gantt\n{title}axisFormat %m/%d\ntickInterval 1day\nTask :a,2026-01-01,2d");
            let top = body.replace("gantt", "gantt\ntopAxis");
            for dark in [false, true] {
                let style = RenderStyle {
                    dark,
                    ..RenderStyle::default()
                };
                let ordinary = render_styled(Kind::Mermaid, &body, style).expect("bottom only");
                let output = render_styled(Kind::Mermaid, &top, style).expect("top and bottom");
                assert!(!ordinary.svg.contains("top-axis-tick"));
                assert_eq!(output.svg.matches("top-axis-tick").count(), 3);
                assert_eq!(output.svg.matches("01/02").count(), 2);
                let graph = mermaid_rs_renderer::parse_mermaid_strict(&top)
                    .expect("top option")
                    .graph;
                let theme = mermaid_rs_renderer::Theme::modern();
                let layout = mermaid_rs_renderer::compute_layout(
                    &graph,
                    &theme,
                    &mermaid_rs_renderer::LayoutConfig::default(),
                );
                let mermaid_rs_renderer::layout::DiagramData::Gantt(gantt) = layout.diagram else {
                    panic!("gantt")
                };
                let y = gantt.top_axis_y.expect("axis position");
                assert!(y + theme.font_size * 0.5 < gantt.chart_y);
                if let Some(title) = gantt.title {
                    assert!(gantt.title_y + title.height * 0.5 + theme.font_size < y);
                } else {
                    assert!(y > theme.font_size);
                }
            }
        }
    }

    #[test]
    fn gantt_consumes_every_statement_and_preserves_keyword_prefixed_labels() {
        for statement in [
            "unknownDirective",
            "weekday",
            "topAxis extra",
            "inclusiveEndDates false",
            "click task call callback()",
            "gantt garbage",
            ":a,2026-01-01,1d",
        ] {
            let source = format!("gantt\nTask :task,2026-01-01,1d\n{statement}");
            assert!(
                render(Kind::Mermaid, &source).is_err(),
                "ignored statement: {statement}"
            );
        }
        for label in [
            "ganttTask",
            "titleTask",
            "sectionTask",
            "dateFormatTask",
            "todayMarkerTask",
            "axisFormatTask",
            "tickIntervalTask",
        ] {
            let source = format!("gantt\n{label} :a,2026-01-01,1d");
            let parsed =
                mermaid_rs_renderer::parse_mermaid_strict(&source).expect("ordinary task label");
            assert_eq!(parsed.graph.gantt_tasks.len(), 1, "lost task: {label}");
            assert_eq!(parsed.graph.gantt_tasks[0].label, label);
        }
        let source = "gantt\ntitle\tTabbed title\nsection\tTabbed section\naxisFormat\t%m/%d\nweekday\tmonday\ntickInterval\t1day\ntodayMarker\toff\nTask :a,2026-01-01,1d";
        assert!(
            render(Kind::Mermaid, source).is_ok(),
            "directive whitespace"
        );
    }

    #[test]
    fn gantt_combined_tags_preserve_milestones_and_are_order_independent() {
        for dark in [false, true] {
            let style = RenderStyle {
                dark,
                ..RenderStyle::default()
            };
            for (first, second) in [
                ("milestone,crit", "crit,milestone"),
                ("done,crit", "crit,done"),
                ("active,crit", "crit,active"),
                ("done,milestone,crit", "crit,milestone,done"),
            ] {
                let source = |tags: &str| format!("gantt\nTask :{tags},t,2026-01-01,1d");
                let a = render_styled(Kind::Mermaid, &source(first), style).expect("combined tags");
                let b =
                    render_styled(Kind::Mermaid, &source(second), style).expect("reordered tags");
                assert_eq!(a.svg, b.svg, "tag order changed display: {first}");
                if first.contains("milestone") {
                    assert!(a.svg.contains("<polygon"), "milestone lost: {first}");
                }
                let without_critical = first
                    .split(',')
                    .filter(|tag| *tag != "crit")
                    .collect::<Vec<_>>()
                    .join(",");
                let task_style = |tags: &str| {
                    let parsed =
                        mermaid_rs_renderer::parse_mermaid_strict(&source(tags)).expect("tags");
                    let theme = if dark {
                        mermaid_rs_renderer::Theme::dark()
                    } else {
                        mermaid_rs_renderer::Theme::modern()
                    };
                    let layout = mermaid_rs_renderer::compute_layout(
                        &parsed.graph,
                        &theme,
                        &mermaid_rs_renderer::LayoutConfig::default(),
                    );
                    let mermaid_rs_renderer::layout::DiagramData::Gantt(gantt) = layout.diagram
                    else {
                        panic!("gantt")
                    };
                    gantt.tasks[0].clone()
                };
                let combined = task_style(first);
                let ordinary = task_style(&without_critical);
                assert_eq!(
                    combined.color, ordinary.color,
                    "critical must preserve completion/active/milestone fill"
                );
                assert_ne!(combined.border_color, ordinary.border_color);
                assert!(combined.border_width > ordinary.border_width);
                assert_eq!(
                    (combined.start, combined.duration),
                    (ordinary.start, ordinary.duration)
                );
            }
        }
    }

    #[test]
    fn gantt_adjacent_critical_milestones_have_separate_painted_bounds() {
        let output = render(Kind::Mermaid, "gantt\ntodayMarker off\nA :milestone,crit,a,2026-01-01,0d\nB :milestone,crit,b,2026-01-01,0d").expect("milestones");
        let bounds = output
            .svg
            .split("<polygon points=\"")
            .skip(1)
            .map(|polygon| {
                let points = polygon.split('"').next().expect("points");
                let ys: Vec<f32> = points
                    .split_whitespace()
                    .map(|point| point.split(',').nth(1).expect("y").parse().expect("number"))
                    .collect();
                let stroke: f32 = polygon
                    .split("stroke-width=\"")
                    .nth(1)
                    .expect("stroke")
                    .split('"')
                    .next()
                    .expect("width")
                    .parse()
                    .expect("number");
                let extension = stroke / std::f32::consts::SQRT_2;
                (
                    ys.iter().copied().fold(f32::INFINITY, f32::min) - extension,
                    ys.iter().copied().fold(f32::NEG_INFINITY, f32::max) + extension,
                )
            })
            .collect::<Vec<_>>();
        assert_eq!(bounds.len(), 2);
        assert!(
            bounds[1].0 - bounds[0].1 >= 1.98,
            "milestone outlines touch: {bounds:?}"
        );
    }

    #[test]
    fn gantt_reference_day_controls_marker_and_implicit_start() {
        let source = "gantt\nTask :a,2026-01-01,4d";
        let base = mermaid_rs_renderer::parse_mermaid_strict(source)
            .expect("chart")
            .graph
            .gantt_schedule[0]
            .0 as i32;
        let parse = |text: &str, day| {
            mermaid_rs_renderer::parse_mermaid_strict_at(text, Some(day)).expect("dated chart")
        };
        let parsed = parse(source, base + 1);
        let theme = mermaid_rs_renderer::Theme::modern();
        let config = mermaid_rs_renderer::LayoutConfig::default();
        let layout = mermaid_rs_renderer::compute_layout(&parsed.graph, &theme, &config);
        let mermaid_rs_renderer::layout::DiagramData::Gantt(gantt) = layout.diagram else {
            panic!("gantt")
        };
        let (x, _) = gantt.today_marker.expect("today marker");
        assert!((x - gantt.chart_x - gantt.chart_width / 4.0).abs() < 0.01);
        for day in [base, base + 1] {
            let relative = parse("gantt\nFirst :1d\nNext :2d", day);
            assert_eq!(
                relative.graph.gantt_schedule,
                [
                    (f64::from(day), f64::from(day + 1)),
                    (f64::from(day + 1), f64::from(day + 3))
                ]
            );
        }
        let styled = source.replace(
            "gantt",
            "gantt\ntodayMarker stroke:#0f0,stroke-width:5px,opacity:0.5",
        );
        let style = RenderStyle {
            reference_day: Some(base + 1),
            ..RenderStyle::default()
        };
        let output = render_styled(Kind::Mermaid, &styled, style).expect("styled marker");
        assert!(output.svg.contains("class=\"today-marker\""));
        assert!(
            output
                .svg
                .contains("stroke=\"#0f0\" stroke-width=\"5\" opacity=\"0.5\"")
        );
        let off = source.replace("gantt", "gantt\ntodayMarker off");
        assert!(
            !render_styled(Kind::Mermaid, &off, style)
                .expect("disabled")
                .svg
                .contains("today-marker")
        );
        assert!(
            !render_styled(
                Kind::Mermaid,
                source,
                RenderStyle {
                    reference_day: Some(base - 1),
                    ..style
                }
            )
            .expect("outside range")
            .svg
            .contains("today-marker")
        );
        for directive in [
            "todayMarker",
            "todayMarker stroke:url(bad)",
            "todayMarker opacity:NaN",
            "todayMarker stroke-width:-1",
            "todayMarker unknown:1",
            "todayMarker opacity:0.5,opacity:1",
        ] {
            assert!(
                render_styled(
                    Kind::Mermaid,
                    &source.replace("gantt", &format!("gantt\n{directive}")),
                    style
                )
                .is_err()
            );
        }
        assert!(
            render_styled(
                Kind::Mermaid,
                source,
                RenderStyle {
                    reference_day: Some(i32::MAX),
                    ..style
                }
            )
            .is_err()
        );
    }

    #[test]
    fn gantt_date_format_controls_tasks_endpoints_and_calendar_dates() {
        let baseline = "gantt\nA :a,2026-02-01,2026-02-05";
        let reference = mermaid_rs_renderer::parse_mermaid_strict(baseline).expect("baseline");
        for (format, start, end) in [
            ("DD-MM-YYYY", "01-02-2026", "05-02-2026"),
            ("YY-MM-DD", "26-02-01", "26-02-05"),
            ("MM/DD/YYYY", "02/01/2026", "02/05/2026"),
            ("YYYYMMDD", "20260201", "20260205"),
            ("YYYY年MM月DD日", "2026年02月01日", "2026年02月05日"),
        ] {
            let source = format!("gantt\ndateFormat {format}\nA :a,{start},{end}");
            let parsed =
                mermaid_rs_renderer::parse_mermaid_strict(&source).expect("declared date format");
            assert_eq!(
                parsed.graph.gantt_schedule, reference.graph.gantt_schedule,
                "{source}"
            );
            assert_eq!(
                parsed.graph.gantt_tasks[0].start.as_deref(),
                Some("2026-02-01")
            );
            assert_eq!(
                parsed.graph.gantt_tasks[0].end.as_deref(),
                Some("2026-02-05")
            );
        }
        let source = "gantt\ndateFormat DD-MM-YYYY\nexcludes 03-01-2026,2026-01-04\nincludes 03-01-2026\nA :a,02-01-2026,2d";
        let parsed = mermaid_rs_renderer::parse_mermaid_strict(source).expect("formatted calendar");
        assert_eq!(
            parsed.graph.gantt_schedule[0].1 - parsed.graph.gantt_schedule[0].0,
            3.0
        );
        for source in [
            "gantt\ndateFormat DD-MM-YYYY\nA :a,2026-01-02,1d",
            "gantt\ndateFormat YYYY-MM-DD\nA :a,2026/01/02,1d",
            "gantt\ndateFormat YYYY-MM-DD\nA :a,2026-1-2,1d",
            "gantt\ndateFormat DD-MM-YYYY\nA :a,30-02-2026,1d",
            "gantt\ndateFormat YY-MM-DD\nA :a,26-13-02,1d",
            "gantt\ndateFormat YYYY-MM-DD HH:mm\nA :a,2026-01-02,1d",
            "gantt\ndateFormat YYYY-YYYY-MM-DD\nA :a,2026-2026-01-02,1d",
            "gantt\ndateFormat\nA :a,2026-01-02,1d",
            "gantt\ndateFormat unsupported\ndateFormat YYYY-MM-DD\nA :a,2026-01-02,1d",
        ] {
            assert!(render(Kind::Mermaid, source).is_err(), "{source}");
        }
    }

    #[test]
    fn gantt_exclusions_extend_schedules_and_preserve_visual_weekend_gaps() {
        assert!(
            render(
                Kind::Mermaid,
                "gantt\nA :a,2026-01-01,999999999999999999999999999999999999d\nB :1M"
            )
            .is_err()
        );
        let body = "A :a,2026-01-02,1d\nB :after a,1d\nC :c,2026-01-02,2d\nManual :m,2026-01-02,2026-01-04";
        let source = format!("gantt\nexcludes weekends\n{body}");
        let parsed = mermaid_rs_renderer::parse_mermaid_strict(&source).expect("weekend calendar");
        let base = parsed.graph.gantt_schedule[0].0;
        assert_eq!(
            parsed
                .graph
                .gantt_schedule
                .iter()
                .map(|(s, e)| (s - base, e - base))
                .collect::<Vec<_>>(),
            [(0.0, 3.0), (3.0, 4.0), (0.0, 4.0), (0.0, 2.0)]
        );
        assert_eq!(
            parsed
                .graph
                .gantt_render_ends
                .iter()
                .map(|e| e - base)
                .collect::<Vec<_>>(),
            [1.0, 4.0, 4.0, 2.0]
        );
        let layout = mermaid_rs_renderer::compute_layout(
            &parsed.graph,
            &mermaid_rs_renderer::Theme::modern(),
            &mermaid_rs_renderer::LayoutConfig::default(),
        );
        let mermaid_rs_renderer::layout::DiagramData::Gantt(gantt) = layout.diagram else {
            panic!("gantt")
        };
        assert_eq!(gantt.excluded_spans.len(), 1);
        assert!((gantt.excluded_spans[0].1 / gantt.chart_width - 0.5).abs() < 1e-5);
        let included = source.replace(
            "excludes weekends",
            "excludes weekends\nincludes 2026-01-03",
        );
        let included =
            mermaid_rs_renderer::parse_mermaid_strict(&included).expect("included Saturday");
        assert_eq!(included.graph.gantt_schedule[0].1 - base, 1.0);
        assert_eq!(included.graph.gantt_schedule[1].0 - base, 1.0);
        let extra = source.replace(
            "excludes weekends",
            "excludes weekends\nexcludes monday,2026-01-06",
        );
        let extra = mermaid_rs_renderer::parse_mermaid_strict(&extra).expect("merged exclusions");
        assert_eq!(extra.graph.gantt_schedule[0].1 - base, 5.0);
        let friday = mermaid_rs_renderer::parse_mermaid_strict(
            "gantt\nweekend friday\nexcludes weekends\nA :a,2026-01-01,1d",
        )
        .expect("Friday weekend");
        assert_eq!(
            friday.graph.gantt_schedule[0].1 - friday.graph.gantt_schedule[0].0,
            3.0
        );
        assert_eq!(
            friday.graph.gantt_render_ends[0] - friday.graph.gantt_schedule[0].0,
            1.0
        );
        for directive in [
            "excludes weekdays",
            "excludes 2026-02-30",
            "includes weekends",
            "weekend monday",
            "excludes",
            "excludes sunday,monday,tuesday,wednesday,thursday,friday,saturday",
        ] {
            assert!(
                render(
                    Kind::Mermaid,
                    &format!("gantt\n{directive}\nA :a,2026-01-02,1d")
                )
                .is_err(),
                "{directive}"
            );
        }
        for dark in [false, true] {
            let output = render_styled(
                Kind::Mermaid,
                &source,
                RenderStyle {
                    dark,
                    ..RenderStyle::default()
                },
            )
            .expect("native exclusions");
            assert!(output.svg.contains("class=\"excluded-day\""));
        }
    }

    #[test]
    fn gantt_end_dates_and_dependency_graph_resolve_before_layout() {
        let source = "gantt\nJoined :Join,after A B,2026-01-10\nA :A,2026-01-01,2d\nB :B,2026-01-02,4d\nWindow :Window,2026-01-01,until Join\nNext :2d";
        let parsed = mermaid_rs_renderer::parse_mermaid_strict(source)
            .expect("forward/multiple dependencies");
        let times = &parsed.graph.gantt_schedule;
        let origin = times[1].0;
        assert_eq!(
            times
                .iter()
                .map(|(start, end)| (start - origin, end - origin))
                .collect::<Vec<_>>(),
            [(5.0, 9.0), (0.0, 2.0), (1.0, 5.0), (0.0, 5.0), (5.0, 7.0)]
        );
        let output = render(Kind::Mermaid, source).expect("resolved chart");
        assert!(output.svg.contains("Joined") && output.svg.contains("Window"));
        for body in [
            "A :a,2026-02-30,1d",
            "A :a,2026-01-01,2026-02-30",
            "A :a,2026-01-03,2026-01-02",
            "A :a,2026-01-01,garbage",
            "A :a,2026-01-01,1..5d",
            "A :a,2026-01-01,-1d",
            "A :a,2026-01-01,1d,extra",
            "A :a,after missing,1d",
            "A :a,2026-01-01,until missing",
            "A :a,after b,1d\nB :b,after a,1d",
            "A :a,2026-01-01,until b\nB :b,after a,1d",
            "A :a,2026-01-01,1d\nB :a,2026-01-02,1d",
        ] {
            assert!(
                render(Kind::Mermaid, &format!("gantt\n{body}")).is_err(),
                "{body}"
            );
        }
    }

    #[test]
    fn gantt_calendar_months_and_years_preserve_month_end() {
        for (start, duration, expected) in [
            ("2026-01-31", "1M", 28.0),
            ("2024-01-31", "1M", 29.0),
            ("2024-02-29", "1y", 365.0),
            ("2023-03-01", "1y", 366.0),
        ] {
            let source = format!("gantt\nTask :a,{start},{duration}");
            let parsed =
                mermaid_rs_renderer::parse_mermaid_strict(&source).expect("calendar duration");
            let layout = mermaid_rs_renderer::compute_layout(
                &parsed.graph,
                &mermaid_rs_renderer::Theme::modern(),
                &mermaid_rs_renderer::LayoutConfig::default(),
            );
            let mermaid_rs_renderer::layout::DiagramData::Gantt(gantt) = layout.diagram else {
                panic!("gantt")
            };
            assert_eq!(gantt.tasks[0].duration, expected, "{source}");
        }
    }

    #[test]
    fn gantt_axis_format_and_interval_change_actual_ticks() {
        let source = "gantt\naxisFormat %m/%d %%\ntickInterval 2day\nTask :a,2026-01-01,6d";
        let parsed = mermaid_rs_renderer::parse_mermaid_strict(source).expect("axis options");
        let layout = mermaid_rs_renderer::compute_layout(
            &parsed.graph,
            &mermaid_rs_renderer::Theme::modern(),
            &mermaid_rs_renderer::LayoutConfig::default(),
        );
        let mermaid_rs_renderer::layout::DiagramData::Gantt(gantt) = &layout.diagram else {
            panic!("gantt")
        };
        assert_eq!(
            gantt
                .ticks
                .iter()
                .map(|t| t.label.as_str())
                .collect::<Vec<_>>(),
            ["01/01 %", "01/03 %", "01/05 %", "01/07 %"]
        );
        let delta = gantt.ticks[1].x - gantt.ticks[0].x;
        assert!((delta * 3.0 - gantt.chart_width).abs() < 0.01);
        let weekly = source.replace("2day", "1week").replace("6d", "14d");
        let output = render(Kind::Mermaid, &weekly).expect("weekly axis");
        assert!(output.svg.contains("01/04 %") && output.svg.contains("01/11 %"));
        let monday = weekly.replace("tickInterval 1week", "tickInterval 1week\nweekday monday");
        let output = render(Kind::Mermaid, &monday).expect("Monday ticks");
        assert!(output.svg.contains("01/05 %") && output.svg.contains("01/12 %"));
        let crossing = source.replace("2026-01-01", "2026-01-30");
        let output = render(Kind::Mermaid, &crossing).expect("cross-month ticks");
        assert!(
            output.svg.contains("01/31 %")
                && output.svg.contains("02/01 %")
                && output.svg.contains("02/03 %")
        );

        for option in [
            "axisFormat %Q",
            "axisFormat %",
            "tickInterval 0day",
            "tickInterval -1day",
            "tickInterval 1fortnight",
        ] {
            assert!(
                render(
                    Kind::Mermaid,
                    &format!("gantt\n{option}\nTask :a,2026-01-01,2d")
                )
                .is_err(),
                "{option}"
            );
        }
        assert!(
            render(
                Kind::Mermaid,
                "gantt\ntickInterval 1day\nTask :a,2026-01-01,3000d"
            )
            .is_err()
        );
        let plain = mermaid_rs_renderer::parse_mermaid_strict("gantt\nTask :a,2026-01-01,1d")
            .expect("short date range");
        let layout = mermaid_rs_renderer::compute_layout(
            &plain.graph,
            &mermaid_rs_renderer::Theme::modern(),
            &mermaid_rs_renderer::LayoutConfig::default(),
        );
        let mermaid_rs_renderer::layout::DiagramData::Gantt(gantt) = layout.diagram else {
            panic!("gantt")
        };
        assert_eq!(
            gantt
                .ticks
                .iter()
                .map(|t| t.label.as_str())
                .collect::<Vec<_>>(),
            ["2026-01-01", "2026-01-02"]
        );
    }

    #[test]
    fn large_finite_pie_weights_keep_real_proportions() {
        let source = "pie showData\n\"A\" : 2e38\n\"B\" : 2e38";
        let parsed = mermaid_rs_renderer::parse_mermaid_strict(source).expect("pie");
        let layout = mermaid_rs_renderer::compute_layout(
            &parsed.graph,
            &mermaid_rs_renderer::Theme::modern(),
            &mermaid_rs_renderer::LayoutConfig::default(),
        );
        let mermaid_rs_renderer::layout::DiagramData::Pie(pie) = &layout.diagram else {
            panic!("pie layout")
        };
        assert_eq!(pie.slices.len(), 2);
        for slice in &pie.slices {
            assert!(((slice.end_angle - slice.start_angle) - std::f32::consts::PI).abs() < 0.0001);
        }
        let vector = render(Kind::Mermaid, source).expect("finite large weights");
        assert_eq!(vector.svg.matches("50%").count(), 2);
        assert!(!vector.svg.contains("inf") && !vector.svg.contains("NaN"));
    }

    #[test]
    fn invalid_pie_entries_do_not_silently_disappear() {
        for source in [
            "pie\n\"A\" : nope",
            "pie\n\"A\" : NaN",
            "pie\n\"A\" : -1",
            "pie\n\"A\" : 10\nunknown ignored",
            "pie\ntitle Empty",
            "pie\nA : 10",
        ] {
            assert!(render(Kind::Mermaid, source).is_err(), "{source}");
        }
        assert!(
            render(
                Kind::Mermaid,
                "pie showData\ntitle Allocation\n\"A:B\" : 10.5\n\"C\" : 20"
            )
            .is_ok()
        );
    }

    #[test]
    fn sequence_numbers_follow_message_time_and_exact_decimal_steps() {
        let source = "sequenceDiagram\nA->>B: plain\nautonumber 0.1 0.2\nA->>B: first\nB->>A: second\nA->>B: third\nautonumber off\nA->>B: unnumbered\nautonumber 10 2\nA->>B: ten\nB->>A: twelve\nautonumber\nA->>B: reset";
        let parsed = mermaid_rs_renderer::parse_mermaid_strict(source).expect("numbering");
        let labels: Vec<_> = parsed
            .graph
            .sequence_message_numbers
            .iter()
            .map(|n| n.map(|n| n.to_string()))
            .collect();
        assert_eq!(
            labels,
            [
                None,
                Some("0.1"),
                Some("0.3"),
                Some("0.5"),
                None,
                Some("10"),
                Some("12"),
                Some("1")
            ]
            .map(|n| n.map(str::to_owned))
        );
        for dark in [false, true] {
            let theme = if dark {
                mermaid_rs_renderer::Theme::dark()
            } else {
                mermaid_rs_renderer::Theme::modern()
            };
            let config = mermaid_rs_renderer::LayoutConfig::default();
            let layout = mermaid_rs_renderer::compute_layout(&parsed.graph, &theme, &config);
            let mermaid_rs_renderer::layout::DiagramData::Sequence(sequence) = &layout.diagram
            else {
                panic!("sequence layout")
            };
            assert_eq!(
                sequence
                    .numbers
                    .iter()
                    .map(|n| n.value.to_string())
                    .collect::<Vec<_>>(),
                ["0.1", "0.3", "0.5", "10", "12", "1"]
            );
            assert!(
                sequence
                    .numbers
                    .iter()
                    .all(|n| n.width >= n.height && n.width.is_finite())
            );
            let vector = render_styled(
                Kind::Mermaid,
                source,
                RenderStyle {
                    dark,
                    ..RenderStyle::default()
                },
            )
            .expect("numbered vector");
            for label in [">0.1<", ">0.3<", ">0.5<", ">10<", ">12<"] {
                assert!(vector.svg.contains(label), "{label}");
            }
        }
    }

    #[test]
    fn invalid_sequence_numbering_reports_diagnostics() {
        for command in [
            "autonumber nope",
            "autonumber 1 2 ignored",
            "autonumber 0.001",
            "autonumber 1 NaN",
            "autonumber -1",
            "autonumber off extra",
            "autonumber 184467440737095517",
        ] {
            assert!(
                render(
                    Kind::Mermaid,
                    &format!("sequenceDiagram\n{command}\nA->>B: message")
                )
                .is_err(),
                "{command}"
            );
        }
        let limit = "sequenceDiagram\nautonumber 184467440737095516.15 0.01\nA->>B: last";
        assert!(render(Kind::Mermaid, limit).is_ok());
        assert!(render(Kind::Mermaid, &format!("{limit}\nA->>B: overflow")).is_err());
    }

    #[test]
    fn sequence_branches_require_their_matching_frame() {
        for source in [
            "sequenceDiagram\nA->>B: hello\nelse missing",
            "sequenceDiagram\nA->>B: hello\nand missing",
            "sequenceDiagram\nA->>B: hello\noption missing",
            "sequenceDiagram\nloop retry\nA->>B: hello\nelse invalid\nend",
            "sequenceDiagram\nalt choice\nA->>B: hello\nand invalid\nend",
            "sequenceDiagram\npar workers\nA->>B: hello\noption invalid\nend",
        ] {
            assert!(render(Kind::Mermaid, source).is_err(), "{source}");
        }
        let source = "sequenceDiagram\nalt choice\nloop retry\nA->>B: send\nend\nelse alternate\nB->>A: respond\nend\npar workers\nA->>B: one\nand second\nB->>A: two\nend\ncritical work\nA->>B: three\noption recovery\nB->>A: four\nend";
        for dark in [false, true] {
            let vector = render_styled(
                Kind::Mermaid,
                source,
                RenderStyle {
                    dark,
                    ..RenderStyle::default()
                },
            )
            .expect("valid nested branches");
            for label in ["alternate", "second", "recovery", "send", "four"] {
                assert!(vector.svg.contains(label), "{label}");
            }
        }
    }

    #[test]
    fn class_members_survive_implicit_nodes_and_opening_line_bodies() {
        for source in [
            "classDiagram\nLedger : +balance\nLedger : +save()",
            "classDiagram\nclass Ledger { +balance\n+save()\n}",
            "classDiagram\nclass Ledger {\n+balance\n+save()\n}",
        ] {
            let parsed = mermaid_rs_renderer::parse_mermaid_strict(source).expect(source);
            let ledger = parsed
                .graph
                .nodes
                .get("Ledger")
                .expect("member owner exists");
            assert!(ledger.label.contains("balance"), "{}", ledger.label);
            assert!(ledger.label.contains("save()"), "{}", ledger.label);
            for dark in [false, true] {
                let vector = render_styled(
                    Kind::Mermaid,
                    source,
                    RenderStyle {
                        dark,
                        ..RenderStyle::default()
                    },
                )
                .expect(source);
                assert!(vector.svg.contains("balance") && vector.svg.contains("save()"));
            }
        }
    }

    #[test]
    fn class_body_suffixes_cannot_silently_disappear() {
        for source in [
            "classDiagram\nclass Ledger { +balance } ignored",
            "classDiagram\nclass Ledger {\n+balance\n} ignored",
        ] {
            let error = render(Kind::Mermaid, source).expect_err(source);
            assert!(error.contains("after class body"), "{error}");
        }
        assert!(
            render(
                Kind::Mermaid,
                "classDiagram\nclass Ledger { +balance }; %% complete"
            )
            .is_ok()
        );
    }

    #[test]
    fn unknown_diagram_statements_are_not_silently_dropped() {
        for (source, marker) in [
            ("flowchart LR\nA-->B\ngarbage ???", "unrecognized flowchart"),
            (
                "sequenceDiagram\nA->>B: Hi\nunknown instruction",
                "unrecognized sequence",
            ),
            ("classDiagram\nclass A\nunknown ???", "unrecognized class"),
        ] {
            let error = render(Kind::Mermaid, source).expect_err(source);
            assert!(error.contains(marker), "{error}");
        }
        assert!(render(Kind::Mermaid, "classDiagram\nclass A {\n+name: String").is_err());
    }

    #[test]
    fn flowchart_statement_boundaries_keep_labels_and_reject_incomplete_nodes() {
        for source in [
            "flowchart LR; A-->B; click A call external()",
            "graph TD\nA-->B; click\tA \"https://example.com\"",
            "flowchart LR\nA[unclosed",
            "flowchart LR\nA(unfinished",
            "flowchart LR\nA{decision",
            "flowchart LR\nA[\"unfinished]",
        ] {
            assert!(render(Kind::Mermaid, source).is_err(), "{source}");
        }
        for source in [
            "flowchart LR\nA[\"text; click A call external()\"]-->B",
            "flowchart LR\nA[\"收益; 成本\"]-->B[(Storage)]",
            "flowchart LR\nA-->B %% comment with [ and \"",
            "flowchart LR\nA>Asymmetric]-->B",
        ] {
            assert!(render(Kind::Mermaid, source).is_ok(), "{source}");
        }
    }

    #[test]
    fn c4_context_description_and_container_technology_use_correct_slots() {
        for name in [
            "Person",
            "Person_Ext",
            "System",
            "SystemDb",
            "SystemQueue",
            "System_Ext",
        ] {
            let source = format!(
                "C4Context\n{name}(a, \"Label\", \"Description\", \"sprite\", \"tags\", \"link\")"
            );
            let graph = mermaid_rs_renderer::parse_mermaid_strict(&source)
                .expect("context")
                .graph;
            let shape = &graph.c4.shapes[0];
            assert_eq!(shape.descr.as_deref(), Some("Description"));
            assert!(shape.type_label.is_none());
            assert!(shape.techn.is_none());
            assert_eq!(shape.sprite.as_deref(), Some("sprite"));
            assert_eq!(shape.tags.as_deref(), Some("tags"));
            assert_eq!(shape.link.as_deref(), Some("link"));
        }
        let graph = mermaid_rs_renderer::parse_mermaid_strict(
            "C4Container\nContainer(a, \"App\", \"Rust\", \"Description\")",
        )
        .expect("container")
        .graph;
        assert_eq!(graph.c4.shapes[0].techn.as_deref(), Some("Rust"));
        assert_eq!(graph.c4.shapes[0].descr.as_deref(), Some("Description"));
    }

    #[test]
    fn sequence_self_return_stays_above_following_content_and_footboxes() {
        for source in [
            "sequenceDiagram\nA->>A: Self",
            "sequenceDiagram\nloop retry\nA->>A: Self\nend\nA->>B: Next",
        ] {
            let graph = mermaid_rs_renderer::parse_mermaid_strict(source)
                .expect("self message")
                .graph;
            let layout = mermaid_rs_renderer::compute_layout(
                &graph,
                &mermaid_rs_renderer::Theme::modern(),
                &mermaid_rs_renderer::LayoutConfig::default(),
            );
            let returned_y = layout.edges[0].points.last().expect("return segment").1;
            let mermaid_rs_renderer::layout::DiagramData::Sequence(sequence) = &layout.diagram
            else {
                panic!("sequence layout")
            };
            assert!(
                sequence
                    .footboxes
                    .iter()
                    .all(|node| node.y > returned_y + 5.0)
            );
            if let Some(next) = layout.edges.get(1) {
                assert!(next.points[0].1 > returned_y);
            }
            if let Some(frame) = sequence.frames.first() {
                assert!(frame.y + frame.height > returned_y);
            }
        }
    }

    #[test]
    fn sequence_message_heads_keep_the_declared_semantics() {
        use mermaid_rs_renderer::ir::{EdgeArrowhead, EdgeDecoration, EdgeStyle};
        for (token, heads, open, cross, dotted) in [
            ("->", 0, false, false, false),
            ("-->", 0, false, false, true),
            ("->>", 1, false, false, false),
            ("-->>", 1, false, false, true),
            ("<<->>", 2, false, false, false),
            ("<<-->>", 2, false, false, true),
            ("-)", 1, true, false, false),
            ("--)", 1, true, false, true),
            ("-x", 0, false, true, false),
            ("--x", 0, false, true, true),
        ] {
            for activation in ["", "+", "-"] {
                // A return suffix requires a live activation on its sender.
                let setup = if activation == "-" {
                    "activate A\n"
                } else {
                    ""
                };
                let source = format!(
                    "sequenceDiagram\nparticipant A\nparticipant B\n{setup}A{token}{activation}B: message ->> literal"
                );
                let graph = mermaid_rs_renderer::parse_mermaid_strict(&source)
                    .expect("sequence syntax")
                    .graph;
                assert_eq!(graph.nodes.len(), 2, "{source}");
                assert_eq!(graph.edges.len(), 1);
                let edge = &graph.edges[0];
                assert_eq!((&*edge.from, &*edge.to), ("A", "B"));
                assert_eq!(
                    usize::from(edge.arrow_start) + usize::from(edge.arrow_end),
                    heads
                );
                assert_eq!(edge.arrow_end_kind == Some(EdgeArrowhead::OpenArrow), open);
                assert_eq!(edge.end_decoration == Some(EdgeDecoration::Cross), cross);
                assert_eq!(edge.style == EdgeStyle::Dotted, dotted);
                assert_eq!(edge.label.as_deref(), Some("message ->> literal"));
                let rendered = render(Kind::Mermaid, &source).expect("native sequence");
                let rendered_heads = rendered.svg.matches("class=\"native-arrowhead\"").count()
                    + rendered.svg.matches("class=\"open-arrowhead\"").count();
                assert_eq!(rendered_heads, heads, "{source}");
                assert!(!rendered.svg.contains("marker-end="));
            }
        }
        assert!(render(Kind::Mermaid, "sequenceDiagram\nA->>>B: invalid").is_err());
    }

    #[test]
    fn native_diagram_heads_do_not_depend_on_svg_markers() {
        for (source, marker, count) in [
            (
                "sequenceDiagram\nA->>B: request\nB-->>A: reply\nA->>A: self",
                "native-arrowhead",
                3,
            ),
            (
                "classDiagram\nBase <|-- Derived\nDerived ..> Service",
                "class-arrowhead",
                2,
            ),
            (
                "architecture-beta\nservice web(server)[Web]\nservice db(database)[DB]\nweb:R <--> L:db",
                "native-arrowhead",
                2,
            ),
            (
                "C4Context\nPerson(user, \"Writer\", \"Person\")\nSystem(app, \"Yu\", \"Editor\")\nBiRel(user, app, \"sync\")",
                "native-arrowhead",
                2,
            ),
            (
                "requirementDiagram\nrequirement a {\nid: 1\ntext: Login\n}\nrequirement b {\nid: 2\ntext: Audit\n}\na - satisfies -> b",
                "requirement-arrowhead",
                1,
            ),
        ] {
            for dark in [false, true] {
                let rendered = render_styled(
                    Kind::Mermaid,
                    source,
                    RenderStyle {
                        dark,
                        ..RenderStyle::default()
                    },
                )
                .expect("native head corpus");
                assert_eq!(
                    rendered.svg.matches(&format!("class=\"{marker}\"")).count(),
                    count,
                    "{source}"
                );
                assert!(!rendered.svg.contains("marker-start="));
                assert!(!rendered.svg.contains("marker-end="));
                assert!(!rendered.svg.contains("<marker"));
            }
        }
    }

    #[test]
    fn er_relationship_gap_reserves_cardinality_and_label_space() {
        for direction in ["TB", "BT", "LR", "RL"] {
            let source =
                format!("erDiagram\ndirection {direction}\nCUSTOMER ||--o{{ ORDER : relationship");
            let graph = mermaid_rs_renderer::parse_mermaid_strict(&source)
                .expect("ER relation")
                .graph;
            let layout = mermaid_rs_renderer::compute_layout(
                &graph,
                &mermaid_rs_renderer::Theme::modern(),
                &mermaid_rs_renderer::LayoutConfig::default(),
            );
            let a = &layout.nodes["CUSTOMER"];
            let b = &layout.nodes["ORDER"];
            let edge = &layout.edges[0];
            let label = edge.label.as_ref().expect("relationship label");
            let (x, y) = edge.label_anchor.expect("positioned label");
            if matches!(direction, "TB" | "BT") {
                let top = if a.y < b.y { a } else { b };
                let bottom = if a.y < b.y { b } else { a };
                assert!(
                    y - label.height / 2.0 > top.y + top.height + 20.0,
                    "{direction} anchor={x},{y} label={label:?} top={top:?} bottom={bottom:?}"
                );
                assert!(y + label.height / 2.0 < bottom.y - 20.0);
            } else {
                let left = if a.x < b.x { a } else { b };
                let right = if a.x < b.x { b } else { a };
                assert!(x - label.width / 2.0 > left.x + left.width + 20.0);
                assert!(x + label.width / 2.0 < right.x - 20.0);
            }
        }
    }

    #[test]
    fn class_relationship_gap_reserves_heads_and_label_space() {
        for direction in ["TB", "BT", "LR", "RL"] {
            let source =
                format!("classDiagram\ndirection {direction}\nCUSTOMER o-- ORDER : relationship");
            let graph = mermaid_rs_renderer::parse_mermaid_strict(&source)
                .expect("class relation")
                .graph;
            let layout = mermaid_rs_renderer::compute_layout(
                &graph,
                &mermaid_rs_renderer::Theme::modern(),
                &mermaid_rs_renderer::LayoutConfig::default(),
            );
            let a = &layout.nodes["CUSTOMER"];
            let b = &layout.nodes["ORDER"];
            let edge = &layout.edges[0];
            let label = edge.label.as_ref().expect("relationship label");
            let (x, y) = edge.label_anchor.expect("positioned label");
            if matches!(direction, "TB" | "BT") {
                let top = if a.y < b.y { a } else { b };
                let bottom = if a.y < b.y { b } else { a };
                assert!(y - label.height / 2.0 > top.y + top.height + 20.0);
                assert!(y + label.height / 2.0 < bottom.y - 20.0);
            } else {
                let left = if a.x < b.x { a } else { b };
                let right = if a.x < b.x { b } else { a };
                assert!(x - label.width / 2.0 > left.x + left.width + 20.0);
                assert!(x + label.width / 2.0 < right.x - 20.0);
            }
        }
    }

    #[test]
    fn er_attributes_keep_semantic_rows_after_wrapping() {
        let source = "erDiagram\nCUSTOMER {\nstring id PK, FK, UK \"closing } remains text with a long comment about 中文 and PK\"\nstring name\n}\nCUSTOMER ||--o{ ORDER : places";
        let graph = mermaid_rs_renderer::parse_mermaid_strict(source)
            .expect("valid ER fixture")
            .graph;
        let config = mermaid_rs_renderer::LayoutConfig {
            max_label_width_chars: 12,
            ..Default::default()
        };
        let theme = mermaid_rs_renderer::Theme::modern();
        let layout = mermaid_rs_renderer::compute_layout(&graph, &theme, &config);
        let node = &layout.nodes["CUSTOMER"];
        let table = node.er_table.as_ref().expect("valid ER fixture");
        assert_eq!(table.rows.len(), 2);
        assert!(table.rows[0].comment.lines.len() > 1);
        assert_eq!(table.rows[0].keys.len(), 3);
        assert_eq!(node.height, table.height);
        assert_eq!(node.width, table.width);
        let svg = mermaid_rs_renderer::render_svg(&layout, &theme, &config);
        assert_eq!(svg.matches("class=\"er-attribute\"").count(), 2);
        for dark in [false, true] {
            let rendered = render_styled(
                Kind::Mermaid,
                source,
                RenderStyle {
                    dark,
                    ..RenderStyle::default()
                },
            )
            .expect("valid ER fixture");
            assert_eq!(rendered.svg.matches("class=\"er-attribute\"").count(), 2);
            assert!(rendered.svg.contains(">UK</text>"));
            assert!(rendered.svg.contains("ORDER"));
        }
        for attribute in [
            "string",
            "string id invalid",
            "string id PK,",
            "string id \"comment\" trailing",
        ] {
            let source = format!("erDiagram\nCUSTOMER {{\n{attribute}\n}}");
            assert!(render(Kind::Mermaid, &source).is_err(), "{source}");
        }
    }

    #[test]
    fn er_aliases_preserve_entity_identity_and_attributes() {
        let source = "erDiagram\nCUSTOMER ||--o{ ORDER : places\nCUSTOMER[\"客户 -- 主体: [一]\"] {\nstring id PK\n}\nORDER[Orders]\nCUSTOMER ||..|{ ORDER : owns";
        let graph = mermaid_rs_renderer::parse_mermaid_strict(source)
            .expect("ER aliases")
            .graph;
        assert_eq!(graph.nodes.len(), 2);
        assert_eq!(graph.edges.len(), 2);
        assert_eq!(
            graph.nodes["CUSTOMER"].label,
            "客户 -- 主体: [一]\n---\nstring id PK"
        );
        assert_eq!(graph.nodes["ORDER"].label, "Orders");
        for edge in &graph.edges {
            assert_eq!(edge.from, "CUSTOMER");
            assert_eq!(edge.to, "ORDER");
        }
        let inline = "erDiagram\n\"A: --\"[\"中文: --\"] ||--o{ B[\"目标 ..\"] : links\n\"A: --\" {\nint id\n}";
        let graph = mermaid_rs_renderer::parse_mermaid_strict(inline)
            .expect("inline aliases")
            .graph;
        assert_eq!(graph.nodes.len(), 2);
        assert_eq!(graph.edges[0].from, "A: --");
        assert_eq!(graph.edges[0].to, "B");
        assert!(graph.nodes["A: --"].label.starts_with("中文: --\n---"));
        for dark in [false, true] {
            let rendered = render_styled(
                Kind::Mermaid,
                source,
                RenderStyle {
                    dark,
                    ..RenderStyle::default()
                },
            )
            .expect("alias vector");
            assert!(rendered.svg.contains("Orders"));
            assert!(rendered.svg.contains("客户"));
            assert_eq!(rendered.svg.matches("class=\"er-attribute\"").count(), 1);
        }
        for invalid in [
            "A[unfinished",
            "A[]",
            "A[Label] trailing",
            "A[one][two]",
            "A[\"bad\" words]",
            "A[Label] { int id } trailing",
        ] {
            assert!(
                render(Kind::Mermaid, &format!("erDiagram\n{invalid}")).is_err(),
                "{invalid}"
            );
        }
    }

    #[test]
    fn er_inline_attributes_keep_keys_comments_and_layout_rows() {
        let attributes = r#"int id PK, FK "中文 } PK, FK remains comment" string name "say \"hello\"" string[] tags int age"#;
        let source = format!("erDiagram\nUSER[Users] {{ {attributes} }}\nUSER ||--o{{ POST : owns");
        let graph = mermaid_rs_renderer::parse_mermaid_strict(&source)
            .expect("inline attributes")
            .graph;
        let theme = mermaid_rs_renderer::Theme::modern();
        let config = mermaid_rs_renderer::LayoutConfig::default();
        let layout = mermaid_rs_renderer::compute_layout(&graph, &theme, &config);
        let table = layout.nodes["USER"]
            .er_table
            .as_ref()
            .expect("attribute table");
        assert_eq!(table.rows.len(), 4);
        assert_eq!(table.rows[0].keys.len(), 2);
        assert_eq!(table.rows[1].keys.len(), 0);
        assert_eq!(
            table.rows[0].comment.lines.join(" "),
            "中文 } PK, FK remains comment"
        );
        assert_eq!(table.rows[1].comment.lines.join(" "), "say \"hello\"");
        assert_eq!(graph.nodes.len(), 2);
        assert!(
            graph.nodes["USER"]
                .label
                .contains("\nstring[] tags\nint age")
        );
        let split = "erDiagram\nUSER {\nint\nid PK,\nFK\nstring name\n}";
        let split_graph = mermaid_rs_renderer::parse_mermaid_strict(split)
            .expect("whitespace-separated attributes")
            .graph;
        let split_layout = mermaid_rs_renderer::compute_layout(&split_graph, &theme, &config);
        assert_eq!(
            split_layout.nodes["USER"]
                .er_table
                .as_ref()
                .expect("split table")
                .rows
                .len(),
            2
        );
        for dark in [false, true] {
            let vector = render_styled(
                Kind::Mermaid,
                &source,
                RenderStyle {
                    dark,
                    ..RenderStyle::default()
                },
            )
            .expect("inline vector");
            assert_eq!(vector.svg.matches("class=\"er-attribute\"").count(), 4);
            assert!(vector.svg.contains("remains"));
            assert!(vector.svg.contains("comment"));
        }
        for attributes in [
            "int id PK, string name",
            "int id string",
            "int id \"unfinished",
            "int id , FK",
            "int id \"comment\" \"extra\"",
        ] {
            assert!(
                render(
                    Kind::Mermaid,
                    &format!("erDiagram\nUSER {{ {attributes} }}")
                )
                .is_err(),
                "{attributes}"
            );
        }
    }

    #[test]
    fn sequence_half_arrows_preserve_direction_style_and_native_heads() {
        use mermaid_rs_renderer::ir::{EdgeArrowhead as Head, EdgeStyle};
        let cases = [
            (r"-|\", Head::HalfTop, false),
            (r"--|\", Head::HalfTop, false),
            ("-|/", Head::HalfBottom, false),
            ("--|/", Head::HalfBottom, false),
            ("/|-", Head::HalfTop, true),
            ("/|--", Head::HalfTop, true),
            (r"\|-", Head::HalfBottom, true),
            (r"\|--", Head::HalfBottom, true),
            (r"-\\", Head::StickTop, false),
            (r"--\\", Head::StickTop, false),
            ("-//", Head::StickBottom, false),
            ("--//", Head::StickBottom, false),
            ("//-", Head::StickTop, true),
            ("//--", Head::StickTop, true),
            (r"\\-", Head::StickBottom, true),
            (r"\\--", Head::StickBottom, true),
        ];
        for (token, head, reverse) in cases {
            for order in [
                "participant A\nparticipant B",
                "participant B\nparticipant A",
            ] {
                let source = format!("sequenceDiagram\n{order}\nA{token}B: 半箭头");
                let graph = mermaid_rs_renderer::parse_mermaid_strict(&source)
                    .expect(token)
                    .graph;
                assert_eq!(graph.nodes.len(), 2, "{token}");
                let edge = &graph.edges[0];
                assert_eq!(
                    (&*edge.from, &*edge.to),
                    if reverse { ("B", "A") } else { ("A", "B") }
                );
                assert_eq!(edge.arrow_end_kind, Some(head));
                assert!(edge.arrow_end && !edge.arrow_start);
                assert_eq!(
                    edge.style,
                    if token.contains("--") {
                        EdgeStyle::Dotted
                    } else {
                        EdgeStyle::Solid
                    }
                );
                for dark in [false, true] {
                    let vector = render_styled(
                        Kind::Mermaid,
                        &source,
                        RenderStyle {
                            dark,
                            ..RenderStyle::default()
                        },
                    )
                    .expect(token);
                    let filled = matches!(head, Head::HalfTop | Head::HalfBottom);
                    let class = if filled {
                        "half-arrowhead"
                    } else {
                        "stick-arrowhead"
                    };
                    assert_eq!(vector.svg.matches(&format!("class=\"{class}\"")).count(), 1);
                    assert!(!vector.svg.contains("class=\"native-arrowhead\""));
                    // Rotating a leftward head must not swap its screen-side half.
                    let leftward = reverse != order.starts_with("participant B");
                    let top = matches!(head, Head::HalfTop | Head::StickTop);
                    let sign = if top != leftward { "-5" } else { "5" };
                    assert!(
                        vector.svg.contains(&format!("M 0 0 L -9 {sign}")),
                        "{token} {order}"
                    );
                }
            }
        }
        for token in [r"-|\>", "-|//", r"-\\\", "--|", "->>()()"] {
            assert!(
                render(
                    Kind::Mermaid,
                    &format!("sequenceDiagram\nA{token}B: invalid")
                )
                .is_err(),
                "{token}"
            );
        }
        let nested = "sequenceDiagram\nautonumber\nA-|/+B: Start\nloop Work\nB--//B: Self\nend\nA/|--B: Reply\ndeactivate B";
        let graph = mermaid_rs_renderer::parse_mermaid_strict(nested)
            .expect("half-arrow activation")
            .graph;
        assert_eq!(graph.sequence_message_numbers.len(), 3);
        assert_eq!(graph.sequence_activations[0].participant, "B");
        assert_eq!(graph.edges[1].from, graph.edges[1].to);
        let vector = render(Kind::Mermaid, nested).expect("loop and self message");
        assert_eq!(vector.svg.matches("class=\"half-arrowhead\"").count(), 2);
        assert_eq!(vector.svg.matches("class=\"stick-arrowhead\"").count(), 1);
    }

    #[test]
    fn sequence_lifecycle_uses_message_geometry_and_rejects_invalid_states() {
        let source = "sequenceDiagram\nparticipant A as 主进程\nA->>A: Prepare\ncreate participant B as 工作进程\nA->>+B: Start\nB->>B: Work\ndestroy B\nB-->>A: Done\nA->>A: Continue";
        let graph = mermaid_rs_renderer::parse_mermaid_strict(source)
            .expect("lifecycle")
            .graph;
        assert_eq!(graph.sequence_created["B"], 1);
        assert_eq!(graph.sequence_destroyed["B"], 3);
        let theme = mermaid_rs_renderer::Theme::modern();
        let config = mermaid_rs_renderer::LayoutConfig::default();
        let layout = mermaid_rs_renderer::compute_layout(&graph, &theme, &config);
        let node = &layout.nodes["B"];
        assert!(node.y > layout.nodes["A"].y + layout.nodes["A"].height);
        assert_eq!(
            layout.edges[1].points.last().expect("creation tip").0,
            node.x
        );
        let mermaid_rs_renderer::layout::DiagramData::Sequence(seq) = &layout.diagram else {
            panic!("sequence data")
        };
        let life = seq
            .lifelines
            .iter()
            .find(|life| life.id == "B")
            .expect("B life");
        assert_eq!(life.y1, node.y + node.height);
        assert_eq!(life.y2, layout.edges[3].points[0].1);
        assert!(life.destroyed && life.y2 > life.y1);
        assert!(!seq.footboxes.iter().any(|node| node.id == "B"));
        assert!(seq.footboxes.iter().any(|node| node.id == "A"));
        for activation in seq
            .activations
            .iter()
            .filter(|item| item.participant == "B")
        {
            assert!(activation.y >= life.y1);
            assert!(activation.y + activation.height <= life.y2 + 0.001);
        }
        for dark in [false, true] {
            let vector = render_styled(
                Kind::Mermaid,
                source,
                RenderStyle {
                    dark,
                    ..RenderStyle::default()
                },
            )
            .expect("native lifecycle");
            assert_eq!(
                vector.svg.matches("class=\"sequence-destruction\"").count(),
                1
            );
            assert!(vector.svg.contains("工作进程"));
        }
        for suffix in [
            "create participant A\nA->>A: duplicate",
            "create participant B\nB->>A: wrong recipient",
            "create participant B",
            "destroy Missing\nA->>Missing: no actor",
            "destroy A",
            "destroy A\nB->>C: unrelated",
            "destroy A\nA->>B: end\nB->>A: stale",
            "destroy A\nA->>B: end\nactivate A",
            "create participant B\ndestroy B\nA->>B: impossible lifetime",
            "create participant B\nnote over A: not a message\nA->>B: delayed",
        ] {
            assert!(
                render(
                    Kind::Mermaid,
                    &format!("sequenceDiagram\nparticipant A\n{suffix}")
                )
                .is_err(),
                "{suffix}"
            );
        }
        let self_end = "sequenceDiagram\nparticipant A\ndestroy A\nA->>A: End";
        let graph = mermaid_rs_renderer::parse_mermaid_strict(self_end)
            .expect("self destruction")
            .graph;
        let layout = mermaid_rs_renderer::compute_layout(&graph, &theme, &config);
        let mermaid_rs_renderer::layout::DiagramData::Sequence(seq) = &layout.diagram else {
            panic!("sequence data")
        };
        assert_eq!(
            seq.lifelines[0].y2,
            layout.edges[0].points.last().expect("self end").1
        );
    }

    #[test]
    fn sequence_actors_keep_human_shape_through_creation_and_footboxes() {
        let source = "sequenceDiagram\nactor A as 中文使用者\nparticipant B as Server\nA->>B: Request\ncreate actor C as 临时工作者\nB->>C: Create\ndestroy C\nC-->>B: Finish\nB-->>A: Reply";
        let graph = mermaid_rs_renderer::parse_mermaid_strict(source)
            .expect("actors")
            .graph;
        assert_eq!(
            graph.nodes["A"].shape,
            mermaid_rs_renderer::ir::NodeShape::Actor
        );
        assert_eq!(
            graph.nodes["C"].shape,
            mermaid_rs_renderer::ir::NodeShape::Actor
        );
        assert_eq!(
            graph.nodes["B"].shape,
            mermaid_rs_renderer::ir::NodeShape::ActorBox
        );
        let layout = mermaid_rs_renderer::compute_layout(
            &graph,
            &mermaid_rs_renderer::Theme::modern(),
            &mermaid_rs_renderer::LayoutConfig::default(),
        );
        let mermaid_rs_renderer::layout::DiagramData::Sequence(seq) = &layout.diagram else {
            panic!("sequence")
        };
        let actor = &layout.nodes["C"];
        assert!(actor.height > actor.label.height + 50.0);
        assert_eq!(
            layout.edges[1].points.last().expect("creation tip").0,
            actor.x + actor.width / 2.0
        );
        assert!(
            seq.footboxes
                .iter()
                .any(|node| node.id == "A"
                    && node.shape == mermaid_rs_renderer::ir::NodeShape::Actor)
        );
        assert!(!seq.footboxes.iter().any(|node| node.id == "C"));
        for dark in [false, true] {
            let rendered = render_styled(
                Kind::Mermaid,
                source,
                RenderStyle {
                    dark,
                    ..RenderStyle::default()
                },
            )
            .expect("native actors");
            assert_eq!(rendered.svg.matches("class=\"sequence-actor\"").count(), 3);
            assert_eq!(
                rendered
                    .svg
                    .matches("class=\"sequence-destruction\"")
                    .count(),
                1
            );
            assert!(rendered.svg.contains("中文使用者"));
            assert!(rendered.svg.contains("临时工作者"));
        }
    }

    #[test]
    fn sequence_inline_return_deactivates_sender_and_preserves_nested_calls() {
        use mermaid_rs_renderer::ir::SequenceActivationKind::{Activate, Deactivate};
        let source = "sequenceDiagram\nparticipant A\nparticipant B\nparticipant C\nA->>+B: Call B\nB->>+C: Call C\nC-->>-B: Return C\nB-->>-A: Return B\nA->>C: Later";
        let graph = mermaid_rs_renderer::parse_mermaid_strict(source)
            .expect("nested calls")
            .graph;
        let actual: Vec<_> = graph
            .sequence_activations
            .iter()
            .map(|event| (event.participant.as_str(), event.index, event.kind))
            .collect();
        assert_eq!(
            actual,
            vec![
                ("B", 0, Activate),
                ("C", 1, Activate),
                ("C", 2, Deactivate),
                ("B", 3, Deactivate)
            ]
        );
        let layout = mermaid_rs_renderer::compute_layout(
            &graph,
            &mermaid_rs_renderer::Theme::modern(),
            &mermaid_rs_renderer::LayoutConfig::default(),
        );
        let mermaid_rs_renderer::layout::DiagramData::Sequence(seq) = &layout.diagram else {
            panic!("sequence")
        };
        assert_eq!(seq.activations.len(), 2);
        for (id, begin, end) in [("B", 0, 3), ("C", 1, 2)] {
            let activation = seq
                .activations
                .iter()
                .find(|activation| activation.participant == id)
                .expect(id);
            assert_eq!(activation.y, layout.edges[begin].points[0].1);
            assert!(
                (activation.y + activation.height - layout.edges[end].points[0].1).abs() < 0.001
            );
            assert!(activation.y + activation.height < layout.edges[4].points[0].1);
        }
        for dark in [false, true] {
            render_styled(
                Kind::Mermaid,
                source,
                RenderStyle {
                    dark,
                    ..RenderStyle::default()
                },
            )
            .expect("nested call vector");
        }
    }

    #[test]
    fn state_transition_heads_are_explicit_native_paths_in_both_directions() {
        for (transition, count) in [("A --> B", 1), ("A <-- B", 1), ("A <--> B", 2)] {
            for direction in ["TB", "LR", "RL", "BT"] {
                let source = format!("stateDiagram-v2\ndirection {direction}\n{transition}");
                let rendered = render(Kind::Mermaid, &source).expect("state direction");
                assert_eq!(
                    rendered.svg.matches("class=\"state-arrowhead\"").count(),
                    count,
                    "{source}"
                );
                assert!(!rendered.svg.contains("marker-end="));
                assert!(!rendered.svg.contains("marker-start="));
                assert!(rendered.svg.contains("edge-0"));
            }
        }
    }

    #[test]
    fn state_notes_keep_multiline_literal_text_and_er_comments_keep_braces() {
        let source = "stateDiagram-v2\nIdle\nstate Running {\nA --> B\n}\nIdle --> Running\nnote right of Idle\nFirst 中文\n\n--> literal arrow\n%%{init: not a directive}%%\n} literal brace\nend note";
        let graph = mermaid_rs_renderer::parse_mermaid_strict(source)
            .expect("state with note")
            .graph;
        assert_eq!(graph.edges.len(), 2);
        assert_eq!(graph.state_notes.len(), 1);
        assert_eq!(graph.state_notes[0].target, "Idle");
        assert_eq!(
            graph.state_notes[0].label,
            "First 中文\n\n--> literal arrow\n%%{init: not a directive}%%\n} literal brace"
        );
        assert!(graph.nodes.contains_key("Idle"));
        let er =
            "erDiagram\nCUSTOMER {\nstring id \"closing } remains text\"\n}\n\"Quoted { entity\"";
        let graph = mermaid_rs_renderer::parse_mermaid_strict(er)
            .expect("ER quoted braces")
            .graph;
        assert_eq!(graph.nodes.len(), 2);
        assert!(
            graph.nodes["CUSTOMER"]
                .label
                .contains("closing } remains text")
        );
        assert!(graph.nodes.contains_key("Quoted { entity"));
        for dark in [false, true] {
            let note = render_styled(
                Kind::Mermaid,
                source,
                RenderStyle {
                    dark,
                    ..RenderStyle::default()
                },
            )
            .expect("note render");
            for text in [
                "First 中文",
                "literal arrow",
                "literal brace",
                "not a directive",
            ] {
                assert!(note.svg.contains(text), "missing {text}");
            }
            assert!(
                render_styled(
                    Kind::Mermaid,
                    er,
                    RenderStyle {
                        dark,
                        ..RenderStyle::default()
                    }
                )
                .is_ok()
            );
        }
        assert!(
            render(
                Kind::Mermaid,
                "stateDiagram-v2\nA --> B\nnote right of A\nUnclosed"
            )
            .is_err()
        );
    }

    #[test]
    fn state_and_er_incomplete_structures_are_diagnostics() {
        for source in [
            "stateDiagram-v2\nstate Outer {\nA --> B",
            "stateDiagram-v2\nA --> B\n}",
            "stateDiagram-v2\nA --> B\nunknown ???",
            "stateDiagram-v2\nA --> B\n--",
            "erDiagram\nCUSTOMER {\nstring id",
            "erDiagram\nCUSTOMER { string id } ignored ???",
            "erDiagram\nCUSTOMER {\nstring id\n} ignored ???",
            "erDiagram\nCUSTOMER\n}",
            "erDiagram\nunknown ???",
        ] {
            assert!(
                render(Kind::Mermaid, source).is_err(),
                "malformed structure accepted: {source}"
            );
        }
    }

    #[test]
    fn gitgraph_parallel_layout_and_native_api_consume_source_options() {
        let body = "gitGraph\ncommit id:\"base\"\nbranch feature\ncommit id:\"feature-tip\"\ncheckout main\ncommit id:\"root-tip\"";
        let source = format!(
            "%%{{init: {{\"gitGraph\": {{\"parallelCommits\":true,\"rotateCommitLabel\":false}}}}}}%%\n{body}"
        );
        let parsed = mermaid_rs_renderer::parse_mermaid_strict(&source).expect("parallel config");
        let mut options = mermaid_rs_renderer::RenderOptions::default();
        parsed
            .apply_source_layout(&mut options.layout)
            .expect("resolved layout");
        let layout =
            mermaid_rs_renderer::compute_layout(&parsed.graph, &options.theme, &options.layout);
        let mermaid_rs_renderer::layout::DiagramData::GitGraph(layout) = layout.diagram else {
            panic!("git layout");
        };
        let feature = layout
            .commits
            .iter()
            .find(|commit| commit.id == "feature-tip")
            .expect("feature");
        let root = layout
            .commits
            .iter()
            .find(|commit| commit.id == "root-tip")
            .expect("root");
        assert_eq!(
            feature.x, root.x,
            "independent children must share the same time column"
        );
        assert_ne!(feature.y, root.y);
        assert!(layout.commits.iter().all(|commit| {
            commit
                .label
                .as_ref()
                .is_none_or(|label| label.transform.is_none())
        }));
        let base = render(Kind::Mermaid, body).expect("sequential");
        let parallel = render(Kind::Mermaid, &source).expect("parallel helper");
        assert!(parallel.width < base.width);
        let source = format!(
            "%%{{init: {{\"gitGraph\": {{\"showBranches\":false,\"showCommitLabel\":false}}}}}}%%\n{body}"
        );
        for svg in [
            mermaid_rs_renderer::render_with_options(
                &source,
                mermaid_rs_renderer::RenderOptions::default(),
            )
            .expect("public render"),
            mermaid_rs_renderer::render_strict(
                &source,
                mermaid_rs_renderer::RenderOptions::default(),
            )
            .expect("strict render"),
            mermaid_rs_renderer::render_with_detailed_timing(
                &source,
                mermaid_rs_renderer::RenderOptions::default(),
            )
            .expect("timed render")
            .svg,
        ] {
            assert!(!svg.contains(">main</text>"));
            assert!(!svg.contains(">root-tip</text>"));
        }
        let base_size =
            mermaid_rs_renderer::measure(body, mermaid_rs_renderer::RenderOptions::default())
                .expect("baseline size");
        let hidden_size =
            mermaid_rs_renderer::measure(&source, mermaid_rs_renderer::RenderOptions::default())
                .expect("hidden size");
        assert!(hidden_size.width < base_size.width);
    }

    #[test]
    fn gitgraph_source_configuration_is_atomic_and_diagnoses_unknown_fields() {
        let mut config = mermaid_rs_renderer::config::GitGraphConfig {
            commit_step: 52.0,
            main_branch_order: 7.0,
            ..Default::default()
        };
        config.apply_source_options(&serde_json::json!({"showBranches":false,"diagramPadding":12.0,"useMaxWidth":false})).expect("partial override");
        assert!(!config.show_branches);
        assert!(!config.use_max_width);
        assert_eq!(config.diagram_padding, 12.0);
        assert_eq!(config.commit_step, 52.0);
        assert_eq!(config.main_branch_order, 7.0);
        let error = config
            .apply_source_options(&serde_json::json!({"showBranches":true,"unknownSetting":true}))
            .expect_err("unknown option");
        assert!(error.to_string().contains("unknownSetting"));
        assert!(
            !config.show_branches,
            "invalid override must not partially mutate the existing options"
        );
        for value in [
            serde_json::json!({"diagramPadding":-1}),
            serde_json::json!({"diagramPadding":1e100}),
            serde_json::json!({"parallelCommits":null}),
        ] {
            assert!(config.apply_source_options(&value).is_err());
        }
        let error = render(
            Kind::Mermaid,
            "%%{init: {gitGraph: {unknownSetting: true}}}%%\ngitGraph\ncommit",
        )
        .expect_err("unsupported source option");
        assert!(error.contains("unknownSetting"), "{error}");
    }

    #[test]
    fn gitgraph_display_configuration_changes_native_output() {
        let body = "gitGraph\ncommit id:\"base\"\nbranch feature\ncommit id:\"feature-tip\"\ncheckout main\ncommit id:\"root-tip\"";
        let source = format!(
            "%%{{init: {{\"gitGraph\": {{\"showBranches\":false,\"showCommitLabel\":false}}}}}}%%\n{body}"
        );
        let base = render(Kind::Mermaid, body).expect("baseline");
        let hidden = render(Kind::Mermaid, &source).expect("hidden decorations");
        assert!(base.svg.contains(">main</text>"));
        assert!(base.svg.contains(">root-tip</text>"));
        assert!(!hidden.svg.contains(">main</text>"));
        assert!(!hidden.svg.contains(">feature</text>"));
        assert!(!hidden.svg.contains(">root-tip</text>"));
        assert!(hidden.svg.contains("commit-bullets"));
        assert!(
            hidden.width < base.width,
            "hidden labels must not reserve the old left margin"
        );
        for value in ["null", "42", "\"false\""] {
            let source =
                format!("%%{{init: {{\"gitGraph\": {{\"showBranches\":{value}}}}}}}%%\n{body}");
            let error = render(Kind::Mermaid, &source).expect_err("invalid boolean");
            assert!(
                error.contains("gitGraph.showBranches must be a boolean"),
                "{error}"
            );
        }
    }

    #[test]
    fn yaml_configuration_and_directives_share_identity_and_ordered_overrides() {
        let source = "---\nconfig:\n  gitGraph:\n    mainBranchName: Root中文\n    showBranches: false\n    showCommitLabel: false\n---\n%%{init: {gitGraph:{showBranches:true}}}%%\ngitGraph\ncommit id:base\nbranch feature\ncommit id:work\ncheckout Root中文\nmerge feature";
        let parsed = mermaid_rs_renderer::parse_mermaid_strict(source).expect("YAML config");
        assert_eq!(parsed.graph.gitgraph.main_branch, "Root中文");
        assert_eq!(parsed.graph.gitgraph.commits[2].parents, ["base", "work"]);
        let mut config = mermaid_rs_renderer::config::LayoutConfig::default();
        parsed
            .apply_source_layout(&mut config)
            .expect("shared config");
        assert!(config.gitgraph.show_branches);
        assert!(!config.gitgraph.show_commit_label);
        let output = render(Kind::Mermaid, source).expect("vector");
        assert!(output.svg.contains("Root中文"));
        assert!(!output.svg.contains(">base</text>"));
    }

    #[test]
    fn yaml_titles_are_visible_measured_and_literal_across_diagram_families() {
        for body in [
            "flowchart LR\nA-->B",
            "sequenceDiagram\nA->>B: Hello",
            "classDiagram\nclass A",
            "stateDiagram-v2\nA-->B",
            "erDiagram\nA ||--o{ B : owns",
            "gantt\ndateFormat YYYY-MM-DD\nsection Work\nTask :a,2026-01-01,1d",
            "pie\n\"A\":10",
            "gitGraph\ncommit",
            "C4Context\nPerson(a,\"Writer\")",
            "mindmap\n  Root\n    Child",
        ] {
            let plain = render(Kind::Mermaid, body).expect("plain");
            let source = format!("---\ntitle: |\n  中文标题 & Example\n  second line\n---\n{body}");
            let titled = render(Kind::Mermaid, &source).expect("YAML title");
            assert!(titled.svg.contains("中文标题 &amp; Example"), "{body}");
            assert!(titled.svg.contains("second line"), "{body}");
            assert!(
                titled.height > plain.height,
                "title height must be included: {body}"
            );
            assert!(
                titled.width >= plain.width,
                "title must not clip the diagram"
            );
        }
        let source = "---\ntitle: |\n  ---\n  %%{init: invalid}%%\n  <script>literal</script>\n---\nflowchart LR\nA-->B";
        let output = render(Kind::Mermaid, source).expect("title is not executable diagram syntax");
        assert!(output.svg.contains("&lt;script&gt;literal&lt;/script&gt;"));
        assert!(output.svg.contains("%%{init: invalid}%%"));
    }

    #[test]
    fn yaml_invalid_or_unconsumed_metadata_returns_diagnostics() {
        for yaml in [
            "null",
            "[]",
            "config: null",
            "config: []",
            "title: 42",
            "title: [a,b]",
            "unknown: true",
            "config:\n  theme: dark",
            "config:\n  gitGraph:\n    unknown: true",
            "config: [unclosed",
            "title: one\ntitle: two",
            "displayMode: unexpected",
        ] {
            let source = format!("---\n{yaml}\n---\ngitGraph\ncommit");
            assert!(
                render(Kind::Mermaid, &source).is_err(),
                "ignored YAML: {yaml}"
            );
        }
        assert!(render(Kind::Mermaid, "---\ntitle: unfinished\ngitGraph\ncommit").is_err());
        let gantt = "---\ndisplayMode: compact\ntitle: 'Work: 中文'\n---\ngantt\nsection Work\nTask :a,2026-01-01,1d";
        let parsed =
            mermaid_rs_renderer::parse_mermaid_strict(gantt).expect("legacy Gantt metadata");
        assert_eq!(parsed.graph.gantt_display_mode.as_deref(), Some("compact"));
        assert_eq!(parsed.graph.gantt_title.as_deref(), Some("Work: 中文"));
    }

    #[test]
    fn native_init_configuration_never_silently_ignores_keys_or_earlier_directives() {
        for (configuration, body) in [
            ("null", "flowchart LR\nA-->B"),
            ("[]", "flowchart LR\nA-->B"),
            ("{theme:'dark'}", "flowchart LR\nA-->B"),
            ("{flowchart:{nodeSpacing:90}}", "flowchart LR\nA-->B"),
            ("{unknownOption:42}", "gitGraph\ncommit"),
            ("{gitGraph:{showBranches:false}}", "flowchart LR\nA-->B"),
            ("{gitGraph:{unknownOption:42}}", "gitGraph\ncommit"),
        ] {
            for suffix in ["", "%%{init: {}}%%\n"] {
                let source = format!("%%{{init: {configuration}}}%%\n{suffix}{body}");
                assert!(
                    render(Kind::Mermaid, &source).is_err(),
                    "unconsumed config: {source}"
                );
            }
        }
    }

    #[test]
    fn multiple_supported_init_directives_merge_before_parsing_and_layout() {
        let source = "%%{init: {gitGraph:{mainBranchName:'Root',showBranches:false}}}%%\n%%{init: {gitGraph:{showCommitLabel:false}}}%%\ngitGraph\ncommit id:\"base\"\nbranch feature\ncommit\ncheckout Root\nmerge feature";
        let parsed =
            mermaid_rs_renderer::parse_mermaid_strict(source).expect("merged configuration");
        assert_eq!(parsed.graph.gitgraph.main_branch, "Root");
        assert_eq!(parsed.graph.gitgraph.branches.len(), 2);
        assert_eq!(parsed.graph.gitgraph.commits[2].parents.len(), 2);
        let mut layout = mermaid_rs_renderer::config::LayoutConfig::default();
        parsed
            .apply_source_layout(&mut layout)
            .expect("apply merged options");
        assert!(!layout.gitgraph.show_branches);
        assert!(!layout.gitgraph.show_commit_label);
        let output = render(Kind::Mermaid, source).expect("native vector");
        assert!(!output.svg.contains(">Root</text>"));
        assert!(!output.svg.contains(">base</text>"));
    }

    #[test]
    fn gitgraph_attributes_are_consumed_outside_quoted_values() {
        let source = r#"gitGraph
commit id:"base"
branch "tag:fake id:fake"
commit id:"work\"中文" tag:"release id:literal tag:literal"
checkout main
merge "tag:fake id:fake" id:"merge-tip" tag:"v1\"正式" type:REVERSE"#;
        let graph = mermaid_rs_renderer::parse_mermaid_strict(source)
            .expect("quoted attributes")
            .graph;
        let work = &graph.gitgraph.commits[1];
        assert_eq!(work.id, "work\"中文");
        assert_eq!(work.tags, ["release id:literal tag:literal"]);
        let merge = &graph.gitgraph.commits[2];
        assert_eq!(merge.id, "merge-tip");
        assert_eq!(merge.tags, ["v1\"正式"]);
        assert_eq!(merge.parents, ["base", "work\"中文"]);
        assert_eq!(
            merge.custom_type,
            Some(mermaid_rs_renderer::ir::GitGraphCommitType::Reverse)
        );
        render(Kind::Mermaid, source).expect("native vector");
    }

    #[test]
    fn gitgraph_invalid_history_and_unconsumed_attributes_are_diagnostics() {
        let prefix =
            "gitGraph\ncommit id:\"base\"\nbranch feature\ncommit id:\"work\"\ncheckout main\n";
        for command in [
            "checkout missing",
            "switch missing",
            "branch feature",
            "branch main",
            "merge missing",
            "merge main",
            "merge feature mystery:1",
            "merge feature type:TYPO",
            "merge feature id:\"base\"",
            "commit id:\"base\"",
            "commit type:TYPO",
            "commit mystery:1",
            "commit id:\"unfinished",
            "commit tag:\"unfinished",
            "commit id:\"one\" id:\"two\"",
            "commit type:NORMAL type:REVERSE",
            "commit id:",
            "commit id:\"\"",
            "commit tag:\"v1\"suffix",
        ] {
            let source = format!("{prefix}{command}");
            assert!(
                render(Kind::Mermaid, &source).is_err(),
                "accepted invalid history: {command}"
            );
        }
    }

    #[test]
    fn gitgraph_header_direction_and_whitespace_are_consumed() {
        for (header, direction) in [
            (
                "gitGraph LR:",
                mermaid_rs_renderer::ir::Direction::LeftRight,
            ),
            ("gitGraph TB:", mermaid_rs_renderer::ir::Direction::TopDown),
            (
                "gitGraph BT:",
                mermaid_rs_renderer::ir::Direction::BottomTop,
            ),
        ] {
            let source = format!(
                "{header}\ncommit\tid:\"base\"\nbranch\tfeature\ncommit\tid:\"work\"\ncheckout\tmain\nmerge\tfeature"
            );
            let graph = mermaid_rs_renderer::parse_mermaid_strict(&source)
                .expect("header and tabs")
                .graph;
            assert_eq!(graph.direction, direction);
            assert_eq!(graph.gitgraph.commits.len(), 3);
            assert_eq!(graph.gitgraph.commits[2].parents, ["base", "work"]);
        }
        for source in [
            "gitGraph sideways\ncommit",
            "gitGraph\ncommit msg:\"ignored\"",
            "gitGraph\nbranch feature\nmerge main",
        ] {
            assert!(render(Kind::Mermaid, source).is_err(), "{source}");
        }
    }

    #[test]
    fn gitgraph_branch_attributes_do_not_become_branch_names() {
        let graph = mermaid_rs_renderer::parse_mermaid_strict(
            r#"gitGraph
commit id:"base"
branch "Feature 中文" order: 2
commit id:"work"
checkout main
merge "Feature 中文" id:"merge-tip" tag:"v1""#,
        )
        .expect("branch attributes")
        .graph;
        assert_eq!(graph.gitgraph.branches.len(), 2);
        assert_eq!(graph.gitgraph.branches[1].name, "Feature 中文");
        assert_eq!(graph.gitgraph.branches[1].order, Some(2.0));
        assert_eq!(graph.gitgraph.commits[2].parents, vec!["base", "work"]);
        assert_eq!(graph.gitgraph.commits[2].id, "merge-tip");
        assert_eq!(graph.gitgraph.commits[2].tags, vec!["v1"]);
        for command in [
            "branch feature order: nope",
            "branch feature order: 1e100",
            "branch feature mystery: 2",
            "branch \"unclosed",
            "checkout main extra",
            "branch \"x\"order:2",
        ] {
            let source = format!("gitGraph\ncommit\n{command}");
            assert!(
                render(Kind::Mermaid, &source).is_err(),
                "unconsumed suffix: {source}"
            );
        }
    }

    #[test]
    fn configured_and_command_branch_names_decode_the_same_escapes() {
        let source = r#"%%{init: {"gitGraph": {"mainBranchName": "Root\"中文"}}}%%
gitGraph
commit id:"base"
branch "Feature\nLine"
commit id:"work"
checkout "Root\"中文"
merge "Feature\nLine""#;
        let graph = mermaid_rs_renderer::parse_mermaid_strict(source)
            .expect("escaped identities")
            .graph;
        assert_eq!(graph.gitgraph.branches.len(), 2);
        assert_eq!(graph.gitgraph.main_branch, "Root\"中文");
        assert_eq!(graph.gitgraph.branches[1].name, "Feature\nLine");
        assert_eq!(graph.gitgraph.commits[2].parents, vec!["base", "work"]);
    }

    #[test]
    fn gitgraph_configured_root_is_used_by_commits_checkout_merge_and_order() {
        let source = r#"%%{init: {"gitGraph": {"mainBranchName": "Root中文", "mainBranchOrder": 2}}}%%
gitGraph
commit id:"base"
branch feature
commit id:"feature-tip"
checkout "Root中文"
commit id:"root-tip"
merge feature id:"merged""#;
        let graph = mermaid_rs_renderer::parse_mermaid_strict(source)
            .expect("configured root")
            .graph;
        assert_eq!(graph.gitgraph.main_branch, "Root中文");
        assert_eq!(graph.gitgraph.branches.len(), 2);
        assert_eq!(graph.gitgraph.branches[0].name, "Root中文");
        assert_eq!(graph.gitgraph.branches[0].order, Some(2.0));
        assert_eq!(graph.gitgraph.commits.len(), 4);
        assert_eq!(
            graph.gitgraph.commits[3].parents,
            vec!["root-tip", "feature-tip"]
        );
        let options = mermaid_rs_renderer::RenderOptions::default();
        let layout = mermaid_rs_renderer::compute_layout(&graph, &options.theme, &options.layout);
        let mermaid_rs_renderer::layout::DiagramData::GitGraph(layout) = layout.diagram else {
            panic!("git layout");
        };
        assert_eq!(
            layout
                .branches
                .iter()
                .map(|branch| branch.name.as_str())
                .collect::<Vec<_>>(),
            vec!["feature", "Root中文"]
        );
        for dark in [false, true] {
            let rendered = render_styled(
                Kind::Mermaid,
                source,
                RenderStyle {
                    dark,
                    ..RenderStyle::default()
                },
            )
            .expect("configured root render");
            assert!(rendered.svg.contains(">Root中文</text>"));
            assert!(!rendered.svg.contains(">main</text>"));
        }
        for config in [
            r#"{"mainBranchName":false}"#,
            r#"{"mainBranchName":"  "}"#,
            r#"{"mainBranchOrder":"second"}"#,
            r#"{"mainBranchOrder":1e100}"#,
            "false",
        ] {
            let source = format!("%%{{init: {{\"gitGraph\": {config}}}}}%%\ngitGraph\ncommit");
            assert!(
                render(Kind::Mermaid, &source).is_err(),
                "invalid config accepted: {source}"
            );
        }
    }

    #[test]
    fn quoted_gitgraph_branch_identity_and_multiline_background_are_consistent() {
        let graph = mermaid_rs_renderer::parse_mermaid_strict(
            r#"gitGraph
commit
branch "First\nSecond"
commit
checkout main
merge "First\nSecond""#,
        )
        .expect("git branches")
        .graph;
        assert_eq!(graph.gitgraph.branches.len(), 2);
        assert_eq!(graph.gitgraph.branches[1].name, "First\nSecond");
        assert_eq!(graph.gitgraph.commits.len(), 3);
        assert_eq!(graph.gitgraph.commits[2].parents.len(), 2);
        let options = mermaid_rs_renderer::RenderOptions::default();
        let layout = mermaid_rs_renderer::compute_layout(&graph, &options.theme, &options.layout);
        let mermaid_rs_renderer::layout::DiagramData::GitGraph(layout) = layout.diagram else {
            panic!("git layout");
        };
        for branch in &layout.branches {
            let label = &branch.label;
            assert!(
                label.text_y >= label.bg_y,
                "text above background: {label:?}"
            );
            assert!(
                label.text_y + label.text_height <= label.bg_y + label.bg_height + 0.01,
                "text below background: {label:?}"
            );
        }
    }

    #[test]
    fn specialized_diagram_labels_use_independent_native_text_lines() {
        for (source, labels) in [
            (
                "sankey-beta\nSource,Target,10",
                vec!["Source", "Target", "10"],
            ),
            (
                "C4Context\nPerson(user, \"Writer\", \"First<br/>Second\")",
                vec!["Writer", "First", "Second"],
            ),
            (
                r#"gitGraph
commit
branch "First\nSecond"
checkout "First\nSecond"
commit"#,
                vec!["First", "Second"],
            ),
        ] {
            for dark in [false, true] {
                let rendered = render_styled(
                    Kind::Mermaid,
                    source,
                    RenderStyle {
                        dark,
                        ..RenderStyle::default()
                    },
                )
                .expect("specialized diagram");
                assert!(
                    !rendered.svg.contains("<tspan"),
                    "positioned spans in {source}"
                );
                for label in &labels {
                    assert!(
                        rendered.svg.contains(&format!(">{label}</text>")),
                        "missing {label} in {}",
                        rendered.svg
                    );
                }
            }
        }
    }

    #[test]
    fn quoted_multiline_flowchart_labels_preserve_text_and_structure() {
        let source = "flowchart TD\nsubgraph S[\"Group\n中文\"]\nA[\"First\nend\n%% literal\nclick A literal\"] -->|\"Yes\n继续\"| B[Done]\nend";
        let graph = mermaid_rs_renderer::parse_mermaid_strict(source)
            .expect("multiline labels")
            .graph;
        assert_eq!(graph.nodes.len(), 2);
        assert_eq!(graph.edges.len(), 1);
        assert_eq!(graph.subgraphs.len(), 1);
        for dark in [false, true] {
            let rendered = render_styled(
                Kind::Mermaid,
                source,
                RenderStyle {
                    dark,
                    ..RenderStyle::default()
                },
            )
            .expect("multiline rendering");
            for label in [
                "Group",
                "中文",
                "First",
                "end",
                "%% literal",
                "click A literal",
                "Yes",
                "继续",
                "Done",
            ] {
                assert!(
                    rendered.svg.contains(&format!(">{label}</text>")),
                    "missing {label}: {}",
                    rendered.svg
                );
            }
        }
        let error = render(
            Kind::Mermaid,
            "flowchart TD\nA[\"First\nSecond\"]\nclick A call external()",
        )
        .expect_err("click after multiline label remains unsupported");
        assert!(error.contains("line 4"), "{error}");
        for source in [
            "flowchart TD\nA[\"First\nSecond\"]\nclick A call external()",
            "flowchart TD\nA[\"First\nSecond",
        ] {
            assert!(render(Kind::Mermaid, source).is_err(), "{source}");
        }
    }

    #[test]
    fn frontmatter_does_not_bypass_native_diagram_validation() {
        let prefix = "%% comment\n\n---\ntitle: Example\n---\n";
        let error = render(Kind::Mermaid, &format!("{prefix}pie\n\"A\" : nope"))
            .expect_err("invalid data after metadata");
        assert!(error.contains("line 7"), "{error}");
        let error = render(
            Kind::Mermaid,
            &format!("{prefix}flowchart LR\nA-->B\nclick A call external()"),
        )
        .expect_err("unsupported action after metadata");
        assert!(error.contains("line 8"), "{error}");
        assert!(render(Kind::Mermaid, &format!("{prefix}pie\n\"A\" : 10")).is_ok());
        assert!(render(Kind::Mermaid, &format!("{prefix}flowchart LR\nA-->B")).is_ok());
        assert!(render(Kind::Mermaid, "---\ntitle: Missing end\npie\n\"A\" : 1").is_err());
        assert!(render(Kind::Mermaid, prefix).is_err());
    }

    #[test]
    fn invalid_input_is_not_successful_output() {
        assert!(render(Kind::Math, r"\notARealYuCommand{x}").is_err());
        assert!(render(Kind::Mermaid, "not a diagram").is_err());
        assert!(render(Kind::Math, "").is_err());
        assert!(render(Kind::Math, &"x".repeat(MAX_SOURCE_BYTES + 1)).is_err());
    }
    #[test]
    fn typst_world_has_no_document_file_access() {
        let world = MathWorld {
            source: Source::detached("x"),
        };
        let other = typst::syntax::RootedPath::new(
            typst::syntax::VirtualRoot::Project,
            typst::syntax::VirtualPath::new("secret.txt").expect("path"),
        )
        .intern();
        assert!(world.source(other).is_err());
        assert!(world.file(other).is_err());
        assert!(world.today(None).is_none());
    }
}
