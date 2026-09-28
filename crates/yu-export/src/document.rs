//! Portable, inert whole-document HTML. Clipboard options remain unchanged.
//! Comrak owns Markdown rendering; Yu owns equation/footnote/TOC relationships
//! and its existing finite HTML vocabulary. No editable document lives here.
use comrak::nodes::NodeValue;
use std::collections::{BTreeMap, HashMap};
use yu_core::{ByteOffset, TextRange};
use yu_markdown::html::{HtmlAttributes, HtmlBlockModel, HtmlDimension, HtmlNodeKind, HtmlTag};
use yu_markdown::{EmbeddedKind, EmbeddedSpan, MarkdownDocument};
use yu_syntax::NodeKind;

pub const MAX_SOURCE_BYTES: usize = 8 * 1024 * 1024;

#[cfg(test)]
#[path = "document_bounds.rs"]
mod bounds;
pub const MAX_OUTPUT_BYTES: usize = 256 * 1024 * 1024;
pub const MAX_RESOURCES: usize = 2048;
/// Encoded resources are counted per occurrence before insertion/expansion.
/// This also bounds repeated references to one otherwise-small cached image.
pub const MAX_RESOURCE_OUTPUT_BYTES: usize = 64 * 1024 * 1024;

const OUTPUT_LIMIT: &str = "HTML 超过 256 MiB 输出预算";

// All size arithmetic precedes allocation. This adapter bounds comrak's
// existing formatter; it does not introduce another Markdown renderer.
fn checked_length(current: usize, extra: usize, limit: usize) -> Result<usize, String> {
    current
        .checked_add(extra)
        .filter(|&size| size <= limit)
        .ok_or_else(|| "导出内容超过字节预算".into())
}
fn append_html(output: &mut String, text: &str) -> Result<(), String> {
    let size = checked_length(output.len(), text.len(), MAX_OUTPUT_BYTES)
        .map_err(|_| OUTPUT_LIMIT.to_owned())?;
    if size > output.capacity() {
        let capacity = size.next_power_of_two().min(MAX_OUTPUT_BYTES);
        output
            .try_reserve_exact(capacity - output.len())
            .map_err(|_| "无法分配导出缓冲区")?;
    }
    output.push_str(text);
    Ok(())
}
struct HtmlSink<'a>(&'a mut String);
impl std::fmt::Write for HtmlSink<'_> {
    fn write_str(&mut self, text: &str) -> std::fmt::Result {
        append_html(self.0, text).map_err(|_| std::fmt::Error)
    }
}
fn join_html(parts: &[&str]) -> Result<String, String> {
    let size = parts
        .iter()
        .try_fold(0, |n, part| checked_length(n, part.len(), MAX_OUTPUT_BYTES))?;
    let mut result = String::new();
    result
        .try_reserve_exact(size)
        .map_err(|_| "无法分配导出缓冲区")?;
    for part in parts {
        result.push_str(part);
    }
    Ok(result)
}

#[derive(Clone, Debug)]
pub struct HtmlOptions {
    pub title: String,
    pub foreground: u32,
    pub background: u32,
    pub link: u32,
    pub font_size: f32,
    pub width: u32,
    pub dark: bool,
    pub reference_day: i32,
}
impl Default for HtmlOptions {
    fn default() -> Self {
        Self {
            title: "Yu 文档".into(),
            foreground: 0x24292fff,
            background: 0xffffffff,
            link: 0x0969daff,
            font_size: 16.0,
            width: 800,
            dark: false,
            reference_day: 0,
        }
    }
}
#[derive(Clone, Debug)]
pub struct ExportImage {
    pub data_uri: String,
    pub width: u32,
    pub height: u32,
    pub baseline_milli: Option<u32>,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ResourceError {
    /// Missing/remote or already-invalid source: requires explicit user consent.
    Warning(String),
    /// Cancellation, resource budget, worker or exporter failure: never a placeholder pass.
    Fatal(String),
}
pub trait HtmlResources {
    fn checkpoint(&mut self) -> Result<(), String>;
    fn image(&mut self, destination: &str) -> Result<ExportImage, ResourceError>;
    fn embedded(
        &mut self,
        span: EmbeddedSpan,
        source: &str,
        options: &HtmlOptions,
    ) -> Result<ExportImage, ResourceError>;
}
#[derive(Clone, Debug)]
pub struct HtmlDocument {
    pub html: String,
    pub warnings: Vec<String>,
    pub image_count: usize,
    pub embedded_count: usize,
}
#[derive(Clone)]
struct Replacement {
    range: TextRange,
    html: String,
    block: bool,
}
struct Slots {
    prefix: String,
    values: Vec<String>,
}
impl Slots {
    fn new(source: &str) -> Self {
        let mut prefix = String::from("YUEXPORTSLOT");
        while source.contains(&prefix) {
            prefix.push('X');
        }
        Self {
            prefix,
            values: Vec::new(),
        }
    }
    fn insert(&mut self, html: String, block: bool) -> String {
        let id = self.values.len();
        self.values.push(html);
        let token = format!("{}{id}Z", self.prefix);
        if block {
            format!("<!--{token}-->\n")
        } else {
            token
        }
    }
    fn block(&self, raw: &str) -> Option<&str> {
        let id = raw
            .trim()
            .strip_prefix("<!--")?
            .strip_suffix("-->")?
            .strip_prefix(&self.prefix)?
            .strip_suffix('Z')?
            .parse::<usize>()
            .ok()?;
        self.values.get(id).map(String::as_str)
    }
    fn expand(&self, input: &str) -> Result<String, String> {
        // Preflight all replacements, including the literal tail, before the
        // first output allocation. A repeated TOC can expand a tiny input.
        let mut size = 0;
        let mut remaining = input;
        while let Some(at) = remaining.find(&self.prefix) {
            size = checked_length(size, at, MAX_OUTPUT_BYTES)?;
            let tail = &remaining[at + self.prefix.len()..];
            let end = tail.find('Z').ok_or("导出占位身份损坏")?;
            let id = tail[..end]
                .parse::<usize>()
                .map_err(|_| "导出占位身份损坏")?;
            size = checked_length(
                size,
                self.values.get(id).ok_or("导出占位身份丢失")?.len(),
                MAX_OUTPUT_BYTES,
            )?;
            remaining = &tail[end + 1..];
        }
        size = checked_length(size, remaining.len(), MAX_OUTPUT_BYTES)?;
        let mut output = String::new();
        output
            .try_reserve_exact(size)
            .map_err(|_| "无法分配导出缓冲区")?;
        let mut rest = input;
        while let Some(at) = rest.find(&self.prefix) {
            output.push_str(&rest[..at]);
            let tail = &rest[at + self.prefix.len()..];
            let end = tail.find('Z').ok_or("导出占位身份损坏")?;
            let id = tail[..end]
                .parse::<usize>()
                .map_err(|_| "导出占位身份损坏")?;
            output.push_str(self.values.get(id).ok_or("导出占位身份丢失")?);
            rest = &tail[end + 1..];
        }
        output.push_str(rest);
        Ok(output)
    }
}
struct Writer<'a, R> {
    document: &'a MarkdownDocument,
    options: &'a HtmlOptions,
    resources: &'a mut R,
    warnings: Vec<String>,
    images: usize,
    embedded: usize,
    resource_output_bytes: usize,
    specials: BTreeMap<(u64, u64), String>,
    ids: HashMap<String, usize>,
    slots: Slots,
}
pub fn export_html_document<R: HtmlResources>(
    document: &MarkdownDocument,
    options: &HtmlOptions,
    resources: &mut R,
) -> Result<HtmlDocument, String> {
    if document.source().as_str().len() > MAX_SOURCE_BYTES {
        return Err("Markdown 超过 8 MiB 导出预算".into());
    }
    if !(12.0..=32.0).contains(&options.font_size)
        || !(360..=1200).contains(&options.width)
        || options.title.len() > 4096
    {
        return Err("导出样式参数无效".into());
    }
    let mut writer = Writer {
        document,
        options,
        resources,
        warnings: Vec::new(),
        images: 0,
        embedded: 0,
        resource_output_bytes: 0,
        specials: BTreeMap::new(),
        ids: HashMap::new(),
        slots: Slots::new(document.source().as_str()),
    };
    writer.render()
}
fn key(range: TextRange) -> (u64, u64) {
    (range.start().get(), range.end().get())
}
fn raw(source: &str, range: TextRange) -> &str {
    &source[range.start().get() as usize..range.end().get() as usize]
}
pub fn escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}
fn user_id(value: &str) -> String {
    use std::fmt::Write;
    let mut result = String::from("yu-user-");
    for byte in value.bytes() {
        let _ = write!(result, "{byte:02x}");
    }
    result
}
/// Deliberately no implicit local-file access or protocol-relative URLs.
fn safe_link(value: &str) -> Option<String> {
    if value.chars().any(char::is_control) || value.trim() != value {
        return None;
    }
    if let Some(anchor) = value.strip_prefix('#') {
        return Some(format!("#{}", user_id(anchor)));
    }
    let scheme = value.split_once(':')?.0.to_ascii_lowercase();
    matches!(scheme.as_str(), "https" | "http" | "mailto").then(|| value.to_owned())
}
fn document_options() -> comrak::Options<'static> {
    let mut options = super::options();
    options.extension.superscript = true;
    options.extension.subscript = true;
    options.extension.highlight = true;
    options.extension.footnotes = true;
    options
}
fn plain_node<'a>(node: &'a comrak::nodes::AstNode<'a>) -> String {
    let mut result = String::new();
    for child in node.descendants() {
        match &child.data().value {
            NodeValue::Text(text) => result.push_str(text),
            NodeValue::Code(code) => result.push_str(&code.literal),
            NodeValue::SoftBreak | NodeValue::LineBreak => result.push(' '),
            _ => {}
        }
    }
    result
}
impl<R: HtmlResources> Writer<'_, R> {
    fn account_resource(&mut self, image: &ExportImage) -> Result<(), String> {
        self.resource_output_bytes = checked_length(
            self.resource_output_bytes,
            image.data_uri.len(),
            MAX_RESOURCE_OUTPUT_BYTES,
        )
        .map_err(|_| "按出现次数计算的内嵌资源超过 64 MiB 输出预算")?;
        Ok(())
    }
    fn warn(&mut self, message: impl Into<String>) {
        let message = message.into();
        if !self.warnings.contains(&message) {
            self.warnings.push(message);
        }
    }
    fn fallback(&mut self, message: &str, source: &str, block: bool) -> String {
        self.warn(message);
        let tag = if block { "pre" } else { "code" };
        format!(
            "<{tag} class=\"yu-diagnostic\" title=\"{}\">{}</{tag}>",
            escape(message),
            escape(source)
        )
    }
    fn image_html(
        &mut self,
        destination: &str,
        alt: &str,
        title: Option<&str>,
    ) -> Result<String, String> {
        self.resources.checkpoint()?;
        self.images += 1;
        if self.images + self.embedded > MAX_RESOURCES {
            return Err("文档超过 2048 项资源预算".into());
        }
        let image = match self.resources.image(destination) {
            Ok(image) => image,
            Err(ResourceError::Fatal(message)) => return Err(message),
            Err(ResourceError::Warning(message)) => {
                return Ok(self.fallback(&message, &format!("图片：{alt}（{message}）"), false));
            }
        };
        self.account_resource(&image)?;
        Ok(format!(
            "<img src=\"{}\" alt=\"{}\" title=\"{}\">",
            escape(&image.data_uri),
            escape(alt),
            escape(title.unwrap_or(""))
        ))
    }
    fn embedded_html(&mut self, span: EmbeddedSpan) -> Result<String, String> {
        self.resources.checkpoint()?;
        self.embedded += 1;
        if self.images + self.embedded > MAX_RESOURCES {
            return Err("文档超过 2048 项资源预算".into());
        }
        let source = match span.kind {
            EmbeddedKind::Math => self.document.equation_source(span),
            EmbeddedKind::Mermaid => self
                .document
                .embedded_source(span)
                .ok_or_else(|| "图表源码身份丢失".into()),
        };
        let input = match source {
            Ok(source) => source,
            Err(message) => {
                return Ok(self.fallback(
                    &message,
                    raw(self.document.source().as_str(), span.source),
                    span.display,
                ));
            }
        };
        let image = match self.resources.embedded(span, &input, self.options) {
            Ok(image) => image,
            Err(ResourceError::Fatal(message)) => return Err(message),
            Err(ResourceError::Warning(message)) => {
                return Ok(self.fallback(
                    &message,
                    raw(self.document.source().as_str(), span.source),
                    span.display,
                ));
            }
        };
        self.account_resource(&image)?;
        let baseline = image
            .baseline_milli
            .map_or(0.0, |base| base as f64 / 1000.0 - f64::from(image.height));
        let role = if span.kind == EmbeddedKind::Math {
            "公式"
        } else {
            "图表"
        };
        let img = format!(
            "<img class=\"yu-embedded\" src=\"{}\" alt=\"{role}\" width=\"{}\" height=\"{}\" style=\"vertical-align:{baseline:.3}px\">",
            escape(&image.data_uri),
            image.width,
            image.height
        );
        if span.display {
            Ok(format!(
                "<div class=\"yu-figure\" id=\"yu-equation-{}\">{img}</div>",
                span.source.start().get()
            ))
        } else if let Some(target) = self.document.equation_reference_target(span.source.start()) {
            Ok(format!(
                "<a href=\"#yu-equation-{}\">{img}</a>",
                target.start().get()
            ))
        } else {
            Ok(img)
        }
    }
    fn render(&mut self) -> Result<HtmlDocument, String> {
        self.resources.checkpoint()?;
        let source = self.document.source().as_str();
        let mut replacements = Vec::new();
        let mut spans = self.document.equation_spans();
        let syntax = self.document.syntax_root().ok_or("文档语法树不可用")?;
        // This is syntax traversal, not another Markdown parser. Equation
        // catalog and renderer input extraction remain owned by yu-markdown.
        for node in syntax.descendants() {
            if node.kind() != NodeKind::FencedCode {
                continue;
            }
            let info = node
                .children()
                .find(|node| node.kind() == NodeKind::CodeInfo);
            if !info.is_some_and(|info| {
                raw(source, info.range())
                    .trim()
                    .eq_ignore_ascii_case("mermaid")
            }) {
                continue;
            }
            let marks: Vec<_> = node
                .children()
                .filter(|node| node.kind() == NodeKind::CodeMark)
                .collect();
            if marks.len() != 2 {
                continue;
            }
            let text: Vec<_> = node
                .children()
                .filter(|node| node.kind() == NodeKind::CodeText)
                .collect();
            let content = text
                .first()
                .zip(text.last())
                .and_then(|(first, last)| TextRange::new(first.range().start(), last.range().end()))
                .unwrap_or_else(|| TextRange::empty(marks[1].range().start()));
            spans.push(EmbeddedSpan {
                source: node.range(),
                content,
                kind: EmbeddedKind::Mermaid,
                display: true,
            });
        }
        spans.sort_by_key(|span| key(span.source));
        spans.dedup();
        for span in spans {
            let html = self.embedded_html(span)?;
            self.specials.insert(key(span.source), html.clone());
            replacements.push(Replacement {
                range: span.source,
                html,
                block: span.display,
            });
        }
        let notes = self.document.footnotes();
        for (index, reference) in notes.references().iter().enumerate() {
            let html = if let Some(number) = reference.number {
                format!(
                    "<sup id=\"yu-ref-{index}\"><a href=\"#yu-note-{number}\">{number}</a></sup>"
                )
            } else {
                self.fallback("脚注缺少唯一的定义", raw(source, reference.source), false)
            };
            self.specials.insert(key(reference.source), html.clone());
            replacements.push(Replacement {
                range: reference.source,
                html,
                block: false,
            });
        }
        let mut title = self.options.title.clone();
        for node in syntax
            .descendants()
            .filter(|node| node.kind() == NodeKind::FrontMatter)
        {
            // Only a single-line literal title is consumed. Other metadata is
            // never included in the output, and no YAML execution is involved.
            for line in raw(source, node.range()).lines().skip(1) {
                if let Some(value) = line.strip_prefix("title:") {
                    let value = value.trim().trim_matches(['\'', '"']);
                    if !value.is_empty() && value.len() <= 4096 {
                        title = value.to_owned();
                    }
                }
            }
            replacements.push(Replacement {
                range: node.range(),
                html: String::new(),
                block: true,
            });
        }
        let mut toc = String::from("<nav class=\"yu-toc\" aria-label=\"目录\"><ul>");
        for (index, heading) in self
            .document
            .table_of_contents()
            .headings()
            .iter()
            .enumerate()
        {
            let arena = comrak::Arena::new();
            let root =
                comrak::parse_document(&arena, raw(source, heading.label), &document_options());
            append_html(
                &mut toc,
                &format!(
                    "<li style=\"margin-left:{}em\"><a href=\"#yu-heading-{index}\">{}</a></li>",
                    heading.level.saturating_sub(1),
                    escape(&plain_node(root))
                ),
            )?;
            replacements.push(Replacement {
                range: TextRange::empty(heading.label.start()),
                html: format!("<a id=\"yu-heading-{index}\"></a>"),
                block: false,
            });
        }
        append_html(&mut toc, "</ul></nav>")?;
        let toc_bytes = toc
            .len()
            .checked_mul(self.document.table_of_contents().markers().len())
            .ok_or(OUTPUT_LIMIT)?;
        checked_length(0, toc_bytes, MAX_OUTPUT_BYTES)?;
        for marker in self.document.table_of_contents().markers() {
            replacements.push(Replacement {
                range: *marker,
                html: toc.clone(),
                block: true,
            });
        }
        let regions = self.document.html_regions();
        for region in &regions.regions {
            let html = if let Ok(model) = &region.model {
                let mut html = String::new();
                for &id in &model.fragment.roots {
                    append_html(&mut html, &self.finite_node(model, id)?)?;
                }
                html
            } else {
                self.fallback(
                    "未支持的 HTML，已保留源码",
                    raw(source, region.source),
                    true,
                )
            };
            replacements.retain(|edit| {
                !(region.source.start() <= edit.range.start()
                    && edit.range.end() <= region.source.end())
            });
            replacements.push(Replacement {
                range: region.source,
                html,
                block: true,
            });
        }
        // Format note bodies with the same Markdown renderer, but never take
        // numbering/order from comrak: mixed HTML references are owned by Yu.
        let mut footer = String::from("<section class=\"yu-notes\" aria-label=\"文末注\"><ol>");
        let mut definitions: Vec<_> = notes.definitions().iter().collect();
        definitions.sort_by_key(|definition| definition.number.unwrap_or(u32::MAX));
        for definition in definitions {
            let content = self.replace_source(definition.source, &replacements)?;
            let marker = raw(source, definition.marker).trim().trim_end_matches(':');
            let input = format!("{marker}\n\n{content}");
            let arena = comrak::Arena::new();
            let options = document_options();
            let root = comrak::parse_document(&arena, &input, &options);
            self.sanitize(root)?;
            let note = root
                .descendants()
                .find(|node| matches!(node.data().value, NodeValue::FootnoteDefinition(_)));
            let mut body = String::new();
            if let Some(note) = note {
                for child in note.children() {
                    comrak::format_html(child, &options, &mut HtmlSink(&mut body))
                        .map_err(|_| OUTPUT_LIMIT)?;
                }
            } else {
                body = self.fallback(
                    "脚注正文转换失败，已保留源码",
                    raw(source, definition.source),
                    true,
                );
            }
            let body = self.slots.expand(&body)?;
            if let Some(number) = definition.number {
                append_html(
                    &mut footer,
                    &format!("<li id=\"yu-note-{number}\" value=\"{number}\">"),
                )?;
                append_html(&mut footer, &body)?;
                for (index, reference) in notes
                    .references()
                    .iter()
                    .enumerate()
                    .filter(|(_, reference)| reference.number == Some(number))
                {
                    let _ = reference;
                    append_html(
                        &mut footer,
                        &format!(" <a href=\"#yu-ref-{index}\" aria-label=\"返回引用\">↩</a>"),
                    )?;
                }
                append_html(&mut footer, "</li>")?;
            } else {
                append_html(&mut footer, "<li class=\"yu-unused-note\">")?;
                append_html(&mut footer, &body)?;
                append_html(&mut footer, "</li>")?;
            }
            replacements.retain(|edit| {
                !(definition.source.start() <= edit.range.start()
                    && edit.range.end() <= definition.source.end())
            });
            replacements.push(Replacement {
                range: definition.source,
                html: String::new(),
                block: true,
            });
        }
        append_html(&mut footer, "</ol></section>")?;
        let whole =
            TextRange::new(ByteOffset::ZERO, self.document.source_len()).ok_or("文档范围错误")?;
        let prepared = self.replace_source(whole, &replacements)?;
        let arena = comrak::Arena::new();
        let options = document_options();
        let root = comrak::parse_document(&arena, &prepared, &options);
        self.sanitize(root)?;
        let mut body = String::new();
        comrak::format_html(root, &options, &mut HtmlSink(&mut body)).map_err(|_| OUTPUT_LIMIT)?;
        let body = self.slots.expand(&body)?;
        self.resources.checkpoint()?;
        let fg = self.options.foreground >> 8;
        let bg = self.options.background >> 8;
        let link = self.options.link >> 8;
        let css = include_str!("document.css");
        let header = format!(
            "<!doctype html>\n<html lang=\"zh-CN\"><head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width,initial-scale=1\"><meta http-equiv=\"Content-Security-Policy\" content=\"default-src 'none'; img-src data:; style-src 'unsafe-inline'; base-uri 'none'; form-action 'none'\"><meta name=\"referrer\" content=\"no-referrer\"><title>{}</title><style>:root{{--fg:#{fg:06x};--bg:#{bg:06x};--link:#{link:06x};--font:{}px;--width:{}px}}\n{css}</style></head><body><main>",
            escape(&title),
            self.options.font_size,
            self.options.width,
        );
        let footer = if notes.definitions().is_empty() {
            ""
        } else {
            &footer
        };
        let html = join_html(&[&header, &body, footer, "</main></body></html>\n"])?;
        Ok(HtmlDocument {
            html,
            warnings: std::mem::take(&mut self.warnings),
            image_count: self.images,
            embedded_count: self.embedded,
        })
    }
    fn replace_source(
        &mut self,
        range: TextRange,
        replacements: &[Replacement],
    ) -> Result<String, String> {
        let source = self.document.source().as_str();
        let mut edits: Vec<_> = replacements
            .iter()
            .filter(|edit| range.start() <= edit.range.start() && edit.range.end() <= range.end())
            .collect();
        edits.sort_by_key(|edit| (edit.range.start(), edit.range.end()));
        let mut output = String::new();
        let mut cursor = range.start().get() as usize;
        for edit in edits {
            let from = edit.range.start().get() as usize;
            let to = edit.range.end().get() as usize;
            if from < cursor {
                return Err("导出语义区间重叠，已停止以避免丢失内容".into());
            }
            output.push_str(&source[cursor..from]);
            if !edit.html.is_empty() {
                output.push_str(&self.slots.insert(edit.html.clone(), edit.block));
            }
            if !edit.block && source[from..to].ends_with('\n') {
                output.push('\n');
            }
            cursor = to;
        }
        output.push_str(&source[cursor..range.end().get() as usize]);
        Ok(output)
    }
    fn finite_node(&mut self, model: &HtmlBlockModel, id: usize) -> Result<String, String> {
        let source = self.document.source().as_str();
        let node = &model.fragment.nodes[id];
        if let Some(html) = self.specials.get(&key(node.source)) {
            return Ok(html.clone());
        }
        match &node.kind {
            HtmlNodeKind::Text => Ok(escape(&yu_markdown::image_markup::decode_image_text(
                raw(source, node.source),
                true,
            ))),
            HtmlNodeKind::Comment => Ok(self.fallback(
                "HTML 注释已作为可读源码保留",
                raw(source, node.source),
                false,
            )),
            HtmlNodeKind::Element { opening, .. } => {
                let Some(element) = &model.resolution.elements[id] else {
                    return Ok(self.fallback(
                        "未支持或不安全的 HTML，已保留源码",
                        raw(source, node.source),
                        false,
                    ));
                };
                if opening.name == "img" {
                    let attrs = &element.attributes;
                    let html = self.image_html(
                        attrs.destination.as_deref().unwrap_or(""),
                        attrs.alternate.as_deref().unwrap_or(""),
                        attrs.title.as_deref(),
                    )?;
                    return Ok(apply_image_dimensions(html, attrs));
                }
                if opening.name == "a"
                    && element
                        .attributes
                        .destination
                        .as_deref()
                        .is_some_and(|url| safe_link(url).is_none())
                {
                    return Ok(self.fallback(
                        "链接地址不在允许范围，已保留源码",
                        raw(source, node.source),
                        false,
                    ));
                }
                let mut html = String::new();
                for (index, heading) in self
                    .document
                    .table_of_contents()
                    .headings()
                    .iter()
                    .enumerate()
                {
                    if heading.source.start() <= node.source.start()
                        && node.source.start() < heading.source.end()
                        && matches!(element.kind, yu_markdown::html::HtmlElementKind::Heading(_))
                    {
                        append_html(&mut html, &format!("<a id=\"yu-heading-{index}\"></a>"))?;
                    }
                }
                append_html(&mut html, &self.open_tag(opening, &element.attributes))?;
                for &child in &node.children {
                    append_html(&mut html, &self.finite_node(model, child)?)?;
                }
                if !matches!(opening.name.as_str(), "br" | "img") {
                    append_html(&mut html, &format!("</{}>", opening.name))?;
                }
                Ok(html)
            }
        }
    }
    fn open_tag(&mut self, tag: &HtmlTag, attrs: &HtmlAttributes) -> String {
        let mut html = format!("<{}", tag.name);
        if let Some(id) = &attrs.id {
            let seen = self.ids.entry(id.clone()).or_default();
            let suffix = if *seen == 0 {
                String::new()
            } else {
                format!("-duplicate-{seen}")
            };
            *seen += 1;
            html.push_str(&format!(" id=\"{}{suffix}\"", user_id(id)));
            if !suffix.is_empty() {
                self.warn("重复的 HTML id 已隔离；同名链接指向首次定义");
            }
        }
        if let Some(title) = &attrs.title {
            html.push_str(&format!(" title=\"{}\"", escape(title)));
        }
        if tag.name == "a"
            && let Some(link) = attrs.destination.as_deref().and_then(safe_link)
        {
            html.push_str(&format!(" href=\"{}\" rel=\"noreferrer\"", escape(&link)));
        }
        if let Some(start) = attrs.start {
            html.push_str(&format!(" start=\"{start}\""));
        }
        if let Some(span) = attrs.column_span {
            html.push_str(&format!(" colspan=\"{span}\""));
        }
        if let Some(span) = attrs.row_span {
            html.push_str(&format!(" rowspan=\"{span}\""));
        }
        if let Some(align) = attrs.alignment {
            let value = match align {
                yu_markdown::html::HtmlAlignment::Left => "left",
                yu_markdown::html::HtmlAlignment::Center => "center",
                yu_markdown::html::HtmlAlignment::Right => "right",
                yu_markdown::html::HtmlAlignment::Justify => "justify",
            };
            html.push_str(&format!(" style=\"text-align:{value}\""));
        }
        if tag.name == "details" {
            html.push_str(" open");
        }
        html.push('>');
        html
    }
    fn sanitize<'a>(&mut self, root: &'a comrak::nodes::AstNode<'a>) -> Result<(), String> {
        let nodes: Vec<_> = root.descendants().collect();
        for node in nodes {
            self.resources.checkpoint()?;
            let value = node.data().value.clone();
            let replacement = match value {
                NodeValue::Image(link) => {
                    Some(self.image_html(&link.url, &plain_node(node), Some(&link.title))?)
                }
                NodeValue::Link(link) if safe_link(&link.url).is_none() => Some(self.fallback(
                    "链接地址不在允许范围",
                    &format!("{} ({})", plain_node(node), link.url),
                    false,
                )),
                NodeValue::Link(mut link) => {
                    link.url = safe_link(&link.url).ok_or("链接校验失败")?;
                    node.data_mut().value = NodeValue::Link(link);
                    None
                }
                NodeValue::HtmlBlock(block) => {
                    let html = self
                        .slots
                        .block(&block.literal)
                        .map(str::to_owned)
                        .unwrap_or_else(|| {
                            self.fallback("未支持或不安全的 HTML，已保留源码", &block.literal, true)
                        });
                    let mut block = block;
                    block.literal = html;
                    node.data_mut().value = NodeValue::HtmlBlock(block);
                    None
                }
                NodeValue::HtmlInline(raw) => {
                    let html = if let Ok(tag) = HtmlTag::parse(&raw, ByteOffset::ZERO) {
                        if tag.closing && tag.semantic_kind().is_some() {
                            format!("</{}>", tag.name)
                        } else if let Ok(attrs) = tag.resolve_attributes(&raw) {
                            if tag.name == "img" {
                                let html = self.image_html(
                                    attrs.destination.as_deref().unwrap_or(""),
                                    attrs.alternate.as_deref().unwrap_or(""),
                                    attrs.title.as_deref(),
                                )?;
                                apply_image_dimensions(html, &attrs)
                            } else if tag.name == "a"
                                && attrs
                                    .destination
                                    .as_deref()
                                    .is_some_and(|url| safe_link(url).is_none())
                            {
                                self.fallback("链接地址不在允许范围", &raw, false)
                            } else {
                                self.open_tag(&tag, &attrs)
                            }
                        } else {
                            self.fallback("未支持或不安全的 HTML 属性，已保留源码", &raw, false)
                        }
                    } else {
                        self.fallback("未支持或不安全的 HTML，已保留源码", &raw, false)
                    };
                    Some(html)
                }
                _ => None,
            };
            if let Some(html) = replacement {
                let children: Vec<_> = node.children().collect();
                for child in children {
                    child.detach();
                }
                node.data_mut().value = NodeValue::HtmlInline(html);
            }
        }
        Ok(())
    }
}
fn apply_image_dimensions(mut html: String, attrs: &HtmlAttributes) -> String {
    if !html.starts_with("<img ") {
        return html;
    }
    let mut css = String::new();
    for (key, value) in [("width", attrs.width), ("height", attrs.height)] {
        if let Some(value) = value {
            let (number, unit) = match value {
                HtmlDimension::Points(n) => (n, "px"),
                HtmlDimension::Percent(n) => (n, "%"),
            };
            css.push_str(&format!("{key}:{number}{unit};"));
        }
    }
    if !css.is_empty() {
        html.insert_str(html.len() - 1, &format!(" style=\"{css}\""));
    }
    html
}
