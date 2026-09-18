//! Table word commands operate on the same projected text as atomic editing.
use super::*;

impl EditorDocument {
    pub(super) fn projected_table_cell(
        &self,
        focus: ByteOffset,
    ) -> Result<Option<VisualText>, EditorDocumentError> {
        let markdown = &self.presentation.markdown;
        let Some(block) = self
            .block_index_for_offset(focus)
            .and_then(|index| markdown.blocks().get(index))
        else {
            return Ok(None);
        };
        let Some(table) = yu_markdown::table_for_block(markdown, block) else {
            return Ok(None);
        };
        let Some(cell) = table
            .visible_cell_for_source(focus.get() as usize)
            .and_then(|address| table.visible_cell(address))
        else {
            return Ok(None);
        };
        let range = TextRange::new(
            ByteOffset::new(cell.start() as u64),
            ByteOffset::new(cell.end() as u64),
        )
        .expect("ordered cell");
        let Some(tree) = markdown.tree() else {
            return Ok(None);
        };
        let decorations = yu_markdown::ExtensionSet::markdown()
            .decorate(
                markdown.source(),
                tree,
                markdown.reference_definitions(),
                markdown.presentation(),
                block,
                None,
            )
            .map_err(DecorationError::from)?;
        Ok(Some(VisualText::new(
            markdown.source(),
            range,
            decorations.set().clone(),
        )?))
    }

    pub(super) fn table_word_target(
        &self,
        focus: ByteOffset,
        forward: bool,
    ) -> Result<Option<ByteOffset>, EditorDocumentError> {
        let Some(visual) = self.projected_table_cell(focus)? else {
            return Ok(None);
        };
        let position = visual.source_to_visual(focus, Bias::After)?.get() as usize;
        let target = if forward {
            next_word_boundary(visual.text(), position)
        } else {
            previous_word_boundary(visual.text(), position)
        };
        // Word commands stay in a cell; Tab/Shift-Tab navigate its neighbours.
        // In particular, extending at an edge must not select a structural pipe.
        if target == position {
            return Ok(Some(focus));
        }
        Ok(Some(visual.visual_to_source(
            yu_core::VisualOffset::new(target as u64),
            Bias::Before,
        )?))
    }
}
