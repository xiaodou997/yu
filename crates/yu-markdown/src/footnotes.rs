//! Revision-owned footnote relationships. Numbers are presentation data only.
use std::collections::{HashMap, VecDeque};
use std::hash::{Hash, Hasher};

use yu_core::{ByteOffset, TextRange};
use yu_syntax::{NodeKind, Tree};
use yu_text::TextSnapshot;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FootnoteDefinition {
    pub source: TextRange,
    pub marker: TextRange,
    pub number: Option<u32>,
    label: Vec<u8>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FootnoteReference {
    pub source: TextRange,
    pub number: Option<u32>,
    /// HTML references retain their editable marker body in canonical bytes.
    pub content: Option<TextRange>,
    label: Vec<u8>,
    owner: Option<usize>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FootnoteDiagnostic {
    pub source: TextRange,
    pub message: &'static str,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FootnoteIndex {
    definitions: Vec<FootnoteDefinition>,
    references: Vec<FootnoteReference>,
    diagnostics: Vec<FootnoteDiagnostic>,
    labels: HashMap<Vec<u8>, Vec<usize>>,
    fingerprint: u64,
}

impl FootnoteIndex {
    pub(crate) fn from_syntax(tree: &Tree, source: &TextSnapshot) -> Self {
        let mut result = Self::default();
        let mut stack = vec![(tree, 0_u32, None)];
        while let Some((tree, from, mut owner)) = stack.pop() {
            let range = |start, end| {
                TextRange::new(ByteOffset::new(start as u64), ByteOffset::new(end as u64))
                    .expect("syntax range")
            };
            if tree.kind() == NodeKind::FootnoteDefinition {
                if let Some((mark, offset)) = (0..tree.child_count())
                    .filter_map(|i| tree.child(i))
                    .find(|(child, _)| child.kind() == NodeKind::FootnoteMark)
                {
                    let start = from + offset;
                    let end = start + mark.len_bytes();
                    if let Some(label) =
                        crate::reference::normalized_label(source, range(start + 2, end - 2))
                    {
                        let id = result.definitions.len();
                        result.labels.entry(label.clone()).or_default().push(id);
                        result.definitions.push(FootnoteDefinition {
                            source: range(from, from + tree.len_bytes()),
                            marker: range(start, end),
                            number: None,
                            label,
                        });
                        owner = Some(id);
                    }
                }
            } else if tree.kind() == NodeKind::FootnoteReference {
                let end = from + tree.len_bytes();
                if let Some(label) =
                    crate::reference::normalized_label(source, range(from + 2, end - 1))
                {
                    result.references.push(FootnoteReference {
                        source: range(from, end),
                        number: None,
                        content: None,
                        label,
                        owner,
                    });
                }
            }
            for i in (0..tree.child_count()).rev() {
                if let Some((child, offset)) = tree.child(i) {
                    stack.push((child, from + offset, owner));
                }
            }
        }
        result.resolve_relationships();
        result
    }

    /// Merge source-backed HTML references into the same document-wide graph.
    /// Always derive from the syntax index, never from an already projected copy.
    pub(crate) fn with_html_regions(
        &self,
        regions: &[crate::html::HtmlRegion],
        source: &TextSnapshot,
    ) -> Self {
        let mut result = self.clone();
        for span in regions
            .iter()
            .filter_map(|region| region.model.as_ref().ok())
            .flat_map(|model| model.footnote_spans())
        {
            let Some(marker) = span.marker_text(source.as_str()) else {
                continue;
            };
            let Some(label) = yu_syntax::footnote_reference_label(&marker) else {
                continue;
            };
            let owner = result.definitions.iter().position(|definition| {
                definition.source.start() <= span.source.start()
                    && span.source.end() <= definition.source.end()
            });
            result.references.retain(|reference| {
                reference.source.start() < span.source.start()
                    || span.source.end() < reference.source.end()
            });
            result.references.push(FootnoteReference {
                source: span.source,
                content: Some(span.content),
                number: None,
                label: crate::reference::normalized_label_text(label),
                owner,
            });
        }
        result
            .references
            .sort_by_key(|reference| reference.source.start());
        result.resolve_relationships();
        result
    }

    fn resolve_relationships(&mut self) {
        let result = self;
        result.diagnostics.clear();
        for definition in &mut result.definitions {
            definition.number = None;
        }
        for reference in &mut result.references {
            reference.number = None;
        }
        let mut nested = vec![Vec::new(); result.definitions.len()];
        let mut pending = VecDeque::new();
        for (id, reference) in result.references.iter().enumerate() {
            if let Some(owner) = reference.owner {
                nested[owner].push(id);
            } else {
                pending.push_back(id);
            }
        }
        let mut next = 1;
        // Body occurrences establish order. References inside a used note are
        // then visited once; cycles cannot recurse or invent additional numbers.
        while let Some(id) = pending.pop_front() {
            let Some(&[definition]) = result
                .labels
                .get(&result.references[id].label)
                .map(Vec::as_slice)
            else {
                continue;
            };
            let entry = &mut result.definitions[definition];
            if entry.number.is_none() {
                entry.number = Some(next);
                next += 1;
                pending.extend(nested[definition].iter().copied());
            }
        }
        for reference in &mut result.references {
            match result.labels.get(&reference.label).map(Vec::as_slice) {
                Some(&[id]) => reference.number = result.definitions[id].number,
                duplicate => result.diagnostics.push(FootnoteDiagnostic {
                    source: reference.source,
                    message: if duplicate.is_some() {
                        "脚注标签有多个定义，请保留唯一的定义。"
                    } else {
                        "找不到此脚注的定义，请添加对应的 [^标签]: 内容。"
                    },
                }),
            }
        }
        for definition in &result.definitions {
            if result.labels[&definition.label].len() > 1 {
                result.diagnostics.push(FootnoteDiagnostic {
                    source: definition.marker,
                    message: "脚注标签有多个定义，请保留唯一的定义。",
                });
            }
        }
        let mut hash = std::collections::hash_map::DefaultHasher::new();
        for reference in &result.references {
            reference.label.hash(&mut hash);
            reference.number.hash(&mut hash);
        }
        for definition in &result.definitions {
            definition.label.hash(&mut hash);
            definition.number.hash(&mut hash);
        }
        result.fingerprint = hash.finish();
    }

    pub fn references(&self) -> &[FootnoteReference] {
        &self.references
    }
    pub fn definition_for_marker(&self, marker: &str) -> Option<&FootnoteDefinition> {
        let label =
            crate::reference::normalized_label_text(yu_syntax::footnote_reference_label(marker)?);
        let &[id] = self.labels.get(&label)?.as_slice() else {
            return None;
        };
        self.definitions.get(id)
    }
    pub fn definitions(&self) -> &[FootnoteDefinition] {
        &self.definitions
    }
    pub fn diagnostics(&self) -> &[FootnoteDiagnostic] {
        &self.diagnostics
    }
    pub(crate) fn fingerprint(&self) -> u64 {
        self.fingerprint
    }
    pub(crate) fn has_reference(&self, range: TextRange) -> bool {
        let i = self
            .references
            .partition_point(|entry| entry.source.end() <= range.start());
        self.references
            .get(i)
            .is_some_and(|entry| entry.source.start() < range.end())
    }

    pub fn navigation_target(&self, at: ByteOffset) -> Option<TextRange> {
        let contains = |range: TextRange| range.start() <= at && at < range.end();
        if let Some(reference) = self.references.iter().find(|entry| contains(entry.source)) {
            let &[id] = self.labels.get(&reference.label)?.as_slice() else {
                return None;
            };
            return Some(self.definitions[id].source);
        }
        let definition = self
            .definitions
            .iter()
            .find(|entry| contains(entry.marker))?;
        if self.labels[&definition.label].len() != 1 {
            return None;
        }
        self.references
            .iter()
            .filter(|entry| entry.label == definition.label)
            .min_by_key(|entry| (entry.owner.is_some(), entry.source.start()))
            .map(|entry| entry.source)
    }

    pub fn diagnostic_at(&self, at: ByteOffset) -> Option<&'static str> {
        self.diagnostics
            .iter()
            .find(|entry| entry.source.start() <= at && at < entry.source.end())
            .map(|entry| entry.message)
    }
}

impl crate::MarkdownDocument {
    pub fn footnotes(&self) -> &FootnoteIndex {
        self.semantic_presentation().footnotes()
    }
}
