//! Spell-check suggestions remain ordinary source transactions with isolated undo.
use super::*;
use yu_text::Edit;

impl EditorDocument {
    pub fn replace_spelling(
        &mut self,
        range: TextRange,
        replacement: &str,
    ) -> Result<CommandResult, EditorDocumentError> {
        if self.composition().is_some() {
            return Err(EditorDocumentError::CompositionActive);
        }
        if range.is_empty()
            || replacement.is_empty()
            || replacement.chars().any(char::is_control)
            || !self.markdown().spelling_ranges(range).contains(&range)
        {
            return Err(EditorDocumentError::Selection(SelectionError::InvalidRange));
        }
        let snapshot = self.snapshot();
        if snapshot
            .as_str()
            .get(range.start().get() as usize..range.end().get() as usize)
            == Some(replacement)
        {
            return Ok(self.command_result(false));
        }
        self.state.history.break_group();
        self.apply_transaction(&Transaction::new(
            self.revision(),
            [Edit::new(range, replacement)],
        ))?;
        self.state.history.break_group();
        Ok(self.command_result(true))
    }
}
