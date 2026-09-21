//! Image inspection and editing use the syntax catalog, never host-side parsing.
use super::*;
use yu_markdown::image_markup::{decode_image_text, escape_image_attribute, update_image_tag};
use yu_text::Edit;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ImageProperties {
    pub revision: Revision,
    pub source: TextRange,
    /// URI destination (percent escapes are retained).
    pub destination: String,
    pub alternative: String,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub html: bool,
}

impl EditorDocument {
    /// Serialize imported resources for each target's document syntax. Image
    /// import is semantic input: HTML cells need image elements, not escaped
    /// Markdown text. All targets remain one transaction and one undo step.
    pub fn insert_local_images(
        &mut self,
        images: &[(&str, &str)],
        drop_offset: Option<ByteOffset>,
    ) -> Result<CommandResult, EditorDocumentError> {
        if self.composition().is_some() {
            return Err(EditorDocumentError::CompositionActive);
        }
        if images.is_empty() {
            return Ok(self.command_result(false));
        }
        let mut markdown = Vec::with_capacity(images.len());
        let mut html = Vec::with_capacity(images.len());
        for &(path, alternative) in images {
            markdown.push(
                crate::local_image_markdown(path, alternative)
                    .ok_or(EditorDocumentError::InvalidImageProperties)?,
            );
            let uri =
                crate::local_image_uri(path).ok_or(EditorDocumentError::InvalidImageProperties)?;
            html.push(
                update_image_tag("<img>", &uri, alternative, None, None)
                    .map_err(|_| EditorDocumentError::InvalidImageProperties)?,
            );
        }
        let markdown: Arc<str> = markdown.join(" ").into();
        let html = html.join(" ");
        let replacement = |range| {
            self.html_table_replacement_text(range, &html)
                .map(|(text, _)| Arc::from(text))
                .unwrap_or_else(|| self.table_source_input(range, Arc::clone(&markdown)))
        };
        if let Some(offset) = drop_offset {
            EditorSelection::cursor(&self.snapshot(), offset, crate::CaretAffinity::Downstream)?;
            let text = replacement(TextRange::empty(offset));
            return self.insert_prepared_image_at(offset, text);
        }
        let edits = self
            .selections()
            .as_slice()
            .iter()
            .map(|selection| {
                let range = selection.ordered_range();
                (range, replacement(range))
            })
            .collect();
        self.state.history.break_group();
        let result = self.apply_selection_edits(edits, HistoryGroup::External, CollapseTo::End);
        self.state.history.break_group();
        result
    }

    // A drop must not replace or pre-mutate the current selection. History
    // captures the original multi-cursor/grid state and restores it on undo.
    fn insert_prepared_image_at(
        &mut self,
        offset: ByteOffset,
        replacement: Arc<str>,
    ) -> Result<CommandResult, EditorDocumentError> {
        let range = TextRange::empty(offset);
        let end = offset
            .get()
            .checked_add(replacement.len() as u64)
            .ok_or(EditorDocumentError::Selection(SelectionError::InvalidRange))?;
        self.state.history.break_group();
        let applied = self.apply_transaction_with_group(
            &Transaction::new(self.revision(), [Edit::new(range, replacement)]),
            HistoryGroup::External,
        );
        self.state.history.break_group();
        applied?;
        let snapshot = self.snapshot();
        self.set_single_selection(EditorSelection::cursor(
            &snapshot,
            ByteOffset::new(end),
            crate::CaretAffinity::Downstream,
        )?);
        self.state
            .history
            .finish_selection(self.presentation.selections.clone());
        Ok(self.command_result(true))
    }

    pub fn image_properties(&self, source: TextRange) -> Option<ImageProperties> {
        let snapshot = self.snapshot();
        let image = yu_markdown::image_spans(self.markdown(), &snapshot, Some(source))
            .into_iter()
            .find(|image| image.source() == source)?;
        let label = snapshot
            .as_str()
            .get(image.label().start().get() as usize..image.label().end().get() as usize)?;
        Some(ImageProperties {
            revision: self.revision(),
            source,
            destination: yu_markdown::image_destination_text(
                &snapshot,
                image,
                self.markdown().reference_definitions(),
            )?,
            alternative: decode_image_text(label, image.is_html()),
            width: image.width(),
            height: image.height(),
            html: image.is_html(),
        })
    }

    /// The inspector captures a revision and exact image range. Reject stale
    /// requests instead of changing a different image after intervening input.
    pub fn update_image_properties(
        &mut self,
        desired: &ImageProperties,
    ) -> Result<CommandResult, EditorDocumentError> {
        if self.composition().is_some() {
            return Err(EditorDocumentError::CompositionActive);
        }
        if desired.revision != self.revision() {
            return Err(EditorDocumentError::InvalidImageProperties);
        }
        let current = self
            .image_properties(desired.source)
            .ok_or(EditorDocumentError::InvalidImageProperties)?;
        if current == *desired {
            return Ok(self.command_result(false));
        }
        let snapshot = self.snapshot();
        let raw = &snapshot.as_str()
            [desired.source.start().get() as usize..desired.source.end().get() as usize];
        // Preserve an inline or reference definition's title when converting to
        // a sized image; shared definitions themselves remain byte-for-byte.
        let mut title = None;
        if !current.html {
            let image = yu_markdown::image_spans(self.markdown(), &snapshot, Some(desired.source))
                .into_iter()
                .find(|image| image.source() == desired.source)
                .ok_or(EditorDocumentError::InvalidImageProperties)?;
            title = yu_markdown::image_title_text(self.markdown(), &snapshot, image);
        }
        let template = if current.html {
            raw.to_owned()
        } else {
            let title = title.map_or_else(String::new, |title| {
                format!(" title=\"{}\"", escape_image_attribute(&title))
            });
            format!("<img{title}>")
        };
        let html_replacement = update_image_tag(
            &template,
            &desired.destination,
            &desired.alternative,
            desired.width,
            desired.height,
        )
        .map_err(|_| EditorDocumentError::InvalidImageProperties)?;
        let replacement = if !current.html && desired.width.is_none() && desired.height.is_none() {
            let image = yu_markdown::image_spans(self.markdown(), &snapshot, Some(desired.source))
                .into_iter()
                .find(|image| image.source() == desired.source)
                .ok_or(EditorDocumentError::InvalidImageProperties)?;
            let label = markdown_literal(&desired.alternative);
            let destination = markdown_destination(&desired.destination);
            if let Some(destination_range) = image.destination() {
                let mut edits = Vec::new();
                if desired.destination != current.destination {
                    let old = &snapshot.as_str()[destination_range.start().get() as usize
                        ..destination_range.end().get() as usize];
                    let destination = if old.starts_with('<') && old.ends_with('>') {
                        format!("<{destination}>")
                    } else {
                        destination
                    };
                    edits.push((destination_range, destination));
                }
                if desired.alternative != current.alternative {
                    edits.push((image.label(), label));
                }
                edits.sort_by_key(|(range, _)| std::cmp::Reverse(range.start()));
                let mut updated = raw.to_owned();
                for (range, value) in edits {
                    let start = (range.start().get() - desired.source.start().get()) as usize;
                    let end = (range.end().get() - desired.source.start().get()) as usize;
                    updated.replace_range(start..end, &value);
                }
                updated
            } else {
                let title = yu_markdown::image_title_text(self.markdown(), &snapshot, image)
                    .map_or_else(String::new, |title| {
                        format!(" \"{}\"", markdown_literal(&title))
                    });
                format!("![{label}]({destination}{title})")
            }
        } else {
            html_replacement
        };
        self.state.history.break_group();
        self.apply_transaction(&Transaction::new(
            self.revision(),
            [Edit::new(desired.source, replacement)],
        ))?;
        self.state.history.break_group();
        Ok(self.command_result(true))
    }
}

fn markdown_literal(value: &str) -> String {
    let mut result = String::new();
    for ch in value.chars() {
        if ch.is_control() {
            result.push(' ');
        } else {
            if ch.is_ascii_punctuation() {
                result.push('\\');
            }
            result.push(ch);
        }
    }
    result
}

// Keep URI percent escapes intact, unlike local-path insertion. Escape only
// bytes that can terminate Markdown URL syntax or trigger syntax unescaping.
fn markdown_destination(value: &str) -> String {
    let mut result = String::new();
    for ch in value.chars() {
        if "()\\&".contains(ch) {
            result.push('\\');
            result.push(ch);
        } else if ch.is_whitespace() || ch.is_control() || "<>".contains(ch) {
            for byte in ch.to_string().bytes() {
                result.push_str(&format!("%{byte:02X}"));
            }
        } else {
            result.push(ch);
        }
    }
    result
}
