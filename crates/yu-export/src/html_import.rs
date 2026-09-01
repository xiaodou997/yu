//! Strict, opt-in HTML-to-Markdown paste policy.
//!
//! This parser is intentionally not a browser HTML parser. It accepts only
//! the semantic subset Yu itself emits (plus equivalent safe markup), rejects
//! unknown tags/attributes, and returns Markdown source. Native adapters can
//! fall back to plain text whenever this policy rejects a fragment.

use std::error::Error;
use std::fmt;

use yu_markdown::TableAlignment;

const MAX_HTML_BYTES: usize = 8 * 1024 * 1024;
const MAX_NODES: usize = 100_000;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HtmlImportError {
    TooLarge,
    Malformed,
    InvalidEntity,
    UnsupportedTag(String),
    UnsupportedAttribute { tag: String, attribute: String },
    UnexpectedClosingTag(String),
    MismatchedClosingTag { expected: String, found: String },
    UnsafeUrl(String),
    InvalidStructure(&'static str),
}

impl fmt::Display for HtmlImportError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TooLarge => formatter.write_str("HTML fragment exceeds the import limit"),
            Self::Malformed => formatter.write_str("malformed HTML fragment"),
            Self::InvalidEntity => formatter.write_str("invalid HTML entity"),
            Self::UnsupportedTag(tag) => write!(formatter, "unsupported HTML tag <{tag}>"),
            Self::UnsupportedAttribute { tag, attribute } => {
                write!(formatter, "unsupported attribute {attribute:?} on <{tag}>")
            }
            Self::UnexpectedClosingTag(tag) => write!(formatter, "unexpected closing tag </{tag}>"),
            Self::MismatchedClosingTag { expected, found } => {
                write!(formatter, "expected </{expected}> but found </{found}>")
            }
            Self::UnsafeUrl(url) => write!(formatter, "unsafe URL in HTML fragment: {url:?}"),
            Self::InvalidStructure(message) => formatter.write_str(message),
        }
    }
}

impl Error for HtmlImportError {}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Attribute {
    name: String,
    value: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Element {
    name: String,
    attributes: Vec<Attribute>,
    children: Vec<Node>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Node {
    Text(String),
    Element(Element),
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct ImportedTableCell {
    markdown: String,
    alignment: TableAlignment,
}

/// Converts an allowlisted semantic HTML fragment to Markdown source.
///
/// This function is deliberately opt-in. It never executes, sanitizes into,
/// or renders HTML; unsupported content returns an error so the caller can
/// fall back to `text/plain` instead of guessing a second document model.
pub fn import_html_fragment(html: &str) -> Result<String, HtmlImportError> {
    if html.len() > MAX_HTML_BYTES {
        return Err(HtmlImportError::TooLarge);
    }
    let mut roots = parse_document(html)?;
    strip_encoding_declaration(&mut roots);
    render_roots(&roots)
}

/// 摘掉顶层的 `<meta charset=...>`。
///
/// 它是**信封不是内容**：剪贴板的 `public.html` 只承载字节，编码要靠这一句
/// 声明（Yu 自己发的、Chrome 发的，第一个标签都是它）。不摘掉的话它会以
/// 「块之间冒出一个行内元素」的形状让整段被拒——于是 Yu 接不住自己发出去的
/// 剪贴板 HTML。
///
/// 只摘顶层是有意的：嵌在 `<p>` 里的 `<meta>` 不是编码声明，继续拒。
fn strip_encoding_declaration(roots: &mut Vec<Node>) {
    roots.retain(|node| !matches!(node, Node::Element(element) if element.name == "meta"));
}

fn parse_document(html: &str) -> Result<Vec<Node>, HtmlImportError> {
    let mut roots = Vec::new();
    let mut stack = Vec::<Element>::new();
    let mut position = 0;
    let mut node_count = 0;

    while position < html.len() {
        if html.as_bytes()[position] == b'<' {
            if html[position..].starts_with("<!--") {
                return Err(HtmlImportError::Malformed);
            }
            if html.as_bytes().get(position + 1) == Some(&b'/') {
                let (name, next) = parse_close_tag(html, position)?;
                let element = stack
                    .pop()
                    .ok_or_else(|| HtmlImportError::UnexpectedClosingTag(name.clone()))?;
                if element.name != name {
                    return Err(HtmlImportError::MismatchedClosingTag {
                        expected: element.name,
                        found: name,
                    });
                }
                append_node(&mut roots, &mut stack, Node::Element(element));
                position = next;
            } else {
                let (name, attributes, self_closing, next) = parse_open_tag(html, position)?;
                ensure_allowed_tag(&name)?;
                validate_attributes(&name, &attributes)?;
                let element = Element {
                    name: name.clone(),
                    attributes,
                    children: Vec::new(),
                };
                node_count += 1;
                if node_count > MAX_NODES {
                    return Err(HtmlImportError::TooLarge);
                }
                if self_closing || is_void_tag(&name) {
                    append_node(&mut roots, &mut stack, Node::Element(element));
                } else {
                    stack.push(element);
                }
                position = next;
            }
        } else {
            let next = html[position..]
                .find('<')
                .map_or(html.len(), |offset| position + offset);
            let text = decode_entities(&html[position..next])?;
            if !text.is_empty() {
                node_count += 1;
                if node_count > MAX_NODES {
                    return Err(HtmlImportError::TooLarge);
                }
                append_node(&mut roots, &mut stack, Node::Text(text));
            }
            position = next;
        }
    }

    if let Some(element) = stack.pop() {
        return Err(HtmlImportError::MismatchedClosingTag {
            expected: element.name,
            found: "EOF".to_owned(),
        });
    }
    Ok(roots)
}

fn append_node(roots: &mut Vec<Node>, stack: &mut [Element], node: Node) {
    if let Some(element) = stack.last_mut() {
        element.children.push(node);
    } else {
        roots.push(node);
    }
}

fn parse_open_tag(
    html: &str,
    start: usize,
) -> Result<(String, Vec<Attribute>, bool, usize), HtmlImportError> {
    let mut quote = None;
    let mut end = None;
    for (offset, character) in html[start + 1..].char_indices() {
        match (quote, character) {
            (Some(expected), value) if value == expected => quote = None,
            (None, '\'' | '"') => quote = Some(character),
            (None, '>') => {
                end = Some(start + 1 + offset);
                break;
            }
            _ => {}
        }
    }
    let end = end.ok_or(HtmlImportError::Malformed)?;
    if quote.is_some() {
        return Err(HtmlImportError::Malformed);
    }

    let mut body = html[start + 1..end].trim();
    let self_closing = body.ends_with('/');
    if self_closing {
        body = body[..body.len() - 1].trim_end();
    }
    let (name, mut position) = read_name(body, 0).ok_or(HtmlImportError::Malformed)?;
    let name = name.to_ascii_lowercase();
    let mut attributes = Vec::new();
    while position < body.len() {
        position = skip_ascii_whitespace(body, position);
        if position == body.len() {
            break;
        }
        let (attribute, next) = read_name(body, position).ok_or(HtmlImportError::Malformed)?;
        let attribute = attribute.to_ascii_lowercase();
        position = skip_ascii_whitespace(body, next);
        let value = if body.as_bytes().get(position) == Some(&b'=') {
            position = skip_ascii_whitespace(body, position + 1);
            let quote = *body
                .as_bytes()
                .get(position)
                .ok_or(HtmlImportError::Malformed)?;
            if quote != b'\'' && quote != b'"' {
                return Err(HtmlImportError::Malformed);
            }
            position += 1;
            let value_start = position;
            while position < body.len() && body.as_bytes()[position] != quote {
                position += 1;
            }
            let value_end = position;
            if position == body.len() {
                return Err(HtmlImportError::Malformed);
            }
            position += 1;
            decode_entities(&body[value_start..value_end])?
        } else if matches!(attribute.as_str(), "checked" | "disabled") {
            String::new()
        } else {
            return Err(HtmlImportError::Malformed);
        };
        if attributes
            .iter()
            .any(|item: &Attribute| item.name == attribute)
        {
            return Err(HtmlImportError::Malformed);
        }
        attributes.push(Attribute {
            name: attribute,
            value,
        });
    }
    Ok((name, attributes, self_closing, end + 1))
}

fn parse_close_tag(html: &str, start: usize) -> Result<(String, usize), HtmlImportError> {
    let end = html[start + 2..]
        .find('>')
        .map(|offset| start + 2 + offset)
        .ok_or(HtmlImportError::Malformed)?;
    let body = html[start + 2..end].trim();
    let (name, position) = read_name(body, 0).ok_or(HtmlImportError::Malformed)?;
    if !body[position..].trim().is_empty() {
        return Err(HtmlImportError::Malformed);
    }
    Ok((name.to_ascii_lowercase(), end + 1))
}

fn read_name(value: &str, start: usize) -> Option<(&str, usize)> {
    let bytes = value.as_bytes();
    let mut end = start;
    while end < bytes.len()
        && (bytes[end].is_ascii_alphanumeric() || matches!(bytes[end], b'-' | b':' | b'_'))
    {
        end += 1;
    }
    (end > start).then_some((&value[start..end], end))
}

fn skip_ascii_whitespace(value: &str, mut position: usize) -> usize {
    while position < value.len() && value.as_bytes()[position].is_ascii_whitespace() {
        position += 1;
    }
    position
}

fn is_void_tag(name: &str) -> bool {
    matches!(name, "br" | "img" | "input" | "meta")
}

fn ensure_allowed_tag(name: &str) -> Result<(), HtmlImportError> {
    if matches!(
        name,
        "p" | "h1"
            | "h2"
            | "h3"
            | "h4"
            | "h5"
            | "h6"
            | "strong"
            | "em"
            | "code"
            | "a"
            | "img"
            | "br"
            | "pre"
            | "blockquote"
            | "ul"
            | "ol"
            | "li"
            | "input"
            | "table"
            | "thead"
            | "tbody"
            | "tr"
            | "th"
            | "td"
            | "hr"
            | "meta"
    ) {
        Ok(())
    } else {
        Err(HtmlImportError::UnsupportedTag(name.to_owned()))
    }
}

fn validate_attributes(name: &str, attributes: &[Attribute]) -> Result<(), HtmlImportError> {
    // S7 第六刀放宽了三处，全部是**因为 Yu 自己的导出现在会发出它们**：
    // comrak 带链接与图片的 `title`，用 GFM 的 `align=` 而不是行内 style，
    // 并且把主题分隔线渲染成 `<hr />`。不放宽的表现不是报错，是 Yu 里 ⌘C 再
    // ⌘V 静静降级成纯文本。**原始 HTML（`<div>`/`<script>`）继续拒**：导出的
    // 是用户自己的文档，导入的是别人的 HTML，这个不对称是有意的。
    let allowed = match name {
        "a" => &["href", "title"][..],
        "img" => &["src", "alt", "title"],
        "ol" => &["start"],
        "code" => &["class"],
        "input" => &["type", "disabled", "checked"],
        // 编码声明只认 charset。`http-equiv` / `content` 那种写法带着一个
        // 完整的 MIME 串，认它就等于开始解析 HTTP 头——那不是这个模块的事。
        "meta" => &["charset"],
        "th" | "td" => &["style", "align"],
        _ => &[][..],
    };
    for attribute in attributes {
        if !allowed.contains(&attribute.name.as_str()) {
            return Err(HtmlImportError::UnsupportedAttribute {
                tag: name.to_owned(),
                attribute: attribute.name.clone(),
            });
        }
    }
    if let Some(value) = attribute(attributes, "href").or_else(|| attribute(attributes, "src")) {
        validate_url(value)?;
    }
    if name == "input"
        && (attribute(attributes, "type") != Some("checkbox")
            || attribute(attributes, "disabled").is_none())
    {
        return Err(HtmlImportError::InvalidStructure(
            "only disabled checkbox inputs are allowed",
        ));
    }
    // CommonMark 允许 `0.` 起头的有序列表，comrak 因此会发 `start="0"`。
    // 这里以前要求严格为正，于是 Yu 自己导出的那一份粘不回来。
    if name == "ol"
        && let Some(value) = attribute(attributes, "start")
        && value.parse::<u64>().is_err()
    {
        return Err(HtmlImportError::InvalidStructure(
            "ordered list start must be a non-negative integer",
        ));
    }
    if name == "code"
        && let Some(value) = attribute(attributes, "class")
        && (!value.starts_with("language-") || value.len() == "language-".len())
    {
        return Err(HtmlImportError::InvalidStructure(
            "code class must use language-*",
        ));
    }
    if matches!(name, "th" | "td") {
        if let Some(value) = attribute(attributes, "style")
            && !matches!(
                value.trim().to_ascii_lowercase().as_str(),
                "text-align: left" | "text-align: center" | "text-align: right"
            )
        {
            return Err(HtmlImportError::InvalidStructure(
                "table alignment style is not allowlisted",
            ));
        }
        if let Some(value) = attribute(attributes, "align")
            && !matches!(
                value.trim().to_ascii_lowercase().as_str(),
                "left" | "center" | "right"
            )
        {
            return Err(HtmlImportError::InvalidStructure(
                "table alignment attribute is not allowlisted",
            ));
        }
    }
    Ok(())
}

fn attribute<'a>(attributes: &'a [Attribute], name: &str) -> Option<&'a str> {
    attributes
        .iter()
        .find(|attribute| attribute.name == name)
        .map(|attribute| attribute.value.as_str())
}

fn validate_url(value: &str) -> Result<(), HtmlImportError> {
    let trimmed = value.trim();
    let lower = trimmed.to_ascii_lowercase();
    if trimmed.chars().any(|character| character.is_control())
        || lower.starts_with("javascript:")
        || lower.starts_with("vbscript:")
        || lower.starts_with("data:")
    {
        return Err(HtmlImportError::UnsafeUrl(value.to_owned()));
    }
    Ok(())
}

fn decode_entities(value: &str) -> Result<String, HtmlImportError> {
    let mut output = String::with_capacity(value.len());
    let mut position = 0;
    while position < value.len() {
        let Some(relative) = value[position..].find('&') else {
            output.push_str(&value[position..]);
            break;
        };
        let start = position + relative;
        output.push_str(&value[position..start]);
        let Some(end_relative) = value[start..].find(';') else {
            return Err(HtmlImportError::InvalidEntity);
        };
        let end = start + end_relative;
        let entity = &value[start + 1..end];
        let decoded = match entity {
            "amp" => '&',
            "lt" => '<',
            "gt" => '>',
            "quot" => '"',
            "apos" => '\'',
            "nbsp" => ' ',
            _ if entity.starts_with("#x") || entity.starts_with("#X") => {
                let number = u32::from_str_radix(&entity[2..], 16)
                    .map_err(|_| HtmlImportError::InvalidEntity)?;
                char::from_u32(number).ok_or(HtmlImportError::InvalidEntity)?
            }
            _ if entity.starts_with('#') => {
                let number = entity[1..]
                    .parse::<u32>()
                    .map_err(|_| HtmlImportError::InvalidEntity)?;
                char::from_u32(number).ok_or(HtmlImportError::InvalidEntity)?
            }
            _ => return Err(HtmlImportError::InvalidEntity),
        };
        output.push(decoded);
        position = end + 1;
    }
    Ok(output)
}

fn render_roots(roots: &[Node]) -> Result<String, HtmlImportError> {
    let has_block = roots
        .iter()
        .any(|node| matches!(node, Node::Element(element) if is_block_tag(&element.name)));
    if !has_block {
        return render_inline_nodes(roots);
    }

    let mut blocks = Vec::new();
    for node in roots {
        match node {
            Node::Text(text) if text.trim().is_empty() => {}
            Node::Element(element) if is_block_tag(&element.name) => {
                blocks.push(render_block(element)?);
            }
            Node::Text(_) => {
                return Err(HtmlImportError::InvalidStructure(
                    "non-whitespace text cannot appear between block elements",
                ));
            }
            Node::Element(_element) => {
                return Err(HtmlImportError::InvalidStructure(
                    "inline element cannot appear at fragment root beside blocks",
                ));
            }
        }
    }
    Ok(blocks.join("\n\n"))
}

fn is_block_tag(name: &str) -> bool {
    matches!(
        name,
        "p" | "h1"
            | "h2"
            | "h3"
            | "h4"
            | "h5"
            | "h6"
            | "pre"
            | "blockquote"
            | "ul"
            | "ol"
            | "table"
            | "hr"
    )
}

fn render_block(element: &Element) -> Result<String, HtmlImportError> {
    match element.name.as_str() {
        "p" => render_inline_nodes(&element.children),
        // **必须核对那一位是 1..=6，不能只看「`h` 开头、两个字符」。**
        // `hr` 正好是那个形状：`b'r' - b'0'` = 66，于是一条主题分隔线被粘成
        // 一个 66 级标题（66 个 `#`）——不 panic、不报错。S7 第六刀给导入器
        // 加 `<hr>` 时由 `export_import_export_is_a_fixed_point` 抓到，
        // 在此之前 Yu 的导出器不发 `<hr>`，这一支永远走不到。
        name if matches!(name.as_bytes(), [b'h', level] if level.is_ascii_digit()
            && *level != b'0'
            && *level <= b'6') =>
        {
            let level = name.as_bytes()[1] - b'0';
            Ok(format!(
                "{} {}",
                "#".repeat(level as usize),
                render_inline_nodes(&element.children)?
            ))
        }
        "blockquote" => {
            let inner = render_roots(&element.children)?;
            Ok(inner
                .lines()
                .map(|line| format!("> {line}"))
                .collect::<Vec<_>>()
                .join("\n"))
        }
        "pre" => render_pre(element),
        "ul" | "ol" => render_list(element),
        "table" => render_table(element),
        // 主题分隔线回写成 `---`。**它紧跟在一个段落后面就是 Setext 二级
        // 标题**，所以这里只发标记本身，块与块之间的那个空行由
        // `render_roots` 统一插——`join("\n\n")`，那个空行正是把两者分开的
        // 东西。少了它不报错，只是一条分隔线变成了上一段的下划线。
        "hr" => Ok("---".to_owned()),
        _ => Err(HtmlImportError::InvalidStructure(
            "unsupported block structure",
        )),
    }
}

fn render_inline_nodes(nodes: &[Node]) -> Result<String, HtmlImportError> {
    let mut output = String::new();
    // `<br />` 后面紧跟的那一个换行是 HTML 的排版空白，不是内容。
    //
    // 几乎每个 Markdown 渲染器（comrak、cmark、marked）都发 `<br />\n`，而
    // 硬换行本身要回写成 `"  \n"`——两个加起来是一个空行，于是**一个段落被
    // 粘成了两个**。不报错、不丢字，只是多了一次分段。S7 第六刀由
    // `export_import_export_is_a_fixed_point` 抓出来：在此之前 Yu 自己的
    // 导出器不发 `<br>`，语料里也就没有这个形状。
    let mut after_hard_break = false;
    for node in nodes {
        match node {
            Node::Text(text) => {
                let text = if after_hard_break {
                    text.strip_prefix('\n').unwrap_or(text)
                } else {
                    text.as_str()
                };
                escape_markdown_text(text, &mut output);
            }
            Node::Element(element) => match element.name.as_str() {
                "strong" => {
                    output.push_str("**");
                    output.push_str(&render_inline_nodes(&element.children)?);
                    output.push_str("**");
                }
                "em" => {
                    output.push('*');
                    output.push_str(&render_inline_nodes(&element.children)?);
                    output.push('*');
                }
                "code" => {
                    let text = text_only(&element.children)?;
                    output.push_str(&inline_code(&text));
                }
                "a" => {
                    let href = attribute(&element.attributes, "href")
                        .ok_or(HtmlImportError::InvalidStructure("link is missing href"))?;
                    output.push('[');
                    output.push_str(&render_inline_nodes(&element.children)?);
                    output.push_str("](");
                    escape_destination(href, &mut output);
                    push_title(attribute(&element.attributes, "title"), &mut output);
                    output.push(')');
                }
                "img" => {
                    let src = attribute(&element.attributes, "src")
                        .ok_or(HtmlImportError::InvalidStructure("image is missing src"))?;
                    let alt = attribute(&element.attributes, "alt").unwrap_or("");
                    output.push_str("![");
                    escape_markdown_text(alt, &mut output);
                    output.push_str("](");
                    escape_destination(src, &mut output);
                    push_title(attribute(&element.attributes, "title"), &mut output);
                    output.push(')');
                }
                "br" => output.push_str("  \n"),
                _ => {
                    return Err(HtmlImportError::InvalidStructure(
                        "block element is not valid inline content",
                    ));
                }
            },
        }
        after_hard_break = matches!(node, Node::Element(element) if element.name == "br");
    }
    Ok(output)
}

/// `[文字](/目标 "标题")` 的那一半。
///
/// 双引号与反斜杠要转义——不转的话一个带引号的 title 会把 Markdown 的目标
/// 括号提前关掉，粘回来的是一段坏掉的源码，而且**导入这一侧不报错**。
fn push_title(title: Option<&str>, output: &mut String) {
    let Some(title) = title else {
        return;
    };
    output.push_str(" \"");
    for character in title.chars() {
        if matches!(character, '"' | '\\') {
            output.push('\\');
        }
        output.push(character);
    }
    output.push('"');
}

fn render_pre(element: &Element) -> Result<String, HtmlImportError> {
    if element.children.len() != 1 {
        return Err(HtmlImportError::InvalidStructure(
            "pre must contain one code element",
        ));
    }
    let Node::Element(code) = &element.children[0] else {
        return Err(HtmlImportError::InvalidStructure(
            "pre must contain one code element",
        ));
    };
    if code.name != "code" {
        return Err(HtmlImportError::InvalidStructure(
            "pre must contain one code element",
        ));
    }
    let body = text_only(&code.children)?;
    let fence_len = max_backtick_run(&body).saturating_add(1).max(3);
    let fence = "`".repeat(fence_len);
    let language = attribute(&code.attributes, "class")
        .and_then(|class| class.strip_prefix("language-"))
        .unwrap_or("");
    let mut output = format!("{fence}{language}\n{body}");
    if !output.ends_with('\n') {
        output.push('\n');
    }
    output.push_str(&fence);
    Ok(output)
}

fn render_list(element: &Element) -> Result<String, HtmlImportError> {
    let ordered = element.name == "ol";
    let start = attribute(&element.attributes, "start")
        .map(|value| value.parse::<u64>())
        .transpose()
        .map_err(|_| HtmlImportError::InvalidStructure("invalid ordered list start"))?
        .unwrap_or(1);
    let mut items = Vec::new();
    for child in &element.children {
        if let Node::Text(text) = child {
            if text.trim().is_empty() {
                continue;
            }
            return Err(HtmlImportError::InvalidStructure(
                "list contains non-whitespace text",
            ));
        }
        let Node::Element(item) = child else {
            unreachable!("handled text above");
        };
        if item.name != "li" {
            return Err(HtmlImportError::InvalidStructure(
                "list must contain li elements",
            ));
        }
        items.push(render_list_item(item, ordered));
    }
    let mut output = Vec::new();
    for (index, item) in items.into_iter().enumerate() {
        let marker = if ordered {
            let number =
                start
                    .checked_add(index as u64)
                    .ok_or(HtmlImportError::InvalidStructure(
                        "ordered list numbering overflows",
                    ))?;
            format!("{number}.")
        } else {
            "-".to_owned()
        };
        output.push(format!("{marker} {}", item?));
    }
    Ok(output.join("\n"))
}

fn render_list_item(element: &Element, _ordered: bool) -> Result<String, HtmlImportError> {
    let mut content = String::new();
    let mut nested = Vec::new();
    for child in &element.children {
        match child {
            Node::Text(text) if text.trim().is_empty() => content.push_str(text),
            Node::Text(text) => escape_markdown_text(text, &mut content),
            Node::Element(child) => match child.name.as_str() {
                "input" => {
                    let checked = attribute(&child.attributes, "checked").is_some();
                    content.push_str(if checked { "[x]" } else { "[ ]" });
                }
                "p" => content.push_str(&render_inline_nodes(&child.children)?),
                "ul" | "ol" => nested.push(render_list(child)?),
                _ => content.push_str(&render_inline_nodes(&[Node::Element(child.clone())])?),
            },
        }
    }
    let mut output = normalize_task_marker(content.trim());
    for list in nested {
        if !output.is_empty() {
            output.push('\n');
        }
        output.push_str(
            &list
                .lines()
                .map(|line| format!("  {line}"))
                .collect::<Vec<_>>()
                .join("\n"),
        );
    }
    Ok(output)
}

fn normalize_task_marker(content: &str) -> String {
    for marker in ["[x]", "[ ]"] {
        if let Some(rest) = content.strip_prefix(marker) {
            if rest.is_empty() {
                return marker.to_owned();
            }
            return format!("{marker} {}", rest.trim_start());
        }
    }
    content.to_owned()
}

fn render_table(element: &Element) -> Result<String, HtmlImportError> {
    let mut header = None;
    let mut rows = Vec::new();
    for child in &element.children {
        let Node::Element(section) = child else {
            if matches!(child, Node::Text(text) if text.trim().is_empty()) {
                continue;
            }
            return Err(HtmlImportError::InvalidStructure(
                "table has invalid children",
            ));
        };
        match section.name.as_str() {
            "thead" => {
                if header.is_some() {
                    return Err(HtmlImportError::InvalidStructure(
                        "table has duplicate thead",
                    ));
                }
                header = Some(read_table_section(section, "th")?);
            }
            "tbody" => rows.extend(read_table_section(section, "td")?),
            _ => {
                return Err(HtmlImportError::InvalidStructure(
                    "table requires thead/tbody",
                ));
            }
        }
    }
    let header = header.ok_or(HtmlImportError::InvalidStructure("table is missing thead"))?;
    if header.len() != 1 || header[0].is_empty() {
        return Err(HtmlImportError::InvalidStructure(
            "table requires one header row",
        ));
    }
    let width = header[0].len();
    if rows.iter().any(|row| row.len() != width) {
        return Err(HtmlImportError::InvalidStructure("table row widths differ"));
    }
    let mut output = Vec::new();
    output.push(render_table_row(&header[0]));
    output.push(
        header[0]
            .iter()
            .map(|cell| table_alignment_marker(cell.alignment))
            .collect::<Vec<_>>()
            .join(" | "),
    );
    output.extend(rows.iter().map(|row| render_table_row(row)));
    Ok(output
        .into_iter()
        .map(|line| format!("| {line} |"))
        .collect::<Vec<_>>()
        .join("\n"))
}

fn read_table_section(
    section: &Element,
    cell_name: &str,
) -> Result<Vec<Vec<ImportedTableCell>>, HtmlImportError> {
    let mut rows = Vec::new();
    for child in &section.children {
        let Node::Element(row) = child else {
            if matches!(child, Node::Text(text) if text.trim().is_empty()) {
                continue;
            }
            return Err(HtmlImportError::InvalidStructure(
                "table section has invalid children",
            ));
        };
        if row.name != "tr" {
            return Err(HtmlImportError::InvalidStructure(
                "table section requires tr",
            ));
        }
        let mut cells = Vec::new();
        for cell in &row.children {
            let Node::Element(cell) = cell else {
                if matches!(cell, Node::Text(text) if text.trim().is_empty()) {
                    continue;
                }
                return Err(HtmlImportError::InvalidStructure(
                    "table row has invalid children",
                ));
            };
            if cell.name != cell_name {
                return Err(HtmlImportError::InvalidStructure(
                    "table cell kind mismatch",
                ));
            }
            cells.push(ImportedTableCell {
                markdown: render_inline_nodes(&cell.children)?,
                alignment: table_cell_alignment(cell)?,
            });
        }
        rows.push(cells);
    }
    Ok(rows)
}

fn render_table_row(row: &[ImportedTableCell]) -> String {
    row.iter()
        .map(|cell| cell.markdown.as_str())
        .collect::<Vec<_>>()
        .join(" | ")
}

/// 对齐有两种写法，**必须两种都认**：行内 `style="text-align: …"` 是 Yu 自研
/// 渲染器发过的那一种（S7 第六刀之前），GFM 的 `align="…"` 是 comrak 发的那
/// 一种。只认一种的表现是自己拷出来的表格粘回来丢掉对齐——不报错。
fn table_cell_alignment(element: &Element) -> Result<TableAlignment, HtmlImportError> {
    if let Some(align) = attribute(&element.attributes, "align") {
        return match align.trim().to_ascii_lowercase().as_str() {
            "left" => Ok(TableAlignment::Left),
            "center" => Ok(TableAlignment::Center),
            "right" => Ok(TableAlignment::Right),
            _ => Err(HtmlImportError::InvalidStructure(
                "table cell alignment is not supported",
            )),
        };
    }
    let Some(style) = attribute(&element.attributes, "style") else {
        return Ok(TableAlignment::Default);
    };
    match style.trim().to_ascii_lowercase().as_str() {
        "text-align: left" => Ok(TableAlignment::Left),
        "text-align: center" => Ok(TableAlignment::Center),
        "text-align: right" => Ok(TableAlignment::Right),
        _ => Err(HtmlImportError::InvalidStructure(
            "table cell alignment is not supported",
        )),
    }
}

fn table_alignment_marker(alignment: TableAlignment) -> String {
    match alignment {
        TableAlignment::Default => "---".to_owned(),
        TableAlignment::Left => ":---".to_owned(),
        TableAlignment::Center => ":---:".to_owned(),
        TableAlignment::Right => "---:".to_owned(),
    }
}

fn text_only(nodes: &[Node]) -> Result<String, HtmlImportError> {
    let mut output = String::new();
    for node in nodes {
        match node {
            Node::Text(text) => output.push_str(text),
            Node::Element(_) => {
                return Err(HtmlImportError::InvalidStructure(
                    "code content must be plain text",
                ));
            }
        }
    }
    Ok(output)
}

fn escape_markdown_text(value: &str, output: &mut String) {
    for character in value.chars() {
        if matches!(
            character,
            '\\' | '*' | '_' | '`' | '[' | ']' | '#' | '!' | '|' | '~'
        ) {
            output.push('\\');
        }
        output.push(character);
    }
}

fn escape_destination(value: &str, output: &mut String) {
    for character in value.chars() {
        if matches!(character, '\\' | ')') {
            output.push('\\');
        }
        output.push(character);
    }
}

fn inline_code(value: &str) -> String {
    let fence = "`".repeat(max_backtick_run(value).saturating_add(1).max(1));
    if value.starts_with('`')
        || value.ends_with('`')
        || value.starts_with(' ')
        || value.ends_with(' ')
    {
        format!("{fence} {value} {fence}")
    } else {
        format!("{fence}{value}{fence}")
    }
}

fn max_backtick_run(value: &str) -> usize {
    let mut maximum = 0;
    let mut current = 0;
    for character in value.chars() {
        if character == '`' {
            current += 1;
            maximum = maximum.max(current);
        } else {
            current = 0;
        }
    }
    maximum
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `<br />` 后面那个换行是排版空白，不是内容。
    ///
    /// 几乎每个渲染器都发 `<br />\n`，而硬换行本身要回写成 `"  \n"`；两个
    /// 加起来是一个空行，于是**一个段落被粘成两个**。不报错、不丢字。
    ///
    /// 反向验证：把 `after_hard_break` 那一段去掉，这条与
    /// `export_import_pair.rs::export_import_export_is_a_fixed_point` 同时红。
    #[test]
    fn a_hard_break_does_not_split_the_paragraph_in_two() {
        let markdown = import_html_fragment("<p>第一行<br />\n第二行</p>")
            .expect("硬换行是 Yu 自己会导出的形状");
        assert_eq!(markdown, "第一行  \n第二行");
        assert!(
            !markdown.contains("\n\n"),
            "多出来的空行会把一个段落分成两个：{markdown:?}"
        );
    }

    /// `<hr>` 不是标题。
    ///
    /// 块分派那一支以前写的是「`h` 开头、两个字符」——`hr` 正好是那个形状，
    /// `b'r' - b'0'` = 66，于是一条主题分隔线被粘成 66 个 `#`。不 panic、
    /// 不报错。这一支在 S7 第六刀给导入器加 `<hr>` 之前永远走不到。
    ///
    /// 反向验证：把 `render_block` 里那条 `1..=6` 的核对换回
    /// `name.starts_with('h') && name.len() == 2`，这条红。
    #[test]
    fn a_thematic_break_is_not_a_heading() {
        assert_eq!(
            import_html_fragment("<p>上</p><hr /><p>下</p>").expect("hr 在白名单里"),
            "上\n\n---\n\n下"
        );
        // 真正的标题仍然要认得，六级到头。
        assert_eq!(
            import_html_fragment("<h6>六</h6>").expect("h6"),
            "###### 六"
        );
        assert!(
            import_html_fragment("<h7>七</h7>").is_err(),
            "h7 不是标题，也不在白名单里"
        );
    }

    /// 链接与图片的 `title` 要带回来，而且**引号要转义**。
    ///
    /// 不转的话一个带引号的 title 会把 Markdown 的目标括号提前关掉，粘回来
    /// 是一段坏掉的源码，导入这一侧一声不响。
    #[test]
    fn link_and_image_titles_survive_with_escaped_quotes() {
        assert_eq!(
            import_html_fragment(r#"<p><a href="/u" title="标题">文字</a></p>"#).expect("title"),
            r#"[文字](/u "标题")"#
        );
        assert_eq!(
            import_html_fragment(
                r#"<p><img src="/a.png" alt="替代" title="带&quot;引号&quot;"></p>"#
            )
            .expect("image title"),
            "![替代](/a.png \"带\\\"引号\\\"\")"
        );
    }

    /// 表格对齐有两种写法，两种都要认。
    ///
    /// `align="left"` 是 comrak（GFM）发的那一种，`style="text-align: left"`
    /// 是 Yu 自研渲染器发过的那一种。只认一种的表现是自己拷出来的表格粘回来
    /// **丢掉对齐**——表格还在，不报错。
    #[test]
    fn table_alignment_is_read_from_both_gfm_and_inline_style() {
        for cell_attribute in [r#"align="right""#, r#"style="text-align: right""#] {
            let html = format!(
                "<table><thead><tr><th {cell_attribute}>a</th></tr></thead>\
                 <tbody><tr><td {cell_attribute}>1</td></tr></tbody></table>"
            );
            let markdown = import_html_fragment(&html).expect("两种写法都该认");
            assert!(
                markdown.contains("---:"),
                "{cell_attribute} 的右对齐丢了：{markdown}"
            );
        }
    }

    #[test]
    fn imports_allowlisted_semantic_fragment() {
        let html = r#"<h2>Yu</h2><p><strong>羽</strong> <a href="https://example.com">link</a></p><ul><li><input type="checkbox" disabled checked> done</li></ul>"#;

        assert_eq!(
            import_html_fragment(html).expect("allowlisted fragment should import"),
            "## Yu\n\n**羽** [link](https://example.com)\n\n- [x] done"
        );
    }

    #[test]
    fn imports_table_alignment_without_inspecting_rendered_text() {
        let html = r#"<table><thead><tr><th style="text-align: left">A</th><th style="text-align: right">B</th></tr></thead><tbody><tr><td style="text-align: left">1</td><td style="text-align: right">2</td></tr></tbody></table>"#;

        assert_eq!(
            import_html_fragment(html).expect("table should import"),
            "| A | B |\n| :--- | ---: |\n| 1 | 2 |"
        );
    }

    #[test]
    fn decodes_entities_and_escapes_markdown_text() {
        let html = "<p>2 &lt; 3 &amp; *x*</p>";

        assert_eq!(
            import_html_fragment(html).expect("text should import"),
            "2 < 3 & \\*x\\*"
        );
    }

    #[test]
    fn imports_fenced_code_with_language_and_safe_backticks() {
        let html = r#"<pre><code class="language-rust">fn main() { ``` }</code></pre>"#;

        assert_eq!(
            import_html_fragment(html).expect("code should import"),
            "````rust\nfn main() { ``` }\n````"
        );
    }

    #[test]
    fn rejects_unsupported_or_unsafe_html() {
        assert!(matches!(
            import_html_fragment("<script>alert(1)</script>"),
            Err(HtmlImportError::UnsupportedTag(tag)) if tag == "script"
        ));
        assert!(matches!(
            import_html_fragment("<a href=\"javascript:alert(1)\">x</a>"),
            Err(HtmlImportError::UnsafeUrl(url)) if url == "javascript:alert(1)"
        ));
        assert!(matches!(
            import_html_fragment("<p class=\"injected\">x</p>"),
            Err(HtmlImportError::UnsupportedAttribute { tag, attribute })
                if tag == "p" && attribute == "class"
        ));
    }

    #[test]
    fn rejects_malformed_structure_and_entities() {
        assert!(matches!(
            import_html_fragment("<p>x</div>"),
            Err(HtmlImportError::MismatchedClosingTag { expected, found })
                if expected == "p" && found == "div"
        ));
        assert!(matches!(
            import_html_fragment("<p>&unknown;</p>"),
            Err(HtmlImportError::InvalidEntity)
        ));
        assert!(matches!(
            import_html_fragment("<table><tbody><tr><td>x</td></tr></tbody></table>"),
            Err(HtmlImportError::InvalidStructure("table is missing thead"))
        ));
    }

    /// 顶层的编码声明是信封，摘掉；嵌在内容里的 `<meta>` 不是，继续拒。
    ///
    /// 这两半必须一起断言：只写前一半的话，把 `strip_encoding_declaration`
    /// 写成「递归摘掉所有 meta」也能全绿，而那等于在正文里给一个未知标签
    /// 开了后门。
    #[test]
    fn a_top_level_encoding_declaration_is_an_envelope_not_content() {
        assert_eq!(
            import_html_fragment("<meta charset=\"utf-8\"><p>羽</p>"),
            Ok("羽".to_owned())
        );
        // Chrome 发的是单引号那种写法。
        assert_eq!(
            import_html_fragment("<meta charset='utf-8'><h2>标题</h2>"),
            Ok("## 标题".to_owned())
        );
        // 嵌在段落里的不是编码声明。
        assert!(matches!(
            import_html_fragment("<p>羽<meta charset=\"utf-8\"></p>"),
            Err(HtmlImportError::InvalidStructure(_))
        ));
        // 只认 charset：`http-equiv` 那种写法带着一个完整的 MIME 串，认它就
        // 等于开始解析 HTTP 头。
        assert!(matches!(
            import_html_fragment(
                "<meta http-equiv=\"content-type\" content=\"text/html\"><p>羽</p>"
            ),
            Err(HtmlImportError::UnsupportedAttribute { .. })
        ));
    }
}
