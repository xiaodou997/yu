//! Finite HTML source metadata. Parsing never executes or rewrites markup.
use yu_core::{ByteOffset, TextRange};
mod attributes;
mod grid;
pub use grid::{HtmlCellSpan, HtmlGridCell, HtmlGridError, HtmlTableGrid};
mod disclosure;
pub use disclosure::HtmlDisclosure;
mod flow;
mod index;
mod projection;
pub use index::{HtmlBlockModel, HtmlIndex, HtmlLink, HtmlModelError, HtmlRegion};
mod semantic;
mod table;
pub use attributes::{HtmlAlignment, HtmlAttributeError, HtmlAttributes, HtmlDimension};
pub use flow::{HtmlFlowBlock, HtmlFlowError, HtmlFlowKind, HtmlFlowPartition, partition_flow};
pub use semantic::{
    HtmlResolution, HtmlResolvedElement, HtmlSemanticDiagnostic, HtmlSemanticError,
};
pub use table::{
    HtmlCellListItem, HtmlCellParagraph, HtmlTable, HtmlTableCell, HtmlTableError, HtmlTableRow,
    HtmlTableSection,
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HtmlAttribute {
    pub name: String,
    pub source: TextRange,
    /// Unquoted source value; entities remain original source bytes.
    pub value: Option<TextRange>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HtmlTag {
    pub name: String,
    pub source: TextRange,
    pub closing: bool,
    pub self_closing: bool,
    pub attributes: Vec<HtmlAttribute>,
}

/// Semantic candidates for native layout. Attribute values and structural
/// validity must still be checked before hiding any source markup.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HtmlElementKind {
    Strong,
    Emphasis,
    Code,
    Strike,
    Underline,
    Highlight,
    Superscript,
    Subscript,
    Break,
    Link,
    Image,
    Heading(u8),
    Paragraph,
    UnorderedList,
    OrderedList,
    ListItem,
    Table,
    TableHead,
    TableBody,
    TableFoot,
    TableRow,
    HeaderCell,
    DataCell,
    Container,
    Details,
    Summary,
}

impl HtmlTag {
    pub fn semantic_kind(&self) -> Option<HtmlElementKind> {
        use HtmlElementKind::*;
        Some(match self.name.as_str() {
            "b" | "strong" => Strong,
            "i" | "em" => Emphasis,
            "code" => Code,
            "s" | "del" | "strike" => Strike,
            "u" => Underline,
            "mark" => Highlight,
            "sup" => Superscript,
            "sub" => Subscript,
            "br" => Break,
            "a" => Link,
            "img" => Image,
            "h1" => Heading(1),
            "h2" => Heading(2),
            "h3" => Heading(3),
            "h4" => Heading(4),
            "h5" => Heading(5),
            "h6" => Heading(6),
            "p" => Paragraph,
            "ul" => UnorderedList,
            "ol" => OrderedList,
            "li" => ListItem,
            "table" => Table,
            "thead" => TableHead,
            "tbody" => TableBody,
            "tfoot" => TableFoot,
            "tr" => TableRow,
            "th" => HeaderCell,
            "td" => DataCell,
            "div" | "span" => Container,
            "details" => Details,
            "summary" => Summary,
            _ => return None,
        })
    }

    /// Duplicate attributes are ambiguous input, never silently first/last wins.
    pub fn unique_attribute(&self, name: &str) -> Result<Option<&HtmlAttribute>, HtmlTagError> {
        let mut matches = self
            .attributes
            .iter()
            .filter(|a| a.name.eq_ignore_ascii_case(name));
        let first = matches.next();
        if matches.next().is_some() {
            return Err(HtmlTagError::Malformed);
        }
        Ok(first)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HtmlTagError {
    Malformed,
    OffsetOverflow,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HtmlNodeKind {
    Text,
    Comment,
    Element {
        opening: Box<HtmlTag>,
        closing: Option<Box<HtmlTag>>,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HtmlNode {
    pub source: TextRange,
    pub parent: Option<usize>,
    pub children: Vec<usize>,
    pub kind: HtmlNodeKind,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HtmlDiagnosticKind {
    MalformedTag,
    UnexpectedClosingTag,
    UnclosedElement,
    LimitExceeded,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HtmlDiagnostic {
    pub source: TextRange,
    pub kind: HtmlDiagnosticKind,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct HtmlFragment {
    pub nodes: Vec<HtmlNode>,
    pub roots: Vec<usize>,
    pub diagnostics: Vec<HtmlDiagnostic>,
}

impl HtmlFragment {
    /// Strict fragment hierarchy: no implied tags, browser recovery, or scripts.
    /// Invalid markup stays addressable as source alongside a diagnostic.
    pub fn parse(raw: &str, base: ByteOffset) -> Result<Self, HtmlTagError> {
        base.get()
            .checked_add(raw.len() as u64)
            .ok_or(HtmlTagError::OffsetOverflow)?;
        let range = |start: usize, end: usize| {
            TextRange::new(
                ByteOffset::new(base.get() + start as u64),
                ByteOffset::new(base.get() + end as u64),
            )
            .expect("ordered fragment range")
        };
        let mut result = Self::default();
        let mut stack: Vec<usize> = Vec::new();
        let mut cursor = 0;
        while cursor < raw.len() {
            if stack.len() >= 128 || result.nodes.len() >= 100_000 {
                let source = range(cursor, raw.len());
                result.diagnostics.push(HtmlDiagnostic {
                    source,
                    kind: HtmlDiagnosticKind::LimitExceeded,
                });
                result.append(HtmlNodeKind::Text, source, stack.last().copied());
                break;
            }
            if let Some(&parent) = stack.last()
                && let HtmlNodeKind::Element { opening, .. } = &result.nodes[parent].kind
                && matches!(
                    opening.name.as_str(),
                    "script" | "style" | "textarea" | "title"
                )
            {
                let name = opening.name.clone();
                let needle = format!("</{name}");
                let closing = raw.as_bytes()[cursor..]
                    .windows(needle.len())
                    .enumerate()
                    .find(|(offset, window)| {
                        window.eq_ignore_ascii_case(needle.as_bytes())
                            && raw
                                .as_bytes()
                                .get(cursor + offset + needle.len())
                                .is_some_and(|byte| *byte == b'>' || space(*byte))
                    })
                    .map(|(offset, _)| offset)
                    .map(|offset| cursor + offset);
                let end = closing.unwrap_or(raw.len());
                if end > cursor {
                    result.append(HtmlNodeKind::Text, range(cursor, end), Some(parent));
                    cursor = end;
                    if cursor == raw.len() {
                        break;
                    }
                }
            }
            let Some(relative) = raw[cursor..].find('<') else {
                result.append(
                    HtmlNodeKind::Text,
                    range(cursor, raw.len()),
                    stack.last().copied(),
                );
                break;
            };
            let start = cursor + relative;
            if start > cursor {
                result.append(
                    HtmlNodeKind::Text,
                    range(cursor, start),
                    stack.last().copied(),
                );
            }
            if raw[start..].starts_with("<!--") {
                let ending = raw[start + 4..]
                    .find("-->")
                    .map(|offset| start + 4 + offset + 3);
                let end = ending.unwrap_or(raw.len());
                result.append(
                    HtmlNodeKind::Comment,
                    range(start, end),
                    stack.last().copied(),
                );
                if ending.is_none() {
                    result.diagnostics.push(HtmlDiagnostic {
                        source: range(start, end),
                        kind: HtmlDiagnosticKind::MalformedTag,
                    });
                }
                cursor = end;
                continue;
            }
            let name_start = start
                + if raw.as_bytes().get(start + 1) == Some(&b'/') {
                    2
                } else {
                    1
                };
            if !raw
                .as_bytes()
                .get(name_start)
                .is_some_and(u8::is_ascii_alphabetic)
            {
                // A literal comparison such as `2 < 3` is text. Do not consume
                // through the next real element's closing angle bracket.
                result.append(
                    HtmlNodeKind::Text,
                    range(start, start + 1),
                    stack.last().copied(),
                );
                cursor = start + 1;
                continue;
            }
            let mut quote = None;
            let mut ending = None;
            for (offset, byte) in raw.as_bytes()[start + 1..].iter().copied().enumerate() {
                if let Some(delimiter) = quote {
                    if byte == delimiter {
                        quote = None;
                    }
                } else if matches!(byte, b'\'' | b'"') {
                    quote = Some(byte);
                } else if byte == b'>' {
                    ending = Some(start + 1 + offset + 1);
                    break;
                }
            }
            let end = ending.unwrap_or(raw.len());
            let source = range(start, end);
            match HtmlTag::parse(&raw[start..end], source.start()) {
                Ok(tag) if tag.closing => {
                    let matching = stack.last().copied().filter(|&id|
                        matches!(&result.nodes[id].kind, HtmlNodeKind::Element { opening, .. } if opening.name == tag.name));
                    if let Some(id) = matching {
                        stack.pop();
                        result.nodes[id].source =
                            TextRange::new(result.nodes[id].source.start(), tag.source.end())
                                .expect("element range");
                        if let HtmlNodeKind::Element { closing, .. } = &mut result.nodes[id].kind {
                            *closing = Some(Box::new(tag));
                        }
                    } else {
                        result.append(HtmlNodeKind::Text, source, stack.last().copied());
                        result.diagnostics.push(HtmlDiagnostic {
                            source,
                            kind: HtmlDiagnosticKind::UnexpectedClosingTag,
                        });
                    }
                }
                Ok(tag) => {
                    let closed = tag.self_closing || tag.is_void();
                    let id = result.append(
                        HtmlNodeKind::Element {
                            opening: Box::new(tag),
                            closing: None,
                        },
                        source,
                        stack.last().copied(),
                    );
                    if !closed {
                        stack.push(id);
                    }
                }
                Err(_) => {
                    result.append(HtmlNodeKind::Text, source, stack.last().copied());
                    result.diagnostics.push(HtmlDiagnostic {
                        source,
                        kind: HtmlDiagnosticKind::MalformedTag,
                    });
                }
            }
            cursor = end;
        }
        for id in stack {
            result.diagnostics.push(HtmlDiagnostic {
                source: result.nodes[id].source,
                kind: HtmlDiagnosticKind::UnclosedElement,
            });
            result.nodes[id].source =
                TextRange::new(result.nodes[id].source.start(), range(0, raw.len()).end())
                    .expect("unclosed range");
        }
        Ok(result)
    }

    fn append(&mut self, kind: HtmlNodeKind, source: TextRange, parent: Option<usize>) -> usize {
        let id = self.nodes.len();
        self.nodes.push(HtmlNode {
            kind,
            source,
            parent,
            children: Vec::new(),
        });
        if let Some(parent) = parent {
            self.nodes[parent].children.push(id);
        } else {
            self.roots.push(id);
        }
        id
    }
}

fn space(byte: u8) -> bool {
    matches!(byte, b' ' | b'\t' | b'\r' | b'\n' | 12)
}
fn name_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b':')
}

impl HtmlTag {
    pub fn is_void(&self) -> bool {
        matches!(
            self.name.as_str(),
            "area"
                | "base"
                | "br"
                | "col"
                | "embed"
                | "hr"
                | "img"
                | "input"
                | "link"
                | "meta"
                | "param"
                | "source"
                | "track"
                | "wbr"
        )
    }

    /// Parse one already delimited tag and retain all attributes, including
    /// unknown and duplicate attributes. Consumers must explicitly accept them.
    pub fn parse(raw: &str, base: ByteOffset) -> Result<Self, HtmlTagError> {
        let bytes = raw.as_bytes();
        if bytes.len() < 3 || bytes[0] != b'<' || bytes[bytes.len() - 1] != b'>' {
            return Err(HtmlTagError::Malformed);
        }
        let range = |start: usize, end: usize| -> Result<TextRange, HtmlTagError> {
            let start = base
                .get()
                .checked_add(start as u64)
                .ok_or(HtmlTagError::OffsetOverflow)?;
            let end = base
                .get()
                .checked_add(end as u64)
                .ok_or(HtmlTagError::OffsetOverflow)?;
            TextRange::new(ByteOffset::new(start), ByteOffset::new(end))
                .ok_or(HtmlTagError::Malformed)
        };
        let limit = bytes.len() - 1;
        let closing = bytes[1] == b'/';
        let mut at = if closing { 2 } else { 1 };
        if at >= limit || !bytes[at].is_ascii_alphabetic() {
            return Err(HtmlTagError::Malformed);
        }
        let name_start = at;
        while at < limit && name_byte(bytes[at]) {
            at += 1;
        }
        let mut tag = Self {
            name: raw[name_start..at].to_ascii_lowercase(),
            source: range(0, bytes.len())?,
            closing,
            self_closing: false,
            attributes: Vec::new(),
        };
        while at < limit {
            let gap = at;
            while at < limit && space(bytes[at]) {
                at += 1;
            }
            if at == limit {
                break;
            }
            if !closing && bytes[at] == b'/' && at + 1 == limit {
                tag.self_closing = true;
                break;
            }
            if closing || at == gap {
                return Err(HtmlTagError::Malformed);
            }
            let start = at;
            if !matches!(bytes[at], b'a'..=b'z' | b'A'..=b'Z' | b'_' | b':') {
                return Err(HtmlTagError::Malformed);
            }
            while at < limit && name_byte(bytes[at]) {
                at += 1;
            }
            let name = raw[start..at].to_ascii_lowercase();
            let name_end = at;
            while at < limit && space(bytes[at]) {
                at += 1;
            }
            let mut value = None;
            if at < limit && bytes[at] == b'=' {
                at += 1;
                while at < limit && space(bytes[at]) {
                    at += 1;
                }
                if at == limit {
                    return Err(HtmlTagError::Malformed);
                }
                if matches!(bytes[at], b'\'' | b'"') {
                    let quote = bytes[at];
                    at += 1;
                    let value_start = at;
                    while at < limit && bytes[at] != quote {
                        at += 1;
                    }
                    if at == limit {
                        return Err(HtmlTagError::Malformed);
                    }
                    value = Some(range(value_start, at)?);
                    at += 1;
                } else {
                    let value_start = at;
                    while at < limit && !space(bytes[at]) {
                        if matches!(bytes[at], b'<' | b'>' | b'\'' | b'"' | b'`' | b'=') {
                            return Err(HtmlTagError::Malformed);
                        }
                        at += 1;
                    }
                    if at == value_start {
                        return Err(HtmlTagError::Malformed);
                    }
                    value = Some(range(value_start, at)?);
                }
            } else {
                // Keep inter-attribute whitespace for the next iteration.
                at = name_end;
            }
            tag.attributes.push(HtmlAttribute {
                name,
                source: range(start, at)?,
                value,
            });
        }
        Ok(tag)
    }
}
