#![forbid(unsafe_code)]

//! Platform-independent document: source, parsing, projection, layout, viewport.
//!
//! 编辑状态本身（history / selection / caret 绑定 / composition）住在
//! `yu-state`，这里再导出它们。剩下的 `EditorDocument` 仍然又是状态又是
//! 布局入口——把两者分开要等 S5 把 projection/layout/viewport 挪走，
//! 理由见 `docs/architecture/overview-v2.md` 第 8 节 S4。

mod accessibility;
mod blockinput;
mod blockview;
mod command;
mod decorations;
mod document;
mod geometry;
mod image;
mod keymap;
mod layout;
mod list;
mod marks;
mod table;
mod viewport;
mod visual;

pub use accessibility::{
    ACCESSIBILITY_SEMANTIC_FLAG_ORDERED, ACCESSIBILITY_SEMANTIC_FLAG_TASK_DONE,
    AccessibilitySemanticKind, AccessibilitySemanticNode, AccessibilitySemanticSnapshot,
    AccessibilityTextError, AccessibilityTextPosition, AccessibilityTextRange,
    AccessibilityTextSnapshot,
};
pub use blockinput::{
    BlockLayoutInput, BlockLineStyleTable, BlockOrnaments, BlockQuoteOrnament, BlockStyleTable,
    HeadingOrnament, MarkerOrnament, style_id,
};
pub use blockview::{BlockCaret, BlockCluster, BlockGlyph, BlockHit, BlockLine, BlockView};
pub use command::{CommandResult, EditorCommand, KeyRouteResult, SourceChange, SourceSync};
pub use decorations::{DecorationCache, DecorationCacheStats, DecorationError};
pub use document::{EditorDocument, EditorDocumentError};
pub use image::ImagePlacement;
pub use keymap::{EditorKey, KeyEvent, KeyModifiers, command_for_key};
pub use layout::{LayoutBackend, LayoutCache, LayoutCacheStats};
pub use table::{
    TableCellLayout, TableLayout, TableLayoutHit, TableResizeCommit, TableResizeGesture,
    TableResizeGestureError, TableResizeHit, TableResizeTarget,
};
pub use viewport::{
    CaretScrollRequest, ViewportBlock, ViewportCaret, ViewportConfig, ViewportError,
    ViewportLayout, ViewportRange, ViewportSnapshot, ViewportSpan, ViewportStats,
};
pub use visual::{VisualText, VisualTextError};
pub use yu_core::{
    CaretAffinity, ClusterMetrics, NativeCaretPosition, ShapedText, ShapingProvider,
    SourceCaretPosition, TextStyle,
};
// 编辑状态住在 yu-state（S4）。这里再导出，好让平台层与 FFI 的路径不变。
pub use yu_core::{VisualOffset, VisualRange};
pub use yu_decoration::{Bias, Decoration, DecorationRange, DecorationSet, StyleId};
pub use yu_layout::{
    BlockLayout, GlyphBox, HeightIndex, HeightIndexError, ImageIntrinsicSize, LayoutConfig,
    LayoutError, LayoutPoint, LayoutRect, MonospaceMetrics,
};
pub use yu_markdown::{
    Block, BlockAnnotation, BlockDecorations, BlockKind, BlockOrnament, ImageSpan, ListMarker,
    MarkdownDocument, TableAlignment, TableBlock, TaskMarker, TaskState, list_marker, task_marker,
};
pub use yu_state::{
    CaretPositionError, CaretPositionMap, CompositionError, CompositionOverlay, EditorSelection,
    HistoryStats, SelectionError,
};
