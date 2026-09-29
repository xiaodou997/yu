//! PDF flow adapter for the already-resolved, inert whole-document HTML.
//! This reuses Yu's fragment parser and table grid, not a second Markdown or
//! browser engine. Numbering, resource identity and links remain owned by the
//! document writer. Native code receives only resolved text, boxes and spans.
use crate::document::{HtmlDocument, MAX_OUTPUT_BYTES};
use serde_json::{Value, json};
use yu_core::{ByteOffset, TextRange};
use yu_markdown::html::{HtmlCellSpan, HtmlFragment, HtmlNodeKind, HtmlTableGrid, HtmlTag};

pub const MAX_PDF_PAGES: usize = 1000;
pub const MIN_FIGURE_SCALE: f64 = 0.25;

#[derive(Clone, Copy, Debug)]
pub struct PageSettings {
    pub width: f64,
    pub height: f64,
    pub margin: f64,
    pub page_numbers: bool,
}
impl PageSettings {
    pub fn from_config(config: &Value) -> Result<Self, String> {
        let (mut width, mut height) =
            match config.get("paper").and_then(Value::as_str).unwrap_or("A4") {
                "A4" => (595.2756, 841.8898),
                "Letter" => (612.0, 792.0),
                _ => return Err("PDF 纸张必须为 A4 或 Letter".into()),
            };
        if config
            .get("landscape")
            .and_then(Value::as_bool)
            .unwrap_or(false)
        {
            std::mem::swap(&mut width, &mut height);
        }
        let margin = match config.get("margin") {
            None => 44.0,
            Some(value) => value.as_f64().ok_or("PDF 页边距无效")?,
        };
        if !margin.is_finite() || !(18.0..=144.0).contains(&margin) {
            return Err("PDF 页边距必须为 18–144 pt".into());
        }
        Ok(Self {
            width,
            height,
            margin,
            page_numbers: config
                .get("pageNumbers")
                .and_then(Value::as_bool)
                .unwrap_or(true),
        })
    }
}

#[derive(Clone, Default)]
struct Style {
    bold: bool,
    italic: bool,
    mono: bool,
    strike: bool,
    underline: bool,
    highlight: bool,
    scale: f64,
    rise: f64,
    link: Option<String>,
}
impl Style {
    fn normal() -> Self {
        Self {
            scale: 1.0,
            ..Self::default()
        }
    }
    fn run(&self, text: &str) -> Value {
        json!({"text":text,"bold":self.bold,"italic":self.italic,"mono":self.mono,
            "strike":self.strike,"underline":self.underline,"highlight":self.highlight,
            "scale":self.scale,"rise":self.rise,"link":self.link})
    }
}
struct Adapter<'a> {
    raw: &'a str,
    tree: HtmlFragment,
    images: Vec<Value>,
}
impl Adapter<'_> {
    fn text(&self, range: TextRange) -> &str {
        &self.raw[range.start().get() as usize..range.end().get() as usize]
    }
    fn attr(&self, tag: &HtmlTag, name: &str) -> Option<String> {
        tag.unique_attribute(name)
            .ok()
            .flatten()
            .and_then(|a| a.value)
            .map(|r| yu_markdown::image_markup::decode_image_text(self.text(r), true))
    }
    // Only the finite declarations emitted by Yu's validated writer are read;
    // this is not a user-CSS interpreter or a browser layout pass.
    fn css(&self, tag: &HtmlTag, property: &str) -> Option<String> {
        self.attr(tag, "style")?
            .split(';')
            .filter_map(|part| part.split_once(':'))
            .find(|(key, _)| key.trim() == property)
            .map(|(_, value)| value.trim().to_owned())
    }
    fn alignment(&self, tag: &HtmlTag) -> Option<String> {
        self.attr(tag, "align")
            .or_else(|| self.css(tag, "text-align"))
            .filter(|value| matches!(value.as_str(), "left" | "right" | "center" | "justify"))
    }
    fn tag(&self, id: usize) -> Option<&HtmlTag> {
        match &self.tree.nodes[id].kind {
            HtmlNodeKind::Element { opening, .. } => Some(opening),
            _ => None,
        }
    }
    fn inline(
        &mut self,
        id: usize,
        style: &Style,
        preserve: bool,
        out: &mut Vec<Value>,
    ) -> Result<(), String> {
        let node = self.tree.nodes[id].clone();
        let HtmlNodeKind::Element { opening, .. } = node.kind else {
            if matches!(node.kind, HtmlNodeKind::Text) {
                let text =
                    yu_markdown::image_markup::decode_image_text(self.text(node.source), true);
                let text = if preserve {
                    text
                } else {
                    let mut normalized = String::new();
                    let mut space = false;
                    for c in text.chars() {
                        if c.is_ascii_whitespace() {
                            if !space {
                                normalized.push(' ');
                            }
                            space = true;
                        } else {
                            normalized.push(c);
                            space = false;
                        }
                    }
                    normalized
                };
                if !text.is_empty() {
                    out.push(style.run(&text));
                }
            }
            return Ok(());
        };
        let mut next = style.clone();
        if let Some(anchor) = self.attr(&opening, "id") {
            out.push(json!({"text":"","anchor":anchor}));
        }
        match opening.name.as_str() {
            "strong" | "b" => next.bold = true,
            "em" | "i" => next.italic = true,
            "code" | "pre" => next.mono = true,
            "s" | "del" | "strike" => next.strike = true,
            "u" => next.underline = true,
            "mark" => next.highlight = true,
            "sup" => {
                next.scale *= 0.75;
                next.rise += 4.0;
            }
            "sub" => {
                next.scale *= 0.75;
                next.rise -= 2.0;
            }
            "a" => {
                next.link = self.attr(&opening, "href");
            }
            "br" => {
                out.push(next.run("\n"));
                return Ok(());
            }
            "input" => {
                out.push(next.run(
                    if opening.unique_attribute("checked").ok().flatten().is_some() {
                        "☑ "
                    } else {
                        "☐ "
                    },
                ));
                return Ok(());
            }
            "img" => {
                let uri = self.attr(&opening, "src").ok_or("PDF 图片身份丢失")?;
                if !uri.starts_with("data:image/png;base64,")
                    && !uri.starts_with("data:image/svg+xml;base64,")
                {
                    return Err("PDF 只接受已冻结的内嵌图片".into());
                }
                let index = self.images.len();
                let dimension = |name, percent| {
                    self.css(&opening, name)
                        .or_else(|| self.attr(&opening, name))
                        .filter(|v| v.ends_with('%') == percent)
                        .and_then(|v| {
                            v.trim_end_matches('%')
                                .trim_end_matches("px")
                                .parse::<f64>()
                                .ok()
                        })
                        .filter(|n| n.is_finite() && *n > 0.0)
                };
                let width = dimension("width", false);
                let height = dimension("height", false);
                let width_percent = dimension("width", true);
                let height_percent = dimension("height", true);
                let descent = self
                    .attr(&opening, "style")
                    .and_then(|s| {
                        s.strip_prefix("vertical-align:")
                            .and_then(|s| s.strip_suffix("px"))
                            .and_then(|s| s.parse::<f64>().ok())
                    })
                    .map(|n| (-n).max(0.0))
                    .unwrap_or(0.0);
                self.images.push(json!({"uri":uri,"width":width,"height":height,"widthPercent":width_percent,"heightPercent":height_percent,"descent":descent}));
                let mut run = next.run("");
                run["image"] = json!(index);
                out.push(run);
                return Ok(());
            }
            "p" | "div" | "span" | "li" | "summary" | "details" | "h1" | "h2" | "h3" | "h4"
            | "h5" | "h6" => {}
            "ul" | "ol" => {}
            name => return Err(format!("PDF 未知内部行内元素：{name}")),
        }
        for child in node.children {
            if matches!(self.tag(child).map(|t| t.name.as_str()), Some("p" | "li"))
                && !out.is_empty()
            {
                out.push(next.run("\n"));
            }
            self.inline(child, &next, preserve || next.mono, out)?;
        }
        Ok(())
    }
    fn paragraph(
        blocks: &mut Vec<Value>,
        runs: &mut Vec<Value>,
        prefix: &mut Vec<Value>,
        kind: &str,
        indent: usize,
        align: &str,
    ) {
        if runs.is_empty() && prefix.is_empty() {
            return;
        }
        let mut all = std::mem::take(prefix);
        all.append(runs);
        if all.iter().all(|r| {
            r.get("image").is_none()
                && r.get("anchor").is_none()
                && r["text"].as_str().unwrap_or("").trim().is_empty()
        }) {
            return;
        }
        blocks.push(
            json!({"kind":"paragraph","style":kind,"indent":indent,"align":align,"runs":all}),
        );
    }
    fn sequence(
        &mut self,
        nodes: &[usize],
        indent: usize,
        kind: &str,
        align: &str,
        mut prefix: Vec<Value>,
        blocks: &mut Vec<Value>,
    ) -> Result<(), String> {
        let mut runs = Vec::new();
        for &id in nodes {
            let Some(tag) = self.tag(id).cloned() else {
                self.inline(id, &Style::normal(), false, &mut runs)?;
                continue;
            };
            match tag.name.as_str() {
                "p" | "h1" | "h2" | "h3" | "h4" | "h5" | "h6" | "pre" | "summary" => {
                    // Formatting-only whitespace between block elements must not consume a list prefix.
                    if runs.iter().any(|r| {
                        !r["text"].as_str().unwrap_or("").trim().is_empty()
                            || r.get("image").is_some()
                    }) {
                        Self::paragraph(blocks, &mut runs, &mut prefix, kind, indent, align);
                    } else {
                        runs.clear();
                    }
                    let style = match tag.name.as_str() {
                        "pre" => "code",
                        "summary" => "h3",
                        "p" => kind,
                        other => other,
                    };
                    let alignment = self.alignment(&tag).unwrap_or_else(|| align.to_owned());
                    self.inline(id, &Style::normal(), tag.name == "pre", &mut runs)?;
                    Self::paragraph(blocks, &mut runs, &mut prefix, style, indent, &alignment);
                }
                "ul" | "ol" => {
                    Self::paragraph(blocks, &mut runs, &mut prefix, kind, indent, align);
                    let mut number = self
                        .attr(&tag, "start")
                        .and_then(|s| s.parse::<u64>().ok())
                        .unwrap_or(1);
                    for child in self.tree.nodes[id].children.clone() {
                        let Some(item) = self.tag(child).cloned() else {
                            continue;
                        };
                        if item.name != "li" {
                            return Err("PDF 列表层级无效".into());
                        }
                        if let Some(value) = self
                            .attr(&item, "value")
                            .and_then(|s| s.parse::<u64>().ok())
                        {
                            number = value;
                        }
                        let text = if kind == "toc" {
                            String::new()
                        } else if tag.name == "ol" {
                            format!("{number}. ")
                        } else {
                            "• ".into()
                        };
                        let mut marker = Style::normal().run(&text);
                        if let Some(anchor) = self.attr(&item, "id") {
                            marker["anchor"] = json!(anchor);
                        }
                        self.sequence(
                            &self.tree.nodes[child].children.clone(),
                            indent + 1,
                            kind,
                            align,
                            vec![marker],
                            blocks,
                        )?;
                        number = number.saturating_add(1);
                    }
                }
                "table" => {
                    Self::paragraph(blocks, &mut runs, &mut prefix, kind, indent, align);
                    blocks.push(self.table(id)?);
                }
                "hr" => {
                    Self::paragraph(blocks, &mut runs, &mut prefix, kind, indent, align);
                    blocks.push(json!({"kind":"rule"}));
                }
                "nav" | "section" | "details" | "blockquote" | "div" => {
                    Self::paragraph(blocks, &mut runs, &mut prefix, kind, indent, align);
                    if tag.name == "div" && self.attr(&tag, "class").as_deref() == Some("yu-figure")
                    {
                        self.inline(id, &Style::normal(), false, &mut runs)?;
                        Self::paragraph(blocks, &mut runs, &mut prefix, "figure", indent, "center");
                    } else {
                        let nested_kind = if tag.name == "nav" {
                            "toc"
                        } else if tag.name == "blockquote" {
                            "quote"
                        } else {
                            kind
                        };
                        let mut anchors = Vec::new();
                        if let Some(anchor) = self.attr(&tag, "id") {
                            anchors.push(json!({"text":"","anchor":anchor}));
                        }
                        let alignment = self.alignment(&tag).unwrap_or_else(|| align.to_owned());
                        self.sequence(
                            &self.tree.nodes[id].children.clone(),
                            indent + usize::from(tag.name == "blockquote"),
                            nested_kind,
                            &alignment,
                            anchors,
                            blocks,
                        )?;
                    }
                }
                _ => self.inline(id, &Style::normal(), false, &mut runs)?,
            }
        }
        Self::paragraph(blocks, &mut runs, &mut prefix, kind, indent, align);
        Ok(())
    }
    fn table(&mut self, id: usize) -> Result<Value, String> {
        let mut groups: Vec<Vec<Vec<usize>>> = Vec::new();
        let mut headers = 0;
        let mut direct = false;
        for child in self.tree.nodes[id].children.clone() {
            let Some(tag) = self.tag(child) else {
                continue;
            };
            if tag.name == "tr" {
                if !direct {
                    groups.push(Vec::new());
                    direct = true;
                }
                let row = self.tree.nodes[child]
                    .children
                    .iter()
                    .copied()
                    .filter(|&n| self.tag(n).is_some())
                    .collect();
                groups.last_mut().ok_or("PDF 表格分组丢失")?.push(row);
            } else if matches!(tag.name.as_str(), "thead" | "tbody" | "tfoot") {
                direct = false;
                let head = tag.name == "thead";
                let mut rows = Vec::new();
                for &row in &self.tree.nodes[child].children {
                    if self.tag(row).is_some_and(|t| t.name == "tr") {
                        rows.push(
                            self.tree.nodes[row]
                                .children
                                .iter()
                                .copied()
                                .filter(|&n| self.tag(n).is_some())
                                .collect::<Vec<_>>(),
                        );
                    }
                }
                if head {
                    headers += rows.len();
                }
                groups.push(rows);
            } else {
                return Err("PDF 表格分组无效".into());
            }
        }
        let spans = groups
            .iter()
            .map(|group| {
                group
                    .iter()
                    .map(|row| {
                        row.iter()
                            .map(|&cell| {
                                let tag = self.tag(cell).ok_or("PDF 表格单元格丢失")?;
                                if !matches!(tag.name.as_str(), "td" | "th") {
                                    return Err("PDF 表格单元格无效");
                                }
                                Ok(HtmlCellSpan {
                                    columns: self
                                        .attr(tag, "colspan")
                                        .and_then(|s| s.parse().ok())
                                        .unwrap_or(1),
                                    rows: self
                                        .attr(tag, "rowspan")
                                        .and_then(|s| s.parse().ok())
                                        .unwrap_or(1),
                                })
                            })
                            .collect::<Result<Vec<_>, _>>()
                    })
                    .collect::<Result<Vec<_>, _>>()
            })
            .collect::<Result<Vec<_>, _>>()?;
        let grid = HtmlTableGrid::build(&spans).map_err(|_| "PDF 表格合并网格无效或超限")?;
        if grid.columns == 0 || grid.rows == 0 {
            return Err("PDF 空表格".into());
        }
        let rows: Vec<_> = groups.into_iter().flatten().collect();
        let mut cells = Vec::new();
        for cell in &grid.cells {
            let node = rows[cell.source_row][cell.source_cell];
            let tag = self.tag(node).ok_or("PDF 单元格身份无效")?;
            let header = tag.name == "th";
            let alignment = self.alignment(tag).unwrap_or_else(|| "left".into());
            let mut runs: Vec<Value> = Vec::new();
            let mut style = Style::normal();
            style.bold = header;
            for child in self.tree.nodes[node].children.clone() {
                if self
                    .tag(child)
                    .is_some_and(|tag| matches!(tag.name.as_str(), "p" | "div"))
                    && runs.iter().any(|r| {
                        !r["text"].as_str().unwrap_or("").trim().is_empty()
                            || r.get("image").is_some()
                    })
                {
                    runs.push(style.run("\n"));
                }
                self.inline(child, &style, false, &mut runs)?;
            }
            cells.push(json!({"row":cell.source_row,"column":cell.column,"rows":cell.rows,"columns":cell.columns,"header":header,"align":alignment,"runs":runs}));
        }
        // A page may split only after every rowspan started in this band ends.
        let mut bands = Vec::new();
        let mut first = 0;
        while first < grid.rows {
            let mut end = first + 1;
            let mut cursor = first;
            while cursor < end {
                for cell in &grid.cells {
                    if cell.source_row == cursor {
                        end = end.max(cursor + cell.rows);
                    }
                }
                cursor += 1;
            }
            bands.push(json!([first, end]));
            first = end;
        }
        Ok(
            json!({"kind":"table","columns":grid.columns,"rows":grid.rows,"headers":headers,"cells":cells,"bands":bands}),
        )
    }
}

pub fn prepare_pdf_packet(
    document: &HtmlDocument,
    settings: PageSettings,
) -> Result<Vec<u8>, String> {
    prepare_flow_packet(document, settings, None)
}
pub(crate) fn prepare_flow_packet(
    document: &HtmlDocument,
    settings: PageSettings,
    png: Option<Value>,
) -> Result<Vec<u8>, String> {
    let raw = document
        .html
        .split_once("<main>")
        .and_then(|(_, s)| s.rsplit_once("</main>").map(|(body, _)| body))
        .ok_or("PDF 内容主体丢失")?;
    let tree = HtmlFragment::parse(raw, ByteOffset::ZERO).map_err(|_| "PDF 内部结构解析失败")?;
    if !tree.diagnostics.is_empty() {
        return Err("PDF 内部结构不完整或超过 100000 节点/128 层预算".into());
    }
    let roots = tree.roots.clone();
    let mut adapter = Adapter {
        raw,
        tree,
        images: Vec::new(),
    };
    let mut blocks = Vec::new();
    adapter.sequence(&roots, 0, "body", "left", Vec::new(), &mut blocks)?;
    let title = document
        .html
        .split_once("<title>")
        .and_then(|(_, s)| s.split_once("</title>").map(|(title, _)| title))
        .unwrap_or("Yu 文档");
    let mut packet = json!({"title":yu_markdown::image_markup::decode_image_text(title,true),"width":settings.width,"height":settings.height,"margin":settings.margin,"pageNumbers":settings.page_numbers,"maxPages":MAX_PDF_PAGES,"minFigureScale":MIN_FIGURE_SCALE,"blocks":blocks,"images":adapter.images});
    if let Some(png) = png {
        packet["maxPages"] = png["maxSegments"].clone();
        packet["png"] = png;
    }
    struct BoundedPacket(Vec<u8>);
    impl std::io::Write for BoundedPacket {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            let total = self
                .0
                .len()
                .checked_add(bytes.len())
                .filter(|&n| n <= MAX_OUTPUT_BYTES)
                .ok_or_else(|| std::io::Error::other("PDF 中间内容超过 256 MiB 预算"))?;
            if total > self.0.capacity() {
                self.0
                    .try_reserve_exact(
                        total.next_power_of_two().min(MAX_OUTPUT_BYTES) - self.0.len(),
                    )
                    .map_err(std::io::Error::other)?;
            }
            self.0.extend_from_slice(bytes);
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let mut data = BoundedPacket(Vec::new());
    serde_json::to_writer(&mut data, &packet)
        .map_err(|error| format!("PDF 内容序列化失败或超过预算：{error}"))?;
    Ok(data.0)
}
