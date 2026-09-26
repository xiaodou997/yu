//! Revision-owned equation labels and render-only reference expansion.
//! Neither generated TeX nor numbering is written back to the source document.
use crate::{EmbeddedKind, EmbeddedSpan, MarkdownDocument, SyntaxNode};
use std::collections::HashMap;
use yu_core::TextRange;
use yu_syntax::NodeKind;
use yu_tex::{Command, Control, parse_document_controls as parse_controls};

#[derive(Clone, Debug)]
struct Number {
    tex: String,
    plain: bool,
}
#[derive(Clone, Debug)]
struct Equation {
    span: EmbeddedSpan,
    source: String,
    controls: Vec<Control>,
    number: Option<Number>,
    error: Option<String>,
}
#[derive(Clone, Debug, Default)]
pub(crate) struct EquationIndex {
    equations: Vec<Equation>,
    labels: HashMap<String, Vec<usize>>,
    by_content: HashMap<(TextRange, bool), usize>,
}

impl MarkdownDocument {
    /// Return TeX with current document labels resolved, preserving canonical
    /// bytes. Errors belong to the requesting formula, including forward refs.
    /// Source target for a standalone equation reference. Compound formulas
    /// containing multiple references remain editable instead of choosing one.
    pub fn equation_reference_target(&self, at: yu_core::ByteOffset) -> Option<TextRange> {
        let index = self
            .equations
            .get_or_init(|| std::sync::Arc::new(EquationIndex::build(self)));
        let reference = index
            .equations
            .iter()
            .find(|entry| entry.span.source.start() <= at && at < entry.span.source.end())?;
        if reference.error.is_some() {
            return None;
        }
        let [control] = reference.controls.as_slice() else {
            return None;
        };
        let Command::Reference { label, .. } = &control.command else {
            return None;
        };
        if !reference.source[..control.range.start].trim().is_empty()
            || !reference.source[control.range.end..].trim().is_empty()
        {
            return None;
        }
        let [id] = index.labels.get(label)?.as_slice() else {
            return None;
        };
        let target = &index.equations[*id];
        (target.error.is_none() && target.number.is_some()).then_some(target.span.source)
    }

    pub fn equation_source(&self, span: EmbeddedSpan) -> Result<String, String> {
        self.equations
            .get_or_init(|| std::sync::Arc::new(EquationIndex::build(self)))
            .prepare(span)
    }
}

impl EquationIndex {
    pub(crate) fn build(document: &MarkdownDocument) -> Self {
        let mut index = Self::default();
        let Some(tree) = document.tree() else {
            return index;
        };
        let mut spans: Vec<_> = SyntaxNode::new(tree, 0)
            .descendants_in(
                TextRange::new(yu_core::ByteOffset::ZERO, document.source_len()).expect("document"),
            )
            .filter_map(|node| span_of(document, node))
            .collect();
        // HTML formulas have source-owned identity but are not Markdown math
        // nodes. The host schedules all formulas through this index, so its
        // ownership set must include the same native HTML leaves as projection.
        spans.extend(
            document
                .html_regions()
                .regions
                .iter()
                .filter_map(|region| region.model.as_ref().ok())
                .flat_map(|model| model.inline_math_spans()),
        );
        spans.sort_by_key(|span| (span.source.start(), span.source.end()));
        spans.dedup();
        let mut next = 1;
        for span in spans {
            let source = document.embedded_source(span).unwrap_or_default();
            let (controls, mut error) = match parse_controls(&source) {
                Ok(controls) => (controls, None),
                Err(error) => (Vec::new(), Some(error)),
            };
            let labels: Vec<_> = controls
                .iter()
                .filter_map(|c| match &c.command {
                    Command::Label(label) => Some(label.clone()),
                    _ => None,
                })
                .collect();
            let tags: Vec<_> = controls
                .iter()
                .filter_map(|c| match &c.command {
                    Command::Tag { text, plain } => Some(Number {
                        tex: text.clone(),
                        plain: *plain,
                    }),
                    _ => None,
                })
                .collect();
            let suppressed = controls
                .iter()
                .any(|c| matches!(c.command, Command::Suppress));
            if !span.display && (!labels.is_empty() || !tags.is_empty()) {
                error = Some("Equation labels and tags require a display formula".into());
            }
            if tags.len() > 1 || (suppressed && (!labels.is_empty() || !tags.is_empty())) {
                error = Some("Conflicting equation numbering commands".into());
            }
            if labels.len() > 1 && source.contains("\\begin{align}") {
                error = Some("Multiple separately labelled align rows are not yet supported; use separate display formulas".into());
            }
            let number = if let Some(tag) = tags.first() {
                Some(tag.clone())
            } else if span.display && !labels.is_empty() && !suppressed {
                let number = Number {
                    tex: next.to_string(),
                    plain: false,
                };
                next += 1;
                Some(number)
            } else {
                None
            };
            let id = index.equations.len();
            for label in labels {
                index.labels.entry(label).or_default().push(id);
            }
            index.by_content.insert((span.content, span.display), id);
            index.equations.push(Equation {
                span,
                source,
                controls,
                number,
                error,
            });
        }
        for (label, entries) in &index.labels {
            if entries.len() > 1 {
                for &entry in entries {
                    index.equations[entry].error =
                        Some(format!("Duplicate equation label: {label}"));
                }
            }
        }
        index
    }

    fn prepare(&self, span: EmbeddedSpan) -> Result<String, String> {
        let id = self
            .by_content
            .get(&(span.content, span.display))
            .ok_or("Formula no longer belongs to this document revision")?;
        let equation = &self.equations[*id];
        if let Some(error) = &equation.error {
            return Err(error.clone());
        }
        let mut output = String::new();
        let mut cursor = 0;
        for control in &equation.controls {
            output.push_str(&equation.source[cursor..control.range.start]);
            if let Command::Reference { label, parentheses } = &control.command {
                let ids = self
                    .labels
                    .get(label)
                    .ok_or_else(|| format!("Undefined equation label: {label}"))?;
                if ids.len() != 1 {
                    return Err(format!("Ambiguous equation label: {label}"));
                }
                let target = &self.equations[ids[0]];
                if let Some(error) = &target.error {
                    return Err(error.clone());
                }
                let number = target
                    .number
                    .as_ref()
                    .ok_or_else(|| format!("Equation label has no number: {label}"))?;
                output.push_str(&number_tex(&number.tex, *parentheses));
            }
            cursor = control.range.end;
        }
        output.push_str(&equation.source[cursor..]);
        if let Some(number) = &equation.number {
            output.push_str("\n\\qquad ");
            output.push_str(&number_tex(&number.tex, !number.plain));
        }
        Ok(output)
    }
}
fn number_tex(tex: &str, parentheses: bool) -> String {
    let number = format!("\\mathrm{{{tex}}}");
    if parentheses {
        format!("\\text{{(}}{number}\\text{{)}}")
    } else {
        number
    }
}
fn span_of(document: &MarkdownDocument, node: SyntaxNode<'_>) -> Option<EmbeddedSpan> {
    let display = match node.kind() {
        NodeKind::InlineMath | NodeKind::EquationReference => false,
        NodeKind::MathBlock | NodeKind::FencedCode => true,
        _ => return None,
    };
    let content = if node.kind() == NodeKind::EquationReference {
        node.range()
    } else if node.kind() == NodeKind::FencedCode {
        let info = node
            .children()
            .find(|n| n.kind() == NodeKind::CodeInfo)?
            .range();
        let language = document
            .source()
            .as_str()
            .get(info.start().get() as usize..info.end().get() as usize)?
            .trim()
            .to_ascii_lowercase();
        if !matches!(language.as_str(), "math" | "latex" | "tex") {
            return None;
        }
        let marks: Vec<_> = node
            .children()
            .filter(|n| n.kind() == NodeKind::CodeMark)
            .collect();
        if marks.len() != 2 {
            return None;
        }
        let text: Vec<_> = node
            .children()
            .filter(|n| n.kind() == NodeKind::CodeText)
            .collect();
        text.first()
            .zip(text.last())
            .and_then(|(first, last)| TextRange::new(first.range().start(), last.range().end()))
            .unwrap_or_else(|| TextRange::empty(marks[1].range().start()))
    } else {
        let marks: Vec<_> = node
            .children()
            .filter(|n| n.kind() == NodeKind::MathMark)
            .collect();
        if marks.len() != 2 {
            return None;
        }
        TextRange::new(marks[0].range().end(), marks[1].range().start())?
    };
    Some(EmbeddedSpan {
        source: node.range(),
        content,
        kind: EmbeddedKind::Math,
        display,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use yu_core::ByteOffset;
    use yu_text::{Edit, TextBuffer, Transaction};

    #[test]
    fn equation_labels_support_forward_refs_tags_and_source_preservation() {
        let source = "See \\eqref{later} and $\\ref{first}$.\n\n$$\nx=1\\label{first}\n$$\n\n$$\ny=2\\tag{A}\\label{later}\n$$\n\n$$z=3\\label{third}$$\n";
        let buffer = TextBuffer::new(source);
        let document = crate::parse(&buffer.snapshot());
        let index = EquationIndex::build(&document);
        assert_eq!(index.equations.len(), 5);
        assert_eq!(
            document.equation_reference_target(index.equations[0].span.source.start()),
            Some(index.equations[3].span.source)
        );
        let resolved: Vec<_> = index
            .equations
            .iter()
            .map(|eq| index.prepare(eq.span).expect("resolved"))
            .collect();
        assert!(resolved[0].contains("\\mathrm{A}"));
        assert!(resolved[0].contains("\\text{(}"));
        assert_eq!(resolved[1], "\\mathrm{1}");
        assert!(resolved[2].contains("x=1") && resolved[2].contains("\\mathrm{1}"));
        assert!(resolved[3].contains("y=2") && resolved[3].contains("\\mathrm{A}"));
        assert!(resolved[4].contains("\\mathrm{2}"));
        assert_eq!(document.source().as_str(), source);
        assert!(
            resolved
                .iter()
                .all(|s| !s.contains("\\label") && !s.contains("\\tag") && !s.contains("\\ref"))
        );
    }

    #[test]
    fn invalid_equation_metadata_never_silently_succeeds() {
        for source in [
            r"missing \eqref{missing}",
            "$$x\\label{a}$$\n\n$$y\\label{a}$$",
            r"$x\label{inline}$",
            r"$$x\tag{A}\tag{B}$$",
            r"$$x\tag{\ref{a}}$$",
            r"$$x\label{bad label}$$",
            r"$$x\label{a}\notag$$",
            r"$$\newcommand{\x}{1} x$$",
        ] {
            let buffer = TextBuffer::new(source);
            let document = crate::parse(&buffer.snapshot());
            let index = EquationIndex::build(&document);
            assert!(!index.equations.is_empty(), "{source}");
            assert!(
                index
                    .equations
                    .iter()
                    .all(|eq| index.prepare(eq.span).is_err()),
                "{source}"
            );
        }
    }

    #[test]
    fn equation_references_recompute_after_incremental_label_edit() {
        let source = "ref \\eqref{a}\n\n$$x\\tag{A}\\label{a}$$\n";
        let mut buffer = TextBuffer::new(source);
        let previous = crate::parse(&buffer.snapshot());
        let index = EquationIndex::build(&previous);
        let reference = index.equations[0].span;
        assert!(
            previous
                .equation_source(reference)
                .expect("first")
                .contains("{A}")
        );
        let at = source.find("tag{A}").expect("tag") + 4;
        let range = TextRange::new(ByteOffset::new(at as u64), ByteOffset::new(at as u64 + 1))
            .expect("range");
        let applied = buffer
            .apply(&Transaction::new(
                buffer.revision(),
                [Edit::new(range, "B")],
            ))
            .expect("edit");
        let next =
            crate::parse_incremental(&previous, applied.result_snapshot(), applied.change_set())
                .expect("incremental");
        assert!(
            next.document()
                .equation_source(reference)
                .expect("updated")
                .contains("{B}")
        );
        assert!(
            previous
                .equation_source(reference)
                .expect("immutable old document")
                .contains("{A}")
        );
    }
}
