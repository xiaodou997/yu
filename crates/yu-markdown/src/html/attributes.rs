use super::{HtmlElementKind, HtmlTag};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HtmlAlignment {
    Left,
    Center,
    Right,
    Justify,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum HtmlDimension {
    Points(f32),
    Percent(f32),
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct HtmlAttributes {
    pub alignment: Option<HtmlAlignment>,
    pub width: Option<HtmlDimension>,
    pub height: Option<HtmlDimension>,
    pub destination: Option<String>,
    pub alternate: Option<String>,
    pub title: Option<String>,
    pub id: Option<String>,
    pub start: Option<u64>,
    pub column_span: Option<usize>,
    pub row_span: Option<usize>,
    pub open: bool,
    /// Inert image metadata retained for property editing and round trips.
    pub image_data: std::collections::BTreeMap<String, Option<String>>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HtmlAttributeError {
    UnknownElement,
    Unsupported(String),
    Duplicate(String),
    InvalidValue(String),
    InvalidRange,
}

impl HtmlTag {
    /// Resolve the explicitly supported finite attributes. Unknown declarations
    /// reject the projection; callers retain the original tag for source editing.
    pub fn resolve_attributes(&self, source: &str) -> Result<HtmlAttributes, HtmlAttributeError> {
        use HtmlElementKind::*;
        let kind = self
            .semantic_kind()
            .ok_or(HtmlAttributeError::UnknownElement)?;
        let mut result = HtmlAttributes::default();
        let mut seen = std::collections::HashSet::new();
        let alignable = matches!(
            kind,
            Paragraph | Heading(_) | Container | Table | TableRow | HeaderCell | DataCell
        );
        for attribute in &self.attributes {
            if !seen.insert(&attribute.name) {
                return Err(HtmlAttributeError::Duplicate(attribute.name.clone()));
            }
            let value = attribute
                .value
                .map(|range| {
                    let raw = source
                        .get(range.start().get() as usize..range.end().get() as usize)
                        .ok_or(HtmlAttributeError::InvalidRange)?;
                    Ok(crate::image_markup::decode_image_text(raw, true))
                })
                .transpose()?;
            let invalid = || HtmlAttributeError::InvalidValue(attribute.name.clone());
            match attribute.name.as_str() {
                "colspan" | "rowspan" if matches!(kind, HeaderCell | DataCell) => {
                    let raw = value.as_deref().ok_or_else(invalid)?.trim();
                    if raw.is_empty() || !raw.bytes().all(|byte| byte.is_ascii_digit()) {
                        return Err(invalid());
                    }
                    let span = raw.parse::<usize>().map_err(|_| invalid())?;
                    if attribute.name == "colspan" {
                        if span == 0 || span > 1000 {
                            return Err(invalid());
                        }
                        result.column_span = Some(span);
                    } else {
                        if span > 65534 {
                            return Err(invalid());
                        }
                        result.row_span = Some(span);
                    }
                }
                "id" => result.id = Some(value.ok_or_else(invalid)?),
                "title" => result.title = Some(value.ok_or_else(invalid)?),
                "align" if alignable => {
                    if result.alignment.is_some() {
                        return Err(invalid());
                    }
                    result.alignment =
                        Some(alignment(value.as_deref().ok_or_else(invalid)?).ok_or_else(invalid)?);
                }
                "style" if alignable => {
                    let value = value.as_deref().ok_or_else(invalid)?;
                    let mut declared = false;
                    for declaration in value.split(';').map(str::trim).filter(|s| !s.is_empty()) {
                        let (name, value) = declaration.split_once(':').ok_or_else(invalid)?;
                        if !name.trim().eq_ignore_ascii_case("text-align")
                            || result.alignment.is_some()
                        {
                            return Err(invalid());
                        }
                        result.alignment = Some(alignment(value).ok_or_else(invalid)?);
                        declared = true;
                    }
                    if !declared {
                        return Err(invalid());
                    }
                }
                "width" | "height" if kind == Image => {
                    let dimension =
                        dimension(value.as_deref().ok_or_else(invalid)?).ok_or_else(invalid)?;
                    if attribute.name == "width" {
                        result.width = Some(dimension);
                    } else {
                        result.height = Some(dimension);
                    }
                }
                name if kind == Image && name.starts_with("data-") && name.len() > 5 => {
                    result.image_data.insert(name.to_owned(), value);
                }
                "src" if kind == Image => result.destination = Some(value.ok_or_else(invalid)?),
                "href" if kind == Link => result.destination = Some(value.ok_or_else(invalid)?),
                "alt" if kind == Image => result.alternate = Some(value.ok_or_else(invalid)?),
                "start" if kind == OrderedList => {
                    result.start = Some(
                        value
                            .as_deref()
                            .ok_or_else(invalid)?
                            .trim()
                            .parse::<u64>()
                            .ok()
                            .filter(|n| *n > 0)
                            .ok_or_else(invalid)?,
                    );
                }
                // HTML boolean attributes are true by presence, including open="false".
                "open" if kind == Details => result.open = true,
                _ => return Err(HtmlAttributeError::Unsupported(attribute.name.clone())),
            }
        }
        Ok(result)
    }
}

fn alignment(value: &str) -> Option<HtmlAlignment> {
    Some(match value.trim().to_ascii_lowercase().as_str() {
        "left" => HtmlAlignment::Left,
        "center" => HtmlAlignment::Center,
        "right" => HtmlAlignment::Right,
        "justify" => HtmlAlignment::Justify,
        _ => return None,
    })
}

fn dimension(value: &str) -> Option<HtmlDimension> {
    let value = value.trim();
    let percent = value.ends_with('%');
    let number = value
        .strip_suffix('%')
        .or_else(|| value.strip_suffix("px"))
        .unwrap_or(value)
        .trim()
        .parse::<f32>()
        .ok()?;
    if !number.is_finite() || number <= 0.0 || number > if percent { 100.0 } else { 100_000.0 } {
        return None;
    }
    Some(if percent {
        HtmlDimension::Percent(number)
    } else {
        HtmlDimension::Points(number)
    })
}
