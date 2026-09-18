//! Clipboard TSV decoding. Quoted line breaks are cell data, never row boundaries.
use super::*;

pub(super) fn parse_tsv(source: &str) -> Result<(usize, Vec<String>), EditorDocumentError> {
    let mut rows = Vec::<Vec<String>>::new();
    let mut row = Vec::new();
    let mut cell = String::new();
    let mut quoted = false;
    let mut closed_quote = false;
    let mut ended_row = false;
    let mut chars = source.chars().peekable();
    while let Some(ch) = chars.next() {
        ended_row = false;
        if quoted {
            if ch == '"' {
                if chars.peek() == Some(&'"') {
                    chars.next();
                    cell.push('"');
                } else {
                    quoted = false;
                    closed_quote = true;
                }
            } else {
                cell.push(ch);
            }
            continue;
        }
        if ch == '\t' || ch == '\r' || ch == '\n' {
            row.push(std::mem::take(&mut cell));
            closed_quote = false;
            if ch != '\t' {
                if ch == '\r' && chars.peek() == Some(&'\n') {
                    chars.next();
                }
                rows.push(std::mem::take(&mut row));
                ended_row = true;
            }
        } else if closed_quote {
            return Err(EditorDocumentError::InvalidTablePaste);
        } else if ch == '"' && cell.is_empty() {
            quoted = true;
        } else {
            cell.push(ch);
        }
    }
    if quoted {
        return Err(EditorDocumentError::InvalidTablePaste);
    }
    if !ended_row {
        row.push(cell);
        rows.push(row);
    }
    let columns = rows.iter().map(Vec::len).max().unwrap_or(1);
    let mut cells = Vec::new();
    for mut row in rows {
        row.resize(columns, String::new());
        cells.extend(row);
    }
    Ok((columns, cells))
}

impl EditorDocument {
    /// Copy a rectangular cell selection as visible, quoted TSV. Projection is
    /// identical to inactive document text and does not alter source or history.
    /// # Errors
    /// Returns an error if selection geometry or text projection is invalid.
    pub fn copy_table_tsv(&self) -> Result<Option<String>, EditorDocumentError> {
        let Some(columns) = self.selections().table_columns() else {
            return Ok(None);
        };
        let selections = self.selections().as_slice();
        let first = selections
            .first()
            .ok_or(EditorDocumentError::InvalidTablePaste)?;
        let block = self
            .block_index_for_offset(first.ordered_range().start())
            .and_then(|index| self.presentation.markdown.blocks().get(index))
            .ok_or(EditorDocumentError::InvalidTablePaste)?;
        let markdown = &self.presentation.markdown;
        let tree = markdown
            .tree()
            .ok_or(EditorDocumentError::InvalidTablePaste)?;
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
        let mut output = String::new();
        for (index, selection) in selections.iter().enumerate() {
            let range = selection.ordered_range();
            if range.start() < block.range().start() || range.end() > block.range().end() {
                return Err(EditorDocumentError::InvalidTablePaste);
            }
            if index > 0 {
                output.push(if index % columns == 0 { '\n' } else { '\t' });
            }
            let visual = VisualText::new(markdown.source(), range, decorations.set().clone())?;
            let text = visual.text();
            if text.contains(['\t', '\r', '\n', '"']) {
                output.push('"');
                output.push_str(&text.replace('"', "\"\""));
                output.push('"');
            } else {
                output.push_str(text);
            }
        }
        Ok(Some(output))
    }

    pub(super) fn tsv_grid(source: &str) -> Result<(usize, Vec<Arc<str>>), EditorDocumentError> {
        let (columns, cells) = parse_tsv(source)?;
        let cells = cells
            .into_iter()
            .map(|cell| {
                let mut markdown = String::new();
                let first_nonwhite = cell.len() - cell.trim_start().len();
                let last_nonwhite = cell.trim_end().len();
                let mut chars = cell.char_indices().peekable();
                while let Some((index, ch)) = chars.next() {
                    if ch == '\r' || ch == '\n' {
                        if ch == '\r' && chars.peek().is_some_and(|(_, next)| *next == '\n') {
                            chars.next();
                        }
                        markdown.push_str("<br>");
                    } else if ch == '\t'
                        || (ch.is_whitespace()
                            && (index < first_nonwhite || index >= last_nonwhite))
                    {
                        use std::fmt::Write;
                        write!(markdown, "&#{};", u32::from(ch)).expect("write to String");
                    } else {
                        if ch.is_ascii_punctuation() {
                            markdown.push('\\');
                        }
                        markdown.push(ch);
                    }
                }
                Arc::from(markdown)
            })
            .collect::<Vec<_>>();
        Ok((columns, cells))
    }

    pub(super) fn paste_clipboard_text(
        &mut self,
        text: Arc<str>,
        tabular: bool,
    ) -> Result<CommandResult, EditorDocumentError> {
        if tabular || (!self.grid_paste_is_outside_table() && text.contains('\t')) {
            self.paste_tsv(text)
        } else {
            self.paste_fragments(vec![text])
        }
    }

    pub(super) fn paste_tsv(
        &mut self,
        source: Arc<str>,
    ) -> Result<CommandResult, EditorDocumentError> {
        let (columns, cells) = Self::tsv_grid(&source)?;
        self.paste_table_grid(columns, cells)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quoted_tabs_newlines_quotes_and_unicode_remain_cell_data() {
        let (width, cells) =
            parse_tsv("\"a\tb\"\t\"first\r\nsecond\"\r\n\"say \"\"羽\"\"\"\t👨‍👩‍👧‍👦\r\n")
                .expect("quoted grid");
        assert_eq!(width, 2);
        assert_eq!(cells, ["a\tb", "first\r\nsecond", "say \"羽\"", "👨‍👩‍👧‍👦"]);
    }

    #[test]
    fn trailing_empty_fields_blank_rows_and_ragged_rows_are_retained() {
        assert_eq!(
            parse_tsv("a\tb\t\nc\n\n").expect("ragged"),
            (
                3,
                vec!["a", "b", "", "c", "", "", "", "", ""]
                    .into_iter()
                    .map(String::from)
                    .collect()
            )
        );
        assert_eq!(
            parse_tsv("a\t").expect("last empty"),
            (2, vec!["a".into(), "".into()])
        );
        assert_eq!(
            parse_tsv("").expect("empty field"),
            (1, vec![String::new()])
        );
        assert_eq!(
            parse_tsv("\"\"\r").expect("quoted empty"),
            (1, vec![String::new()])
        );
        assert_eq!(parse_tsv(" a \t\" b \"").expect("spaces").1, [" a ", " b "]);
    }

    #[test]
    fn malformed_quoted_fields_are_rejected_without_guessing() {
        for source in [
            "\"unterminated",
            "\"closed\"tail\tx",
            "a\t\"b",
            "\"a\"\"",
            "\"a\" \tb",
        ] {
            assert_eq!(
                parse_tsv(source),
                Err(EditorDocumentError::InvalidTablePaste)
            );
        }
        assert_eq!(
            parse_tsv("a\"b\tx").expect("unquoted quote").1,
            ["a\"b", "x"]
        );
    }

    #[test]
    fn tsv_command_preserves_literal_markdown_and_atomic_history() {
        let source = "> | H |\r\n> | :--- |\r\n> | z |";
        let mut document = EditorDocument::new(source);
        let at = ByteOffset::new(source.find('z').expect("cell") as u64);
        document.select_table_cells(at, at).expect("cell");
        document
            .execute(EditorCommand::PasteTsv(
                "**bold**\tx|y\r\n羽\t\"say \"\"hi\"\"\"\r\n".into(),
            ))
            .expect("paste TSV");
        let expected = concat!(
            "> | H |  |\r\n> | :--- | --- |\r\n",
            r"> | \*\*bold\*\* | x\|y |",
            "\r\n",
            r#"> | 羽 | say \"hi\" |"#
        );
        assert_eq!(document.snapshot().as_str(), expected);
        assert_eq!(document.selections().table_columns(), Some(2));
        document.execute(EditorCommand::Undo).expect("undo");
        assert_eq!(document.snapshot().as_str(), source);
        document.execute(EditorCommand::Redo).expect("redo");
        assert_eq!(document.snapshot().as_str(), expected);
    }

    #[test]
    fn malformed_tsv_fails_before_any_edit() {
        let mut document = EditorDocument::new("original");
        let revision = document.revision();
        for text in ["\"bad", "\"closed\"tail"] {
            let command = EditorCommand::PasteTsv(text.into());
            assert!(!document.command_available(&command));
            assert_eq!(
                document
                    .execute(command)
                    .expect_err("malformed quoted input"),
                EditorDocumentError::InvalidTablePaste
            );
            assert_eq!(document.snapshot().as_str(), "original");
            assert_eq!(document.revision(), revision);
        }
        assert!(!document.command_available(&EditorCommand::Undo));
    }

    #[test]
    fn multiline_tsv_encodes_data_and_projects_it_without_changing_table_shape() {
        let source = "> | H | V |\r\n> | --- | --- |\r\n> | x | y |";
        let mut document = EditorDocument::new(source);
        let at = ByteOffset::new(source.find('x').expect("cell") as u64);
        document.select_table_cells(at, at).expect("selection");
        document
            .execute(EditorCommand::PasteTsv(
                "\" first\r\nsecond \"\t\"a\tb\"\r\n\"<br>\"\t\" \"".into(),
            ))
            .expect("multiline TSV");
        let expected = "> | H | V |\r\n> | --- | --- |\r\n> | &#32;first<br>second&#32; | a&#9;b |\r\n> | \\<br\\> | &#32; |";
        assert_eq!(document.snapshot().as_str(), expected);
        let layout = document
            .block_layout(0, LayoutConfig::new(500.0, 16.0))
            .expect("layout");
        assert!(layout.visual().text().contains(" first\nsecond "));
        assert!(layout.visual().text().contains("a\tb"));
        assert!(layout.visual().text().contains("<br>"));
        assert_eq!(layout.table().expect("table").cells().len(), 6);
        document.execute(EditorCommand::Undo).expect("undo");
        assert_eq!(document.snapshot().as_str(), source);
        document.execute(EditorCommand::Redo).expect("redo");
        assert_eq!(document.snapshot().as_str(), expected);
    }

    #[test]
    fn copied_tsv_preserves_visible_data_and_roundtrips_quoted_boundaries() {
        for prefix in ["", "> "] {
            let source = format!("{prefix}| H | V |\n{prefix}| --- | --- |\n{prefix}| x | y |");
            let mut document = EditorDocument::new(source.clone());
            let at = ByteOffset::new(source.find('x').expect("cell") as u64);
            document.select_table_cells(at, at).expect("select");
            let input = "\" first\nsecond \"\t\"a\tb\"\n\"say \"\"羽\"\"\"\t\n**literal**\t<br>";
            document
                .execute(EditorCommand::PasteTsv(input.into()))
                .expect("paste");
            let before = document.snapshot().as_str().to_owned();
            let revision = document.revision();
            let copied = document.copy_table_tsv().expect("copy").expect("grid");
            assert_eq!(
                parse_tsv(&copied).expect("decode copy"),
                parse_tsv(input).expect("decode input")
            );
            assert_eq!(document.revision(), revision);
            assert_eq!(document.snapshot().as_str(), before);
            document
                .execute(EditorCommand::PasteTsv(copied.into()))
                .expect("paste again");
            assert_eq!(document.snapshot().as_str(), before);
        }
        let mut styled = EditorDocument::new("| **bold** | `code` |\n| --- | --- |");
        styled
            .select_table_cells(ByteOffset::new(2), ByteOffset::new(14))
            .expect("select styled");
        assert_eq!(
            styled.copy_table_tsv().expect("copy"),
            Some("bold\tcode".into())
        );
    }

    #[test]
    fn plain_tabbed_text_only_infers_a_grid_inside_a_table() {
        let mut plain = EditorDocument::new("");
        plain
            .execute(EditorCommand::PasteClipboardText {
                text: "a\tb\n".into(),
                tabular: false,
            })
            .expect("plain paste");
        assert_eq!(plain.snapshot().as_str(), "a\tb\n");
        let mut explicit = EditorDocument::new("");
        explicit
            .execute(EditorCommand::PasteClipboardText {
                text: "a\tb\n".into(),
                tabular: true,
            })
            .expect("tabular paste");
        assert_eq!(explicit.snapshot().as_str(), "| a | b |\n| --- | --- |");
        let mut table = EditorDocument::new("| x |\n| --- |");
        table
            .select_table_cells(ByteOffset::new(2), ByteOffset::new(2))
            .expect("cell");
        table
            .execute(EditorCommand::PasteClipboardText {
                text: "a\tb\n".into(),
                tabular: false,
            })
            .expect("contextual grid");
        assert_eq!(table.snapshot().as_str(), "| a | b |\n| --- | --- |");
    }
}
