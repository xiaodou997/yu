#![forbid(unsafe_code)]

//! A bounded, native Math renderer for Yu embedded Markdown resources.
//!
//! This is deliberately a small TeX-like subset rather than a promise of
//! full LaTeX compatibility. It gives the resource pipeline a real renderer
//! with deterministic output, strict input limits, XML escaping, and no
//! JavaScript/WebView dependency. The output is backend-neutral SVG and can be
//! handed to a later scene/Metal consumer through `yu-assets`.

use std::fmt::{self, Write as _};

use yu_assets::{
    EmbeddedRenderError, EmbeddedRenderPayload, EmbeddedRenderRequest, EmbeddedRenderer,
    EmbeddedResourceKind,
};

/// Maximum UTF-8 bytes accepted by the bounded renderer.
pub const MAX_SOURCE_BYTES: usize = 16 * 1024;
const MAX_PARSE_DEPTH: usize = 64;
const BASE_FONT_SIZE: f32 = 24.0;
const HORIZONTAL_PADDING: f32 = 8.0;
const VERTICAL_PADDING: f32 = 6.0;
const SCRIPT_SCALE: f32 = 0.68;
const FRACTION_GAP: f32 = 3.0;
const FRACTION_LINE: f32 = 1.0;
const RADICAL_GAP: f32 = 3.0;

/// Deterministic native Math renderer configuration.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MathRenderer {
    font_size: f32,
}

impl Default for MathRenderer {
    fn default() -> Self {
        Self {
            font_size: BASE_FONT_SIZE,
        }
    }
}

impl MathRenderer {
    /// Creates a renderer with a positive finite base font size in points.
    #[must_use]
    pub fn new(font_size: f32) -> Option<Self> {
        if font_size.is_finite() && font_size > 0.0 {
            Some(Self { font_size })
        } else {
            None
        }
    }

    #[must_use]
    pub const fn font_size(self) -> f32 {
        self.font_size
    }

    /// Renders one Math source string to an owned SVG payload.
    pub fn render_source(
        &self,
        source: &str,
    ) -> Result<EmbeddedRenderPayload, EmbeddedRenderError> {
        if source.len() > MAX_SOURCE_BYTES || source.trim().is_empty() {
            return Err(EmbeddedRenderError::InvalidSource);
        }
        let root = Parser::new(source)
            .parse()
            .map_err(|_| EmbeddedRenderError::InvalidSource)?;
        let metrics = measure(&root, self.font_size);
        if !metrics.is_finite() {
            return Err(EmbeddedRenderError::Render);
        }
        let width = (metrics.width + HORIZONTAL_PADDING * 2.0).ceil();
        let height = (metrics.ascent + metrics.descent + VERTICAL_PADDING * 2.0).ceil();
        let width = u32::try_from(width as u64).map_err(|_| EmbeddedRenderError::Render)?;
        let height = u32::try_from(height as u64).map_err(|_| EmbeddedRenderError::Render)?;
        let mut svg = String::new();
        write_svg_header(&mut svg, width, height).map_err(|_| EmbeddedRenderError::Render)?;
        render_node(
            &mut svg,
            &root,
            HORIZONTAL_PADDING,
            VERTICAL_PADDING + metrics.ascent,
            self.font_size,
        )
        .map_err(|_| EmbeddedRenderError::Render)?;
        svg.push_str("</g></svg>");
        EmbeddedRenderPayload::svg(width, height, svg).map_err(|_| EmbeddedRenderError::Render)
    }
}

impl EmbeddedRenderer for MathRenderer {
    fn render(
        &self,
        request: &EmbeddedRenderRequest,
    ) -> Result<EmbeddedRenderPayload, EmbeddedRenderError> {
        if request.kind() != EmbeddedResourceKind::Math {
            return Err(EmbeddedRenderError::Unsupported);
        }
        self.render_source(request.source())
    }
}

#[derive(Clone, Debug, PartialEq)]
enum Node {
    Text(String),
    Row(Vec<Node>),
    Fraction {
        numerator: Box<Node>,
        denominator: Box<Node>,
    },
    Radical(Box<Node>),
    Script {
        base: Box<Node>,
        superscript: Option<Box<Node>>,
        subscript: Option<Box<Node>>,
    },
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct Metrics {
    width: f32,
    ascent: f32,
    descent: f32,
}

impl Metrics {
    const fn new(width: f32, ascent: f32, descent: f32) -> Self {
        Self {
            width,
            ascent,
            descent,
        }
    }

    fn height(self) -> f32 {
        self.ascent + self.descent
    }

    fn is_finite(self) -> bool {
        self.width.is_finite() && self.ascent.is_finite() && self.descent.is_finite()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ParseError {
    UnexpectedEnd,
    UnexpectedCharacter,
    UnknownCommand,
    DuplicateScript,
    TooDeep,
}

struct Parser {
    chars: Vec<char>,
    index: usize,
    depth: usize,
}

impl Parser {
    fn new(source: &str) -> Self {
        Self {
            chars: source.chars().collect(),
            index: 0,
            depth: 0,
        }
    }

    fn parse(mut self) -> Result<Node, ParseError> {
        let node = self.parse_row(false)?;
        if self.index != self.chars.len() {
            return Err(ParseError::UnexpectedCharacter);
        }
        Ok(node)
    }

    fn parse_row(&mut self, grouped: bool) -> Result<Node, ParseError> {
        self.enter_depth()?;
        let mut nodes = Vec::new();
        while let Some(&character) = self.chars.get(self.index) {
            if character == '}' {
                if grouped {
                    break;
                }
                return Err(ParseError::UnexpectedCharacter);
            }
            if character.is_whitespace() {
                self.index += 1;
                self.push_text(&mut nodes, " ");
                continue;
            }
            let atom = self.parse_atom()?;
            let atom = self.parse_scripts(atom)?;
            nodes.push(atom);
        }
        if grouped {
            if self.chars.get(self.index) != Some(&'}') {
                return Err(ParseError::UnexpectedEnd);
            }
            self.index += 1;
        }
        self.leave_depth();
        if nodes.is_empty() {
            return Err(ParseError::UnexpectedEnd);
        }
        if nodes.len() == 1 {
            match nodes.pop() {
                Some(node) => Ok(node),
                None => Err(ParseError::UnexpectedEnd),
            }
        } else {
            Ok(Node::Row(nodes))
        }
    }

    fn parse_atom(&mut self) -> Result<Node, ParseError> {
        let character = *self
            .chars
            .get(self.index)
            .ok_or(ParseError::UnexpectedEnd)?;
        match character {
            '{' => {
                self.index += 1;
                self.parse_row(true)
            }
            '}' | '^' | '_' => Err(ParseError::UnexpectedCharacter),
            '\\' => self.parse_command(),
            _ => {
                self.index += 1;
                if character.is_control() {
                    Err(ParseError::UnexpectedCharacter)
                } else {
                    Ok(Node::Text(character.to_string()))
                }
            }
        }
    }

    fn parse_scripts(&mut self, base: Node) -> Result<Node, ParseError> {
        let mut superscript = None;
        let mut subscript = None;
        while let Some(&character) = self.chars.get(self.index) {
            let target = match character {
                '^' => &mut superscript,
                '_' => &mut subscript,
                _ => break,
            };
            if target.is_some() {
                return Err(ParseError::DuplicateScript);
            }
            self.index += 1;
            let script = if self.chars.get(self.index) == Some(&'{') {
                self.index += 1;
                self.parse_row(true)?
            } else {
                self.parse_atom()?
            };
            *target = Some(Box::new(script));
        }
        if superscript.is_none() && subscript.is_none() {
            Ok(base)
        } else {
            Ok(Node::Script {
                base: Box::new(base),
                superscript,
                subscript,
            })
        }
    }

    fn parse_command(&mut self) -> Result<Node, ParseError> {
        self.index += 1;
        let first = *self
            .chars
            .get(self.index)
            .ok_or(ParseError::UnexpectedEnd)?;
        if !first.is_ascii_alphabetic() {
            self.index += 1;
            return match first {
                '\\' => Ok(Node::Text(" ".into())),
                '{' | '}' | '^' | '_' | '%' | '$' | '#' | '&' => Ok(Node::Text(first.to_string())),
                ',' | ';' => Ok(Node::Text(" ".into())),
                '!' => Ok(Node::Text(String::new())),
                _ => Err(ParseError::UnknownCommand),
            };
        }
        let start = self.index;
        while self
            .chars
            .get(self.index)
            .is_some_and(|character| character.is_ascii_alphabetic())
        {
            self.index += 1;
        }
        let name: String = self.chars[start..self.index].iter().collect();
        match name.as_str() {
            "frac" => Ok(Node::Fraction {
                numerator: Box::new(self.parse_required_group()?),
                denominator: Box::new(self.parse_required_group()?),
            }),
            "sqrt" => Ok(Node::Radical(Box::new(self.parse_required_group()?))),
            "text" | "mathrm" | "mathbf" | "operatorname" => self.parse_required_group(),
            "left" | "right" => Ok(Node::Text(String::new())),
            "quad" | "qquad" => Ok(Node::Text("  ".into())),
            _ => command_text(&name)
                .map(|text| Node::Text(text.into()))
                .ok_or(ParseError::UnknownCommand),
        }
    }

    fn parse_required_group(&mut self) -> Result<Node, ParseError> {
        if self.chars.get(self.index) != Some(&'{') {
            return Err(ParseError::UnexpectedCharacter);
        }
        self.index += 1;
        self.parse_row(true)
    }

    fn push_text(&self, nodes: &mut Vec<Node>, text: &str) {
        if let Some(Node::Text(previous)) = nodes.last_mut() {
            previous.push_str(text);
        } else {
            nodes.push(Node::Text(text.into()));
        }
    }

    fn enter_depth(&mut self) -> Result<(), ParseError> {
        self.depth = self.depth.saturating_add(1);
        if self.depth > MAX_PARSE_DEPTH {
            Err(ParseError::TooDeep)
        } else {
            Ok(())
        }
    }

    fn leave_depth(&mut self) {
        self.depth = self.depth.saturating_sub(1);
    }
}

fn command_text(name: &str) -> Option<&'static str> {
    Some(match name {
        "alpha" => "α",
        "beta" => "β",
        "gamma" => "γ",
        "delta" => "δ",
        "epsilon" => "ϵ",
        "theta" => "θ",
        "lambda" => "λ",
        "mu" => "μ",
        "pi" => "π",
        "sigma" => "σ",
        "phi" => "ϕ",
        "omega" => "ω",
        "Gamma" => "Γ",
        "Delta" => "Δ",
        "Theta" => "Θ",
        "Lambda" => "Λ",
        "Pi" => "Π",
        "Sigma" => "Σ",
        "Phi" => "Φ",
        "Omega" => "Ω",
        "cdot" => "⋅",
        "times" => "×",
        "pm" => "±",
        "mp" => "∓",
        "le" | "leq" => "≤",
        "ge" | "geq" => "≥",
        "ne" | "neq" => "≠",
        "infty" => "∞",
        "to" | "rightarrow" => "→",
        "leftarrow" => "←",
        "mapsto" => "↦",
        "sum" => "∑",
        "prod" => "∏",
        "int" => "∫",
        "partial" => "∂",
        "nabla" => "∇",
        "sin" => "sin",
        "cos" => "cos",
        "tan" => "tan",
        "log" => "log",
        "ln" => "ln",
        "lim" => "lim",
        _ => return None,
    })
}

fn measure(node: &Node, font_size: f32) -> Metrics {
    match node {
        Node::Text(text) => Metrics::new(
            text_width(text, font_size),
            font_size * 0.78,
            font_size * 0.22,
        ),
        Node::Row(nodes) => {
            let mut width = 0.0;
            let mut ascent: f32 = 0.0;
            let mut descent: f32 = 0.0;
            for child in nodes {
                let child_metrics = measure(child, font_size);
                width += child_metrics.width;
                ascent = ascent.max(child_metrics.ascent);
                descent = descent.max(child_metrics.descent);
            }
            Metrics::new(width, ascent, descent)
        }
        Node::Fraction {
            numerator,
            denominator,
        } => {
            let numerator = measure(numerator, font_size * 0.9);
            let denominator = measure(denominator, font_size * 0.9);
            Metrics::new(
                numerator.width.max(denominator.width) + 2.0 * FRACTION_GAP,
                numerator.height() + FRACTION_GAP + FRACTION_LINE / 2.0,
                denominator.height() + FRACTION_GAP + FRACTION_LINE / 2.0,
            )
        }
        Node::Radical(child) => {
            let child = measure(child, font_size);
            Metrics::new(
                font_size * 0.8 + RADICAL_GAP + child.width,
                child.ascent + font_size * 0.08,
                child.descent,
            )
        }
        Node::Script {
            base,
            superscript,
            subscript,
        } => {
            let base = measure(base, font_size);
            let superscript = superscript
                .as_deref()
                .map(|node| measure(node, font_size * SCRIPT_SCALE));
            let subscript = subscript
                .as_deref()
                .map(|node| measure(node, font_size * SCRIPT_SCALE));
            let script_width = superscript
                .as_ref()
                .map_or(0.0, |metrics| metrics.width)
                .max(subscript.as_ref().map_or(0.0, |metrics| metrics.width));
            let superscript_ascent = superscript.map_or(0.0, |metrics| {
                font_size * 0.55 + metrics.ascent + metrics.descent * 0.2
            });
            let subscript_descent = subscript.map_or(0.0, |metrics| {
                font_size * 0.45 + metrics.descent + metrics.ascent * 0.2
            });
            Metrics::new(
                base.width + script_width,
                base.ascent.max(superscript_ascent),
                base.descent.max(subscript_descent),
            )
        }
    }
}

fn text_width(text: &str, font_size: f32) -> f32 {
    text.chars()
        .map(|character| {
            if character.is_whitespace() {
                font_size * 0.35
            } else if character.is_ascii() {
                font_size * 0.58
            } else {
                font_size * 0.86
            }
        })
        .sum()
}

fn render_node(
    svg: &mut String,
    node: &Node,
    x: f32,
    baseline: f32,
    font_size: f32,
) -> fmt::Result {
    match node {
        Node::Text(text) => {
            if text.is_empty() {
                return Ok(());
            }
            write!(
                svg,
                "<text x=\"{x:.2}\" y=\"{baseline:.2}\" font-size=\"{font_size:.2}\">"
            )?;
            escape_xml(svg, text)?;
            svg.push_str("</text>");
        }
        Node::Row(nodes) => {
            let mut cursor = x;
            for child in nodes {
                render_node(svg, child, cursor, baseline, font_size)?;
                cursor += measure(child, font_size).width;
            }
        }
        Node::Fraction {
            numerator,
            denominator,
        } => {
            let numerator_metrics = measure(numerator, font_size * 0.9);
            let denominator_metrics = measure(denominator, font_size * 0.9);
            let width = numerator_metrics.width.max(denominator_metrics.width) + 2.0 * FRACTION_GAP;
            let metrics = measure(node, font_size);
            let top = baseline - metrics.ascent;
            let line_y = baseline - FRACTION_LINE / 2.0;
            let numerator_x = x + (width - numerator_metrics.width) / 2.0;
            let denominator_x = x + (width - denominator_metrics.width) / 2.0;
            render_node(
                svg,
                numerator,
                numerator_x,
                top + numerator_metrics.ascent,
                font_size * 0.9,
            )?;
            write!(
                svg,
                "<line x1=\"{x:.2}\" y1=\"{line_y:.2}\" x2=\"{:.2}\" y2=\"{line_y:.2}\" stroke-width=\"1\"/> ",
                x + width
            )?;
            render_node(
                svg,
                denominator,
                denominator_x,
                baseline + FRACTION_LINE / 2.0 + FRACTION_GAP + denominator_metrics.ascent,
                font_size * 0.9,
            )?;
        }
        Node::Radical(child) => {
            let radical_width = font_size * 0.8;
            write!(
                svg,
                "<text x=\"{x:.2}\" y=\"{baseline:.2}\" font-size=\"{font_size:.2}\">√</text>"
            )?;
            let child_x = x + radical_width + RADICAL_GAP;
            render_node(svg, child, child_x, baseline, font_size)?;
            let child_metrics = measure(child, font_size);
            let bar_y = baseline - child_metrics.ascent - font_size * 0.08;
            write!(
                svg,
                "<line x1=\"{child_x:.2}\" y1=\"{bar_y:.2}\" x2=\"{:.2}\" y2=\"{bar_y:.2}\" stroke-width=\"1\"/> ",
                child_x + child_metrics.width
            )?;
        }
        Node::Script {
            base,
            superscript,
            subscript,
        } => {
            render_node(svg, base, x, baseline, font_size)?;
            let base_width = measure(base, font_size).width;
            let script_x = x + base_width;
            if let Some(superscript) = superscript {
                render_node(
                    svg,
                    superscript,
                    script_x,
                    baseline - font_size * 0.55,
                    font_size * SCRIPT_SCALE,
                )?;
            }
            if let Some(subscript) = subscript {
                render_node(
                    svg,
                    subscript,
                    script_x,
                    baseline + font_size * 0.65,
                    font_size * SCRIPT_SCALE,
                )?;
            }
        }
    }
    Ok(())
}

fn write_svg_header(svg: &mut String, width: u32, height: u32) -> fmt::Result {
    write!(
        svg,
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{width}\" height=\"{height}\" viewBox=\"0 0 {width} {height}\"><g fill=\"currentColor\" stroke=\"currentColor\" font-family=\"STIX Two Math, Cambria Math, Times New Roman, serif\" text-rendering=\"geometricPrecision\">"
    )
}

fn escape_xml(output: &mut String, text: &str) -> fmt::Result {
    for character in text.chars() {
        match character {
            '&' => output.push_str("&amp;"),
            '<' => output.push_str("&lt;"),
            '>' => output.push_str("&gt;"),
            '"' => output.push_str("&quot;"),
            '\'' => output.push_str("&apos;"),
            _ => output.push(character),
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use yu_core::{ByteOffset, Revision, TextRange};

    fn request(kind: EmbeddedResourceKind, source: &str) -> EmbeddedRenderRequest {
        EmbeddedRenderRequest::new(
            Revision::INITIAL,
            TextRange::new(ByteOffset::ZERO, ByteOffset::new(source.len() as u64)).expect("range"),
            kind,
            source,
        )
        .expect("request")
    }

    #[test]
    fn renders_basic_fraction_scripts_and_greek_to_svg() {
        let renderer = MathRenderer::default();
        let payload = renderer
            .render(&request(
                EmbeddedResourceKind::Math,
                r"\frac{\alpha_1 + x^2}{\sqrt{\beta}}",
            ))
            .expect("math SVG");
        let svg = payload.markup().expect("SVG markup");
        assert!(svg.starts_with("<svg "));
        assert!(svg.contains("α"));
        assert!(svg.contains("β"));
        assert!(svg.contains("<line"));
        assert!(payload.dimensions().width() > 0);
        assert!(payload.dimensions().height() > 0);
    }

    #[test]
    fn escapes_xml_in_text_nodes() {
        let renderer = MathRenderer::default();
        let payload = renderer
            .render(&request(EmbeddedResourceKind::Math, r"a < b & c"))
            .expect("math SVG");
        let svg = payload.markup().expect("SVG markup");
        assert!(svg.contains("&lt;"));
        assert!(svg.contains("&amp;"));
        assert!(!svg.contains("a < b"));
    }

    #[test]
    fn rejects_unknown_commands_and_wrong_kind() {
        let renderer = MathRenderer::default();
        assert_eq!(
            renderer.render(&request(EmbeddedResourceKind::Math, r"\unknown{x}")),
            Err(EmbeddedRenderError::InvalidSource)
        );
        assert_eq!(
            renderer.render(&request(EmbeddedResourceKind::Mermaid, "graph TD")),
            Err(EmbeddedRenderError::Unsupported)
        );
    }

    #[test]
    fn rejects_empty_and_oversized_sources() {
        let renderer = MathRenderer::default();
        assert_eq!(
            renderer.render_source(" \n\t"),
            Err(EmbeddedRenderError::InvalidSource)
        );
        let oversized = "x".repeat(MAX_SOURCE_BYTES + 1);
        assert_eq!(
            renderer.render_source(&oversized),
            Err(EmbeddedRenderError::InvalidSource)
        );
    }

    #[test]
    fn cache_publishes_math_renderer_output() {
        let renderer = MathRenderer::default();
        let mut cache = yu_assets::EmbeddedResourceCache::new();
        let request = request(EmbeddedResourceKind::Math, "x^2 + y^2");
        assert!(matches!(
            cache.request(request),
            yu_assets::EmbeddedRequestResult::Pending
        ));
        let result = cache
            .render_pending(Revision::INITIAL, &renderer)
            .expect("render")
            .expect("publication");
        let yu_assets::EmbeddedRequestResult::Ready(publication) = result else {
            panic!("expected ready publication");
        };
        assert_eq!(publication.kind(), EmbeddedResourceKind::Math);
        assert_eq!(
            publication.payload().format(),
            yu_assets::EmbeddedRenderFormat::Svg
        );
    }
}
