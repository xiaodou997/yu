//! Platform-neutral TSF/ACP projection helpers.
//!
//! TSF's ACP positions are UTF-16 code-unit offsets. Yu's canonical source is
//! UTF-8, but `TextSnapshot` is already the single authority for converting
//! between the two. This module only adds TSF's signed-i32 bounds and projects
//! an active composition over canonical source; it never owns a second source.

use std::fmt;

use yu_core::{ByteOffset, CaretAffinity, TextRange, Utf16Offset, Utf16Range};
use yu_editor::{CompositionOverlay, EditorSelection};
use yu_text::TextSnapshot;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct AcpRange {
    start: i32,
    end: i32,
}

impl AcpRange {
    pub(crate) fn new(start: i32, end: i32) -> Result<Self, AcpError> {
        if start < 0 || end < start {
            return Err(AcpError::InvalidRange { start, end });
        }
        Ok(Self { start, end })
    }

    pub(crate) const fn start(self) -> i32 {
        self.start
    }

    pub(crate) const fn end(self) -> i32 {
        self.end
    }

    pub(crate) fn len(self) -> u32 {
        (self.end - self.start) as u32
    }

    pub(crate) fn contains(self, position: i32) -> bool {
        position >= self.start && position <= self.end
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct AcpSelection {
    pub(crate) range: AcpRange,
    pub(crate) caret_at_start: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ProjectedComposition {
    pub(crate) range: AcpRange,
    pub(crate) selection: AcpRange,
    pub(crate) canonical: TextRange,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct AcpProjection {
    utf16: Vec<u16>,
    canonical_len: i32,
    composition: Option<ProjectedComposition>,
    selection: AcpSelection,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum AcpError {
    PositionOutOfRange(i32),
    InvalidRange { start: i32, end: i32 },
    OffsetOverflow,
    InsideSurrogatePair(i32),
    Selection(String),
}

impl fmt::Display for AcpError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::PositionOutOfRange(position) => {
                write!(
                    formatter,
                    "ACP position {position} is outside the text stream"
                )
            }
            Self::InvalidRange { start, end } => {
                write!(formatter, "invalid ACP range [{start}, {end})")
            }
            Self::OffsetOverflow => formatter.write_str("text stream exceeds TSF ACP i32 range"),
            Self::InsideSurrogatePair(position) => {
                write!(
                    formatter,
                    "ACP position {position} splits a UTF-16 surrogate pair"
                )
            }
            Self::Selection(error) => formatter.write_str(error),
        }
    }
}

impl std::error::Error for AcpError {}

impl AcpProjection {
    pub(crate) fn new(
        snapshot: &TextSnapshot,
        selection: EditorSelection,
        composition: Option<&CompositionOverlay>,
    ) -> Result<Self, AcpError> {
        let canonical: Vec<u16> = snapshot.as_str().encode_utf16().collect();
        let canonical_len = i32::try_from(canonical.len()).map_err(|_| AcpError::OffsetOverflow)?;

        let (utf16, projected_composition, projected_selection) = if let Some(composition) =
            composition
        {
            if composition.base_revision() != snapshot.revision() {
                return Err(AcpError::Selection(
                    "composition and source revisions differ".to_owned(),
                ));
            }
            let canonical_range = composition.replacement_range();
            let source_start = snapshot
                .utf16_offset(canonical_range.start())
                .map_err(|error| AcpError::Selection(error.to_string()))?;
            let source_end = snapshot
                .utf16_offset(canonical_range.end())
                .map_err(|error| AcpError::Selection(error.to_string()))?;
            let source_start =
                i32::try_from(source_start.get()).map_err(|_| AcpError::OffsetOverflow)?;
            let source_end =
                i32::try_from(source_end.get()).map_err(|_| AcpError::OffsetOverflow)?;

            let preedit: Vec<u16> = composition.text().encode_utf16().collect();
            let preedit_len = i32::try_from(preedit.len()).map_err(|_| AcpError::OffsetOverflow)?;
            let projected_end = source_start
                .checked_add(preedit_len)
                .ok_or(AcpError::OffsetOverflow)?;

            let mut utf16 = Vec::with_capacity(
                canonical
                    .len()
                    .saturating_sub((source_end - source_start) as usize)
                    .saturating_add(preedit.len()),
            );
            utf16.extend_from_slice(&canonical[..source_start as usize]);
            utf16.extend_from_slice(&preedit);
            utf16.extend_from_slice(&canonical[source_end as usize..]);

            let local = composition.selection_utf16();
            let local_start =
                i32::try_from(local.start().get()).map_err(|_| AcpError::OffsetOverflow)?;
            let local_end =
                i32::try_from(local.end().get()).map_err(|_| AcpError::OffsetOverflow)?;
            let selected_start = source_start
                .checked_add(local_start)
                .ok_or(AcpError::OffsetOverflow)?;
            let selected_end = source_start
                .checked_add(local_end)
                .ok_or(AcpError::OffsetOverflow)?;
            let composition_range = AcpRange::new(source_start, projected_end)?;
            let composition_selection = AcpRange::new(selected_start, selected_end)?;
            if !composition_range.contains(composition_selection.start())
                || !composition_range.contains(composition_selection.end())
            {
                return Err(AcpError::InvalidRange {
                    start: composition_selection.start(),
                    end: composition_selection.end(),
                });
            }

            (
                utf16,
                Some(ProjectedComposition {
                    range: composition_range,
                    selection: composition_selection,
                    canonical: canonical_range,
                }),
                AcpSelection {
                    range: composition_selection,
                    caret_at_start: false,
                },
            )
        } else {
            let range = selection
                .utf16_range(snapshot)
                .map_err(|error| AcpError::Selection(error.to_string()))?;
            let start = i32::try_from(range.start().get()).map_err(|_| AcpError::OffsetOverflow)?;
            let end = i32::try_from(range.end().get()).map_err(|_| AcpError::OffsetOverflow)?;
            (
                canonical,
                None,
                AcpSelection {
                    range: AcpRange::new(start, end)?,
                    caret_at_start: selection.focus() < selection.anchor(),
                },
            )
        };

        i32::try_from(utf16.len()).map_err(|_| AcpError::OffsetOverflow)?;
        Ok(Self {
            utf16,
            canonical_len,
            composition: projected_composition,
            selection: projected_selection,
        })
    }

    pub(crate) fn text(&self) -> &[u16] {
        &self.utf16
    }

    pub(crate) fn end_acp(&self) -> i32 {
        i32::try_from(self.utf16.len()).expect("constructor bounded ACP to i32")
    }

    pub(crate) const fn selection(&self) -> AcpSelection {
        self.selection
    }

    pub(crate) const fn composition(&self) -> Option<ProjectedComposition> {
        self.composition
    }

    pub(crate) fn normalized_range(&self, start: i32, end: i32) -> Result<AcpRange, AcpError> {
        let end = if end == -1 { self.end_acp() } else { end };
        let range = AcpRange::new(start, end)?;
        if range.end() > self.end_acp() {
            return Err(AcpError::PositionOutOfRange(range.end()));
        }
        self.require_utf16_boundary(range.start())?;
        self.require_utf16_boundary(range.end())?;
        Ok(range)
    }

    pub(crate) fn require_utf16_boundary(&self, position: i32) -> Result<(), AcpError> {
        if position < 0 || position > self.end_acp() {
            return Err(AcpError::PositionOutOfRange(position));
        }
        let position = position as usize;
        if position > 0
            && position < self.utf16.len()
            && (0xDC00..=0xDFFF).contains(&self.utf16[position])
            && (0xD800..=0xDBFF).contains(&self.utf16[position - 1])
        {
            return Err(AcpError::InsideSurrogatePair(position as i32));
        }
        Ok(())
    }

    /// Converts a projected ACP position to canonical UTF-16. Positions inside
    /// active preedit text deliberately have no canonical source coordinate.
    pub(crate) fn projected_to_canonical(&self, position: i32) -> Result<Option<i32>, AcpError> {
        self.require_utf16_boundary(position)?;
        let Some(composition) = self.composition else {
            return Ok(Some(position));
        };
        if position < composition.range.start() {
            return Ok(Some(position));
        }
        if position > composition.range.end() {
            let canonical_start = self.canonical_utf16_start(composition)?;
            let canonical_end = self.canonical_utf16_end(composition)?;
            let removed = canonical_end - canonical_start;
            let inserted = composition.range.len() as i32;
            return position
                .checked_add(removed - inserted)
                .map(Some)
                .ok_or(AcpError::OffsetOverflow);
        }
        if position == composition.range.start() {
            return Ok(Some(self.canonical_utf16_start(composition)?));
        }
        if position == composition.range.end() {
            return Ok(Some(self.canonical_utf16_end(composition)?));
        }
        Ok(None)
    }

    pub(crate) fn composition_local_range(
        &self,
        range: AcpRange,
    ) -> Result<Option<AcpRange>, AcpError> {
        let Some(composition) = self.composition else {
            return Ok(None);
        };
        if range.start() < composition.range.start() || range.end() > composition.range.end() {
            return Ok(None);
        }
        Ok(Some(AcpRange::new(
            range.start() - composition.range.start(),
            range.end() - composition.range.start(),
        )?))
    }

    fn canonical_utf16_start(&self, composition: ProjectedComposition) -> Result<i32, AcpError> {
        let prefix = self
            .utf16
            .get(..composition.range.start() as usize)
            .ok_or(AcpError::OffsetOverflow)?;
        i32::try_from(prefix.len()).map_err(|_| AcpError::OffsetOverflow)
    }

    fn canonical_utf16_end(&self, composition: ProjectedComposition) -> Result<i32, AcpError> {
        let start = self.canonical_utf16_start(composition)?;
        let removed = self
            .canonical_len
            .checked_sub(
                self.end_acp()
                    .checked_sub(composition.range.len() as i32)
                    .ok_or(AcpError::OffsetOverflow)?,
            )
            .ok_or(AcpError::OffsetOverflow)?;
        start.checked_add(removed).ok_or(AcpError::OffsetOverflow)
    }
}

pub(crate) fn canonical_acp_to_byte(
    snapshot: &TextSnapshot,
    position: i32,
) -> Result<ByteOffset, AcpError> {
    if position < 0 {
        return Err(AcpError::PositionOutOfRange(position));
    }
    snapshot
        .byte_offset_for_utf16(Utf16Offset::new(position as u64))
        .map_err(|error| match error {
            yu_text::TextPositionError::Utf16InsideScalar(_) => {
                AcpError::InsideSurrogatePair(position)
            }
            _ => AcpError::PositionOutOfRange(position),
        })
}

pub(crate) fn canonical_acp_range_to_source(
    snapshot: &TextSnapshot,
    range: AcpRange,
) -> Result<TextRange, AcpError> {
    let start = canonical_acp_to_byte(snapshot, range.start())?;
    let end = canonical_acp_to_byte(snapshot, range.end())?;
    TextRange::new(start, end).ok_or(AcpError::InvalidRange {
        start: range.start(),
        end: range.end(),
    })
}

/// Validates a TSF QueryInsert probe without projecting the hypothetical
/// replacement length into the current document. QueryInsert's result points
/// must stay inside the existing text store; callers use cch only to decide
/// whether they can accept the replacement, not to grow the returned ACP
/// range before the edit actually happens.
pub(crate) fn query_insert_range(
    projection: &AcpProjection,
    start: i32,
    end: i32,
) -> Result<AcpRange, AcpError> {
    projection.normalized_range(start, end)
}

/// Returns the largest prefix no longer than requested that ends on a UTF-16
/// scalar boundary. This keeps chunked ITextStoreACP::GetText reads from
/// returning a dangling high surrogate and advancing pacpNext into the middle
/// of a surrogate pair.
pub(crate) fn utf16_prefix_len(units: &[u16], requested: usize) -> usize {
    let mut len = requested.min(units.len());
    if len > 0
        && len < units.len()
        && (0xD800..=0xDBFF).contains(&units[len - 1])
        && (0xDC00..=0xDFFF).contains(&units[len])
    {
        len -= 1;
    }
    len
}

pub(crate) fn local_selection_utf16(text: &str, range: AcpRange) -> Result<Utf16Range, AcpError> {
    let len = i32::try_from(text.encode_utf16().count()).map_err(|_| AcpError::OffsetOverflow)?;
    if range.end() > len {
        return Err(AcpError::PositionOutOfRange(range.end()));
    }
    Utf16Range::new(
        Utf16Offset::new(range.start() as u64),
        Utf16Offset::new(range.end() as u64),
    )
    .ok_or(AcpError::InvalidRange {
        start: range.start(),
        end: range.end(),
    })
}

pub(crate) fn replace_local_utf16(
    text: &str,
    range: AcpRange,
    replacement: &str,
) -> Result<(String, Utf16Range), AcpError> {
    let source = yu_text::TextBuffer::new(text.to_owned()).snapshot();
    let start = canonical_acp_to_byte(&source, range.start())?;
    let end = canonical_acp_to_byte(&source, range.end())?;
    let start_usize = usize::try_from(start.get()).map_err(|_| AcpError::OffsetOverflow)?;
    let end_usize = usize::try_from(end.get()).map_err(|_| AcpError::OffsetOverflow)?;
    let mut updated = String::with_capacity(
        text.len()
            .saturating_sub(end_usize.saturating_sub(start_usize))
            .saturating_add(replacement.len()),
    );
    updated.push_str(
        text.get(..start_usize)
            .ok_or(AcpError::PositionOutOfRange(range.start()))?,
    );
    updated.push_str(replacement);
    updated.push_str(
        text.get(end_usize..)
            .ok_or(AcpError::PositionOutOfRange(range.end()))?,
    );
    let caret = range
        .start()
        .checked_add(
            i32::try_from(replacement.encode_utf16().count())
                .map_err(|_| AcpError::OffsetOverflow)?,
        )
        .ok_or(AcpError::OffsetOverflow)?;
    Ok((
        updated,
        Utf16Range::new(
            Utf16Offset::new(caret as u64),
            Utf16Offset::new(caret as u64),
        )
        .ok_or(AcpError::InvalidRange {
            start: caret,
            end: caret,
        })?,
    ))
}

pub(crate) fn selection_from_acp(
    snapshot: &TextSnapshot,
    selection: AcpSelection,
) -> Result<EditorSelection, AcpError> {
    let start = canonical_acp_to_byte(snapshot, selection.range.start())?;
    let end = canonical_acp_to_byte(snapshot, selection.range.end())?;
    let (anchor, focus) = if selection.caret_at_start {
        (end, start)
    } else {
        (start, end)
    };
    EditorSelection::range(snapshot, anchor, focus, CaretAffinity::Downstream)
        .map_err(|error| AcpError::Selection(error.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use yu_core::Revision;

    fn snapshot(text: &str) -> TextSnapshot {
        yu_text::TextBuffer::new(text.to_owned()).snapshot()
    }

    #[test]
    fn canonical_ranges_preserve_emoji_boundaries() {
        let source = snapshot("a🙂中");
        let range =
            canonical_acp_range_to_source(&source, AcpRange::new(1, 3).expect("test range"))
                .expect("whole emoji");
        assert_eq!(range.start().get(), 1);
        assert_eq!(range.end().get(), 5);
        assert!(
            canonical_acp_range_to_source(&source, AcpRange::new(1, 2).expect("test range"))
                .is_err()
        );
        assert!(
            canonical_acp_range_to_source(&source, AcpRange::new(1, 5).expect("test range"))
                .is_err()
        );
    }

    #[test]
    fn local_preedit_replacement_keeps_unicode_and_utf16_caret() {
        let range = AcpRange::new(1, 3).expect("test range");
        let selection = local_selection_utf16("a🙂中", range).expect("local selection");
        assert_eq!(selection.start().get(), 1);
        assert_eq!(selection.end().get(), 3);
        assert!(local_selection_utf16("a🙂中", AcpRange::new(1, 5).expect("test range")).is_err());
        let (text, caret) = replace_local_utf16("a🙂中", range, "你好😀").expect("replace emoji");
        assert_eq!(text, "a你好😀中");
        assert_eq!(caret.start().get(), 5);
        assert_eq!(caret.end().get(), 5);
        assert!(
            replace_local_utf16("a🙂中", AcpRange::new(1, 2).expect("test range"), "x").is_err()
        );
    }

    fn selection(snapshot: &TextSnapshot, anchor: u64, focus: u64) -> EditorSelection {
        EditorSelection::range(
            snapshot,
            ByteOffset::new(anchor),
            ByteOffset::new(focus),
            CaretAffinity::Downstream,
        )
        .expect("selection")
    }

    #[test]
    fn canonical_acp_rejects_the_middle_of_a_surrogate_pair() {
        let source = snapshot("a🙂中");
        assert_eq!(
            canonical_acp_to_byte(&source, 0)
                .expect("test fixture")
                .get(),
            0
        );
        assert_eq!(
            canonical_acp_to_byte(&source, 1)
                .expect("test fixture")
                .get(),
            1
        );
        assert!(matches!(
            canonical_acp_to_byte(&source, 2),
            Err(AcpError::InsideSurrogatePair(2))
        ));
        assert_eq!(
            canonical_acp_to_byte(&source, 3)
                .expect("test fixture")
                .get(),
            5
        );
        assert_eq!(
            canonical_acp_to_byte(&source, 4)
                .expect("test fixture")
                .get(),
            8
        );
    }

    #[test]
    fn backward_selection_keeps_the_active_end() {
        let source = snapshot("abc羽");
        let projection =
            AcpProjection::new(&source, selection(&source, 6, 1), None).expect("projection");
        assert_eq!(
            projection.selection().range,
            AcpRange::new(1, 4).expect("test fixture")
        );
        assert!(projection.selection().caret_at_start);

        let restored = selection_from_acp(&source, projection.selection()).expect("restore");
        assert_eq!(restored.anchor().get(), 6);
        assert_eq!(restored.focus().get(), 1);
    }

    #[test]
    fn composition_replaces_canonical_utf16_without_mutating_source() {
        let source = snapshot("ab🙂cd");
        let replacement =
            TextRange::new(ByteOffset::new(2), ByteOffset::new(6)).expect("test fixture");
        let overlay = CompositionOverlay::new(
            Revision::INITIAL,
            replacement,
            "日本語",
            Utf16Range::new(Utf16Offset::new(2), Utf16Offset::new(2)).expect("test fixture"),
        )
        .expect("test fixture");
        let projection = AcpProjection::new(&source, selection(&source, 2, 6), Some(&overlay))
            .expect("test fixture");

        assert_eq!(
            String::from_utf16(projection.text()).expect("test fixture"),
            "ab日本語cd"
        );
        let composition = projection.composition().expect("test fixture");
        assert_eq!(
            composition.range,
            AcpRange::new(2, 5).expect("test fixture")
        );
        assert_eq!(
            composition.selection,
            AcpRange::new(4, 4).expect("test fixture")
        );
        assert_eq!(
            projection.projected_to_canonical(2).expect("test fixture"),
            Some(2)
        );
        assert_eq!(
            projection.projected_to_canonical(3).expect("test fixture"),
            None
        );
        assert_eq!(
            projection.projected_to_canonical(5).expect("test fixture"),
            Some(4)
        );
        assert_eq!(
            projection.projected_to_canonical(6).expect("test fixture"),
            Some(5)
        );
        assert_eq!(source.as_str(), "ab🙂cd");
    }

    #[test]
    fn projected_ranges_only_become_local_when_fully_inside_preedit() {
        let source = snapshot("hello");
        let replacement = TextRange::empty(ByteOffset::new(2));
        let overlay = CompositionOverlay::new(
            Revision::INITIAL,
            replacement,
            "日本",
            Utf16Range::new(Utf16Offset::new(2), Utf16Offset::new(2)).expect("test fixture"),
        )
        .expect("test fixture");
        let projection = AcpProjection::new(&source, selection(&source, 2, 2), Some(&overlay))
            .expect("test fixture");
        assert_eq!(
            projection
                .composition_local_range(AcpRange::new(2, 4).expect("test fixture"))
                .expect("test fixture"),
            Some(AcpRange::new(0, 2).expect("test fixture"))
        );
        assert_eq!(
            projection
                .composition_local_range(AcpRange::new(1, 3).expect("test fixture"))
                .expect("test fixture"),
            None
        );
    }

    #[test]
    fn query_insert_never_returns_positions_outside_the_current_stream() {
        let source = snapshot("ab🙂");
        let projection =
            AcpProjection::new(&source, selection(&source, 0, 0), None).expect("projection");

        let range = query_insert_range(&projection, 1, 4).expect("valid existing range");
        assert_eq!(range, AcpRange::new(1, 4).expect("fixture"));
        assert!(query_insert_range(&projection, 1, 5).is_err());
    }

    #[test]
    fn chunked_utf16_reads_do_not_split_surrogate_pairs() {
        let units: Vec<u16> = "a🙂b".encode_utf16().collect();
        assert_eq!(utf16_prefix_len(&units, 1), 1);
        assert_eq!(utf16_prefix_len(&units, 2), 1);
        assert_eq!(utf16_prefix_len(&units, 3), 3);
        assert_eq!(utf16_prefix_len(&units, 99), units.len());
    }
}
