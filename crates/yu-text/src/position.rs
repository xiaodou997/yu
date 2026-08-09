use std::error::Error;
use std::fmt;

use yu_core::{ByteOffset, LineIndex, Utf16Offset};

/// A source-coordinate lookup failed against one immutable snapshot.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TextPositionError {
    ByteOutOfBounds {
        offset: ByteOffset,
        len: ByteOffset,
    },
    NotUtf8Boundary(ByteOffset),
    Utf16OutOfBounds {
        offset: Utf16Offset,
        len: Utf16Offset,
    },
    Utf16InsideScalar(Utf16Offset),
    LineOutOfBounds {
        line: LineIndex,
        line_count: u64,
    },
}

impl fmt::Display for TextPositionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ByteOutOfBounds { offset, len } => write!(
                formatter,
                "byte offset {} exceeds snapshot length {}",
                offset.get(),
                len.get()
            ),
            Self::NotUtf8Boundary(offset) => {
                write!(
                    formatter,
                    "byte offset {} splits a UTF-8 scalar",
                    offset.get()
                )
            }
            Self::Utf16OutOfBounds { offset, len } => write!(
                formatter,
                "UTF-16 offset {} exceeds snapshot length {}",
                offset.get(),
                len.get()
            ),
            Self::Utf16InsideScalar(offset) => write!(
                formatter,
                "UTF-16 offset {} splits a surrogate pair",
                offset.get()
            ),
            Self::LineOutOfBounds { line, line_count } => write!(
                formatter,
                "line index {} exceeds snapshot line count {}",
                line.get(),
                line_count
            ),
        }
    }
}

impl Error for TextPositionError {}
