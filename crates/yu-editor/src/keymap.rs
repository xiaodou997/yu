use crate::EditorCommand;

/// Logical keys understood by the platform-independent editor command layer.
///
/// Printable text is intentionally not represented as an editing command here.
/// Native text-input adapters deliver committed text through their text-input
/// protocol (for example `insertText`/`NSTextInputClient` on macOS), while this
/// map is reserved for commands that must be consumed before text insertion.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EditorKey {
    Character(char),
    Enter,
    Tab,
    Backspace,
    Delete,
    Left,
    Right,
    Up,
    Down,
    Escape,
}

/// Modifier bits shared by native key adapters.
///
/// The `Command` bit means the platform's primary application shortcut
/// modifier. It maps to Command on macOS and can map to Control on a future
/// Windows/Linux adapter without changing command semantics.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct KeyModifiers(u8);

impl KeyModifiers {
    pub const NONE: Self = Self(0);
    pub const COMMAND: Self = Self(1 << 0);
    pub const SHIFT: Self = Self(1 << 1);
    pub const CONTROL: Self = Self(1 << 2);
    pub const OPTION: Self = Self(1 << 3);

    const ALL_BITS: u8 = Self::COMMAND.0 | Self::SHIFT.0 | Self::CONTROL.0 | Self::OPTION.0;

    /// Creates modifiers from native adapter bits, ignoring platform flags
    /// that do not participate in the editor command contract (Caps Lock,
    /// numeric-pad and function-key flags, for example).
    #[must_use]
    pub const fn from_bits(bits: u8) -> Self {
        Self(bits & Self::ALL_BITS)
    }

    #[must_use]
    pub const fn bits(self) -> u8 {
        self.0
    }

    #[must_use]
    pub const fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }
}

impl std::ops::BitOr for KeyModifiers {
    type Output = Self;

    fn bitor(self, rhs: Self) -> Self::Output {
        Self::from_bits(self.0 | rhs.0)
    }
}

/// One logical key event passed to the command resolver.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct KeyEvent {
    key: EditorKey,
    modifiers: KeyModifiers,
}

impl KeyEvent {
    #[must_use]
    pub const fn new(key: EditorKey, modifiers: KeyModifiers) -> Self {
        Self { key, modifiers }
    }

    #[must_use]
    pub const fn key(self) -> EditorKey {
        self.key
    }

    #[must_use]
    pub const fn modifiers(self) -> KeyModifiers {
        self.modifiers
    }
}

/// Resolves a native key event into a source-editing command.
///
/// `None` means the event must remain in the platform text-input/default
/// command path. In particular, printable text without a command modifier is
/// deliberately not inserted here, so IME composition remains authoritative.
#[must_use]
pub fn command_for_key(event: KeyEvent) -> Option<EditorCommand> {
    let key = event.key();
    let modifiers = event.modifiers();

    if let EditorKey::Character(character) = key {
        let character = character.to_ascii_lowercase();
        if character == 'z' && modifiers == KeyModifiers::COMMAND {
            return Some(EditorCommand::undo());
        }
        if character == 'z' && modifiers == (KeyModifiers::COMMAND | KeyModifiers::SHIFT) {
            return Some(EditorCommand::redo());
        }
        return None;
    }

    match (key, modifiers) {
        (EditorKey::Enter, KeyModifiers::NONE | KeyModifiers::SHIFT) => {
            Some(EditorCommand::insert_newline())
        }
        (EditorKey::Tab, KeyModifiers::NONE) => Some(EditorCommand::indent_list()),
        (EditorKey::Tab, KeyModifiers::SHIFT) => Some(EditorCommand::outdent_list()),
        (EditorKey::Backspace, KeyModifiers::NONE) => Some(EditorCommand::DeleteBackward),
        (EditorKey::Delete, KeyModifiers::NONE) => Some(EditorCommand::DeleteForward),
        (EditorKey::Left, KeyModifiers::NONE) => Some(EditorCommand::MoveLeft),
        (EditorKey::Right, KeyModifiers::NONE) => Some(EditorCommand::MoveRight),
        (EditorKey::Left, KeyModifiers::OPTION | KeyModifiers::CONTROL) => {
            Some(EditorCommand::move_word_left())
        }
        (EditorKey::Right, KeyModifiers::OPTION | KeyModifiers::CONTROL) => {
            Some(EditorCommand::move_word_right())
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn event(key: EditorKey, modifiers: KeyModifiers) -> KeyEvent {
        KeyEvent::new(key, modifiers)
    }

    #[test]
    fn macos_undo_and_redo_shortcuts_are_distinct() {
        assert_eq!(
            command_for_key(event(EditorKey::Character('z'), KeyModifiers::COMMAND,)),
            Some(EditorCommand::undo())
        );
        assert_eq!(
            command_for_key(event(
                EditorKey::Character('Z'),
                KeyModifiers::COMMAND | KeyModifiers::SHIFT,
            )),
            Some(EditorCommand::redo())
        );
        assert_eq!(
            command_for_key(event(
                EditorKey::Character('z'),
                KeyModifiers::COMMAND | KeyModifiers::OPTION,
            )),
            None
        );
    }

    #[test]
    fn structural_keys_map_to_editor_commands() {
        assert_eq!(
            command_for_key(event(EditorKey::Enter, KeyModifiers::NONE)),
            Some(EditorCommand::insert_newline())
        );
        assert_eq!(
            command_for_key(event(EditorKey::Enter, KeyModifiers::SHIFT)),
            Some(EditorCommand::insert_newline())
        );
        assert_eq!(
            command_for_key(event(EditorKey::Tab, KeyModifiers::NONE)),
            Some(EditorCommand::indent_list())
        );
        assert_eq!(
            command_for_key(event(EditorKey::Tab, KeyModifiers::SHIFT)),
            Some(EditorCommand::outdent_list())
        );
        assert_eq!(
            command_for_key(event(EditorKey::Backspace, KeyModifiers::NONE)),
            Some(EditorCommand::DeleteBackward)
        );
        assert_eq!(
            command_for_key(event(EditorKey::Delete, KeyModifiers::NONE)),
            Some(EditorCommand::DeleteForward)
        );
        assert_eq!(
            command_for_key(event(EditorKey::Left, KeyModifiers::NONE)),
            Some(EditorCommand::MoveLeft)
        );
        assert_eq!(
            command_for_key(event(EditorKey::Right, KeyModifiers::NONE)),
            Some(EditorCommand::MoveRight)
        );
        assert_eq!(
            command_for_key(event(EditorKey::Left, KeyModifiers::OPTION)),
            Some(EditorCommand::move_word_left())
        );
        assert_eq!(
            command_for_key(event(EditorKey::Right, KeyModifiers::CONTROL)),
            Some(EditorCommand::move_word_right())
        );
    }

    #[test]
    fn printable_text_and_unowned_shortcuts_stay_on_native_input_path() {
        assert_eq!(
            command_for_key(event(EditorKey::Character('a'), KeyModifiers::NONE)),
            None
        );
        assert_eq!(
            command_for_key(event(EditorKey::Character('a'), KeyModifiers::COMMAND)),
            None
        );
        assert_eq!(
            command_for_key(event(EditorKey::Escape, KeyModifiers::NONE)),
            None
        );
    }

    #[test]
    fn unknown_native_modifier_bits_are_ignored() {
        let modifiers = KeyModifiers::from_bits(KeyModifiers::COMMAND.bits() | (1 << 7));
        assert_eq!(
            command_for_key(event(EditorKey::Character('z'), modifiers)),
            Some(EditorCommand::undo())
        );
    }
}
