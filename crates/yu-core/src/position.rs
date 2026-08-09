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
}
