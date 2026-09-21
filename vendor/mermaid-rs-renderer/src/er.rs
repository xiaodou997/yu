//! ER attribute syntax is consumed before any visual line wrapping.
use anyhow::{Result, bail};

#[derive(Debug, Clone)]
pub(crate) struct Attribute {
    pub name: String,
    pub data_type: String,
    pub keys: Vec<String>,
    pub comment: String,
}

/// Split a body fragment into semantic attributes before visual wrapping.
/// Quoted comments and comma-separated key lists remain attached to their field.
pub(crate) fn split_attributes(source: &str) -> Result<Vec<String>> {
    #[derive(Clone, Copy, PartialEq)]
    enum Kind {
        Word,
        Comma,
        Comment,
    }
    let mut tokens = Vec::new();
    let mut cursor = 0;
    while cursor < source.len() {
        let ch = source[cursor..].chars().next().expect("character boundary");
        if ch.is_whitespace() {
            cursor += ch.len_utf8();
            continue;
        }
        let start = cursor;
        let kind = if ch == ',' {
            cursor += 1;
            Kind::Comma
        } else if ch == '"' {
            cursor += 1;
            let mut escaped = false;
            let mut closed = false;
            for ch in source[cursor..].chars() {
                cursor += ch.len_utf8();
                if escaped {
                    escaped = false;
                } else if ch == '\\' {
                    escaped = true;
                } else if ch == '"' {
                    closed = true;
                    break;
                }
            }
            if !closed {
                bail!("unclosed ER attribute comment: {source}");
            }
            Kind::Comment
        } else {
            cursor += source[cursor..]
                .find(|ch: char| ch.is_whitespace() || matches!(ch, ',' | '"'))
                .unwrap_or(source.len() - cursor);
            Kind::Word
        };
        tokens.push((kind, start, cursor));
    }
    let key = |index: usize| {
        tokens.get(index).is_some_and(|&(kind, start, end)| {
            kind == Kind::Word && matches!(&source[start..end], "PK" | "FK" | "UK")
        })
    };
    let mut attributes = Vec::new();
    let mut index = 0;
    while index < tokens.len() {
        let start = tokens[index].1;
        if tokens[index].0 != Kind::Word
            || tokens
                .get(index + 1)
                .is_none_or(|token| token.0 != Kind::Word)
        {
            bail!("ER attribute requires a type and name: {source}");
        }
        index += 2;
        if key(index) {
            index += 1;
            while tokens
                .get(index)
                .is_some_and(|token| token.0 == Kind::Comma)
            {
                index += 1;
                if !key(index) {
                    bail!("invalid ER attribute key list: {source}");
                }
                index += 1;
            }
        }
        if tokens
            .get(index)
            .is_some_and(|token| token.0 == Kind::Comment)
        {
            index += 1;
        }
        let text = &source[start..tokens[index - 1].2];
        let attribute = parse_attribute(text)?;
        // The existing layout IR stores one semantic attribute per line. Source
        // whitespace may span lines, but must never create extra visual rows.
        let mut row = format!("{} {}", attribute.data_type, attribute.name);
        if !attribute.keys.is_empty() {
            row.push(' ');
            row.push_str(&attribute.keys.join(", "));
        }
        if !attribute.comment.is_empty() {
            row.push(' ');
            row.push_str(&serde_json::to_string(&attribute.comment)?);
        }
        attributes.push(row);
    }
    Ok(attributes)
}

pub(crate) fn parse_attribute(source: &str) -> Result<Attribute> {
    let data_type = source.split_whitespace().next().unwrap_or_default();
    let rest = source.trim()[data_type.len()..].trim_start();
    let name_end = rest.find(char::is_whitespace).unwrap_or(rest.len());
    let name = &rest[..name_end];
    if data_type.is_empty() || name.is_empty() {
        bail!("ER attribute requires a type and name: {source}");
    }
    let suffix = rest[name_end..].trim();
    let (keys_source, comment) = if let Some(start) = suffix.find('"') {
        let comment: String = json5::from_str(&suffix[start..])
            .map_err(|_| anyhow::anyhow!("invalid ER attribute comment: {source}"))?;
        (suffix[..start].trim(), comment)
    } else {
        (suffix, String::new())
    };
    let mut keys = Vec::new();
    if !keys_source.is_empty() {
        for key in keys_source.split(',') {
            let key = key.trim();
            if !matches!(key, "PK" | "FK" | "UK") {
                bail!("unsupported ER attribute key or trailing content: {source}");
            }
            if !keys.iter().any(|existing| existing == key) {
                keys.push(key.to_string());
            }
        }
    }
    Ok(Attribute {
        name: name.to_string(),
        data_type: data_type.to_string(),
        keys,
        comment,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn comments_do_not_become_keys_or_attributes() {
        let attr = parse_attribute(r#"string id PK, FK, UK "PK } long comment""#).unwrap();
        assert_eq!(attr.name, "id");
        assert_eq!(attr.keys, ["PK", "FK", "UK"]);
        assert_eq!(attr.comment, "PK } long comment");
        for source in [
            "string",
            "string id nonsense",
            "string id PK,",
            "string id \"unfinished",
            "string id \"ok\" trailing",
        ] {
            assert!(parse_attribute(source).is_err(), "{source}");
        }
    }
}
