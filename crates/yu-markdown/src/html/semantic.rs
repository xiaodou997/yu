use super::{HtmlAttributeError, HtmlAttributes, HtmlElementKind, HtmlFragment, HtmlNodeKind};
use yu_core::TextRange;

#[derive(Clone, Debug, PartialEq)]
pub struct HtmlResolvedElement {
    pub kind: HtmlElementKind,
    pub attributes: HtmlAttributes,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HtmlSemanticError {
    MalformedFragment,
    UnknownElement,
    InvalidParent,
    InvalidChildren,
    Attributes(HtmlAttributeError),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HtmlSemanticDiagnostic {
    pub source: TextRange,
    pub error: HtmlSemanticError,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct HtmlResolution {
    /// Aligned with fragment node indices. None means retain source, not drop it.
    pub elements: Vec<Option<HtmlResolvedElement>>,
    pub diagnostics: Vec<HtmlSemanticDiagnostic>,
}

impl HtmlFragment {
    pub fn resolve(&self, source: &str) -> HtmlResolution {
        let mut result = HtmlResolution {
            elements: vec![None; self.nodes.len()],
            diagnostics: Vec::new(),
        };
        if !self.diagnostics.is_empty() {
            result
                .diagnostics
                .extend(self.diagnostics.iter().map(|d| HtmlSemanticDiagnostic {
                    source: d.source,
                    error: HtmlSemanticError::MalformedFragment,
                }));
            return result;
        }
        for (id, node) in self.nodes.iter().enumerate() {
            let HtmlNodeKind::Element { opening, closing } = &node.kind else {
                continue;
            };
            if node
                .parent
                .is_some_and(|parent| result.elements[parent].is_none())
            {
                continue;
            }
            let resolved = (|| {
                let kind = opening
                    .semantic_kind()
                    .ok_or(HtmlSemanticError::UnknownElement)?;
                let attributes = opening
                    .resolve_attributes(source)
                    .map_err(HtmlSemanticError::Attributes)?;
                // TeX is one opaque text leaf. Nested markup, comments and
                // multiline/display forms must not be silently flattened.
                // An empty body remains a formula so deleting its last letter
                // does not lose identity while the user is editing it.
                if attributes.inline_math || attributes.footnote_reference {
                    let closing = closing.as_ref().ok_or(HtmlSemanticError::InvalidChildren)?;
                    if node
                        .children
                        .iter()
                        .any(|&child| !matches!(self.nodes[child].kind, HtmlNodeKind::Text))
                    {
                        return Err(HtmlSemanticError::InvalidChildren);
                    }
                    let body = source
                        .get(
                            opening.source.end().get() as usize
                                ..closing.source.start().get() as usize,
                        )
                        .ok_or(HtmlSemanticError::InvalidChildren)?;
                    let decoded = crate::image_markup::decode_image_text(body, true);
                    if decoded.contains(['\r', '\n', '\0'])
                        || attributes.footnote_reference
                            && yu_syntax::footnote_reference_label(&decoded).is_none()
                    {
                        return Err(HtmlSemanticError::InvalidChildren);
                    }
                }
                let parent = node
                    .parent
                    .and_then(|parent| result.elements[parent].as_ref().map(|p| p.kind));
                if !valid_parent(kind, parent) {
                    return Err(HtmlSemanticError::InvalidParent);
                }
                let parent_name = &opening.name;
                let mut substantive_child_seen = false;
                for &child in &node.children {
                    match &self.nodes[child].kind {
                        HtmlNodeKind::Element { opening, .. } => {
                            if kind == HtmlElementKind::Details
                                && opening.semantic_kind() == Some(HtmlElementKind::Summary)
                                && substantive_child_seen
                            {
                                return Err(HtmlSemanticError::InvalidChildren);
                            }
                            substantive_child_seen = true;
                            if !valid_child(kind, parent_name, opening) {
                                return Err(HtmlSemanticError::InvalidChildren);
                            }
                        }
                        HtmlNodeKind::Text => {
                            let range = self.nodes[child].source;
                            let text = source
                                .get(range.start().get() as usize..range.end().get() as usize)
                                .ok_or(HtmlSemanticError::InvalidChildren)?;
                            if !text.trim().is_empty() {
                                substantive_child_seen = true;
                                if only_structural_children(kind) {
                                    return Err(HtmlSemanticError::InvalidChildren);
                                }
                            }
                        }
                        _ => {}
                    }
                }
                Ok(HtmlResolvedElement { kind, attributes })
            })();
            match resolved {
                Ok(element) => result.elements[id] = Some(element),
                Err(error) => result.diagnostics.push(HtmlSemanticDiagnostic {
                    source: node.source,
                    error,
                }),
            }
        }
        result
    }
}

fn only_structural_children(kind: HtmlElementKind) -> bool {
    use HtmlElementKind::*;
    matches!(
        kind,
        UnorderedList | OrderedList | Table | TableHead | TableBody | TableFoot | TableRow
    )
}

fn valid_parent(kind: HtmlElementKind, parent: Option<HtmlElementKind>) -> bool {
    use HtmlElementKind::*;
    match kind {
        ListItem => matches!(parent, Some(UnorderedList | OrderedList)),
        TableHead | TableBody | TableFoot => parent == Some(Table),
        TableRow => matches!(parent, Some(Table | TableHead | TableBody | TableFoot)),
        HeaderCell | DataCell => parent == Some(TableRow),
        Summary => parent == Some(Details),
        _ => true,
    }
}

fn valid_child(parent: HtmlElementKind, parent_name: &str, tag: &super::HtmlTag) -> bool {
    use HtmlElementKind::*;
    let child = tag.semantic_kind();
    let inline_only = parent_name == "span"
        || matches!(
            parent,
            Paragraph
                | Heading(_)
                | Strong
                | Emphasis
                | Code
                | Strike
                | Underline
                | Highlight
                | Superscript
                | Subscript
                | Link
        );
    if inline_only
        && (tag.name == "div"
            || matches!(
                child,
                Some(
                    Heading(_)
                        | Paragraph
                        | UnorderedList
                        | OrderedList
                        | ListItem
                        | Table
                        | TableHead
                        | TableBody
                        | TableFoot
                        | TableRow
                        | HeaderCell
                        | DataCell
                        | Details
                        | Summary
                )
            ))
    {
        return false;
    }
    match parent {
        UnorderedList | OrderedList => child == Some(ListItem),
        Table => matches!(child, Some(TableHead | TableBody | TableFoot | TableRow)),
        TableHead | TableBody | TableFoot => child == Some(TableRow),
        TableRow => matches!(child, Some(HeaderCell | DataCell)),
        _ => true,
    }
}
