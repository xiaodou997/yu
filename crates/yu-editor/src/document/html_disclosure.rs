use super::*;

impl EditorDocument {
    /// Navigation may expose hidden text without inserting an `open` attribute
    /// or recording an edit. Source ranges stay valid across this projection change.
    pub fn reveal_html_range(&mut self, target: TextRange) -> Result<bool, EditorDocumentError> {
        if self.composition.is_some() {
            return Err(EditorDocumentError::CompositionActive);
        }
        if !Arc::make_mut(&mut self.presentation.markdown).reveal_html_range(target) {
            return Ok(false);
        }
        self.invalidate_html_presentation();
        Ok(true)
    }

    fn invalidate_html_presentation(&mut self) {
        self.presentation.render_identity = Arc::new(());
        self.presentation.layout_snapshot = None;
        self.presentation.decorations.clear();
        self.presentation.layouts.clear();
        self.presentation.viewport.clear();
        self.state.preferred_x = None;
    }

    pub fn html_disclosure_header_at(
        &self,
        at: ByteOffset,
    ) -> Option<yu_markdown::html::HtmlDisclosure> {
        if self.source_mode() || self.composition().is_some() {
            return None;
        }
        self.markdown
            .html_regions()
            .regions
            .iter()
            .find(|region| region.source.start() <= at && at < region.source.end())?
            .model
            .as_ref()
            .ok()?
            .disclosure_header_at(at)
    }

    /// Change the visible details header at a source position as one undoable
    /// source edit. Only its open attribute changes; body bytes remain intact.
    pub fn toggle_html_details(
        &mut self,
        at: ByteOffset,
    ) -> Result<CommandResult, EditorDocumentError> {
        if self.composition().is_some() {
            return Err(EditorDocumentError::CompositionActive);
        }
        if self.source_mode() {
            return Ok(self.command_result(false));
        }
        let disclosure = self.html_disclosure_header_at(at);
        let Some(disclosure) = disclosure else {
            return Ok(self.command_result(false));
        };
        if disclosure.open && disclosure.open_attribute.is_none() {
            let changed = Arc::make_mut(&mut self.presentation.markdown)
                .close_revealed_html(disclosure.opening);
            if changed {
                self.relocate_folded_selections(&disclosure)?;
                self.invalidate_html_presentation();
            }
            return Ok(CommandResult::with_source_change(
                self.revision(),
                self.selection(),
                changed,
                None,
            ));
        }
        let edit = if let Some(attribute) = disclosure.open_attribute {
            yu_text::Edit::new(attribute, "")
        } else {
            let at = ByteOffset::new(disclosure.opening.end().get() - 1);
            yu_text::Edit::new(TextRange::empty(at), " open")
        };
        self.state.history.break_group();
        self.apply_transaction_with_group(
            &Transaction::new(self.revision(), [edit]),
            HistoryGroup::External,
        )?;
        if disclosure.open
            && let Some(closed) = self.html_disclosure_header_at(disclosure.opening.start())
            && self.relocate_folded_selections(&closed)?
        {
            self.state
                .history
                .finish_selection(self.presentation.selections.clone());
        }
        self.state.history.break_group();
        Ok(self.command_result(true))
    }

    fn relocate_folded_selections(
        &mut self,
        disclosure: &yu_markdown::html::HtmlDisclosure,
    ) -> Result<bool, EditorDocumentError> {
        let snapshot = self.snapshot();
        let destination = disclosure
            .summary_content
            .map_or(disclosure.opening.start(), |summary| summary.end());
        let mut changed = false;
        let primary = self.selections().primary_index();
        let selections = self
            .selections()
            .as_slice()
            .iter()
            .map(|selection| {
                let range = selection.ordered_range();
                let hidden = if selection.is_empty() {
                    disclosure.body.start() <= range.start()
                        && range.start() <= disclosure.body.end()
                } else {
                    range.start() < disclosure.body.end() && disclosure.body.start() < range.end()
                };
                if hidden {
                    changed = true;
                    EditorSelection::cursor(&snapshot, destination, selection.affinity())
                } else {
                    Ok(*selection)
                }
            })
            .collect::<Result<Vec<_>, SelectionError>>()?;
        if changed {
            self.set_selections(selections, primary)?;
        }
        Ok(changed)
    }
}
