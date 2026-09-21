//! Small, source-preserving image-tag vocabulary. This is not an HTML layout
//! engine: unknown attributes are retained, and no attribute executes code.
use std::ops::Range;

pub const MAX_IMAGE_DIMENSION: u32 = 100_000;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ImageAttribute {
    pub name: String,
    /// Includes whitespace before the attribute, for lossless removal.
    pub source: Range<usize>,
    pub value: Option<Range<usize>>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ImageTag {
    pub length: usize,
    pub closing: usize,
    pub attributes: Vec<ImageAttribute>,
}

impl ImageTag {
    #[must_use]
    pub fn attribute(&self, name: &str) -> Option<&ImageAttribute> {
        self.attributes
            .iter()
            .find(|attribute| attribute.name == name)
    }

    #[must_use]
    pub fn value<'a>(&self, source: &'a str, name: &str) -> Option<&'a str> {
        source.get(self.attribute(name)?.value.clone()?)
    }

    pub fn dimension(&self, source: &str, name: &str) -> Result<Option<u32>, ImageMarkupError> {
        if self.attribute(name).is_none() {
            return Ok(None);
        }
        let value = self.value(source, name).ok_or(ImageMarkupError)?;
        let value = decode_image_text(value, true)
            .trim()
            .parse::<u32>()
            .map_err(|_| ImageMarkupError)?;
        if value == 0 || value > MAX_IMAGE_DIMENSION {
            return Err(ImageMarkupError);
        }
        Ok(Some(value))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ImageMarkupError;

/// Parse one complete img tag at the start of a syntax-owned HTML span.
/// Reject malformed or duplicate attributes instead of guessing precedence.
#[must_use]
pub fn parse_image_tag(source: &str) -> Option<ImageTag> {
    let bytes = source.as_bytes();
    if !bytes.get(..4)?.eq_ignore_ascii_case(b"<img") {
        return None;
    }
    if !bytes
        .get(4)
        .is_some_and(|b| b.is_ascii_whitespace() || matches!(b, b'/' | b'>'))
    {
        return None;
    }
    let mut cursor = 4;
    let mut attributes = Vec::<ImageAttribute>::new();
    loop {
        let start = cursor;
        while bytes.get(cursor).is_some_and(u8::is_ascii_whitespace) {
            cursor += 1;
        }
        let closing = cursor;
        if bytes.get(cursor) == Some(&b'>') || bytes.get(cursor..cursor + 2) == Some(b"/>") {
            cursor += if bytes[cursor] == b'/' { 2 } else { 1 };
            return Some(ImageTag {
                length: cursor,
                closing,
                attributes,
            });
        }
        if cursor == start {
            return None;
        }
        let name_start = cursor;
        while bytes
            .get(cursor)
            .is_some_and(|b| b.is_ascii_alphanumeric() || b"-_:".contains(b))
        {
            cursor += 1;
        }
        if cursor == name_start {
            return None;
        }
        let name = source.get(name_start..cursor)?.to_ascii_lowercase();
        if attributes.iter().any(|attribute| attribute.name == name) {
            return None;
        }
        let name_end = cursor;
        while bytes.get(cursor).is_some_and(u8::is_ascii_whitespace) {
            cursor += 1;
        }
        let value = if bytes.get(cursor) == Some(&b'=') {
            cursor += 1;
            while bytes.get(cursor).is_some_and(u8::is_ascii_whitespace) {
                cursor += 1;
            }
            let quote = *bytes.get(cursor)?;
            if matches!(quote, b'\'' | b'"') {
                cursor += 1;
                let from = cursor;
                while *bytes.get(cursor)? != quote {
                    cursor += 1;
                }
                let value = from..cursor;
                cursor += 1;
                Some(value)
            } else {
                let from = cursor;
                while let Some(byte) = bytes.get(cursor) {
                    if byte.is_ascii_whitespace()
                        || *byte == b'>'
                        || bytes.get(cursor..cursor + 2) == Some(b"/>")
                    {
                        break;
                    }
                    if b"<\"'`=".contains(byte) {
                        return None;
                    }
                    cursor += 1;
                }
                if from == cursor {
                    return None;
                }
                Some(from..cursor)
            }
        } else {
            cursor = name_end;
            None
        };
        attributes.push(ImageAttribute {
            name,
            source: start..cursor,
            value,
        });
    }
}

/// Update one image tag without rewriting unrelated attributes or surrounding
/// source. Unchanged values retain their original quoting and entity spelling.
/// The destination is a URI, already encoded by the caller when it is a path.
pub fn update_image_tag(
    source: &str,
    destination: &str,
    alternative: &str,
    width: Option<u32>,
    height: Option<u32>,
) -> Result<String, ImageMarkupError> {
    if destination.is_empty()
        || destination.contains('\0')
        || alternative.contains('\0')
        || [width, height]
            .into_iter()
            .flatten()
            .any(|value| value == 0 || value > MAX_IMAGE_DIMENSION)
    {
        return Err(ImageMarkupError);
    }
    let tag = parse_image_tag(source).ok_or(ImageMarkupError)?;
    if tag.length != source.len() {
        return Err(ImageMarkupError);
    }
    let width = width.map(|value| value.to_string());
    let height = height.map(|value| value.to_string());
    let values = [
        ("src", Some(destination)),
        ("alt", Some(alternative)),
        ("width", width.as_deref()),
        ("height", height.as_deref()),
    ];
    let mut edits = Vec::new();
    let mut appended = String::new();
    for (name, value) in values {
        if let Some(attribute) = tag.attribute(name) {
            let old = tag
                .value(source, name)
                .map(|value| decode_image_text(value, true));
            let same_dimension = matches!(name, "width" | "height")
                && value
                    .and_then(|value| value.parse::<u32>().ok())
                    .is_some_and(|value| tag.dimension(source, name) == Ok(Some(value)));
            if value.is_some() && (old.as_deref() == value || same_dimension) {
                continue;
            }
            let replacement = value.map_or_else(String::new, |value| {
                // Retain indentation/newlines preceding the attribute.
                let raw = &source[attribute.source.clone()];
                let whitespace =
                    &raw[..raw.len() - raw.trim_start_matches(char::is_whitespace).len()];
                format!("{whitespace}{name}=\"{}\"", escape_image_attribute(value))
            });
            edits.push((attribute.source.clone(), replacement));
        } else if let Some(value) = value {
            appended.push_str(&format!(" {name}=\"{}\"", escape_image_attribute(value)));
        }
    }
    if !appended.is_empty() {
        edits.push((tag.closing..tag.closing, appended));
    }
    edits.sort_by_key(|(range, _)| std::cmp::Reverse(range.start));
    let mut result = source.to_owned();
    for (range, replacement) in edits {
        result.replace_range(range, &replacement);
    }
    Ok(result)
}

/// Decode syntax escapes before URI percent-decoding in the resource layer.
/// HTML does not use Markdown backslash escaping.
#[must_use]
pub fn decode_image_text(source: &str, html: bool) -> String {
    let mut result = String::new();
    let mut at = 0;
    while at < source.len() {
        let bytes = source.as_bytes();
        if !html && bytes[at] == b'\\' && bytes.get(at + 1).is_some_and(u8::is_ascii_punctuation) {
            result.push(char::from(bytes[at + 1]));
            at += 2;
            continue;
        }
        if bytes[at] == b'&'
            && let Some(end) = bytes[at..].iter().take(34).position(|&byte| byte == b';')
            && let Some(decoded) = crate::extension::entity::decode(&source[at..at + end + 1])
        {
            decoded.append_to(&mut result);
            at += end + 1;
            continue;
        }
        let ch = source[at..].chars().next().expect("valid UTF-8 cursor");
        result.push(ch);
        at += ch.len_utf8();
    }
    result
}

#[must_use]
pub fn escape_image_attribute(source: &str) -> String {
    source
        .replace('&', "&amp;")
        .replace('"', "&quot;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn changing_alternative_text_keeps_untouched_numeric_attribute_spelling() {
        let source = "<img src='x.png' width = '00128' height=\" 64 \" alt='old'>";
        let updated = update_image_tag(source, "x.png", "new", Some(128), Some(64))
            .expect("valid property update");
        assert_eq!(
            updated,
            "<img src='x.png' width = '00128' height=\" 64 \" alt=\"new\">"
        );
    }

    #[test]
    fn property_edits_preserve_unknown_attributes_and_unchanged_spelling() {
        let source = "<IMG data-own='保留' SRC = 'a&amp;b.png'\n alt='羽' width=120 height=60 />";
        assert_eq!(
            update_image_tag(source, "a&b.png", "羽", Some(120), Some(60))
                .expect("valid image attribute fixture"),
            source
        );
        let changed = update_image_tag(source, "new&file.png", "图\"片", Some(240), None)
            .expect("valid image attribute fixture");
        assert_eq!(
            changed,
            "<IMG data-own='保留' src=\"new&amp;file.png\"\n alt=\"图&quot;片\" width=\"240\" />"
        );
        let tag = parse_image_tag(&changed).expect("valid image attribute fixture");
        assert_eq!(tag.dimension(&changed, "width"), Ok(Some(240)));
        assert_eq!(tag.dimension(&changed, "height"), Ok(None));
    }

    #[test]
    fn property_edits_add_missing_fields_and_reject_invalid_inputs() {
        let result = update_image_tag("<img src=x>", "x", "<羽>", Some(20), Some(10))
            .expect("valid image attribute fixture");
        assert_eq!(
            result,
            "<img src=x alt=\"&lt;羽&gt;\" width=\"20\" height=\"10\">"
        );
        for source in ["<img src=x>tail", "<img src=x src=y>", "plain"] {
            assert!(update_image_tag(source, "x", "", None, None).is_err());
        }
        assert!(update_image_tag("<img src=x>", "", "", None, None).is_err());
        assert!(update_image_tag("<img src=x>", "x", "", Some(0), None).is_err());
        assert!(update_image_tag("<img src=x>", "x", "", None, Some(100001)).is_err());
        assert_eq!(
            update_image_tag("<img src=x width>", "x", "", None, None)
                .expect("valid image attribute fixture"),
            "<img src=x alt=\"\">"
        );
    }

    #[test]
    fn image_tags_keep_attribute_ranges_and_decode_only_their_values() {
        let source =
            "<IMG src = 'a&amp;b.png' data-keep=raw alt=羽 width=\"&#51;00\" disabled />tail";
        let tag = parse_image_tag(source).expect("image tag");
        assert_eq!(&source[..tag.length], source.trim_end_matches("tail"));
        assert_eq!(
            decode_image_text(tag.value(source, "src").expect("src"), true),
            "a&b.png"
        );
        assert_eq!(tag.dimension(source, "width"), Ok(Some(300)));
        assert_eq!(tag.dimension(source, "height"), Ok(None));
        assert_eq!(
            &source[tag
                .attribute("data-keep")
                .expect("unknown attribute")
                .source
                .clone()],
            " data-keep=raw"
        );
    }
    #[test]
    fn malformed_tags_and_dimensions_are_not_silently_accepted() {
        for source in [
            "<image src=x>",
            "<img src='x>",
            "<img src=x SRC=y>",
            "<img src=>",
            "<img src=x<y>",
            "<img src=x alt=\"unterminated",
        ] {
            assert!(parse_image_tag(source).is_none(), "{source}");
        }
        for value in ["0", "-1", "100%", "nan", "100001"] {
            let source = format!("<img src=x width='{value}'>");
            assert!(
                parse_image_tag(&source)
                    .expect("tag")
                    .dimension(&source, "width")
                    .is_err()
            );
        }
    }
    #[test]
    fn syntax_escaping_and_literal_percent_are_kept_distinct() {
        assert_eq!(
            decode_image_text(r"a\(b\)&amp;%2520.png", false),
            "a(b)&%2520.png"
        );
        assert_eq!(decode_image_text(r"a\&amp;.png", false), "a&amp;.png");
        assert_eq!(decode_image_text(r"a\(b\).png", true), r"a\(b\).png");
    }
}
