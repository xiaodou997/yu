#![forbid(unsafe_code)]

//! Platform-independent editor state.

mod accessibility;
mod caret;
mod command;
mod composition;
mod document;
mod history;
mod layout;
mod list;
mod projection;
mod selection;
mod viewport;

pub use accessibility::{
    AccessibilityTextError, AccessibilityTextPosition, AccessibilityTextRange,
    AccessibilityTextSnapshot,
};
pub use caret::{
    CaretAffinity, CaretPositionError, CaretPositionMap, NativeCaretPosition, SourceCaretPosition,
};
pub use command::{CommandResult, EditorCommand};
pub use composition::{CompositionError, CompositionOverlay};
pub use document::{EditorDocument, EditorDocumentError};
pub use history::HistoryStats;
pub use layout::{LayoutBackend, LayoutCache, LayoutCacheStats};
pub use projection::{ProjectionCache, ProjectionCacheStats};
pub use selection::{EditorSelection, SelectionError};
pub use viewport::{
    ViewportBlock, ViewportConfig, ViewportError, ViewportLayout, ViewportRange, ViewportRect,
    ViewportSnapshot, ViewportStats,
};
pub use yu_layout::{
    ClusterMetrics, HeightIndex, HeightIndexError, LayoutCaret, LayoutConfig, LayoutError,
    LayoutHit, LayoutPoint, LayoutSnapshot, MonospaceMetrics, ShapedText, ShapingProvider,
    VisualCluster, VisualLine,
};
pub use yu_markdown::{Block, BlockKind, MarkdownDocument};
pub use yu_projection::{
    BlockProjection, BlockProjectionKind, CodeProjection, Projection, ProjectionBias,
    ProjectionError, VisualOffset, VisualRange, VisualRun, VisualRunKind, VisualRunStyle,
};
