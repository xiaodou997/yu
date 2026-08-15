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
    let roots = parse_document(html)?;
    render_roots(&roots)
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
    matches!(name, "br" | "img" | "input")
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
    ) {
        Ok(())
    } else {
        Err(HtmlImportError::UnsupportedTag(name.to_owned()))
    }
}

fn validate_attributes(name: &str, attributes: &[Attribute]) -> Result<(), HtmlImportError> {
    let allowed = match name {
        "a" => &["href"][..],
        "img" => &["src", "alt"],
        "ol" => &["start"],
        "code" => &["class"],
        "input" => &["type", "disabled", "checked"],
        "th" | "td" => &["style"],
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
    if name == "ol"
        && let Some(value) = attribute(attributes, "start")
        && value
            .parse::<u64>()
            .ok()
            .filter(|number| *number > 0)
            .is_none()
    {
        return Err(HtmlImportError::InvalidStructure(
            "ordered list start must be a positive integer",
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
    if matches!(name, "th" | "td")
        && let Some(value) = attribute(attributes, "style")
        && !matches!(
            value.trim().to_ascii_lowercase().as_str(),
            "text-align: left" | "text-align: center" | "text-align: right"
        )
    {
        return Err(HtmlImportError::InvalidStructure(
            "table alignment style is not allowlisted",
        ));
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
    )
}

fn render_block(element: &Element) -> Result<String, HtmlImportError> {
    match element.name.as_str() {
        "p" => render_inline_nodes(&element.children),
        name if name.starts_with('h') && name.len() == 2 => {
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
        _ => Err(HtmlImportError::InvalidStructure(
            "unsupported block structure",
        )),
    }
}

fn render_inline_nodes(nodes: &[Node]) -> Result<String, HtmlImportError> {
    let mut output = String::new();
    for node in nodes {
        match node {
            Node::Text(text) => escape_markdown_text(text, &mut output),
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
    }
    Ok(output)
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

fn table_cell_alignment(element: &Element) -> Result<TableAlignment, HtmlImportError> {
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
}
