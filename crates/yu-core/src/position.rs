use std::fmt;

/// A byte offset into a UTF-8 source snapshot.
///
/// The offset is meaningful only for the revision from which it was produced.
#[derive(Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ByteOffset(u64);

impl ByteOffset {
    pub const ZERO: Self = Self(0);

    #[must_use]
    pub const fn new(value: u64) -> Self {
        Self(value)
    }

    #[must_use]
    pub const fn get(self) -> u64 {
        self.0
    }

    #[must_use]
    pub const fn checked_add(self, bytes: u64) -> Option<Self> {
        match self.0.checked_add(bytes) {
            Some(value) => Some(Self(value)),
            None => None,
        }
    }
}

impl fmt::Debug for ByteOffset {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "ByteOffset({})", self.0)
    }
}

impl From<u32> for ByteOffset {
    fn from(value: u32) -> Self {
        Self(u64::from(value))
    }
}

impl TryFrom<usize> for ByteOffset {
    type Error = std::num::TryFromIntError;

    fn try_from(value: usize) -> Result<Self, Self::Error> {
        Ok(Self(value.try_into()?))
    }
}

impl TryFrom<ByteOffset> for usize {
    type Error = std::num::TryFromIntError;

    fn try_from(value: ByteOffset) -> Result<Self, Self::Error> {
        value.0.try_into()
    }
}

/// A zero-based logical line index. Lines are separated by LF source bytes.
#[derive(Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct LineIndex(u64);

impl LineIndex {
    pub const ZERO: Self = Self(0);

    #[must_use]
    pub const fn new(value: u64) -> Self {
        Self(value)
    }

    #[must_use]
    pub const fn get(self) -> u64 {
        self.0
    }
}

impl fmt::Debug for LineIndex {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "LineIndex({})", self.0)
    }
}

/// An offset measured in UTF-16 code units for native text-system bridges.
#[derive(Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Utf16Offset(u64);

impl Utf16Offset {
    pub const ZERO: Self = Self(0);

    #[must_use]
    pub const fn new(value: u64) -> Self {
        Self(value)
    }

    #[must_use]
    pub const fn get(self) -> u64 {
        self.0
    }
}

impl fmt::Debug for Utf16Offset {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "Utf16Offset({})", self.0)
    }
}

/// A half-open range measured in UTF-16 code units.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct Utf16Range {
    start: Utf16Offset,
    end: Utf16Offset,
}

impl Utf16Range {
    #[must_use]
    pub const fn new(start: Utf16Offset, end: Utf16Offset) -> Option<Self> {
        if start.get() <= end.get() {
            Some(Self { start, end })
        } else {
            None
        }
    }

    #[must_use]
    pub const fn empty(at: Utf16Offset) -> Self {
        Self { start: at, end: at }
    }

    #[must_use]
    pub const fn start(self) -> Utf16Offset {
        self.start
    }

    #[must_use]
    pub const fn end(self) -> Utf16Offset {
        self.end
    }

    #[must_use]
    pub const fn len(self) -> u64 {
        self.end.get() - self.start.get()
    }

    #[must_use]
    pub const fn is_empty(self) -> bool {
        self.start.get() == self.end.get()
    }
}

impl fmt::Debug for Utf16Range {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "Utf16Range({}..{})",
            self.start.get(),
            self.end.get()
        )
    }
}

/// A half-open UTF-8 byte range belonging to one source revision.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct TextRange {
    start: ByteOffset,
    end: ByteOffset,
}

impl TextRange {
    #[must_use]
    pub const fn new(start: ByteOffset, end: ByteOffset) -> Option<Self> {
        if start.get() <= end.get() {
            Some(Self { start, end })
        } else {
            None
        }
    }

    #[must_use]
    pub const fn empty(at: ByteOffset) -> Self {
        Self { start: at, end: at }
    }

    #[must_use]
    pub const fn start(self) -> ByteOffset {
        self.start
    }

    #[must_use]
    pub const fn end(self) -> ByteOffset {
        self.end
    }

    #[must_use]
    pub const fn len(self) -> u64 {
        self.end.get() - self.start.get()
    }

    #[must_use]
    pub const fn is_empty(self) -> bool {
        self.start.get() == self.end.get()
    }

    #[must_use]
    pub const fn contains(self, offset: ByteOffset) -> bool {
        self.start.get() <= offset.get() && offset.get() < self.end.get()
    }
}

impl fmt::Debug for TextRange {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "TextRange({}..{})",
            self.start.get(),
            self.end.get()
        )
    }
}

/// Monotonically increasing identity of a document state.
#[derive(Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Revision(u64);

impl Revision {
    pub const INITIAL: Self = Self(0);

    #[must_use]
    pub const fn new(value: u64) -> Self {
        Self(value)
    }

    #[must_use]
    pub const fn get(self) -> u64 {
        self.0
    }

    #[must_use]
    pub const fn next(self) -> Option<Self> {
        match self.0.checked_add(1) {
            Some(value) => Some(Self(value)),
            None => None,
        }
    }
}

impl fmt::Debug for Revision {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "Revision({})", self.0)
    }
}

/// Determines which side of inserted or replacement text an anchor follows.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Affinity {
    #[default]
    Before,
    After,
}

/// A source position that can be mapped through a [`yu_text::ChangeSet`].
///
/// The mapping implementation lives in `yu-text`; the type is kept here so
/// editor, syntax and platform contracts can refer to it without cycles.
///
/// [`yu_text::ChangeSet`]: https://docs.rs/yu-text
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct TextAnchor {
    revision: Revision,
    offset: ByteOffset,
    affinity: Affinity,
}

impl TextAnchor {
    #[must_use]
    pub const fn new(revision: Revision, offset: ByteOffset, affinity: Affinity) -> Self {
        Self {
            revision,
            offset,
            affinity,
        }
    }

    #[must_use]
    pub const fn revision(self) -> Revision {
        self.revision
    }

    #[must_use]
    pub const fn offset(self) -> ByteOffset {
        self.offset
    }

    #[must_use]
    pub const fn affinity(self) -> Affinity {
        self.affinity
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_range_rejects_reverse_bounds() {
        assert!(TextRange::new(ByteOffset::new(9), ByteOffset::new(3)).is_none());
    }

    #[test]
    fn text_range_is_half_open() {
        let range = TextRange::new(ByteOffset::new(2), ByteOffset::new(4))
            .expect("test range should be valid");

        assert!(range.contains(ByteOffset::new(2)));
        assert!(range.contains(ByteOffset::new(3)));
        assert!(!range.contains(ByteOffset::new(4)));
    }

    #[test]
    fn line_indexes_are_zero_based() {
        assert_eq!(LineIndex::ZERO.get(), 0);
        assert!(LineIndex::new(2) > LineIndex::new(1));
    }
}

/// An offset in the projected UTF-8 visual stream.
///
/// It is not a source byte offset. A visual offset is only meaningful for the
/// projection revision and range that produced it.
#[derive(Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct VisualOffset(u64);

impl VisualOffset {
    pub const ZERO: Self = Self(0);

    #[must_use]
    pub const fn new(value: u64) -> Self {
        Self(value)
    }

    #[must_use]
    pub const fn get(self) -> u64 {
        self.0
    }

    #[must_use]
    pub const fn checked_add(self, bytes: u64) -> Option<Self> {
        match self.0.checked_add(bytes) {
            Some(value) => Some(Self(value)),
            None => None,
        }
    }
}

impl fmt::Debug for VisualOffset {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "VisualOffset({})", self.0)
    }
}

impl TryFrom<usize> for VisualOffset {
    type Error = std::num::TryFromIntError;

    fn try_from(value: usize) -> Result<Self, Self::Error> {
        Ok(Self(value.try_into()?))
    }
}

/// A half-open range in projected UTF-8 visual bytes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct VisualRange {
    start: VisualOffset,
    end: VisualOffset,
}

impl VisualRange {
    #[must_use]
    pub const fn new(start: VisualOffset, end: VisualOffset) -> Option<Self> {
        if start.get() <= end.get() {
            Some(Self { start, end })
        } else {
            None
        }
    }

    #[must_use]
    pub const fn empty(at: VisualOffset) -> Self {
        Self { start: at, end: at }
    }

    #[must_use]
    pub const fn start(self) -> VisualOffset {
        self.start
    }

    #[must_use]
    pub const fn end(self) -> VisualOffset {
        self.end
    }

    #[must_use]
    pub const fn len(self) -> u64 {
        self.end.get() - self.start.get()
    }

    #[must_use]
    pub const fn is_empty(self) -> bool {
        self.start.get() == self.end.get()
    }
}

/// Selects one of the two visual caret locations available at a line boundary.
///
/// This is deliberately separate from [`Affinity`], which controls how source
/// anchors follow edits. Caret affinity is a layout concern: upstream is the end
/// of the preceding visual line, downstream is the start of the next.
/// `docs/specs/coordinates.md` forbids reusing one for the other.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum CaretAffinity {
    Upstream,
    #[default]
    Downstream,
}

/// A caret anchored to a UTF-8 source position in one immutable revision.
///
/// Constructing one does **not** mean the position is valid: this type only
/// records `(revision, offset, affinity)`. Whether the offset is on a character
/// boundary of that revision is checked by the caret map that produced it
/// (`yu_state::CaretPositionMap`), not here.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct SourceCaretPosition {
    revision: Revision,
    offset: ByteOffset,
    affinity: CaretAffinity,
}

impl SourceCaretPosition {
    #[must_use]
    pub const fn new(revision: Revision, offset: ByteOffset, affinity: CaretAffinity) -> Self {
        Self {
            revision,
            offset,
            affinity,
        }
    }

    #[must_use]
    pub const fn revision(self) -> Revision {
        self.revision
    }

    #[must_use]
    pub const fn offset(self) -> ByteOffset {
        self.offset
    }

    #[must_use]
    pub const fn affinity(self) -> CaretAffinity {
        self.affinity
    }
}

/// A caret position expressed in the UTF-16 coordinates used by native text systems.
///
/// Same caveat as [`SourceCaretPosition`]: the constructor records, it does not
/// validate. A UTF-16 offset in the middle of a surrogate pair is rejected by the
/// caret map, not by this type.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct NativeCaretPosition {
    revision: Revision,
    offset: Utf16Offset,
    affinity: CaretAffinity,
}

impl NativeCaretPosition {
    #[must_use]
    pub const fn new(revision: Revision, offset: Utf16Offset, affinity: CaretAffinity) -> Self {
        Self {
            revision,
            offset,
            affinity,
        }
    }

    #[must_use]
    pub const fn revision(self) -> Revision {
        self.revision
    }

    #[must_use]
    pub const fn offset(self) -> Utf16Offset {
        self.offset
    }

    #[must_use]
    pub const fn affinity(self) -> CaretAffinity {
        self.affinity
    }
}
