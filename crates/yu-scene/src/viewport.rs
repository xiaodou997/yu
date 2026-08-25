use std::ops::Range;

use yu_core::{Revision, TextRange};

use crate::SceneError;

/// Owned geometry for one block selected by a revision-bound viewport query.
///
/// The `kind` value is intentionally opaque to `yu-scene`: syntax/parser
/// knowledge stays outside the retained scene crate while native bridges can
/// carry a stable tag through to a future block renderer.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ViewportBlockGeometry {
    revision: Revision,
    index: usize,
    source: TextRange,
    y: f32,
    height: f32,
    measured: bool,
    kind: u8,
}

impl ViewportBlockGeometry {
    pub fn new(
        revision: Revision,
        index: usize,
        source: TextRange,
        y: f32,
        height: f32,
        measured: bool,
        kind: u8,
    ) -> Result<Self, SceneError> {
        if !y.is_finite() || y < 0.0 {
            return Err(SceneError::InvalidViewportInput(
                "viewport block y must be finite and non-negative",
            ));
        }
        if !height.is_finite() || height <= 0.0 {
            return Err(SceneError::InvalidViewportInput(
                "viewport block height must be finite and positive",
            ));
        }
        let bottom = y + height;
        if !bottom.is_finite() {
            return Err(SceneError::InvalidViewportInput(
                "viewport block bounds must be finite",
            ));
        }
        Ok(Self {
            revision,
            index,
            source,
            y,
            height,
            measured,
            kind,
        })
    }

    #[must_use]
    pub const fn revision(self) -> Revision {
        self.revision
    }

    #[must_use]
    pub const fn index(self) -> usize {
        self.index
    }

    #[must_use]
    pub const fn source(self) -> TextRange {
        self.source
    }

    #[must_use]
    pub const fn y(self) -> f32 {
        self.y
    }

    #[must_use]
    pub const fn height(self) -> f32 {
        self.height
    }

    #[must_use]
    pub const fn measured(self) -> bool {
        self.measured
    }

    #[must_use]
    pub const fn kind(self) -> u8 {
        self.kind
    }

    #[must_use]
    pub fn bottom(self) -> f32 {
        self.y + self.height
    }
}

/// A validated, owned viewport metadata snapshot suitable for scene work.
///
/// It contains no source text, parser nodes, layout caches or native objects.
/// Blocks are ordered, contiguous in block index, and all belong to the same
/// source revision. The snapshot can therefore be sent to a scene/layout
/// worker and discarded as one unit when its revision becomes stale.
#[derive(Clone, Debug, PartialEq)]
pub struct ViewportSceneInput {
    revision: Revision,
    block_range: Range<usize>,
    content_height: f32,
    blocks: Vec<ViewportBlockGeometry>,
}

impl ViewportSceneInput {
    pub fn new(
        revision: Revision,
        block_range: Range<usize>,
        content_height: f32,
        blocks: Vec<ViewportBlockGeometry>,
    ) -> Result<Self, SceneError> {
        if block_range.start > block_range.end {
            return Err(SceneError::InvalidViewportInput(
                "viewport block range must be ordered",
            ));
        }
        if !content_height.is_finite() || content_height < 0.0 {
            return Err(SceneError::InvalidViewportInput(
                "viewport content height must be finite and non-negative",
            ));
        }
        if blocks.len() != block_range.end - block_range.start {
            return Err(SceneError::InvalidViewportInput(
                "viewport block count must match its range",
            ));
        }

        let mut previous_source: Option<TextRange> = None;
        let mut previous_y: Option<f32> = None;
        for (offset, block) in blocks.iter().copied().enumerate() {
            if block.revision() != revision {
                return Err(SceneError::ViewportRevisionMismatch {
                    expected: revision,
                    actual: block.revision(),
                });
            }
            let expected_index = block_range.start + offset;
            if block.index() != expected_index {
                return Err(SceneError::InvalidViewportInput(
                    "viewport block indices must be contiguous",
                ));
            }
            if let Some(y) = previous_y
                && block.y() < y
            {
                return Err(SceneError::InvalidViewportInput(
                    "viewport block origins must be monotonic",
                ));
            }
            if block.bottom() > content_height + 0.001 {
                return Err(SceneError::InvalidViewportInput(
                    "viewport block exceeds content height",
                ));
            }
            if let Some(source) = previous_source
                && source.end() > block.source().start()
            {
                return Err(SceneError::InvalidViewportInput(
                    "viewport block source ranges must be ordered",
                ));
            }
            previous_y = Some(block.y());
            previous_source = Some(block.source());
        }

        Ok(Self {
            revision,
            block_range,
            content_height,
            blocks,
        })
    }

    #[must_use]
    pub const fn revision(&self) -> Revision {
        self.revision
    }

    #[must_use]
    pub fn block_range(&self) -> Range<usize> {
        self.block_range.clone()
    }

    #[must_use]
    pub const fn content_height(&self) -> f32 {
        self.content_height
    }

    #[must_use]
    pub fn blocks(&self) -> &[ViewportBlockGeometry] {
        &self.blocks
    }

    #[must_use]
    pub fn block(&self, index: usize) -> Option<ViewportBlockGeometry> {
        self.blocks
            .get(index.checked_sub(self.block_range.start)?)
            .copied()
            .filter(|block| block.index() == index)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use yu_core::{ByteOffset, TextRange};

    fn source(start: u64, end: u64) -> TextRange {
        TextRange::new(ByteOffset::new(start), ByteOffset::new(end)).expect("range")
    }

    #[test]
    fn viewport_scene_input_validates_revision_order_and_content_bounds() {
        let revision = Revision::new(4);
        let first = ViewportBlockGeometry::new(revision, 2, source(0, 3), 4.0, 2.0, true, 3)
            .expect("first block");
        let second = ViewportBlockGeometry::new(revision, 3, source(4, 8), 6.0, 2.0, false, 2)
            .expect("second block");
        let input =
            ViewportSceneInput::new(revision, 2..4, 8.0, vec![first, second]).expect("input");
        assert_eq!(input.revision(), revision);
        assert_eq!(input.block_range(), 2..4);
        assert_eq!(input.content_height(), 8.0);
        assert_eq!(input.block(3), Some(second));
    }

    #[test]
    fn viewport_scene_input_rejects_stale_or_partial_blocks() {
        let revision = Revision::new(4);
        let stale =
            ViewportBlockGeometry::new(Revision::new(3), 0, source(0, 1), 0.0, 1.0, false, 0)
                .expect("stale metadata is locally valid");
        assert!(matches!(
            ViewportSceneInput::new(revision, 0..1, 1.0, vec![stale]),
            Err(SceneError::ViewportRevisionMismatch { .. })
        ));

        let block = ViewportBlockGeometry::new(revision, 0, source(0, 1), 0.0, 2.0, true, 0)
            .expect("block");
        assert_eq!(
            ViewportSceneInput::new(revision, 0..1, 1.0, vec![block]),
            Err(SceneError::InvalidViewportInput(
                "viewport block exceeds content height"
            ))
        );
    }
}
