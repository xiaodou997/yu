#![allow(dead_code)]

use std::fmt::Debug;

use yu_core::{ByteOffset, Revision, Utf16Offset, Utf16Range};
use yu_editor::{EditorCommand, EditorDocument, EditorSelection};

/// Small behavior-test harness for revision-bound editor interactions.
///
/// The notation reserves one `|` for a collapsed caret and `⟦`/`⟧` for the
/// anchor/focus endpoints of a selection. A source containing several `|`
/// characters is treated as literal source, which keeps Markdown tables
/// usable; callers can use [`EditorScenario::from_source`] when a source has a
/// single literal pipe and needs an explicit caret.
pub struct EditorScenario {
    document: EditorDocument,
}

impl EditorScenario {
    /// Creates a scenario from marked source, placing an unmarked document
    /// caret at the end when no marker is present.
    #[must_use]
    pub fn new(marked_source: &str) -> Self {
        let parsed = ParsedState::parse(marked_source);
        let mut scenario = Self {
            document: EditorDocument::new(parsed.source),
        };
        if let Some((anchor, focus)) = parsed.selection {
            scenario.set_selection_bytes(anchor, focus);
        }
        scenario
    }

    /// Creates a scenario from literal source, with the default end caret.
    #[must_use]
    pub fn from_source(source: &str) -> Self {
        Self {
            document: EditorDocument::new(source),
        }
    }

    /// Returns the canonical source without markers.
    #[must_use]
    pub fn source(&self) -> String {
        self.document.snapshot().as_str().to_owned()
    }

    /// Returns the current canonical revision.
    #[must_use]
    pub fn revision(&self) -> Revision {
        self.document.revision()
    }

    /// Returns the current editor selection.
    #[must_use]
    pub fn selection(&self) -> EditorSelection {
        self.document.selection()
    }

    /// Asserts source and caret/selection using the same notation accepted by
    /// [`EditorScenario::new`].
    pub fn expect_state(&mut self, expected: &str) -> &mut Self {
        let parsed = ParsedState::parse(expected);
        assert_eq!(
            self.source(),
            parsed.source,
            "canonical source differs from expected editor state"
        );
        if let Some((anchor, focus)) = parsed.selection {
            assert_eq!(
                (
                    self.selection().anchor().get(),
                    self.selection().focus().get()
                ),
                (anchor, focus),
                "selection differs from expected editor state"
            );
        } else {
            assert_eq!(
                self.selection().focus().get(),
                self.source().len() as u64,
                "an unmarked expected state requires the caret at source end"
            );
            assert!(
                self.selection().is_empty(),
                "an unmarked expected state requires a collapsed caret"
            );
        }
        self
    }

    /// Asserts the canonical source without changing selection expectations.
    pub fn expect_source(&mut self, expected: &str) -> &mut Self {
        assert_eq!(self.source(), expected, "canonical source differs");
        self
    }

    /// Asserts the current revision.
    pub fn expect_revision(&mut self, expected: Revision) -> &mut Self {
        assert_eq!(self.revision(), expected, "editor revision differs");
        self
    }

    /// Asserts the currently projected preedit without treating it as source.
    pub fn expect_composition(&mut self, expected: &str) -> &mut Self {
        assert_eq!(
            self.document
                .composition()
                .map(|composition| composition.text()),
            Some(expected),
            "composition overlay differs"
        );
        self
    }

    /// Asserts that no transient IME overlay remains active.
    pub fn expect_no_composition(&mut self) -> &mut Self {
        assert!(
            self.document.composition().is_none(),
            "expected no active composition overlay"
        );
        self
    }

    /// Places a collapsed caret at a UTF-8 byte boundary.
    pub fn set_caret(&mut self, byte_offset: usize) -> &mut Self {
        self.set_selection_bytes(byte_offset as u64, byte_offset as u64);
        self
    }

    /// Sets an ordered or backward selection using UTF-8 byte offsets.
    pub fn set_selection(&mut self, anchor: usize, focus: usize) -> &mut Self {
        self.set_selection_bytes(anchor as u64, focus as u64);
        self
    }

    pub fn insert(&mut self, text: &str) -> &mut Self {
        self.execute(EditorCommand::insert_text(text), "insert text")
    }

    pub fn backspace(&mut self) -> &mut Self {
        self.execute(EditorCommand::DeleteBackward, "backspace")
    }

    pub fn delete_forward(&mut self) -> &mut Self {
        self.execute(EditorCommand::DeleteForward, "forward delete")
    }

    pub fn left(&mut self) -> &mut Self {
        self.execute(EditorCommand::MoveLeft, "move left")
    }

    pub fn right(&mut self) -> &mut Self {
        self.execute(EditorCommand::MoveRight, "move right")
    }

    pub fn word_left(&mut self) -> &mut Self {
        self.execute(EditorCommand::move_word_left(), "move word left")
    }

    pub fn word_right(&mut self) -> &mut Self {
        self.execute(EditorCommand::move_word_right(), "move word right")
    }

    pub fn up(&mut self) -> &mut Self {
        self.execute(EditorCommand::move_up(), "move up")
    }

    pub fn down(&mut self) -> &mut Self {
        self.execute(EditorCommand::move_down(), "move down")
    }

    pub fn shift_up(&mut self) -> &mut Self {
        self.execute(EditorCommand::move_up_extend(), "extend up")
    }

    pub fn shift_down(&mut self) -> &mut Self {
        self.execute(EditorCommand::move_down_extend(), "extend down")
    }

    pub fn enter(&mut self) -> &mut Self {
        self.execute(EditorCommand::insert_newline(), "insert newline")
    }

    pub fn indent(&mut self) -> &mut Self {
        self.execute(EditorCommand::indent_list(), "indent list")
    }

    pub fn outdent(&mut self) -> &mut Self {
        self.execute(EditorCommand::outdent_list(), "outdent list")
    }

    pub fn undo(&mut self) -> &mut Self {
        self.execute(EditorCommand::undo(), "undo")
    }

    pub fn redo(&mut self) -> &mut Self {
        self.execute(EditorCommand::redo(), "redo")
    }

    pub fn toggle_task(&mut self, block: usize) -> &mut Self {
        self.execute(EditorCommand::toggle_task(block), "toggle task")
    }

    /// Starts an IME overlay over the current selection and places its active
    /// selection at the end of the supplied preedit text.
    pub fn begin_composition(&mut self, preedit: &str) -> &mut Self {
        let replacement_range = self.selection().ordered_range();
        let preedit_end = preedit.encode_utf16().count() as u64;
        let selection = Utf16Range::empty(Utf16Offset::new(preedit_end));
        self.document
            .begin_composition(replacement_range, preedit, selection)
            .unwrap_or_else(|error| panic!("begin composition failed: {error}"));
        self.document
            .update_composition(preedit, selection)
            .unwrap_or_else(|error| panic!("initial composition update failed: {error}"));
        self
    }

    pub fn update_composition(&mut self, preedit: &str) -> &mut Self {
        let end = preedit.encode_utf16().count() as u64;
        self.document
            .update_composition(preedit, Utf16Range::empty(Utf16Offset::new(end)))
            .unwrap_or_else(|error| panic!("composition update failed: {error}"));
        self
    }

    pub fn commit_composition(&mut self, committed: &str) -> &mut Self {
        self.document
            .commit_composition(committed)
            .unwrap_or_else(|error| panic!("composition commit failed: {error}"));
        self
    }

    pub fn cancel_composition(&mut self) -> &mut Self {
        assert!(
            self.document.cancel_composition(),
            "expected an active composition to cancel"
        );
        self
    }

    /// 放一组光标（UTF-8 字节偏移），`primary` 是其中哪一个是主光标。
    pub fn set_carets(&mut self, offsets: &[usize], primary: usize) -> &mut Self {
        let ranges: Vec<_> = offsets.iter().map(|offset| (*offset, *offset)).collect();
        self.set_selections(&ranges, primary)
    }

    /// 放一组选区（anchor, focus 的 UTF-8 字节偏移）。
    pub fn set_selections(&mut self, ranges: &[(usize, usize)], primary: usize) -> &mut Self {
        let snapshot = self.document.snapshot();
        let selections: Vec<_> = ranges
            .iter()
            .map(|(anchor, focus)| {
                EditorSelection::range(
                    &snapshot,
                    ByteOffset::new(*anchor as u64),
                    ByteOffset::new(*focus as u64),
                    yu_editor::CaretAffinity::Downstream,
                )
                .unwrap_or_else(|error| panic!("invalid test selection: {error}"))
            })
            .collect();
        self.document
            .set_selections(selections, primary)
            .unwrap_or_else(|error| panic!("setting test selections failed: {error}"));
        self
    }

    /// 全部选区的 `(start, end)`，按文档顺序。
    #[must_use]
    pub fn selection_ranges(&self) -> Vec<(u64, u64)> {
        self.document
            .selections()
            .as_slice()
            .iter()
            .map(|selection| {
                let range = selection.ordered_range();
                (range.start().get(), range.end().get())
            })
            .collect()
    }

    /// 断言全部选区。**这是多光标用例的主判据**：只断 primary 会让「另外几个
    /// 光标没动」静默通过。
    pub fn expect_selections(&mut self, expected: &[(u64, u64)]) -> &mut Self {
        assert_eq!(
            self.selection_ranges(),
            expected.to_vec(),
            "selections differ from expected"
        );
        self
    }

    /// 断言主选区是第几条。
    pub fn expect_primary_index(&mut self, expected: usize) -> &mut Self {
        assert_eq!(
            self.document.selections().primary_index(),
            expected,
            "primary index differs"
        );
        self
    }

    #[must_use]
    pub fn document(&self) -> &EditorDocument {
        &self.document
    }

    #[must_use]
    pub fn document_mut(&mut self) -> &mut EditorDocument {
        &mut self.document
    }

    fn execute(&mut self, command: EditorCommand, description: &str) -> &mut Self {
        self.document
            .execute(command)
            .unwrap_or_else(|error| panic!("{description} failed: {error}"));
        self
    }

    fn set_selection_bytes(&mut self, anchor: u64, focus: u64) {
        let snapshot = self.document.snapshot();
        let selection = EditorSelection::range(
            &snapshot,
            ByteOffset::new(anchor),
            ByteOffset::new(focus),
            yu_editor::CaretAffinity::Downstream,
        )
        .unwrap_or_else(|error| panic!("invalid test selection: {error}"));
        self.document
            .set_selection(selection)
            .unwrap_or_else(|error| panic!("setting test selection failed: {error}"));
    }
}

#[derive(Debug)]
struct ParsedState {
    source: String,
    selection: Option<(u64, u64)>,
}

impl ParsedState {
    fn parse(marked: &str) -> Self {
        let pipe_count = marked.matches('|').count();
        let mut source = String::with_capacity(marked.len());
        let mut caret = None;
        let mut anchor = None;
        let mut focus = None;

        for character in marked.chars() {
            match character {
                '|' if pipe_count == 1 => {
                    assert!(
                        caret.replace(source.len() as u64).is_none(),
                        "state notation contains multiple caret markers"
                    );
                }
                '⟦' => {
                    assert!(
                        anchor.replace(source.len() as u64).is_none(),
                        "state notation contains multiple anchor markers"
                    );
                }
                '⟧' => {
                    assert!(
                        focus.replace(source.len() as u64).is_none(),
                        "state notation contains multiple focus markers"
                    );
                }
                _ => source.push(character),
            }
        }

        assert!(
            caret.is_none() || (anchor.is_none() && focus.is_none()),
            "state notation cannot mix caret and selection markers"
        );
        let selection = match (caret, anchor, focus) {
            (Some(caret), None, None) => Some((caret, caret)),
            (None, Some(anchor), Some(focus)) => Some((anchor, focus)),
            (None, None, None) => None,
            _ => panic!("state notation must contain both ⟦ and ⟧ markers"),
        };
        Self { source, selection }
    }
}

impl Debug for EditorScenario {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("EditorScenario")
            .field("source", &self.source())
            .field("selection", &self.selection())
            .finish()
    }
}
