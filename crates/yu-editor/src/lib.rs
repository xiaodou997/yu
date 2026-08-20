#![forbid(unsafe_code)]

//! Platform-independent editor state.

mod accessibility;
mod caret;
mod command;
mod composition;
mod document;
mod history;
mod keymap;
mod layout;
mod list;
mod projection;
mod selection;
mod viewport;

pub use accessibility::{
    ACCESSIBILITY_SEMANTIC_FLAG_ORDERED, ACCESSIBILITY_SEMANTIC_FLAG_TASK_DONE,
    AccessibilitySemanticKind, AccessibilitySemanticNode, AccessibilitySemanticSnapshot,
    AccessibilityTextError, AccessibilityTextPosition, AccessibilityTextRange,
    AccessibilityTextSnapshot,
};
pub use caret::{
    CaretAffinity, CaretPositionError, CaretPositionMap, NativeCaretPosition, SourceCaretPosition,
};
pub use command::{CommandResult, EditorCommand, KeyRouteResult, SourceChange, SourceSync};
pub use composition::{CompositionError, CompositionOverlay};
pub use document::{EditorDocument, EditorDocumentError};
pub use history::HistoryStats;
pub use keymap::{EditorKey, KeyEvent, KeyModifiers, command_for_key};
pub use layout::{LayoutBackend, LayoutCache, LayoutCacheStats};
pub use projection::{ProjectionCache, ProjectionCacheStats};
pub use selection::{EditorSelection, SelectionError};
pub use viewport::{
    CaretScrollRequest, ViewportBlock, ViewportCaret, ViewportConfig, ViewportError,
    ViewportLayout, ViewportRange, ViewportRect, ViewportSnapshot, ViewportStats,
};
pub use yu_layout::{
    BlockQuoteLayout, ClusterMetrics, HeightIndex, HeightIndexError, ImageIntrinsicSize,
    LayoutCaret, LayoutConfig, LayoutError, LayoutHit, LayoutPoint, LayoutSnapshot,
    MonospaceMetrics, ShapedText, ShapingProvider, TableCellLayout, TableLayoutHit,
    TableLayoutSnapshot, TableResizeCommit, TableResizeGesture, TableResizeGestureError,
    TableResizeHit, TableResizeTarget, VisualCluster, VisualLine,
};
pub use yu_markdown::{
    Block, BlockKind, ListMarker, MarkdownDocument, TableAlignment, TaskMarker, TaskState,
    list_marker, task_marker,
};
pub use yu_projection::{
    BlockProjection, BlockProjectionKind, BlockQuotePresentation, CodeProjection, ImageSource,
    LeadingMarker, Projection, ProjectionBias, ProjectionError, TableProjection, VisualOffset,
    VisualRange, VisualRun, VisualRunKind, VisualRunStyle,
};
